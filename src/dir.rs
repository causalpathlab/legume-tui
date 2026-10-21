//! Where sessions live: one socket per session in a directory only its
//! owner can enter, and the listing of the ones still alive.
//!
//! A session's id is its socket's name (`tool-view-1`), fixed for life.
//! The name the user gives it, its label, sits beside the socket in a file
//! of its own; it is unique among the tool's live sessions, and goes with
//! the socket.

use crate::wire;
use std::collections::HashMap;
use std::io;
use std::os::unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// How long a host has to answer a question about itself.
const ASK: Duration = Duration::from_secs(2);

/// The longest session name; a socket path has a short length limit.
const NAME_MAX: usize = 64;

/// The per-user directory of session sockets, made owner-only if new and
/// refused if anyone else could reach it: `$LEGUME_TUI_DIR`, else
/// `legume-tui` under `$XDG_RUNTIME_DIR`, else `legume-tui-<uid>` in the
/// temporary directory.
pub fn runtime_dir() -> io::Result<PathBuf> {
    let uid = rustix::process::getuid().as_raw();
    let dir = match std::env::var_os(crate::DIR_VAR).filter(|v| !v.is_empty()) {
        Some(d) => PathBuf::from(d),
        None => match std::env::var_os("XDG_RUNTIME_DIR").filter(|v| !v.is_empty()) {
            Some(x) if Path::new(&x).is_dir() => Path::new(&x).join("legume-tui"),
            _ => std::env::temp_dir().join(format!("legume-tui-{uid}")),
        },
    };
    private_dir(&dir, uid)?;
    Ok(dir)
}

/// Make `dir` with mode 0700, or check that the one there is a real
/// directory of `uid`'s that nobody else can enter.
pub(crate) fn private_dir(dir: &Path, uid: u32) -> io::Result<()> {
    match std::fs::DirBuilder::new().mode(0o700).create(dir) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
        Err(e) => return Err(e),
    }
    let meta = std::fs::symlink_metadata(dir)?;
    let refuse = |why: &str| {
        Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            format!("{}: {why}", dir.display()),
        ))
    };
    if !meta.is_dir() {
        return refuse("not a directory");
    }
    if meta.uid() != uid {
        return refuse("owned by another user");
    }
    if meta.permissions().mode() & 0o077 != 0 {
        return refuse("open to other users (expected mode 0700)");
    }
    Ok(())
}

/// The socket of session `name` in `dir`.
pub(crate) fn socket(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.sock"))
}

/// The one-line status of session `name` in `dir` (see
/// [`crate::Session::status`]).
pub(crate) fn status_file(dir: &Path, name: &str) -> PathBuf {
    dir.join(format!("{name}.status"))
}

/// What of the attached terminal's environment session `id` in `dir`
/// was passed (see [`crate::Session::terminal_env`]).
pub(crate) fn terminal_env_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.env"))
}

/// The label of session `id` in `dir` (see [`set_label`]).
pub(crate) fn label_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.label"))
}

/// What the host of session `id` in `dir` writes to its stderr.
pub(crate) fn log_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.log"))
}

/// What the program of session `id` in `dir` leaves when it ends (see
/// [`crate::Session::leave`]): kept after its socket goes, until shown.
pub(crate) fn leave_file(dir: &Path, id: &str) -> PathBuf {
    dir.join(format!("{id}.leave"))
}

/// The session a socket's path is for: its id, if it is one.
fn id_of(path: &Path) -> Option<&str> {
    path.file_name()?.to_str()?.strip_suffix(".sock")
}

/// Hold `dir` locked until the returned file is dropped, so names are
/// claimed, labels given, and stale sockets replaced, one at a time.
fn lock(dir: &Path) -> io::Result<std::fs::File> {
    let f = std::fs::File::open(dir)?;
    rustix::fs::flock(&f, rustix::fs::FlockOperation::LockExclusive)?;
    Ok(f)
}

/// Bind `path`, replacing a stale socket there; refuse a live one.
pub(crate) fn bind(path: &Path) -> io::Result<UnixListener> {
    let _lock = lock(path.parent().unwrap_or(Path::new(".")))?;
    bind_locked(path)
}

/// [`bind`], with the directory already locked.
fn bind_locked(path: &Path) -> io::Result<UnixListener> {
    if path.exists() {
        if alive(path) {
            return Err(io::Error::new(
                io::ErrorKind::AddrInUse,
                format!("a session already listens on {}", path.display()),
            ));
        }
        std::fs::remove_file(path)?;
    }
    let listener = UnixListener::bind(path)?;
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    Ok(listener)
}

