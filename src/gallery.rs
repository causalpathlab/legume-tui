//! What a viewer has saved: a log of each figure and a small thumbnail of
//! it, kept in a directory the viewer names (`.senna-view/`,
//! `.pinto-view/`, `.lupin-view/`) where it runs, so the list lasts from
//! one session to the next. The runs' own directories are not touched.
//!
//! The store is senna's (`src/view/saved.rs`: newest first, a save of the
//! same file replacing the old, thumbnails nothing refers to deleted (here
//! once a while old, and never when the log could not be read), the log
//! read again before each change so two viewers in one directory keep
//! each other's saves, `relocate` into a folder) with pinto's thumbnails
//! (`src/view/saved.rs`: [`thumb::shrink`] in linear light, never
//! enlarged). lupin's (`src/tui/gallery.rs`) comes in through a
//! [`Policy`]: entries whose files are gone are kept and reported by
//! [`Gallery::check`], a log that cannot be read is set aside rather than
//! emptied, the log is private and nothing is moved or deleted on the word
//! of one that is not this user's alone, and with feature `gallery-trace`
//! each file is recorded with its size, time and SHA-256. What a viewer
//! keeps beside each entry (lupin's panel and manifest) is the entry's
//! [`Entry::extra`], flattened into it on disk.
//!
//! The logs senna, pinto and lupin wrote before load as they are, and a
//! plain policy writes senna's and pinto's fields and no more.

pub mod strip;
pub mod thumb;

use image::RgbaImage;
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::io;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// Width of a thumbnail on disk, in pixels.
pub const THUMB_WIDTH: u32 = 240;
/// Saves remembered; older ones go, with their thumbnails.
pub const KEEP: usize = 200;
/// The log's file name in the gallery's directory.
pub const LOG: &str = "saved.json";
/// The thumbnails' directory in the gallery's.
const THUMBS: &str = "thumbs";
/// Seconds a thumbnail no entry names is kept: another viewer may have
/// written it and not yet its log.
pub const ORPHAN_AGE: u64 = 600;

/// What happens to an entry whose file is gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Missing {
    /// Dropped when the log is read, with its thumbnail (senna, pinto).
    Drop,
    /// Kept, and reported by [`Gallery::check`] (lupin).
    Keep,
}

/// What happens to a log that cannot be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BadLog {
    /// The gallery starts empty, and the next save writes over it.
    Empty,
    /// It is moved aside to `saved.json.bad` ([`Gallery::set_aside`]).
    SetAside,
}

/// How a viewer keeps its gallery.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Policy {
    pub missing: Missing,
    pub on_bad_log: BadLog,
    /// The directory and log are made private, and files are moved or
    /// deleted only on the word of a log this user alone can write.
    pub private: bool,
    /// Each file of a save is recorded, with its size, time and (with
    /// feature `gallery-trace`) SHA-256; a file is moved or deleted only
    /// while it is the one recorded.
    pub hash_files: bool,
}

impl Policy {
    /// senna's and pinto's: what is gone goes, a bad log starts afresh.
    #[must_use]
    pub const fn plain() -> Self {
        Self {
            missing: Missing::Drop,
            on_bad_log: BadLog::Empty,
            private: false,
            hash_files: false,
        }
    }

    /// lupin's: every file traced and checked, nothing dropped, a bad log
    /// set aside, the log private.
    #[cfg(feature = "gallery-trace")]
    #[must_use]
    pub const fn traced() -> Self {
        Self {
            missing: Missing::Keep,
            on_bad_log: BadLog::SetAside,
            private: true,
            hash_files: true,
        }
    }
}

impl Default for Policy {
    fn default() -> Self {
        Self::plain()
    }
}

/// One file of a save, as it was written.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct FileRecord {
    pub path: PathBuf,
    pub size: u64,
    /// Modification time, seconds since the Unix epoch; a quick check
    /// before the hash.
    #[serde(default)]
    pub mtime: u64,
    /// SHA-256 of the contents, hex; empty without feature
    /// `gallery-trace`.
    #[serde(default)]
    pub hash: String,
}

