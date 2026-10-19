//! Detachable terminal sessions for the legume tools.
//!
//! A TUI started through [`Tool::session`] runs in a *host*: a small
//! process, off the user's login session, that owns a pseudo-terminal and
//! relays bytes between it and the user's terminal. The user can leave
//! with Ctrl+Alt+D ([`detach_key`]; or lose the connection) and come back
//! by starting the tool again or with `<tool> attach`; the program, and
//! whatever it started, keeps running in between. Ctrl+Alt+S
//! ([`switch_key`]) goes to another of the tool's sessions. A session
//! keeps its id (`tool-view-1`) for life and may carry a label the user
//! gives it, on detach or later. The host never changes the bytes, so
//! kitty, sixel and iTerm2 images reach the terminal as the program wrote
//! them.
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! use legume_tui::{Modes, Mouse, Tool};
//! const TOOL: Tool = Tool::new("tool");
//!
//! legume_tui::serve_if_host(); // first thing in main
//! // ...
//! let session = TOOL.session("view", &legume_tui::Options::default())?;
//! let mut terminal = session.terminal_with(Modes::new().mouse(Mouse::Buttons))?;
//! while !session.stop_requested() {
//!     session.draw(&mut terminal, |f| { /* ... */ })?;
//!     session.status("what the program is doing, in a line");
//!     let round = session.poll_with(&mut terminal, std::time::Duration::from_secs(1), || {
//!         // after an attach: ask the terminal again, e.g. with
//!         // legume_tui::graphics::reprobe
//!         Ok(())
//!     })?;
//!     if let legume_tui::Round::Event(event) = round {
//!         // handle the event ...
//!     }
//!     // do the work ...
//! #   break;
//! }
//! session.restore();
//! # Ok(())
//! # }
//! ```
//!
//! Sessions need Unix. Elsewhere the same calls run every TUI plain
//! ([`Bypass::Unsupported`]): [`Tool::session`] returns a plain session,
//! and the `attach`, `sessions`, `kill` and `rename` handlers say that
//! sessions are not supported here.

// The keys' spelling, for help screens everywhere; read only in sessions.
#[cfg_attr(not(sessions), allow(dead_code))]
mod chord;
mod mode;
#[cfg(unix)]
#[cfg_attr(not(any(sessions, feature = "child")), allow(dead_code))]
mod pty;
mod screen;

#[cfg(sessions)]
mod ask;
#[cfg(sessions)]
mod client;
#[cfg(sessions)]
pub mod dir;
#[cfg(sessions)]
mod host;
#[cfg(sessions)]
mod picker;
#[cfg(sessions)]
mod scan;
#[cfg(sessions)]
mod stop;
#[cfg(sessions)]
mod wire;

#[cfg(not(sessions))]
mod plain;
#[cfg(not(sessions))]
use plain::{dir, session_at, stop, Control};

#[cfg(all(feature = "child", unix))]
pub mod child;
#[cfg(feature = "clap")]
pub mod cli;
#[cfg(feature = "graphics")]
pub mod graphics;
#[cfg(feature = "logs")]
pub mod logs;
#[cfg(feature = "map")]
pub mod map;
#[cfg(all(feature = "testing", sessions))]
pub mod testing;

#[cfg(sessions)]
pub use dir::{runtime_dir, Listed};
pub use mode::{hosts_by_default, Bypass};
pub use screen::{Keyboard, Modes, Mouse};
#[cfg(sessions)]
pub use wire::Size;

use ratatui::crossterm::event::{self, Event};
use std::ffi::OsString;
use std::io;
#[cfg(sessions)]
use std::os::fd::{BorrowedFd, FromRawFd, OwnedFd, RawFd};
#[cfg(sessions)]
use std::path::Path;
use std::path::PathBuf;
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Set to `1` (anything but empty or `0`) to run every TUI plain.
pub const NO_DETACH_VAR: &str = "LEGUME_TUI_NO_DETACH";
/// The detach key in place of Ctrl+Alt+D, e.g. `ctrl+alt+k` or `ctrl+\`:
/// Ctrl, Alt or not, and a letter or one of `@ \ ] ^ _`.
pub const DETACH_KEY_VAR: &str = "LEGUME_TUI_DETACH_KEY";
/// The switch key in place of Ctrl+Alt+S, spelled as for
/// [`DETACH_KEY_VAR`].
pub const SWITCH_KEY_VAR: &str = "LEGUME_TUI_SWITCH_KEY";
/// Set by a host in its program's environment, to the session's id. A
/// TUI that finds it set and is not that program runs plain: no host is
/// started inside another.
pub const SESSION_VAR: &str = "LEGUME_TUI_SESSION";
/// Where the sockets live, in place of the default runtime directory.
pub const DIR_VAR: &str = "LEGUME_TUI_DIR";
/// Set in a host process: the name of the session it serves.
#[cfg(sessions)]
pub(crate) const HOST_VAR: &str = "LEGUME_TUI_HOST";
/// The launching terminal's size, for the host to start the pty with.
#[cfg(sessions)]
pub(crate) const SIZE_VAR: &str = "LEGUME_TUI_SIZE";
/// The file descriptor of the program's control channel.
pub(crate) const CONTROL_VAR: &str = "LEGUME_TUI_CONTROL";
/// The host's process id: only its own child takes the control channel.
pub(crate) const PARENT_VAR: &str = "LEGUME_TUI_HOST_PID";
/// The file descriptor of the listening socket the launching client
/// bound for its host.
#[cfg(sessions)]
pub(crate) const LISTEN_VAR: &str = "LEGUME_TUI_LISTEN";

/// What of its environment an attaching client passes on, for
/// [`Session::terminal_env`]: what says which terminal it is, whether it
/// is inside tmux, and whether it is reached over ssh. The list may grow
/// in a minor release.
pub const TERMINAL_VARS: &[&str] = &[
    "TERM",
    "TERM_PROGRAM",
    "TERM_PROGRAM_VERSION",
    "KITTY_WINDOW_ID",
    "LC_TERMINAL",
    "COLORTERM",
    "TMUX",
    "WEZTERM_EXECUTABLE",
    "KONSOLE_VERSION",
    "ITERM_SESSION_ID",
    "SSH_CONNECTION",
    "SSH_CLIENT",
    "SSH_TTY",
];

/// How long a terminal has to answer a query.
#[cfg_attr(not(any(sessions, feature = "graphics")), allow(dead_code))]
const ASK_TIMEOUT: Duration = Duration::from_millis(500);