/// Claim a session in `dir` for the tool whose sessions start with
/// `prefix`: with a name, the id `base-name` and the name as its label;
/// else the first `base-N` (N from 1) no live session has, as its id or
/// label, unlabelled. Sessions in `known` are taken as live without asking
/// (the tool's sessions as [`live`] just listed them); others are asked.
/// Its socket is bound here, by the launching process, so two launches at
/// once cannot share a name and the launcher knows the socket it reaches
/// is its own host's. What an ended session of the id left
/// ([`take_leave`]) is taken under the same lock, for the launch to say:
/// a session that ended after the launch took what the ended ones left
/// is not silenced by the new one taking its id. With `skip_left`, an
/// unnamed claim passes over the numbers whose ended sessions left words
/// nobody has seen yet, so a claim that cannot say them keeps them.
pub(crate) fn claim(
    dir: &Path,
    prefix: &str,
    base: &str,
    name: Option<&str>,
    known: &[Listed],
    skip_left: bool,
) -> io::Result<Claimed> {
    let _lock = lock(dir)?;
    let mut taken = Taken::read(dir, prefix, None, known)?;
    // Another live session that answers to id `id`, in full or short.
    let mut held = |id: &str| {
        taken
            .holder(id)
            .or_else(|| taken.holder(id.strip_prefix(prefix).unwrap_or(id)))
    };
    if let Some(n) = name {
        let id = format!("{base}-{n}");
        check_name(&id)?;
        if let Some(other) = held(&id) {
            return Err(taken_by(&id, &other));
        }
        let path = socket(dir, &id);
        let listener = bind_locked(&path)?;
        if let Err(e) = label_locked(dir, &mut taken, &id, n) {
            let _ = std::fs::remove_file(&path);
            return Err(e);
        }
        let left = take_leave(dir, &id);
        return Ok(Claimed {
            name: id,
            listener,
            left,
        });
    }
    for n in 1.. {
        let name = format!("{base}-{n}");
        check_name(&name)?;
        if held(&name).is_some() || (skip_left && leave_file(dir, &name).exists()) {
            continue;
        }
        match bind_locked(&socket(dir, &name)) {
            Ok(listener) => {
                // What an earlier session of the number left behind.
                let _ = std::fs::remove_file(label_file(dir, &name));
                let left = take_leave(dir, &name);
                return Ok(Claimed {
                    name,
                    listener,
                    left,
                });
            }
            Err(e) if e.kind() == io::ErrorKind::AddrInUse => continue,
            Err(e) => return Err(e),
        }
    }
    unreachable!("some number is free")
}

/// A session [`claim`] claimed.
#[derive(Debug)]
pub(crate) struct Claimed {
    pub name: String,
    pub listener: UnixListener,
    /// What the ended session of the id left, taken.
    pub left: Option<String>,
}

/// Refuse a name that would not make a plain file name.
pub(crate) fn check_name(name: &str) -> io::Result<()> {
    let fine = !name.is_empty()
        && name.len() <= NAME_MAX
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    if fine {
        Ok(())
    } else {
        Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("session name {name:?}: use letters, digits, '-', '_' or '.'"),
        ))
    }
}

/// A live session, as `sessions` and the resume picker list it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Listed {
    /// The session's id, its socket's name.
    pub name: String,
    /// The name the user gave it, if any.
    pub label: Option<String>,
    /// The program's process id.
    pub pid: u32,
    /// Whether a client is attached now (to another terminal, as seen
    /// from here).
    pub attached: bool,
    /// When the session started, in seconds since the epoch.
    pub started: u64,
    /// The program's command line.
    pub command: String,
    /// The program's last [`crate::Session::status`], if it set one.
    pub status: Option<String>,
}

impl Listed {
    /// What a user types to name it: its label, else its id.
    pub fn handle(&self) -> &str {
        self.label.as_deref().unwrap_or(&self.name)
    }

    /// As listings show it: `label (id)`, or the id alone.
    pub fn shown(&self) -> String {
        match &self.label {
            Some(l) => format!("{l} ({})", self.name),
            None => self.name.clone(),
        }
    }

    /// Its label as said after its id (` (labelled work)`), or nothing.
    pub fn labelled(&self) -> String {
        self.label
            .as_ref()
            .map_or_else(String::new, |l| format!(" (labelled {l})"))
    }