impl FileRecord {
    /// The file at `path` as it is now; its contents are read, a piece
    /// at a time, only to hash them.
    pub fn of(path: &Path) -> io::Result<Self> {
        let reading =
            |e: io::Error| io::Error::new(e.kind(), format!("reading {}: {e}", path.display()));
        let m = std::fs::metadata(path).map_err(reading)?;
        Ok(Self {
            path: absolute(path),
            size: m.len(),
            mtime: mtime_of(&m),
            hash: hash(path).map_err(reading)?,
        })
    }

    /// The file on disk is still this record: same size and mtime, else the
    /// same hash; `None` when it is gone.
    fn unchanged(&self) -> Option<bool> {
        let m = std::fs::metadata(&self.path).ok()?;
        if m.len() != self.size {
            return Some(false);
        }
        if mtime_of(&m) == self.mtime && self.mtime != 0 {
            return Some(true);
        }
        Some(!self.hash.is_empty() && Self::of(&self.path).is_ok_and(|now| now.hash == self.hash))
    }
}

/// The SHA-256 of the file at `path` in hex, read a piece at a time.
#[cfg(feature = "gallery-trace")]
fn hash(path: &Path) -> io::Result<String> {
    use sha2::{Digest, Sha256};
    use std::io::Read;
    let mut f = std::fs::File::open(path)?;
    let mut hasher = Sha256::new();
    let mut buf = vec![0; 1 << 16];
    loop {
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => hasher.update(&buf[..n]),
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(hasher
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect())
}

/// No hash, and nothing read, without feature `gallery-trace`.
#[cfg(not(feature = "gallery-trace"))]
fn hash(_: &Path) -> io::Result<String> {
    Ok(String::new())
}

/// One save.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Entry<X = ()> {
    /// The file, as an absolute path: the PDF of a set.
    pub path: PathBuf,
    /// What was saved, as the save dialog put it.
    pub what: String,
    /// When, in seconds since the Unix epoch.
    pub when: u64,
    /// The thumbnail's file name in `thumbs/`, if one was made.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thumb: Option<String>,
    /// Every file of the save, recorded under [`Policy::hash_files`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<FileRecord>,
    /// What the viewer keeps beside it.
    #[serde(flatten)]
    pub extra: X,
}

impl<X> Entry<X> {
    /// The file's name.
    #[must_use]
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default()
    }

    /// The files a move or delete acts on: those recorded, else the one.
    fn targets(&self) -> Vec<PathBuf> {
        if self.files.is_empty() {
            vec![self.path.clone()]
        } else {
            self.files.iter().map(|f| f.path.clone()).collect()
        }
    }
}

/// What a check finds about an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Status {
    Intact,
    Changed,
    Missing,
}

impl Status {
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Intact => "ok",
            Self::Changed => "changed since export",
            Self::Missing => "missing",
        }
    }

    /// The row's colour; `None` for the default.
    #[must_use]
    pub fn color(self) -> Option<ratatui::style::Color> {
        use ratatui::style::Color;
        match self {
            Self::Intact => None,
            Self::Changed => Some(Color::Yellow),
            Self::Missing => Some(Color::Red),
        }
    }
}

/// Every save remembered in one directory, newest first.
#[derive(Clone, Debug)]
pub struct Gallery<X = ()> {
    dir: PathBuf,
    policy: Policy,
    entries: Vec<Entry<X>>,
    /// A log that could not be read was moved aside to this file.
    set_aside: Option<PathBuf>,
}

impl<X> Gallery<X> {
    /// The directory the log is in.
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    #[must_use]
    pub fn policy(&self) -> Policy {
        self.policy
    }

    #[must_use]
    pub fn entries(&self) -> &[Entry<X>] {
        &self.entries
    }