/// How long a launching client waits for its host's socket.
#[cfg(sessions)]
const LAUNCH: Duration = Duration::from_secs(5);
/// The shortest wait of [`Session::poll`] while detached, and the longest
/// while the host is yet to say whether a terminal is attached.
const DETACHED_POLL: Duration = Duration::from_secs(1);
/// How long a TUI started inside a session waits for its host to say
/// whether a terminal is attached, as it starts.
#[cfg(sessions)]
const FIRST_WORD: Duration = Duration::from_secs(1);

/// How a TUI wants its session.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Run plain, in this terminal, as before (`--no-detach`).
    pub no_detach: bool,
    /// Ask for hosting where it is not the default (see
    /// [`hosts_by_default`]).
    pub detach: bool,
    /// The session's label, unique among the tool's live sessions; its id
    /// is the tool, the screen and the label, in place of the next free
    /// number. A new session always starts, without the resume picker,
    /// and if it cannot be hosted [`Tool::session`] fails rather than run
    /// it plain.
    pub name: Option<String>,
    /// The user named what to open (a file, a run): start a new session
    /// without the resume picker; a one-line hint names the detached
    /// sessions instead.
    pub target: bool,
}

/// The detach key as a help screen shows it: `Ctrl+Alt+D`, or what
/// [`DETACH_KEY_VAR`] sets.
pub fn detach_key() -> String {
    chord::Key::detach().display()
}

/// The switch key as a help screen shows it: `Ctrl+Alt+S`, or what
/// [`SWITCH_KEY_VAR`] sets.
pub fn switch_key() -> String {
    chord::Key::switch().display()
}

/// A session of this process offers the detach and switch keys
/// ([`Session::offers_keys`]).
static KEYS_OFFERED: AtomicBool = AtomicBool::new(false);

/// A key a help screen lists ([`help_keys`]).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct HelpKey {
    /// The key as [`detach_key`] spells it: `Ctrl+Alt+D`.
    pub key: String,
    /// What it does in a word: `detach`.
    pub short: String,
    /// What it does in a phrase that stands on its own.
    pub long: String,
}

/// The detach and switch keys, each with what it does, for a help screen
/// to list: none until a session of this process offers them
/// ([`Session::offers_keys`]).
pub fn help_keys() -> Vec<HelpKey> {
    if !KEYS_OFFERED.load(SeqCst) {
        return Vec::new();
    }
    vec![
        HelpKey {
            key: detach_key(),
            short: "detach".into(),
            long: format!(
                "detach: leave it running (named on the first detach); \
                 `{} attach` (or starting it again) returns",
                exe_name()
            ),
        },
        HelpKey {
            key: switch_key(),
            short: "switch".into(),
            long: "switch: leave it running and pick another session".into(),
        },
    ]
}

/// The session [`Tool::session`] opened first in this process, for code
/// far from where it was opened (a worker thread starting a child, say);
/// None before. It is the same session: what one does, the other sees.
pub fn current() -> Option<&'static Session> {
    CURRENT.get()
}

/// The session [`current`] gives.
static CURRENT: OnceLock<Session> = OnceLock::new();

/// Whether this process runs in a session: the program a host started, or
/// a program started inside it, at any depth ([`SESSION_VAR`] is set).
pub fn in_session() -> bool {
    std::env::var_os(SESSION_VAR).is_some_and(|v| !v.is_empty())
}

/// The running executable's file name, as a user types it.
pub(crate) fn exe_name() -> String {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "<tool>".into())
}

/// If this process was started as a session's host, serve that session
/// and exit; otherwise return at once. Call it first thing in `main`, so a
/// host does none of the tool's own work before it starts the program.
pub fn serve_if_host() {
    #[cfg(sessions)]
    if let Some(name) = std::env::var_os(HOST_VAR) {
        host::serve_session(&name.to_string_lossy());
    }
}

/// A tool whose TUIs run in sessions: its sessions are those whose ids
/// start with its name and `-` (`tool-view-1`), and only those are
/// offered, listed and found for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    name: &'static str,
}

impl Tool {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// The tool's name, as its sessions' ids start.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// What every id of the tool's sessions starts with.
    pub fn prefix(&self) -> String {
        format!("{}-", self.name)
    }

    /// Start the session of the tool's TUI `screen`: where sessions are
    /// not supported, always a plain one ([`Bypass::Unsupported`], unless
    /// the user opted out).
    #[cfg(not(sessions))]
    pub fn session(&self, screen: &str, opts: &Options) -> io::Result<Session> {
        let _ = screen;
        let session = match mode::mode(opts, &mode::Env::read()) {
            mode::Mode::Plain(why @ (Bypass::Flag | Bypass::Env)) => Session::bypassed(why),
            _ => Session::bypassed(Bypass::Unsupported),
        };
        let _ = CURRENT.set(session.handle());
        Ok(session)
    }

    /// Start the session of the tool's TUI `screen`, named
    /// `tool-screen-N` (`tool-screen-<name>` with [`Options::name`]).
    ///
    /// Where the TUI is hosted (by default, in an interactive terminal
    /// outside tmux and screen), the call in the launching process does
    /// not return. If the tool has live sessions and nothing was named
    /// ([`Options::target`], [`Options::name`]), a picker first offers to
    /// return to one of them. Otherwise it starts the host, which runs
    /// this executable again with the same arguments, relays the terminal
    /// until the user detaches or the program ends, then exits the process
    /// with the program's status. In that program the call returns a
    /// hosted [`Session`]. Elsewhere it returns a plain one and the TUI
    /// runs as before. Call it before any work the program would otherwise
    /// repeat. Where sessions are not supported, always a plain one
    /// ([`Bypass::Unsupported`], unless the user opted out). The first one
    /// a process opens is also [`current`].
    ///
    /// Before a launch, what the tool's ended sessions left
    /// ([`Session::leave`]) is said, once.
    #[cfg(sessions)]
    pub fn session(&self, screen: &str, opts: &Options) -> io::Result<Session> {
        serve_if_host();
        let session = match mode::mode(opts, &mode::Env::read()) {
            mode::Mode::Hosted => match control() {
                Some(control) => Session::hosted(control),
                // The channel was not ours after all (closed, or reused).
                None => Session::nested(Bypass::Nested),
            },
            mode::Mode::Plain(Bypass::Nested) => Session::nested(Bypass::Nested),
            mode::Mode::Plain(why) => Session::bypassed(why),
            mode::Mode::Launch => {
                let launched = dir::runtime_dir().and_then(|dir| {
                    for (id, text) in dir::take_left(&dir, &self.prefix()) {
                        eprintln!("{}", dir::ended(&id, &text));
                    }
                    let live = dir::live(&dir, &self.prefix()).unwrap_or_default();
                    if let Some(code) = self.resume(&dir, opts, &live) {
                        std::process::exit(code);
                    }
                    self.launch(&dir, screen, opts, &live)
                });
                match launched {
                    Ok(code) => std::process::exit(code),
                    // Asked for by name: not one to run plain in its place.
                    Err(e) if opts.name.is_some() => return Err(e),
                    Err(e) => Session::no_host(format!(
                        "legume-tui: running without a session ({e}), so {} does \
                         not detach; {NO_DETACH_VAR}=1 runs without trying",
                        detach_key()
                    )),
                }
            }
        };
        let _ = CURRENT.set(session.handle());
        Ok(session)
    }
}