    /// `attached` or `detached`.
    pub fn state(&self) -> &'static str {
        if self.attached {
            "attached"
        } else {
            "detached"
        }
    }

    /// How long ago the session started, e.g. `5m ago`.
    pub fn age(&self) -> String {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let s = now.saturating_sub(self.started);
        match s {
            0..60 => format!("{s}s ago"),
            60..3600 => format!("{}m ago", s / 60),
            3600..86400 => format!("{}h ago", s / 3600),
            _ => format!("{}d ago", s / 86400),
        }
    }
}

/// What a host said when asked about itself.
pub(crate) enum Asked {
    Answered(Listed),
    /// It took the question but did not answer in time: busy, spared.
    Busy,
    /// It took the question and let it go: it is ending, spared (it takes
    /// its own files away).
    Closing,
    /// Asking failed otherwise (a connection refused for another reason
    /// than nothing listening, an answer that is not one): why. Spared.
    Failed(io::Error),
    /// Nothing listens: the socket is stale.
    Stale,
}

/// Whether connecting failed as `e` because nothing listens on the
/// socket: no host behind it.
pub(crate) fn nothing_listens(e: &io::Error) -> bool {
    matches!(
        e.kind(),
        io::ErrorKind::ConnectionRefused | io::ErrorKind::NotFound
    )
}

/// Ask the host behind `path` about itself. The status is not filled in.
pub(crate) fn asked(path: &Path) -> Asked {
    let gone = |e: &io::Error| {
        matches!(
            e.kind(),
            io::ErrorKind::BrokenPipe
                | io::ErrorKind::ConnectionReset
                | io::ErrorKind::NotConnected
                | io::ErrorKind::UnexpectedEof
        )
    };
    let mut s = match wire::connect(path) {
        Ok(s) => s,
        Err(e) if nothing_listens(&e) => return Asked::Stale,
        Err(e) if gone(&e) => return Asked::Closing,
        Err(e) => return Asked::Failed(e),
    };
    let reply = wire::set_timeout(&s, true, Some(ASK))
        .and_then(|()| wire::set_timeout(&s, false, Some(ASK)))
        .and_then(|()| wire::write_frame(&s, wire::INFO, &[]))
        .and_then(|()| wire::read_frame(&mut s));
    let body = match reply {
        Ok(Some((wire::INFO_REPLY, b))) if b.len() >= 13 => b,
        // Let go before it answered.
        Ok(None) => return Asked::Closing,
        Ok(Some(_)) => {
            return Asked::Failed(io::Error::new(
                io::ErrorKind::InvalidData,
                "the host's answer is not one",
            ))
        }
        Err(e)
            if matches!(
                e.kind(),
                io::ErrorKind::WouldBlock | io::ErrorKind::TimedOut
            ) =>
        {
            return Asked::Busy
        }
        Err(e) if gone(&e) => return Asked::Closing,
        Err(e) => return Asked::Failed(e),
    };
    Asked::Answered(Listed {
        name: path
            .file_stem()
            .map_or_else(String::new, |n| n.to_string_lossy().into_owned()),
        label: None,
        pid: u32::from_be_bytes([body[0], body[1], body[2], body[3]]),
        attached: body[4] == 1,
        started: u64::from_be_bytes(body[5..13].try_into().unwrap_or_default()),
        command: String::from_utf8_lossy(&body[13..]).into_owned(),
        status: None,
    })
}

/// Ask the host behind `path` about itself. None when nothing answers.
/// The status is not filled in.
pub(crate) fn ask(path: &Path) -> Option<Listed> {
    match asked(path) {
        Asked::Answered(listed) => Some(listed),
        _ => None,
    }
}

/// The first line of the file at `path`, trimmed, if there is one.
fn read_line(path: &Path) -> Option<String> {
    let s = std::fs::read_to_string(path).ok()?;
    let s = s.trim();
    (!s.is_empty()).then(|| s.to_string())
}

/// Replace `<name>.<ext>` in `dir` with `text` at once: written beside it,
/// then renamed over it. The file written beside it is this call's own,
/// so two threads of a process replacing the same file do not share it.
fn replace_file(dir: &Path, name: &str, ext: &str, text: &str) -> io::Result<()> {
    static CALLS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let call = CALLS.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let tmp = dir.join(format!(".{name}.{ext}.{}.{call}", std::process::id()));
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, dir.join(format!("{name}.{ext}")))
}

/// The status line of session `name` in `dir`, if there is one.
fn read_status(dir: &Path, name: &str) -> Option<String> {
    read_line(&status_file(dir, name))
}