    /// Where a log that could not be read was moved, since this gallery
    /// was opened.
    #[must_use]
    pub fn set_aside(&self) -> Option<&Path> {
        self.set_aside.as_deref()
    }

    /// The thumbnail of `e`, if it has one.
    #[must_use]
    pub fn thumb(&self, e: &Entry<X>) -> Option<PathBuf> {
        e.thumb.as_ref().map(|t| self.dir.join(THUMBS).join(t))
    }

    fn drop_thumb(&self, e: &Entry<X>) {
        if let Some(t) = self.thumb(e) {
            let _ = std::fs::remove_file(t);
        }
    }
}

impl<X: Serialize + DeserializeOwned> Gallery<X> {
    /// The saves logged in `dir`. Under [`Missing::Drop`] those whose file
    /// or thumbnail is gone are left out, with their thumbnails; thumbnails
    /// no entry names are deleted once [`ORPHAN_AGE`] old, and only when
    /// the log was read. A log that cannot be read starts an empty gallery,
    /// set aside first under [`BadLog::SetAside`].
    pub fn open(dir: &Path, policy: Policy) -> Self {
        let log = dir.join(LOG);
        let mut set_aside = None;
        let mut read = true;
        let logged: Vec<Entry<X>> = match std::fs::read(&log) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_else(|_| {
                read = false;
                if policy.on_bad_log == BadLog::SetAside {
                    let bad = dir.join(format!("{LOG}.bad"));
                    if std::fs::rename(&log, &bad).is_ok() {
                        set_aside = Some(bad);
                    }
                }
                Vec::new()
            }),
            Err(e) => {
                read = e.kind() == io::ErrorKind::NotFound;
                Vec::new()
            }
        };
        let thumbs = dir.join(THUMBS);
        let entries: Vec<Entry<X>> = match policy.missing {
            Missing::Keep => logged,
            Missing::Drop => {
                let (kept, gone): (Vec<_>, Vec<_>) = logged.into_iter().partition(|e| {
                    e.path.exists() && e.thumb.as_ref().is_none_or(|t| thumbs.join(t).exists())
                });
                for e in &gone {
                    if let Some(t) = &e.thumb {
                        let _ = std::fs::remove_file(thumbs.join(t));
                    }
                }
                kept
            }
        };
        // Another viewer writes its thumbnail before its log, so a young
        // one no entry names yet is spared; with no log read, all are.
        if let (true, Ok(listing)) = (read, std::fs::read_dir(&thumbs)) {
            let kept: std::collections::HashSet<&str> =
                entries.iter().filter_map(|e| e.thumb.as_deref()).collect();
            let old = |f: &std::fs::DirEntry| {
                f.metadata()
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age.as_secs() >= ORPHAN_AGE)
            };
            for f in listing.filter_map(Result::ok) {
                if !kept.contains(f.file_name().to_string_lossy().as_ref()) && old(&f) {
                    let _ = std::fs::remove_file(f.path());
                }
            }
        }
        Self {
            dir: dir.into(),
            policy,
            entries,
            set_aside,
        }
    }

    /// The gallery in directory `name` (`.senna-view`) where the viewer
    /// runs.
    pub fn here(name: &str, policy: Policy) -> Self {
        Self::open(Path::new(name), policy)
    }

    /// The log as it is on disk now: another viewer may have saved since.
    pub fn reload(&mut self) {
        let fresh = Self::open(&self.dir, self.policy);
        self.entries = fresh.entries;
        if fresh.set_aside.is_some() {
            self.set_aside = fresh.set_aside;
        }
    }

    /// Remember that `files` were just saved, showing `picture`: at the
    /// top, replacing an earlier save of the same file. The entry is the
    /// PDF among them, else the first. The log is read again first, so two
    /// viewers in one directory keep each other's saves.
    pub fn add(
        &mut self,
        files: &[impl AsRef<Path>],
        what: &str,
        picture: Option<&RgbaImage>,
        extra: X,
    ) -> io::Result<()> {
        self.reload();
        // A private log this user does not own alone is not taken over by
        // saving over it: its entries would then be trusted to move and
        // delete files.
        if self.policy.private && std::fs::symlink_metadata(self.dir.join(LOG)).is_ok() {
            self.entries = read_private(&self.dir)?;
        }
        let files: Vec<&Path> = files.iter().map(AsRef::as_ref).collect();
        let main = files
            .iter()
            .find(|f| f.extension().is_some_and(|e| e == "pdf"))
            .or(files.first())
            .ok_or_else(|| io::Error::other("nothing was saved"))?;
        let path = main.canonicalize()?;
        let records = if self.policy.hash_files {
            files
                .iter()
                .map(|f| FileRecord::of(f))
                .collect::<io::Result<Vec<_>>>()?
        } else {
            Vec::new()
        };
        self.make_dir()?;
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(io::Error::other)?;
        let thumb = match picture {
            Some(p) => {
                let thumbs = self.dir.join(THUMBS);
                std::fs::create_dir_all(&thumbs)?;
                let name = format!("{}.png", now.as_nanos());
                thumb::write_png(&thumb::shrink(p, THUMB_WIDTH), &thumbs.join(&name))?;
                Some(name)
            }
            None => None,
        };
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            let old = self.entries.remove(i);
            self.drop_thumb(&old);
        }
        self.entries.insert(
            0,
            Entry {
                path,
                what: what.into(),
                when: now.as_secs(),
                thumb,
                files: records,
                extra,
            },
        );
        for old in self.entries.split_off(self.entries.len().min(KEEP)) {
            self.drop_thumb(&old);
        }
        self.write()
    }

    /// Each entry against the files on disk: a file whose size and mtime
    /// are as recorded is taken as unchanged without reading it. An entry
    /// with no files recorded is intact while its file is there.
    #[must_use]
    pub fn check(&self) -> Vec<Status> {
        self.entries
            .iter()
            .map(|e| {
                if e.files.is_empty() {
                    return if e.path.exists() {
                        Status::Intact
                    } else {
                        Status::Missing
                    };
                }
                let mut status = Status::Intact;
                for f in &e.files {
                    match f.unchanged() {
                        None => return Status::Missing,
                        Some(false) => status = Status::Changed,
                        Some(true) => {}
                    }
                }
                status
            })
            .collect()
    }

    /// Take the save of `path` off the list, with its thumbnail; with
    /// `delete`, its files go too.
    pub fn remove(&mut self, path: &Path, delete: bool) -> io::Result<()> {
        let i = self.find(path)?;
        if delete {
            self.verify(&self.entries[i])?;
        }
        let old = self.entries.remove(i);
        if delete {
            for f in old.targets() {
                std::fs::remove_file(&f).map_err(|e| {
                    io::Error::new(e.kind(), format!("deleting {}: {e}", f.display()))
                })?;
            }
        }
        self.drop_thumb(&old);
        self.write()
    }

    /// Move the save of `path` to `to`, keeping its place in the list.
    /// `to` may be a folder (one that exists, or ends in `/`): the files
    /// keep their names there. Otherwise it is the new name, with or
    /// without the file's extension; every file of a set moves to it with
    /// its own extension. A file already there is never replaced, and a
    /// failed move puts back the files already moved. Returns where the
    /// listed file is now.
    pub fn relocate(&mut self, path: &Path, to: &Path) -> io::Result<PathBuf> {
        let i = self.find(path)?;
        self.verify(&self.entries[i])?;
        let e = &self.entries[i];
        let base = new_base(&e.path, to);
        let from = e.targets();
        let targets: Vec<PathBuf> = from.iter().map(|f| with_ext_of(&base, f)).collect();
        for t in &targets {
            if t.exists() {
                return Err(io::Error::new(
                    io::ErrorKind::AlreadyExists,
                    format!("{} exists", t.display()),
                ));
            }
        }
        if let Some(parent) = base.parent().filter(|p| !p.as_os_str().is_empty()) {
            std::fs::create_dir_all(parent)?;
        }
        for (k, (f, t)) in from.iter().zip(&targets).enumerate() {
            if let Err(err) = move_file(f, t) {
                for (g, u) in from.iter().zip(&targets).take(k) {
                    let _ = move_file(u, g);
                }
                return Err(io::Error::new(
                    err.kind(),
                    format!("moving {} to {}: {err}", f.display(), t.display()),
                ));
            }
        }
        let main = with_ext_of(&base, &e.path).canonicalize()?;
        let e = &mut self.entries[i];
        for (f, t) in e.files.iter_mut().zip(&targets) {
            f.path = absolute(t);
        }
        e.path = main.clone();
        self.write()?;
        Ok(main)
    }

    /// Where `path` is listed, in the log as it is on disk (another viewer
    /// may have changed it since the row on screen was drawn); under a
    /// private policy, read only from a log this user alone can write, as
    /// files are moved or deleted on its word.
    fn find(&mut self, path: &Path) -> io::Result<usize> {
        if self.policy.private {
            self.entries = read_private(&self.dir)?;
        } else {
            self.reload();
        }
        self.entries
            .iter()
            .position(|e| e.path == path)
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("{} is no longer listed", path.display()),
                )
            })
    }

    /// Under [`Policy::hash_files`], every file of `e` is the save it was
    /// logged as: beside the listed file with the same base name, and
    /// unchanged since. Nothing is deleted or moved on the log's word alone.
    fn verify(&self, e: &Entry<X>) -> io::Result<()> {
        if !self.policy.hash_files {
            return Ok(());
        }
        let not_there = |p: &Path, err: io::Error| {
            io::Error::new(err.kind(), format!("{} is not there", p.display()))
        };
        let base = std::fs::canonicalize(&e.path)
            .map_err(|err| not_there(&e.path, err))?
            .with_extension("");
        for f in &e.files {
            let c = std::fs::canonicalize(&f.path).map_err(|err| not_there(&f.path, err))?;
            if c.with_extension("") != base {
                return Err(io::Error::other(format!(
                    "{} is not a file of the save {}",
                    c.display(),
                    e.path.display()
                )));
            }
            if f.hash.is_empty() || FileRecord::of(&c)?.hash != f.hash {
                return Err(io::Error::other(format!(
                    "{} has changed since it was saved; not touching it",
                    c.display()
                )));
            }
        }
        Ok(())
    }

    /// The gallery's directory, made private under a private policy (an
    /// existing one is left as it is).
    fn make_dir(&self) -> io::Result<()> {
        let mut make = std::fs::DirBuilder::new();
        make.recursive(true);
        #[cfg(unix)]
        if self.policy.private {
            std::os::unix::fs::DirBuilderExt::mode(&mut make, 0o700);
        }
        make.create(&self.dir)
    }

    /// Write the log: to a fresh temporary file (created exclusively,
    /// under a random name, readable by this user only), then into place.
    fn write(&self) -> io::Result<()> {
        self.make_dir()?;
        let mut tmp = tempfile::NamedTempFile::new_in(&self.dir)?;
        serde_json::to_writer_pretty(&mut tmp, &self.entries)?;
        tmp.persist(self.dir.join(LOG)).map_err(|e| e.error)?;
        Ok(())
    }
}