#[cfg(sessions)]
impl Tool {
    /// Before a launch: with nothing named and `live` sessions of the
    /// tool, the picker; a status to exit with if the user returned to
    /// one, or chose neither. With something named, a hint naming the
    /// detached sessions.
    fn resume(&self, dir: &Path, opts: &Options, live: &[Listed]) -> Option<i32> {
        if live.is_empty() {
            return None;
        }
        if opts.target || opts.name.is_some() {
            let detached: Vec<_> = live.iter().filter(|l| !l.attached).collect();
            if !detached.is_empty() {
                eprintln!(
                    "[detached: {}; `{} attach NAME` to return]",
                    dir::names(detached),
                    exe_name()
                );
            }
            return None;
        }
        match picker::choose_or_say(dir, self, true, None, live.to_vec())? {
            picker::Choice::Attach(name) => {
                let attached = client::attach_and_report(dir, self, &name);
                Some(attached.unwrap_or_else(|e| {
                    eprintln!("legume-tui: cannot attach to {name} ({e})");
                    1
                }))
            }
            picker::Choice::Quit => Some(0),
            picker::Choice::New => None,
        }
    }

    /// Start a host for a new session of `screen` in `dir` and be its
    /// first client; returns the status to exit with. `live` are the
    /// tool's sessions as listed just now. Fails, with no host and nothing
    /// of the session left behind, if the host cannot start.
    fn launch(&self, dir: &Path, screen: &str, opts: &Options, live: &[Listed]) -> io::Result<i32> {
        use std::os::fd::AsRawFd;
        // Bound here, so the name is ours and so is the host we reach on it.
        let base = format!("{}-{screen}", self.name);
        let claimed = dir::claim(dir, &self.prefix(), &base, opts.name.as_deref(), live)?;
        if let Some(text) = &claimed.left {
            eprintln!("{}", dir::ended(&claimed.name, text));
        }
        let mut claim = Claim {
            dir,
            name: claimed.name,
            listener: Some(claimed.listener),
            up: false,
        };
        let name = claim.name.clone();
        let path = dir::socket(dir, &name);
        let size = Size::of(io::stdout()).unwrap_or_default();
        let log = dir::log_file(dir, &name);
        let listen = claim.listener.as_ref().map_or(-1, |l| l.as_raw_fd());
        let mut host = std::process::Command::new(std::env::current_exe()?);
        host.args(std::env::args_os().skip(1))
            .env(HOST_VAR, &name)
            .env(SIZE_VAR, size.to_env())
            .env(LISTEN_VAR, listen.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&log)?);
        // In the child, between fork and exec: only system calls.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut host, move || {
                // Off the user's session before it runs a line of its own: a
                // ctrl+c or a hangup there while it starts cannot reach it.
                rustix::process::setsid()?;
                rustix::io::fcntl_setfd(
                    BorrowedFd::borrow_raw(listen),
                    rustix::io::FdFlags::empty(),
                )?;
                Ok(())
            });
        }
        let spawned = host.spawn();
        // The host has its own copy now.
        claim.listener = None;
        let mut host = spawned?;
        // Up once it answers: its program is started and it serves clients.
        let start = std::time::Instant::now();
        loop {
            if dir::ask(&path).is_some() {
                break;
            }
            let ended = host.try_wait()?.is_some();
            if ended || start.elapsed() > LAUNCH {
                if !ended {
                    // Its program goes with it: the pty closes, it hangs up.
                    let _ = host.kill();
                    let _ = host.wait();
                }
                // Read before the claim takes it away.
                let said = std::fs::read_to_string(&log).unwrap_or_default();
                let said = said.trim();
                return Err(io::Error::other(if said.is_empty() {
                    "the session host did not start".to_string()
                } else {
                    said.to_string()
                }));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // The host outlives us; it is not ours to wait for, and what it
        // left is its own to take away.
        drop(host);
        claim.up = true;
        // From here the program runs: a failure must not start a second
        // copy plain, so it is reported and left to attach to.
        match client::attach_and_report(dir, self, &name) {
            Ok(code) => Ok(code),
            Err(e) => {
                eprintln!("legume-tui: cannot relay {name} ({e}); it runs on, detached");
                Ok(1)
            }
        }
    }
}

/// A session claimed for a launch: until its host is up, a failure takes
/// away all it left (socket, label, log).
#[cfg(sessions)]
struct Claim<'a> {
    dir: &'a Path,
    name: String,
    /// Ours until the host has its own copy.
    listener: Option<std::os::unix::net::UnixListener>,
    up: bool,
}

#[cfg(sessions)]
impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if !self.up {
            // Closed first, so nothing listens on the socket.
            self.listener = None;
            let _ = dir::clear(self.dir, &self.name);
        }
    }
}

/// How many attaches this process has heard of ([`attaches`]).
static ATTACHES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// How many times a terminal has attached to this process's session, as
/// heard by the last [`Session::poll`] (or [`Session::take_repaint`],
/// [`Session::attached`]); 0 when plain. A cache of what suits the
/// terminal (an image picker) that is not where the loop takes the
/// repaint can keep the count it was made at and make itself again when
/// the count is another.
pub fn attaches() -> u64 {
    ATTACHES.load(SeqCst)
}

/// What the host says about attaching, as this process reads it: the
/// control channel of the program the host started, or a watch on the
/// host by a TUI that program started on the same terminal. Shared by
/// every session the process opens.
#[cfg(sessions)]
struct Control {
    fd: OwnedFd,
    attached: AtomicBool,
    repaint: AtomicBool,
    /// Whether the host has said how things stand: from the start for
    /// the control channel (until a word comes, nobody is attached), from
    /// its first word (or its end) for a watch, which the host answers
    /// with the state now.
    told: AtomicBool,
}

#[cfg(sessions)]
impl Control {
    fn new(fd: OwnedFd, told: bool) -> Arc<Self> {
        Arc::new(Self {
            fd,
            attached: AtomicBool::new(false),
            repaint: AtomicBool::new(false),
            told: AtomicBool::new(told),
        })
    }