/// Replace the status line of session `name` in `dir` at once. Kept to
/// one short line.
pub(crate) fn write_status(dir: &Path, name: &str, text: &str) -> io::Result<()> {
    const LONGEST: usize = 200;
    let line: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(LONGEST)
        .collect();
    replace_file(dir, name, "status", line.trim())
}

/// Replace what session `id` in `dir` has of the attached terminal's
/// environment at once.
pub(crate) fn write_terminal_env(
    dir: &Path,
    id: &str,
    vars: &[(String, String)],
) -> io::Result<()> {
    let text = String::from_utf8_lossy(&crate::wire::encode_env(vars)).into_owned();
    replace_file(dir, id, "env", &text)
}

/// The longest text a program leaves ([`write_leave`]).
const LEAVE_MAX: usize = 4096;

/// Replace what session `id` in `dir` leaves when it ends at once: lines,
/// their other control characters made spaces, cut at [`LEAVE_MAX`].
pub(crate) fn write_leave(dir: &Path, id: &str, text: &str) -> io::Result<()> {
    let text: String = text
        .trim_end()
        .chars()
        .map(|c| if c.is_control() && c != '\n' { ' ' } else { c })
        .take(LEAVE_MAX)
        .collect();
    replace_file(dir, id, "leave", &text)
}

/// What session `id` in `dir` left, taken away: shown once.
pub(crate) fn take_leave(dir: &Path, id: &str) -> Option<String> {
    let path = leave_file(dir, id);
    let text = std::fs::read_to_string(&path).ok()?;
    std::fs::remove_file(&path).ok()?;
    Some(text)
}

/// What the ended sessions in `dir` whose ids start with `prefix` left,
/// in id order, taken away: each id and its text. A live session's is
/// left for when it ends.
pub(crate) fn take_left(dir: &Path, prefix: &str) -> Vec<(String, String)> {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut ended: Vec<String> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let id = name.strip_suffix(".leave")?;
            (id.starts_with(prefix) && check_name(id).is_ok()).then(|| id.to_string())
        })
        .filter(|id| !alive(&socket(dir, id)))
        .collect();
    ended.sort_by(|a, b| natural(a, b));
    ended
        .into_iter()
        .filter_map(|id| {
            let text = take_leave(dir, &id)?;
            Some((id, text))
        })
        .collect()
}

/// What `id` left, as a user reads it: `id ended: text`.
pub(crate) fn ended(id: &str, text: &str) -> String {
    format!("{id} ended: {text}")
}

/// What session `id` in `dir` has of the attached terminal's
/// environment; None before any attach.
pub(crate) fn read_terminal_env(dir: &Path, id: &str) -> Option<Vec<(String, String)>> {
    let bytes = std::fs::read(terminal_env_file(dir, id)).ok()?;
    Some(crate::wire::decode_env(&bytes))
}

/// The label of session `id` in `dir`, if it has one.
pub(crate) fn read_label(dir: &Path, id: &str) -> Option<String> {
    read_line(&label_file(dir, id))
}

/// Label live session `id` of the tool whose sessions start with
/// `prefix`. The label is checked as a name, and refused if another live
/// session of the tool has it as its label or its id (in full or after
/// `prefix`). Written beside the old one, then renamed over it.
pub(crate) fn set_label(dir: &Path, prefix: &str, id: &str, label: &str) -> io::Result<()> {
    let _lock = lock(dir)?;
    // Ended while the user was typing: no file for a session that is gone.
    if !alive(&socket(dir, id)) {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("{id} has ended"),
        ));
    }
    let mut taken = Taken::read(dir, prefix, Some(id), &[])?;
    label_locked(dir, &mut taken, id, label)
}

/// [`set_label`], with the directory already locked and read.
fn label_locked(dir: &Path, taken: &mut Taken, id: &str, label: &str) -> io::Result<()> {
    check_name(label)?;
    if let Some(other) = taken.holder(label) {
        return Err(taken_by(label, &other));
    }
    replace_file(dir, id, "label", label)
}

/// What the tool's sessions in a directory answer to (their ids, in full
/// and after the tool's prefix, and their labels), read once with the
/// directory locked. Whether one is live is asked only when its name
/// clashes, and once.
struct Taken<'a> {
    dir: &'a Path,
    /// Each name, and the sessions that answer to it.
    names: HashMap<String, Vec<String>>,
    /// Live as listed just now: not asked again.
    known: &'a [Listed],
    /// Whether each session asked about was live.
    asked: HashMap<String, bool>,
}