/// The entries of the log in `dir`, only when the directory and the log are
/// this user's, no one else can write them, and neither is a link: a log
/// another user can edit, or swap for another, could name any of this
/// user's files, and files are deleted or moved on its word. The log is
/// checked on the file that was opened, and that file must be the one at
/// the log's path.
fn read_private<X: DeserializeOwned>(dir: &Path) -> io::Result<Vec<Entry<X>>> {
    use std::io::Read;
    let log = dir.join(LOG);
    let mut f = std::fs::File::open(&log)
        .map_err(|e| io::Error::new(e.kind(), format!("opening {}: {e}", log.display())))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        // This process's user: the owner of a file it has just created.
        let me = tempfile::tempfile()?.metadata()?.uid();
        let mine = |m: &std::fs::Metadata| m.uid() == me && m.mode() & 0o022 == 0;
        let refuse = |p: &Path| {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "{} is not this user's alone (a link, or others can write it): not \
                     deleting or moving files on its word (chmod go-w, or remove it to start \
                     a new log)",
                    p.display()
                ),
            )
        };
        let d = std::fs::symlink_metadata(dir)?;
        if d.file_type().is_symlink() || !d.is_dir() || !mine(&d) {
            return Err(refuse(dir));
        }
        let (at, opened) = (std::fs::symlink_metadata(&log)?, f.metadata()?);
        let same = at.dev() == opened.dev() && at.ino() == opened.ino();
        if at.file_type().is_symlink() || !opened.is_file() || !same || !mine(&opened) {
            return Err(refuse(&log));
        }
    }
    let mut bytes = Vec::new();
    f.read_to_end(&mut bytes)?;
    serde_json::from_slice(&bytes).map_err(|e| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("reading {}: {e}", log.display()),
        )
    })
}