    /// Whether the host is yet to say how things stand: a round's wait is
    /// kept short meanwhile, as its first word wakes nothing.
    fn untold(&self) -> bool {
        !self.told.load(SeqCst)
    }

    /// Read what the host said since last time.
    fn drain(&self) {
        let mut buf = [0u8; 64];
        loop {
            match rustix::io::read(&self.fd, &mut buf) {
                Ok(0) => {
                    // The host is gone: nobody will attach again.
                    self.attached.store(false, SeqCst);
                    self.told.store(true, SeqCst);
                    return;
                }
                Ok(n) => self.heard(&buf[..n]),
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return,
            }
        }
    }

    /// Whether a client is attached, as last said.
    fn attached(&self) -> bool {
        self.drain();
        self.attached.load(SeqCst)
    }

    /// Whether an attach asked for a repaint since last time.
    fn take_repaint(&self) -> bool {
        self.drain();
        self.repaint.swap(false, SeqCst)
    }

    fn heard(&self, said: &[u8]) {
        if !said.is_empty() {
            self.told.store(true, SeqCst);
        }
        for &b in said {
            match b {
                host::ATTACHED => {
                    self.attached.store(true, SeqCst);
                    self.repaint.store(true, SeqCst);
                    ATTACHES.fetch_add(1, SeqCst);
                }
                host::DETACHED => self.attached.store(false, SeqCst),
                _ => {}
            }
        }
    }
}

/// A file descriptor handed down to this process, named in environment
/// variable `var`, if it is open here and of type `kind`: ours from here,
/// closed with the process and not passed on to what it starts.
#[cfg(sessions)]
pub(crate) fn inherited_fd(var: &str, kind: rustix::fs::FileType) -> Option<OwnedFd> {
    let fd: RawFd = std::env::var(var).ok()?.parse().ok()?;
    if fd < 0 {
        return None;
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let st = rustix::fs::fstat(borrowed).ok()?;
    if rustix::fs::FileType::from_raw_mode(st.st_mode) != kind {
        return None;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC).ok()?;
    Some(fd)
}

/// The control channel named in the environment, taken once per process;
/// None if it is not a pipe open here.
#[cfg(sessions)]
fn control() -> Option<Arc<Control>> {
    static CONTROL: OnceLock<Option<Arc<Control>>> = OnceLock::new();
    CONTROL
        .get_or_init(|| {
            let fd = inherited_fd(CONTROL_VAR, rustix::fs::FileType::Fifo)?;
            rustix::io::ioctl_fionbio(&fd, true).ok()?;
            Some(Control::new(fd, true))
        })
        .clone()
}

/// A watch on the host of the session named in the environment, for a
/// TUI a hosted program started on the same terminal; taken once per
/// process. None if there is no such host, or our terminal is not its.
///
/// The host answers a watch with the state now, from a thread of its own.
/// That word is waited for here, up to [`FIRST_WORD`], so the first round
/// knows whether to draw: a round's wait watches only the terminal, and a
/// word that comes during it wakes nothing. A word later than that is
/// heard within [`DETACHED_POLL`], as rounds wait no longer until it
/// comes ([`Control::untold`]). The words after it need no wake of their
/// own: an attach comes with SIGWINCH to the terminal's foreground, which
/// is this TUI on the host's terminal, and a detach drops what is drawn
/// until the next round hears it. A TUI on another terminal draws there,
/// whatever the host's state.
#[cfg(sessions)]
fn watch() -> Option<Arc<Control>> {
    static WATCH: OnceLock<Option<Arc<Control>>> = OnceLock::new();
    WATCH
        .get_or_init(|| {
            use std::io::Read;
            if !rustix::termios::isatty(io::stdout()) {
                return None;
            }
            let name = std::env::var(SESSION_VAR).ok()?;
            dir::check_name(&name).ok()?;
            let path = dir::socket(&dir::runtime_dir().ok()?, &name);
            let sock = wire::connect(path).ok()?;
            wire::write_frame(&sock, wire::WATCH, &[]).ok()?;
            // None if it did not come in time: rounds wait short till then.
            let mut word = [0u8; 1];
            let mut first = None;
            if wire::set_timeout(&sock, true, Some(FIRST_WORD)).is_ok() {
                first = loop {
                    match (&sock).read(&mut word) {
                        Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
                        read => break read.ok(),
                    }
                };
            }
            sock.set_nonblocking(true).ok()?;
            let control = Control::new(sock.into(), first.is_some());
            // Nothing read (the host gone) leaves nobody attached.
            control.heard(&word[..first.unwrap_or(0)]);
            Some(control)
        })
        .clone()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// The alternate screen is a program screen's now: from
/// [`Session::terminal`] to [`Session::restore`], but not while it is
/// handed over ([`Session::run_child`], [`Session::off_screen`]). The
/// terminal is the process's, whichever session has it.
static SHOWN: AtomicBool = AtomicBool::new(false);

/// Whether a screen of this process is shown now ([`SHOWN`]).
#[cfg_attr(not(feature = "logs"), allow(dead_code))]
pub(crate) fn screen_shown() -> bool {
    SHOWN.load(SeqCst)
}

pub(crate) fn set_shown(shown: bool) {
    SHOWN.store(shown, SeqCst);
    // Log records held while it was up are written now.
    #[cfg(feature = "logs")]
    if !shown {
        logs::replay();
    }
}

/// A TUI's session: hosted (detachable) or plain. It stands where
/// `ratatui::init` and `ratatui::restore` stood, and runs each round of
/// the program's loop ([`Session::poll`], [`Session::draw`]).
///
/// The one [`Tool::session`] returns restores the terminal when dropped;
/// [`current`] gives the same session to the rest of the process.
pub struct Session {
    inner: Arc<Inner>,
    /// This one restores the terminal when dropped.
    owner: bool,
}

/// What a session is, shared by the handles to it.
struct Inner {
    /// The host's word on attaching: hosted, or nested in a host.
    control: Option<Arc<Control>>,
    bypass: Option<Bypass>,
    /// A repaint is owed: the first in a plain session, one after a child
    /// had the terminal, and one asked for ([`Session::request_repaint`]).
    owed: AtomicBool,
    /// [`Session::terminal`] set the terminal up and it is not restored.
    on: AtomicBool,
    /// `kill` asks the program to stop while the screen is up.
    armed: Mutex<Option<stop::Armed>>,
    /// The modes the screen turns on.
    modes: Mutex<Modes>,
    /// Whether the terminal has kitty's keyboard enhancement, and the
    /// attach ([`attaches`]) that was asked at.
    keyboard: Mutex<Option<(u64, bool)>>,
    /// An event read in a round that ended in a repaint, for the next.
    pending: Mutex<Option<Event>>,
    /// A round of this screen said [`Round::Stopped`].
    stop_said: AtomicBool,
    /// The session's directory and id, in a session's host or nested in
    /// one: where [`Session::status`] writes.
    at: Option<(PathBuf, String)>,
    /// What [`Session::status`] said last.
    said: Mutex<String>,
    /// What a plain session leaves ([`Session::leave`]), said when its
    /// screen goes.
    left: Mutex<Option<String>>,
    /// Why a launch could not host ([`Bypass::NoHost`]); see
    /// [`Session::restore`].
    no_host: Option<String>,
}

/// What one round's wait ([`Session::poll`]) ended with.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Round {
    /// An event came: a key, the mouse, a paste, a resize.
    Event(Event),
    /// Nothing came in the time given.
    Timeout,
    /// The screen was cleared for the terminal attached now (or taken
    /// back from a child, or asked for by [`Session::request_repaint`]),
    /// and the terminal asked again: draw it whole.
    Repainted,
    /// `kill` asked the program to stop ([`Session::stop_requested`]):
    /// said once a screen, in the first round after it came. Later rounds
    /// wait as before, and `stop_requested` stays true for the program,
    /// or a screen after this one, to check.
    Stopped,
}

impl Round {
    /// The event, if one came.
    pub fn event(self) -> Option<Event> {
        match self {
            Self::Event(e) => Some(e),
            _ => None,
        }
    }
}

/// Which process group a child of the program runs in
/// ([`Session::command`], [`ChildEnv::apply`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Group {
    /// The program's own, as [`Command`] starts it: for a child that has
    /// the terminal ([`Session::run_child`]), or one that should hear what
    /// the program hears (a ctrl+c typed while the terminal is cooked, a
    /// hangup).
    #[default]
    Shared,
    /// Always one of its own (Unix), which the program stops itself: for a
    /// runner's child (`child::run_one`), so a stop reaches all it started
    /// and nothing typed or sent to the program does. While a
    /// `child::Stopper` holds it, a hangup of the program is passed on to
    /// it, saves aside.
    Own,
    /// One of its own while the program is hosted, so no signal meant for
    /// the program (a wake after an attach) reaches it; plain, the
    /// program's own, so a hangup of the user's terminal reaches it with
    /// the program. For a child the program waits for itself that is not
    /// held by a `child::Stopper`.
    OwnWhenHosted,
}