impl<'a> Taken<'a> {
    /// The sessions in `dir` whose ids start with `prefix`, but `me`.
    fn read(
        dir: &'a Path,
        prefix: &str,
        me: Option<&str>,
        known: &'a [Listed],
    ) -> io::Result<Self> {
        let mut names: HashMap<String, Vec<String>> = HashMap::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            let Some(id) = id_of(&path).filter(|o| Some(*o) != me && o.starts_with(prefix)) else {
                continue;
            };
            let short = id.strip_prefix(prefix).map(str::to_string);
            for name in [Some(id.to_string()), short, read_label(dir, id)]
                .into_iter()
                .flatten()
            {
                names.entry(name).or_default().push(id.to_string());
            }
        }
        Ok(Self {
            dir,
            names,
            known,
            asked: HashMap::new(),
        })
    }

    /// The live session that answers to `name`.
    fn holder(&mut self, name: &str) -> Option<String> {
        let Self {
            dir,
            names,
            known,
            asked,
        } = self;
        names.get(name)?.iter().find_map(|id| {
            let live = known.iter().any(|l| &l.name == id)
                || *asked
                    .entry(id.clone())
                    .or_insert_with(|| alive(&socket(dir, id)));
            live.then(|| id.clone())
        })
    }
}

/// `name` refused, as one `other` answers to.
fn taken_by(name: &str, other: &str) -> io::Error {
    io::Error::new(
        io::ErrorKind::AddrInUse,
        format!("{name} is taken by {other}"),
    )
}

/// Remove what session `id` left in `dir` (socket, status, label, the
/// terminal's environment and log, but not what its program left to be
/// shown, [`leave_file`]), unless a live host has the id again; under the
/// lock, so a launch claiming the id meanwhile keeps what it made.
pub(crate) fn clear(dir: &Path, id: &str) -> io::Result<()> {
    let _lock = lock(dir)?;
    let path = socket(dir, id);
    if !alive(&path) {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(status_file(dir, id));
        let _ = std::fs::remove_file(label_file(dir, id));
        let _ = std::fs::remove_file(terminal_env_file(dir, id));
        let _ = std::fs::remove_file(log_file(dir, id));
    }
    Ok(())
}

/// Whether a host still listens on `path`.
pub(crate) fn alive(path: &Path) -> bool {
    UnixStream::connect(path).is_ok()
}

/// The live sessions in `dir` whose ids start with `prefix`, in id
/// order. A socket nobody listens on is removed on the way, with its
/// status and label.
pub fn live(dir: &Path, prefix: &str) -> io::Result<Vec<Listed>> {
    let mut sockets = Vec::new();
    for entry in std::fs::read_dir(dir)? {
        let path = entry?.path();
        let Some(name) = id_of(&path).filter(|n| n.starts_with(prefix)) else {
            continue;
        };
        sockets.push((name.to_string(), path));
    }
    // All asked at once: a host slow to answer holds up only itself.
    let answers: Vec<_> = std::thread::scope(|s| {
        let asking: Vec<_> = sockets
            .iter()
            .map(|(_, path)| s.spawn(move || asked(path)))
            .collect();
        asking
            .into_iter()
            .map(|a| {
                a.join()
                    .unwrap_or_else(|_| Asked::Failed(io::Error::other("asking it panicked")))
            })
            .collect()
    });
    let mut found = Vec::new();
    for ((name, _), answer) in sockets.into_iter().zip(answers) {
        match answer {
            Asked::Answered(listed) => found.push(Listed {
                status: read_status(dir, &name),
                label: read_label(dir, &name),
                ..listed
            }),
            // Refused: no host behind it.
            Asked::Stale => clear(dir, &name)?,
            // A host busy enough not to answer still accepts connections,
            // so it is spared.
            Asked::Busy | Asked::Closing | Asked::Failed(_) => {}
        }
    }
    found.sort_by(|a, b| natural(&a.name, &b.name));
    Ok(found)
}

/// Names in order with their trailing numbers compared as numbers, so
/// `x-2` comes before `x-10`.
fn natural(a: &str, b: &str) -> std::cmp::Ordering {
    let split = |s: &str| {
        let head = s.trim_end_matches(|c: char| c.is_ascii_digit());
        (head.to_string(), s[head.len()..].parse::<u64>().ok())
    };
    split(a).cmp(&split(b))
}

/// The one live session `wanted` names among the tool's sessions in
/// `dir` ([`pick_from`]).
#[cfg(any(test, feature = "clap"))]
pub(crate) fn pick(dir: &Path, prefix: &str, wanted: Option<&str>) -> io::Result<Listed> {
    pick_from(&live(dir, prefix)?, prefix, wanted)
}