/// The name, without extension, a file at `from` takes when moved to
/// `to`: in `to` under its own when `to` is a folder, else `to` without
/// `from`'s extension.
fn new_base(from: &Path, to: &Path) -> PathBuf {
    let typed = to.to_string_lossy();
    if to.is_dir() || typed.ends_with('/') {
        return to.join(from.file_stem().unwrap_or_default());
    }
    match from.extension() {
        Some(ext) => {
            let ext = format!(".{}", ext.to_string_lossy());
            PathBuf::from(typed.strip_suffix(&ext).unwrap_or(&typed))
        }
        None => to.to_path_buf(),
    }
}

/// `base` with the extension of `file`, if it has one.
fn with_ext_of(base: &Path, file: &Path) -> PathBuf {
    match file.extension() {
        Some(ext) => PathBuf::from(format!("{}.{}", base.display(), ext.to_string_lossy())),
        None => base.to_path_buf(),
    }
}

/// Move `from` to `to`; across file systems a rename fails, so copy, then
/// remove.
fn move_file(from: &Path, to: &Path) -> io::Result<()> {
    if std::fs::rename(from, to).is_err() {
        std::fs::copy(from, to)?;
        std::fs::remove_file(from)?;
    }
    Ok(())
}

/// `p` resolved, or joined to the working directory when it cannot be.
fn absolute(p: &Path) -> PathBuf {
    p.canonicalize().unwrap_or_else(|_| {
        std::env::current_dir()
            .map(|d| d.join(p))
            .unwrap_or_else(|_| p.to_path_buf())
    })
}

/// Modification time of `m`, seconds since the Unix epoch.
fn mtime_of(m: &std::fs::Metadata) -> u64 {
    m.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |d| d.as_secs())
}

/// Seconds since the Unix epoch.
#[must_use]
pub fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// How long ago `when` was, at `now` (both seconds since the epoch).
#[must_use]
pub fn ago(when: u64, now: u64) -> String {
    match now.saturating_sub(when) {
        s if s < 60 => "just now".into(),
        s if s < 3600 => format!("{} min ago", s / 60),
        s if s < 86_400 => format!("{} h ago", s / 3600),
        s => format!("{} d ago", s / 86_400),
    }
}

#[cfg(test)]
#[path = "tests/gallery.rs"]
mod tests;

#[cfg(all(test, feature = "gallery-trace"))]
#[path = "tests/gallery_trace.rs"]
mod trace_tests;