/// How a session starts its children: the environment it passes on, and
/// whether it is hosted (for [`Group::OwnWhenHosted`]). Owned, for children
/// started away from the session (on a worker thread).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ChildEnv {
    vars: Vec<(OsString, OsString)>,
    hosted: bool,
}

impl ChildEnv {
    /// The variables set on a child: the user's opt-out
    /// ([`NO_DETACH_VAR`]) when the session runs plain by `--no-detach` or
    /// by that variable. Inside a session, [`SESSION_VAR`] is passed on
    /// already.
    pub fn vars(&self) -> &[(OsString, OsString)] {
        &self.vars
    }

    /// Set `cmd` up as the session starts its children, in `group`.
    pub fn apply<'a>(&self, cmd: &'a mut Command, group: Group) -> &'a mut Command {
        cmd.envs(self.vars.iter().map(|(k, v)| (k, v)));
        #[cfg(unix)]
        if group == Group::Own || (group == Group::OwnWhenHosted && self.hosted) {
            std::os::unix::process::CommandExt::process_group(cmd, 0);
        }
        #[cfg(not(unix))]
        let _ = group;
        cmd
    }
}

impl Session {
    fn new(control: Option<Arc<Control>>, bypass: Option<Bypass>) -> Self {
        let at = control.as_ref().and_then(|_| session_at());
        if control.is_some() {
            KEYS_OFFERED.store(true, SeqCst);
        }
        let inner = Inner {
            // Plain, the screen starts with one.
            owed: AtomicBool::new(control.is_none()),
            control,
            bypass,
            on: AtomicBool::new(false),
            armed: Mutex::new(None),
            modes: Mutex::new(Modes::default()),
            keyboard: Mutex::new(None),
            pending: Mutex::new(None),
            stop_said: AtomicBool::new(false),
            at,
            said: Mutex::new(String::new()),
            left: Mutex::new(None),
            no_host: None,
        };
        Self {
            inner: Arc::new(inner),
            owner: true,
        }
    }

    /// Another handle to the session, which does not restore the terminal
    /// when dropped.
    fn handle(&self) -> Self {
        Self {
            inner: Arc::clone(&self.inner),
            owner: false,
        }
    }

    #[cfg(sessions)]
    fn hosted(control: Arc<Control>) -> Self {
        Self::new(Some(control), None)
    }

    fn bypassed(why: Bypass) -> Self {
        Self::new(None, Some(why))
    }

    /// Plain, as a launch could not host it ([`Bypass::NoHost`]): `why`
    /// is said now and again as each screen goes, since the screen opening
    /// next covers it at once.
    #[cfg(sessions)]
    fn no_host(why: String) -> Self {
        eprintln!("{why}");
        let mut session = Self::bypassed(Bypass::NoHost);
        // Not shared yet: set before anything else holds it.
        if let Some(inner) = Arc::get_mut(&mut session.inner) {
            inner.no_host = Some(why);
        }
        session
    }

    /// Plain, inside another TUI's session: repaints after every attach
    /// as the hosted program does, when the host can be reached.
    #[cfg(sessions)]
    fn nested(why: Bypass) -> Self {
        Self::new(watch(), Some(why))
    }

    /// A session that is never hosted, for a screen the tool opens by
    /// itself: plain, as before ([`Bypass::Tool`]), or, inside a session
    /// ([`in_session`]), following it as a TUI started there does
    /// ([`Bypass::Nested`]). Unlike a plain run the user asked for, it
    /// passes no opt-out on to the children it starts.
    pub fn plain(tool: &Tool) -> Self {
        let _ = tool;
        #[cfg(sessions)]
        if in_session() {
            return Self::nested(Bypass::Nested);
        }
        Self::bypassed(Bypass::Tool)
    }

    /// The session's id; None when plain.
    pub fn name(&self) -> Option<String> {
        self.is_hosted()
            .then(|| self.inner.at.as_ref().map(|(_, id)| id.clone()))
            .flatten()
    }

    /// Whether the program runs in a host.
    pub fn is_hosted(&self) -> bool {
        self.inner.bypass.is_none()
    }