/// The one session of `live` that `wanted` names (by its label, its id in
/// full or after `prefix`, or the end of its id: `foo` for
/// `tool-view-foo`, in that order), or, with nothing wanted, the only one
/// there is.
#[cfg(any(test, feature = "clap"))]
pub(crate) fn pick_from(live: &[Listed], prefix: &str, wanted: Option<&str>) -> io::Result<Listed> {
    let not_found = |msg: String| io::Error::new(io::ErrorKind::NotFound, msg);
    match wanted {
        Some(w) => {
            let exact = live
                .iter()
                .find(|l| l.label.as_deref() == Some(w))
                .or_else(|| {
                    live.iter()
                        .find(|l| l.name == w || l.name == format!("{prefix}{w}"))
                });
            if let Some(l) = exact {
                return Ok(l.clone());
            }
            let tail = format!("-{w}");
            match live
                .iter()
                .filter(|l| l.name.ends_with(&tail))
                .collect::<Vec<_>>()
                .as_slice()
            {
                [one] => Ok((*one).clone()),
                [] => Err(not_found(format!("no live session named {w}"))),
                many => Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("{w} could be any of: {}", names(many.iter().copied())),
                )),
            }
        }
        None => match live {
            [one] => Ok(one.clone()),
            [] => Err(not_found("no live sessions".into())),
            many => Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{} live sessions; name one: {}",
                    many.len(),
                    names(many.iter())
                ),
            )),
        },
    }
}

/// What to type for each of `listed` ([`Listed::handle`]), comma
/// separated.
pub(crate) fn names<'a>(listed: impl IntoIterator<Item = &'a Listed>) -> String {
    listed
        .into_iter()
        .map(Listed::handle)
        .collect::<Vec<_>>()
        .join(", ")
}

/// How long to wait for a session asked to stop, and when to say it is
/// still being waited for.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Patience {
    /// The longest wait.
    pub limit: Duration,
    /// When to say so first, and how often again after that; None to say
    /// nothing.
    pub say: Option<(Duration, Duration)>,
}

/// Ask the host on `path` to stop its program, and wait up to `wait` for
/// the session to end, saying nothing; true if it did.
pub(crate) fn stop(path: &Path, force: bool, wait: Duration) -> io::Result<bool> {
    let quiet = Patience {
        limit: wait,
        say: None,
    };
    stop_waiting(path, force, quiet, || {})
}

/// Ask the host on `path` to stop its program, and wait for the session
/// to end as `patience` says, calling `waiting` at its times; true if it
/// did. The host holds the connection the stop was asked on until it
/// ends, so the wait is a read that ends with it.
pub(crate) fn stop_waiting(
    path: &Path,
    force: bool,
    patience: Patience,
    mut waiting: impl FnMut(),
) -> io::Result<bool> {
    use std::io::Read;
    let mut s = wire::connect(path)?;
    wire::set_timeout(&s, true, Some(ASK))?;
    wire::write_frame(&s, wire::KILL, &[u8::from(force)])?;
    let _ = wire::read_frame(&mut s);
    let start = std::time::Instant::now();
    let mut say_at = patience.say.map(|(first, _)| first);
    let mut buf = [0u8; 64];
    loop {
        let waited = start.elapsed();
        if waited >= patience.limit {
            return Ok(false);
        }
        if let (Some(at), Some((_, every))) = (say_at, patience.say) {
            if waited >= at {
                waiting();
                say_at = Some(at + every);
                continue;
            }
        }
        let until = say_at.map_or(patience.limit, |at| at.min(patience.limit));
        // A zero timeout would be none at all.
        let left = (until - waited).max(Duration::from_millis(1));
        wire::set_timeout(&s, true, Some(left))?;
        match s.read(&mut buf) {
            // What the host tells watchers: not ours to read.
            Ok(n) if n > 0 => {}
            Err(e)
                if matches!(
                    e.kind(),
                    io::ErrorKind::WouldBlock
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::Interrupted
                ) => {}
            // Closed: the host is gone.
            _ => break,
        }
    }
    // Its host has taken its files away, unless it was gone too fast to.
    if let (Some(dir), Some(id)) = (path.parent(), path.file_stem()) {
        let _ = clear(dir, &id.to_string_lossy());
    }
    Ok(true)
}

#[cfg(test)]
#[path = "tests/dir.rs"]
mod tests;