    /// Say what the program is doing, in one short line, for the picker
    /// and `<tool> sessions` to show. Call it every round: the same line
    /// again is not written. It does nothing outside a session.
    pub fn status(&self, text: &str) {
        let Some((dir, id)) = &self.inner.at else {
            return;
        };
        let mut said = lock(&self.inner.said);
        if *said != text {
            let _ = dir::write_status(dir, id, text);
            *said = text.to_string();
        }
    }

    /// Leave `text` (a run's summary, the error it ended with) to be read
    /// when the program has ended, in place of printing it. Hosted, it is
    /// written beside the session's socket; if a terminal is attached
    /// when the program ends, that terminal alone says it, after the
    /// program's last output, and nothing stays (a `kill` from another
    /// terminal does not say it). If the program ends with none attached,
    /// or that terminal leaves (a key, a signal) before it takes it, it is
    /// kept and said, once, by the next `<tool> attach`, `sessions`,
    /// start of the tool, or the `kill` that ended it. Plain, it is
    /// printed (on stderr) when the screen is restored, or now if it is
    /// not up. A later call replaces it.
    pub fn leave(&self, text: &str) {
        match &self.inner.at {
            Some((dir, id)) if self.is_hosted() => {
                let _ = dir::write_leave(dir, id, text);
            }
            _ if self.screen_up() => *lock(&self.inner.left) = Some(text.to_string()),
            _ => eprintln!("{}", text.trim_end()),
        }
    }

    /// Hold `error`, which ends the program, where the user will see it,
    /// then return for the program to end with it, saying it as it would
    /// anyway (`main` returning it, say). It is the session's status
    /// (`stopped: ...`). With the screen up and no terminal attached, it
    /// is drawn there until a terminal attaches and a key is pressed, or
    /// `kill` comes. Hosted, it is left ([`Session::leave`]) while no
    /// terminal is attached, so a program that ends unseen has it said,
    /// once, by whoever looks next; a terminal attached as it returns
    /// hears the program say it, and nothing is left. Plain, nothing is
    /// left: the program's own words are the ones seen.
    pub fn hold_error(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        error: impl std::fmt::Display,
    ) -> io::Result<()> {
        use ratatui::crossterm::event::KeyEventKind;
        use ratatui::widgets::{Paragraph, Wrap};
        let text = format!("{error:#}");
        let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
        self.status(&format!("stopped: {line}"));
        // Left first, for a program that never gets to say it.
        let hosted = self.is_hosted().then_some(self.inner.at.as_ref()).flatten();
        if let Some((dir, id)) = hosted {
            let _ = dir::write_leave(dir, id, &text);
        }
        if !self.attached() && self.screen_up() {
            let shown = format!("{text}\n\nany key quits");
            while !self.stop_requested() {
                self.draw(terminal, |f| {
                    let message = Paragraph::new(shown.as_str()).wrap(Wrap { trim: false });
                    f.render_widget(message, f.area());
                })?;
                match self.poll(terminal, Duration::from_millis(250))? {
                    Round::Event(Event::Key(k)) if k.kind == KeyEventKind::Press => break,
                    _ => {}
                }
            }
        }
        // Seen as the program says it: not said again.
        if let (Some((dir, id)), true) = (hosted, self.attached()) {
            let _ = dir::take_leave(dir, id);
        }
        Ok(())
    }

    /// The value of environment variable `name` (one of
    /// [`TERMINAL_VARS`]) in the terminal attached last: a hosted program
    /// may be attached from another terminal than the one it started in,
    /// and the client passes these on as it attaches. An attached terminal
    /// without it is None. Before any attach, when plain, and for other
    /// names, this process's own environment.
    pub fn terminal_env(&self, name: &str) -> Option<String> {
        let passed = self
            .inner
            .at
            .as_ref()
            .filter(|_| TERMINAL_VARS.contains(&name));
        if let Some(vars) = passed.and_then(|(dir, id)| dir::read_terminal_env(dir, id)) {
            return vars.into_iter().find(|(k, _)| k == name).map(|(_, v)| v);
        }
        std::env::var(name).ok()
    }

    /// Why the session is plain; None when hosted.
    pub fn bypass(&self) -> Option<Bypass> {
        self.inner.bypass
    }

    /// Whether the detach and switch keys work here ([`detach_key`],
    /// [`switch_key`], [`help_keys`]): hosted, or started inside a session
    /// whose host it can follow.
    pub fn offers_keys(&self) -> bool {
        self.inner.control.is_some()
    }

    /// How this session starts its children, as owned data for children
    /// started away from it ([`ChildEnv::apply`]).
    pub fn child_env(&self) -> ChildEnv {
        let mut vars = Vec::new();
        if matches!(self.inner.bypass, Some(Bypass::Flag | Bypass::Env)) {
            vars.push((NO_DETACH_VAR.into(), "1".into()));
        }
        ChildEnv {
            vars,
            hosted: self.is_hosted(),
        }
    }

    /// Set `cmd` up to run as this session does, in `group`, for a program
    /// the tool starts and waits for itself ([`Session::run_child`] does
    /// this): [`ChildEnv::apply`] of [`Session::child_env`].
    pub fn command<'a>(&self, cmd: &'a mut Command, group: Group) -> &'a mut Command {
        self.child_env().apply(cmd, group)
    }

    /// The terminal, raw, in the alternate screen, with a panic hook that
    /// restores it (as `ratatui::init`). From here, in a session, `kill`
    /// asks the program to stop ([`Session::stop_requested`]).
    pub fn terminal(&self) -> io::Result<ratatui::DefaultTerminal> {
        self.terminal_with(Modes::default())
    }

    /// [`Session::terminal`], with `modes` on: the session turns them on
    /// again after every attach and off when the terminal is handed back
    /// or on a panic. With [`Keyboard::IfSupported`] the terminal is asked
    /// first.
    pub fn terminal_with(&self, modes: Modes) -> io::Result<ratatui::DefaultTerminal> {
        *lock(&self.inner.modes) = modes;
        let t = ratatui::try_init()?;
        screen::off_on_panic();
        if !self.inner.on.swap(true, SeqCst) {
            // A new screen hears a stop that came before it once too.
            self.inner.stop_said.store(false, SeqCst);
            if self.inner.control.is_some() {
                *lock(&self.inner.armed) = stop::Armed::new();
            }
        }
        screen::turn_on(modes, self.keyboard_for(modes))?;
        set_shown(true);
        // Crossterm's event source made now, with its resize handler, so
        // a stop before the first poll wakes that poll.
        let _ = event::poll(Duration::ZERO);
        Ok(t)
    }

    /// Change the modes the screen has on.
    pub fn set_modes(&self, modes: Modes) -> io::Result<()> {
        *lock(&self.inner.modes) = modes;
        if self.inner.on.load(SeqCst) {
            screen::turn_off()?;
            screen::turn_on(modes, self.keyboard_for(modes))?;
        }
        Ok(())
    }

    /// Whether keys come with kitty's keyboard enhancement as the modes
    /// ask: [`Keyboard::On`], or [`Keyboard::IfSupported`] where the
    /// terminal attached now said it has it (asked as the screen came up
    /// and again after every attach; false while detached).
    pub fn keyboard_enhanced(&self) -> bool {
        match lock(&self.inner.modes).keyboard_enhancement {
            Keyboard::Off => false,
            Keyboard::On(_) => true,
            Keyboard::IfSupported(_) => self.attached() && self.keyboard_supported(false),
        }
    }

    /// Whether `modes` get the keyboard's enhancement here, the terminal
    /// asked if they depend on it and it was not asked since the last
    /// attach.
    fn keyboard_for(&self, modes: Modes) -> bool {
        matches!(modes.keyboard_enhancement, Keyboard::IfSupported(_))
            && self.keyboard_supported(true)
    }

    /// Whether the terminal attached now has kitty's keyboard enhancement,
    /// as it said since the last attach; else asked when `may_ask` (a
    /// terminal must be attached), else false.
    fn keyboard_supported(&self, may_ask: bool) -> bool {
        let now = attaches();
        if let Some((at, supported)) = *lock(&self.inner.keyboard) {
            if at == now {
                return supported;
            }
        }
        if !may_ask || !self.attached() {
            return false;
        }
        // No answer in time is no enhancement: not asked again until the
        // next attach.
        #[cfg(sessions)]
        {
            let supported = ask::keyboard().unwrap_or(false);
            self.heard_keyboard(supported);
            supported
        }
        #[cfg(not(sessions))]
        false
    }

    /// What the terminal attached now said of kitty's keyboard
    /// enhancement, asked by itself or by another query (the graphics
    /// one); none is asked where sessions are not supported.
    #[cfg_attr(not(sessions), allow(dead_code))]
    pub(crate) fn heard_keyboard(&self, supported: bool) {
        *lock(&self.inner.keyboard) = Some((attaches(), supported));
    }

    /// Hand the terminal back (as `ratatui::restore`), its modes off; also
    /// done when the session [`Tool::session`] returned is dropped. From
    /// here `kill` ends the program as by default, but a stop that came
    /// before stays asked ([`Session::stop_requested`]). What a plain
    /// session left ([`Session::leave`]) is said now, then, unless
    /// panicking, why a launch could not host it ([`Bypass::NoHost`]), as
    /// each screen goes: the screen covered it as it opened.
    pub fn restore(&self) {
        if self.inner.on.swap(false, SeqCst) {
            let _ = screen::turn_off();
            ratatui::restore();
            *lock(&self.inner.armed) = None;
            set_shown(false);
            if let Some(text) = lock(&self.inner.left).take() {
                eprintln!("{}", text.trim_end());
            }
            // Not after a panic: it is no cause of one.
            if !std::thread::panicking() {
                if let Some(why) = &self.inner.no_host {
                    eprintln!("{why}");
                }
            }
        }
    }

    /// Whether the screen is up: from [`Session::terminal`] to
    /// [`Session::restore`], handed over for a while or not.
    pub fn screen_up(&self) -> bool {
        self.inner.on.load(SeqCst)
    }

    /// Whether to repaint everything now and probe the terminal again:
    /// true once after each attach in a hosted session (or one started
    /// inside it), on the first call in a plain one, after a child had the
    /// terminal ([`Session::run_child`]), and once after
    /// [`Session::request_repaint`]. Cached graphics (an image picker and
    /// the protocols made with it) must be rebuilt, since the terminal may
    /// not be the one they were made for. [`Session::poll`] asks it.
    pub fn take_repaint(&self) -> bool {
        match &self.inner.control {
            Some(c) => {
                let asked = c.take_repaint();
                // Owed, it waits for a terminal to be there.
                asked | (c.attached() && self.inner.owed.swap(false, SeqCst))
            }
            None => self.inner.owed.swap(false, SeqCst),
        }
    }

    /// Owe a repaint, as after an attach: the next [`Session::poll`]
    /// clears the screen, asks the terminal again (its hook) and returns
    /// [`Round::Repainted`]. For a screen that follows another on the same
    /// terminal (a picker) and wants its own first probe.
    pub fn request_repaint(&self) {
        self.inner.owed.store(true, SeqCst);
    }

    /// Whether the host of a TUI started inside a session is yet to say
    /// whether a terminal is attached ([`watch`]).
    fn untold(&self) -> bool {
        self.inner.control.as_ref().is_some_and(|c| c.untold())
    }

    /// Whether a terminal is there to draw on. While detached, skip
    /// drawing (output is dropped) and do not read the terminal's size;
    /// keep the program's own work going at a slow poll. Always true when
    /// plain, but for a TUI started inside a session.
    pub fn attached(&self) -> bool {
        match &self.inner.control {
            Some(c) => c.attached(),
            None => true,
        }
    }

    /// Whether `kill` asked the program to stop (it arrives as SIGTERM):
    /// stop as the quit key does, wrapping up running work. A second
    /// `kill` ends the program at once. Asked while the screen is up
    /// ([`Session::terminal`] to [`Session::restore`]), while a child runs
    /// ([`Session::run_child`]) and while work runs that a stop waits for
    /// ([`Session::stoppable`]); elsewhere `kill` ends the program at once.
    /// Always false when plain, but for a TUI started inside a session.
    pub fn stop_requested(&self) -> bool {
        self.inner.control.is_some() && stop::heard()
    }

    /// Run `work`, the program's own, with `kill` taken as a request to
    /// stop meanwhile ([`Session::stop_requested`]), as while the screen
    /// is up, but without bringing the screen up: a stop waits for `work`
    /// to end (`work` may ask, to end early). The stop stays asked after,
    /// for the program to wind down; from there `kill` ends it at once
    /// again, if nothing else takes it.
    pub fn stoppable<T>(&self, work: impl FnOnce() -> T) -> T {
        let armed = self.inner.control.as_ref().and_then(|_| stop::Armed::new());
        let done = work();
        drop(armed);
        done
    }

    /// Run `cmd` on this terminal ([`Session::command`] sets it up, in the
    /// program's process group: it has the terminal) and wait for it, as
    /// [`std::process::Child::wait`] does, at any point of the program.
    /// While it runs, `kill` reaches only this program, so a stop is
    /// passed on to the child as SIGTERM, which a TUI takes through its
    /// own stop path ([`Session::stop_requested`]). With the screen up,
    /// the child (a TUI of its own) is handed the terminal and it is taken
    /// back after, to be repainted whole; a stop that came meanwhile stays
    /// asked. Before the screen is up, or after it, a stop that came ends
    /// the program once the child is gone, as `kill` does there, unless
    /// work a stop waits for runs it ([`Session::stoppable`],
    /// [`Session::off_screen`]): then the stop stays asked too.
    pub fn run_child(&self, cmd: &mut Command) -> io::Result<ExitStatus> {
        self.command(cmd, Group::Shared);
        let screen = self.hand_off();
        let armed = self.inner.control.as_ref().and_then(|_| stop::Armed::new());
        let heard = stop::heard();
        let waited = cmd
            .spawn()
            .and_then(|child| stop::wait(child, armed.is_some()));
        if screen {
            self.take_back()?;
        }
        let came = armed.is_some() && !heard && stop::heard();
        // Taken by more than this wait: the screen, or work around it.
        let taken = armed.as_ref().is_some_and(stop::Armed::outer);
        drop(armed);
        if came && !screen && !taken {
            stop::end_as_by_default();
        }
        waited
    }

    /// Run `f`, work of this process that writes to the terminal as a
    /// plain program does (a progress line, a message), off the screen:
    /// with the screen up, its modes are off, raw mode left and the normal
    /// screen shown while `f` runs, and the screen is taken back after, to
    /// be repainted whole. Up or not, `f` runs as in
    /// [`Session::stoppable`]: `kill` meanwhile asks the program to stop
    /// ([`Session::stop_requested`], which `f` may ask), and the stop
    /// waits for `f` to end.
    pub fn off_screen<T>(&self, f: impl FnOnce() -> T) -> io::Result<T> {
        let screen = self.hand_off();
        let done = self.stoppable(f);
        if screen {
            self.take_back()?;
        }
        Ok(done)
    }

    /// With the screen up, hand the terminal over as it was before it
    /// (modes off, cooked, the normal screen), the stop left armed;
    /// whether the screen was up.
    fn hand_off(&self) -> bool {
        let screen = self.inner.on.load(SeqCst);
        if screen {
            let _ = screen::turn_off();
            ratatui::restore();
            set_shown(false);
        }
        screen
    }

    /// Take the terminal back after [`Session::hand_off`]: raw, in the
    /// alternate screen, a repaint owed (which turns the modes on again).
    fn take_back(&self) -> io::Result<()> {
        use ratatui::crossterm::{execute, terminal};
        terminal::enable_raw_mode()?;
        execute!(io::stdout(), terminal::EnterAlternateScreen)?;
        set_shown(true);
        self.inner.owed.store(true, SeqCst);
        Ok(())
    }

    /// Bring the screen back after an attach: into the alternate screen,
    /// cleared, so the next draw paints all of it, and the session's modes
    /// on again ([`Session::terminal_with`]).
    pub fn repaint(&self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        self.repaint_with(terminal, || Ok(()))
    }

    /// [`Session::repaint`], with `probe` (asking the terminal anything)
    /// done before the modes are on: a mouse report would land amid the
    /// terminal's answer.
    fn repaint_with(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        probe: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        use ratatui::crossterm::{execute, terminal::EnterAlternateScreen};
        execute!(io::stdout(), EnterAlternateScreen)?;
        // A resize to the whole screen clears it and forgets what was
        // drawn, without the cursor query `clear` makes (a round trip to a
        // terminal that may have just left again).
        let size = terminal.size()?;
        terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))?;
        probe()?;
        let modes = *lock(&self.inner.modes);
        screen::turn_on(modes, self.keyboard_for(modes))
    }

    /// One round's wait: [`Session::poll_with`], nothing asked of the
    /// terminal after an attach.
    pub fn poll(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        wanted: Duration,
    ) -> io::Result<Round> {
        self.poll_with(terminal, wanted, || Ok(()))
    }

    /// One round's wait for the next event, up to `wanted` (at least a
    /// second while detached). A repaint owed ([`Session::take_repaint`])
    /// is done first, `on_repaint` called in it to ask the terminal again
    /// (before the modes are on: see `graphics::reprobe`), and the round
    /// ends at once, [`Round::Repainted`], for the screen to be drawn
    /// whole; an event read as an attach came is kept for the next round.
    /// A `kill` ends the wait ([`Round::Stopped`], once a screen).
    pub fn poll_with(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        wanted: Duration,
        on_repaint: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<Round> {
        if let Some(event) = lock(&self.inner.pending).take() {
            return Ok(Round::Event(event));
        }
        if self.take_repaint() {
            self.repaint_with(terminal, on_repaint)?;
            return Ok(Round::Repainted);
        }
        if self.stop_news() {
            return Ok(Round::Stopped);
        }
        let wait = if self.attached() {
            wanted
        } else if self.untold() {
            // Its first word would not wake the wait.
            DETACHED_POLL
        } else {
            wanted.max(DETACHED_POLL)
        };
        if !event::poll(wait)? {
            return Ok(if self.stop_news() {
                Round::Stopped
            } else {
                Round::Timeout
            });
        }
        let event = event::read()?;
        // An attach woke the wait: the screen repainted before the event
        // is taken and drawn.
        if self.take_repaint() {
            self.repaint_with(terminal, on_repaint)?;
            *lock(&self.inner.pending) = Some(event);
            return Ok(Round::Repainted);
        }
        if self.stop_news() {
            *lock(&self.inner.pending) = Some(event);
            return Ok(Round::Stopped);
        }
        Ok(Round::Event(event))
    }

    /// Whether a stop came that no round of this screen has said yet; it
    /// is said now.
    fn stop_news(&self) -> bool {
        self.stop_requested() && !self.inner.stop_said.swap(true, SeqCst)
    }

    /// Draw a frame, as `terminal.draw` does, while a terminal is there to
    /// draw on ([`Session::attached`]); nothing while detached.
    pub fn draw(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        render: impl FnOnce(&mut ratatui::Frame),
    ) -> io::Result<()> {
        if self.attached() {
            terminal.draw(render)?;
        }
        Ok(())
    }
}

/// The directory and id of the session named in the environment.
#[cfg(sessions)]
fn session_at() -> Option<(PathBuf, String)> {
    let name = std::env::var(SESSION_VAR).ok()?;
    dir::check_name(&name).ok()?;
    Some((dir::runtime_dir().ok()?, name))
}

impl Drop for Session {
    fn drop(&mut self) {
        if self.owner {
            self.restore();
        }
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
