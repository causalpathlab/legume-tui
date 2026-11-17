//! A file browser for a popup or a pane, one folder at a time: its
//! folders, then the files that are wanted, each with a line about it,
//! narrowed to what has been typed, typed paths followed.
//!
//! What is wanted, how a file is described, which one to start on and why
//! one is refused are the caller's, through [`Wanted`]; the browser lists
//! (on a worker thread, so describing can be slow), moves, narrows, marks
//! and draws in a [`Look`]'s styles. [`Simple`] is the plain case: files
//! kept by name, each with a note. A `.zarr` store is a folder on disk but
//! a file here ([`STORES`]): never opened, offered through
//! [`Wanted::store`].
//!
//! From senna's and pinto's browsers, with faba's (`Find`, `FindList` and
//! the path helpers) and lupin's picker (files described behind the
//! listing, every file on request).

use crate::look::Look;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use std::borrow::Cow;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

/// One spelling for a path: absolute, with `.` and `..` resolved by name.
/// Symlinks are kept as given, so a linked file keeps the name it was given.
#[must_use]
pub fn normalize(path: &Path) -> PathBuf {
    let abs = std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf());
    let mut out = PathBuf::new();
    for c in abs.components() {
        match c {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            c => out.push(c),
        }
    }
    out
}

/// The home folder, if known.
#[must_use]
pub fn home() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from)
}

/// `p` with the home folder as `~`.
#[must_use]
pub fn tilde(p: &Path) -> String {
    if let Some(rest) = home().and_then(|h| p.strip_prefix(h).ok().map(Path::to_path_buf)) {
        return if rest.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", rest.display())
        };
    }
    p.display().to_string()
}

/// `p` relative to the working directory when it is under it, else as
/// given.
#[must_use]
pub fn relative(p: &Path) -> PathBuf {
    std::env::current_dir()
        .ok()
        .and_then(|cwd| p.strip_prefix(cwd).ok().map(Path::to_path_buf))
        .filter(|r| !r.as_os_str().is_empty())
        .unwrap_or_else(|| p.to_path_buf())
}

/// A path as shown: relative to the working directory when it is under
/// it (see [`relative`]).
#[must_use]
pub fn shown(p: &Path) -> String {
    relative(p).to_string_lossy().into_owned()
}

/// A file's size as a browser notes it, `12.3 MB`; none when it cannot be
/// read.
#[must_use]
pub fn size(path: &Path) -> Option<String> {
    let bytes = std::fs::metadata(path).ok()?.len() as f64;
    let (v, unit) = [("kB", 1e3), ("MB", 1e6), ("GB", 1e9)]
        .iter()
        .rev()
        .find(|(_, s)| bytes >= *s)
        .map_or((bytes, "B"), |(u, s)| (bytes / s, *u));
    Some(if unit == "B" {
        format!("{v:.0} B")
    } else {
        format!("{v:.1} {unit}")
    })
}

/// A path as typed into a line: `~` is home, and the rest is made absolute
/// with `.` and `..` resolved (see [`normalize`]).
#[must_use]
pub fn typed_path(text: &str) -> PathBuf {
    let path = match (text.strip_prefix('~'), home()) {
        (Some(rest), Some(h)) if rest.is_empty() || rest.starts_with('/') => {
            h.join(rest.trim_start_matches('/'))
        }
        _ => PathBuf::from(text),
    };
    normalize(&path)
}

/// Item `at` moved by `d`, kept within `0..=last`.
pub use crate::keys::moved;

/// The first of `rows` visible items of a `len`-long list that keeps item
/// `at` near the middle.
#[must_use]
pub fn first_visible(at: usize, len: usize, rows: usize) -> usize {
    at.saturating_sub(rows.saturating_sub(1) / 2)
        .min(len.saturating_sub(rows))
}

/// The `rows` lines of `lines` around line `at`.
#[must_use]
pub fn window(lines: Vec<Line<'static>>, at: usize, rows: usize) -> Vec<Line<'static>> {
    let first = first_visible(at, lines.len(), rows);
    lines.into_iter().skip(first).take(rows).collect()
}

/// What has been typed into a list to narrow it: letters, digits and
/// punctuation type, so a file pane takes no letter keys of its own.
#[derive(Default, Debug, Clone)]
pub struct Find {
    pub text: String,
}

impl Find {
    /// Take `key` if it edits the text: a typed character (not Space, which
    /// panes keep for choosing, nor a chord), or Backspace and Esc while
    /// there is text.
    pub fn key(&mut self, key: &KeyEvent) -> bool {
        let chord = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        match key.code {
            KeyCode::Char(c) if c != ' ' && !chord => self.text.push(c),
            KeyCode::Backspace if !self.text.is_empty() => {
                self.text.pop();
            }
            KeyCode::Esc if !self.text.is_empty() => self.text.clear(),
            _ => return false,
        }
        true
    }

    /// Where `key` jumps when typing a path: `/` with nothing typed to the
    /// root, `~` to home, and `/` after a name to that folder in `cwd` (the
    /// name as typed, else `dir`, the highlighted folder), so typing
    /// `/data/runs/` walks there.
    #[must_use]
    pub fn jump(&self, key: &KeyEvent, cwd: &Path, dir: Option<PathBuf>) -> Option<PathBuf> {
        let KeyCode::Char(c @ ('/' | '~')) = key.code else {
            return None;
        };
        match (c, self.text.as_str()) {
            ('/', "") => cwd.ancestors().last().map(Path::to_path_buf),
            ('~', "") => home(),
            ('/', "..") => cwd.parent().map(Path::to_path_buf),
            ('/', name) => Some(cwd.join(name)).filter(|p| p.is_dir()).or(dir),
            _ => None,
        }
    }

    /// A pane title's tail naming the text, if any.
    #[must_use]
    pub fn tag(&self) -> String {
        if self.text.is_empty() {
            String::new()
        } else {
            format!("· find: {} ", self.text)
        }
    }
}

/// What a narrowed list says when nothing matches the typed text.
pub const NOTHING_MATCHES: &str = "nothing matches; Backspace or Esc";

/// A list of named items narrowed by what has been typed, with a cursor
/// over those shown.
pub struct FindList<T> {
    all: Vec<T>,
    /// `all`'s names in lower case, to match against.
    lower: Vec<String>,
    /// Indices into `all` of the items shown.
    shown: Vec<usize>,
    /// The cursor, among the items shown.
    pub at: usize,
    pub find: Find,
}

impl<T> Default for FindList<T> {
    fn default() -> Self {
        Self {
            all: Vec::new(),
            lower: Vec::new(),
            shown: Vec::new(),
            at: 0,
            find: Find::default(),
        }
    }
}

impl<T: AsRef<str>> FindList<T> {
    /// Show all of `all`, with nothing typed and the cursor at `at`.
    pub fn set(&mut self, all: Vec<T>, at: usize) {
        self.lower = all.iter().map(|x| x.as_ref().to_lowercase()).collect();
        self.shown = (0..all.len()).collect();
        self.all = all;
        self.at = at.min(self.shown.len().saturating_sub(1));
        self.find = Find::default();
    }

    /// The items shown, in order.
    pub fn shown(&self) -> impl Iterator<Item = &T> {
        self.shown.iter().map(|&i| &self.all[i])
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.shown.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.shown.is_empty()
    }

    /// The item under the cursor.
    #[must_use]
    pub fn highlighted(&self) -> Option<&T> {
        self.shown.get(self.at).map(|&i| &self.all[i])
    }

    pub fn step(&mut self, d: isize) {
        self.at = moved(self.at, d, self.len().saturating_sub(1));
    }

    /// Put the cursor on the item shown named `name`; whether there is one.
    pub fn select(&mut self, name: &str) -> bool {
        let at = self.shown().position(|x| x.as_ref() == name);
        self.at = at.unwrap_or(self.at);
        at.is_some()
    }

    /// Take `key` if it edits the typed text (see [`Find::key`]) or moves
    /// the cursor (↑/↓, PgUp/PgDn; letters type, so not `j`/`k`).
    pub fn key(&mut self, key: &KeyEvent) -> bool {
        let was = self.shown.get(self.at).copied();
        if self.find.key(key) {
            self.narrow();
            // Back to the whole list: still on the item it was on.
            if self.find.text.is_empty() {
                if let Some(at) = was.and_then(|w| self.shown.iter().position(|&i| i == w)) {
                    self.at = at;
                }
            }
            return true;
        }
        let d = match key.code {
            KeyCode::Up => -1,
            KeyCode::Down => 1,
            KeyCode::PageUp => -10,
            KeyCode::PageDown => 10,
            _ => return false,
        };
        self.step(d);
        true
    }

    /// Show the items holding the typed text, any case, never `..`; the
    /// cursor on the first that starts with it, else the first.
    fn narrow(&mut self) {
        let text = self.find.text.to_lowercase();
        let lower = &self.lower;
        self.shown = (0..lower.len())
            .filter(|&i| text.is_empty() || (lower[i] != ".." && lower[i].contains(&text)))
            .collect();
        self.at = self
            .shown
            .iter()
            .position(|&i| lower[i].starts_with(&text))
            .unwrap_or(0);
    }

    /// What an empty list says: `empty`, or that nothing matches.
    #[must_use]
    pub fn empty_note<'a>(&self, empty: &'a str) -> &'a str {
        if self.find.text.is_empty() {
            empty
        } else {
            NOTHING_MATCHES
        }
    }
}

/// The suffixes of folders listed as files: a zarr store is a folder, but
/// it is one matrix.
pub const STORES: [&str; 1] = [".zarr"];

/// What a browser looks for, and how it speaks of it. Cloned to the worker
/// that lists a folder.
pub trait Wanted: Clone + Send + 'static {
    /// What is shown about a listed file or folder: its size, a run's
    /// kind, a panel's counts.
    type About: Clone + Send + 'static;

    /// The popup's words.
    fn header(&self) -> Header;

    /// The file `name` at `path` with what to show of it, if it is wanted.
    /// Called on the worker thread; it may read the file.
    fn file(&self, path: &Path, name: &str) -> Option<Self::About>;

    /// The `.zarr` store `name` at `path` as a file, when stores are wanted.
    fn store(&self, _path: &Path, _name: &str) -> Option<Self::About> {
        None
    }

    /// What to show beside folder `name` at `path`: a folder of outputs
    /// can say so. Folders are listed either way.
    fn folder(&self, _path: &Path, _name: &str) -> Option<Self::About> {
        None
    }

    /// One line about a file or folder.
    fn describe<'a>(&self, about: &'a Self::About) -> Cow<'a, str>;

    /// A file's row given the widest name shown, `name_w`: by default the
    /// name padded to it, then [`Self::describe`]'s line ([`name_then`]).
    /// A row that starts with the name has the rest drawn as a hint.
    fn row(&self, name: &str, about: &Self::About, name_w: usize) -> String {
        name_then(name, &self.describe(about), name_w)
    }

    /// Why file `name` cannot be taken, though it is listed.
    fn refuse(&self, _name: &str, _about: &Self::About) -> Option<Cow<'static, str>> {
        None
    }

    /// The file to start on among those listed in `dir`, starred when the
    /// header says what a star means.
    fn best(&self, _dir: &Path, _files: &[(&str, &Self::About)]) -> Option<String> {
        None
    }

    /// Whether space marks several files to take together.
    fn many(&self) -> bool {
        false
    }

    /// A key of the caller's own, such as an option it offers while
    /// browsing (its header says which). Returns whether it was one.
    fn key(&mut self, _k: &KeyEvent) -> bool {
        false
    }
}

/// A row of the name padded to `name_w`, then `about`; just the name when
/// there is nothing to say.
#[must_use]
pub fn name_then(name: &str, about: &str, name_w: usize) -> String {
    if about.is_empty() {
        name.to_string()
    } else {
        format!("{name:<name_w$}  {about}")
    }
}

/// What the popup says about what is wanted: [`Header::new`], then what
/// else it says.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Header {
    pub title: Cow<'static, str>,
    /// Lines under the title.
    pub notes: Vec<Cow<'static, str>>,
    /// The files wanted, as in "no {what} here".
    pub what: Cow<'static, str>,
    /// What the star by the starting file means; no star without it.
    pub star: Option<Cow<'static, str>>,
    /// What enter does to a file: "take", "open", "annotate".
    pub verb: Cow<'static, str>,
    /// A line naming the rows' columns, over them.
    pub columns: Option<Cow<'static, str>>,
}

impl Header {
    /// A popup titled `title` for `what`, whose enter takes a file.
    #[must_use]
    pub fn new(title: impl Into<Cow<'static, str>>, what: impl Into<Cow<'static, str>>) -> Self {
        Self {
            title: title.into(),
            what: what.into(),
            verb: "take".into(),
            ..Self::default()
        }
    }

    /// With a line under the title.
    #[must_use]
    pub fn note(mut self, note: impl Into<Cow<'static, str>>) -> Self {
        self.notes.push(note.into());
        self
    }

    /// With a star by the starting file, meaning `star`.
    #[must_use]
    pub fn star(mut self, star: impl Into<Cow<'static, str>>) -> Self {
        self.star = Some(star.into());
        self
    }

    /// With enter doing `verb` to a file.
    #[must_use]
    pub fn verb(mut self, verb: impl Into<Cow<'static, str>>) -> Self {
        self.verb = verb.into();
        self
    }

    /// With a line naming the rows' columns.
    #[must_use]
    pub fn columns(mut self, columns: impl Into<Cow<'static, str>>) -> Self {
        self.columns = Some(columns.into());
        self
    }
}

/// One listed name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Entry<A> {
    /// `..`, the parent.
    Up,
    /// A folder, with what [`Wanted::folder`] said of it.
    Dir(String, Option<A>),
    /// A wanted file.
    File(String, A),
    /// A file not wanted, listed only while [`Browser::all`] is on.
    Other(String),
}

impl<A> Entry<A> {
    #[must_use]
    pub fn name(&self) -> &str {
        match self {
            Entry::Up => "..",
            Entry::Dir(n, _) | Entry::File(n, _) | Entry::Other(n) => n,
        }
    }

    /// A wanted file.
    #[must_use]
    pub fn is_file(&self) -> bool {
        matches!(self, Entry::File(..))
    }

    /// A folder or `..`.
    #[must_use]
    pub fn is_dir(&self) -> bool {
        matches!(self, Entry::Up | Entry::Dir(..))
    }

    /// `..`, then folders, then files.
    fn rank(&self) -> u8 {
        match self {
            Entry::Up => 0,
            Entry::Dir(..) => 1,
            Entry::File(..) | Entry::Other(_) => 2,
        }
    }
}

/// What a key did in the browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Not a browser key; the caller's to handle.
    Ignored,
    /// It moved, opened a folder, narrowed, marked or said why not.
    Moved,
    /// Esc with nothing typed.
    Cancelled,
    Chosen(Chosen),
}

/// Files chosen, never none: the one under the cursor, or, where several
/// can be taken, the marked ones.
#[derive(Debug, PartialEq, Eq)]
pub struct Chosen {
    first: PathBuf,
    rest: Vec<PathBuf>,
}

impl Chosen {
    fn one(path: PathBuf) -> Self {
        Chosen {
            first: path,
            rest: Vec::new(),
        }
    }

    /// The file, where one is taken at a time.
    #[must_use]
    pub fn file(self) -> PathBuf {
        debug_assert!(
            self.rest.is_empty(),
            "several files chosen where one is taken"
        );
        self.first
    }

    /// Every file taken, in order.
    #[must_use]
    pub fn files(self) -> Vec<PathBuf> {
        std::iter::once(self.first).chain(self.rest).collect()
    }
}

/// Files marked to take together, from any folder, in the order marked.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Marks {
    order: Vec<PathBuf>,
    set: HashSet<PathBuf>,
}

impl Marks {
    /// Mark `path`, or unmark it if it was; whether it is marked now.
    pub fn toggle(&mut self, path: PathBuf) -> bool {
        if self.remove(&path) {
            return false;
        }
        self.insert(path);
        true
    }

    /// Mark `path`; whether it was not yet.
    pub fn insert(&mut self, path: PathBuf) -> bool {
        let new = self.set.insert(path.clone());
        if new {
            self.order.push(path);
        }
        new
    }

    /// Unmark `path`; whether it was marked.
    pub fn remove(&mut self, path: &Path) -> bool {
        let was = self.set.remove(path);
        if was {
            self.order.retain(|p| p != path);
        }
        was
    }

    #[must_use]
    pub fn contains(&self, path: &Path) -> bool {
        self.set.contains(path)
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.order.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.order.is_empty()
    }

    /// The marked files, in the order marked.
    pub fn iter(&self) -> impl Iterator<Item = &Path> {
        self.order.iter().map(PathBuf::as_path)
    }

    pub fn clear(&mut self) {
        self.order.clear();
        self.set.clear();
    }

    /// The marked files, in order, leaving none marked.
    pub fn take(&mut self) -> Vec<PathBuf> {
        self.set.clear();
        std::mem::take(&mut self.order)
    }
}

/// The plain [`Wanted`]: files kept by name, each with what a noter says
/// of it, the noter told whether it speaks of a folder. Stores are offered
/// as files when `keep` takes their names.
#[derive(Clone)]
pub struct Simple {
    header: Header,
    keep: Arc<Keep>,
    note: Arc<Mutex<Box<Noter>>>,
    many: bool,
}

/// Which file names a [`Simple`] browser keeps.
pub type Keep = dyn Fn(&str) -> bool + Send + Sync;

/// What a [`Simple`] browser says beside an entry, told whether it is a
/// folder: a file's size, "faba output".
pub type Noter = dyn FnMut(&Path, bool) -> Option<Cow<'static, str>> + Send;

impl Simple {
    /// Files `keep` takes by name, each with what `note` says of it.
    pub fn new(
        header: Header,
        keep: impl Fn(&str) -> bool + Send + Sync + 'static,
        note: impl FnMut(&Path, bool) -> Option<Cow<'static, str>> + Send + 'static,
    ) -> Self {
        Self {
            header,
            keep: Arc::new(keep),
            note: Arc::new(Mutex::new(Box::new(note))),
            many: false,
        }
    }

    /// Files `keep` takes by name, with nothing said of them.
    pub fn keeping(header: Header, keep: impl Fn(&str) -> bool + Send + Sync + 'static) -> Self {
        Self::new(header, keep, |_, _| None)
    }

    /// Space marks several files to take together.
    #[must_use]
    pub fn marking(mut self) -> Self {
        self.many = true;
        self
    }

    fn noted(&self, path: &Path, dir: bool) -> Option<Cow<'static, str>> {
        let mut note = self.note.lock().unwrap_or_else(PoisonError::into_inner);
        note(path, dir)
    }
}

impl Wanted for Simple {
    type About = Option<Cow<'static, str>>;

    fn header(&self) -> Header {
        self.header.clone()
    }

    fn file(&self, path: &Path, name: &str) -> Option<Self::About> {
        (self.keep)(name).then(|| self.noted(path, false))
    }

    fn store(&self, path: &Path, name: &str) -> Option<Self::About> {
        self.file(path, name)
    }

    fn folder(&self, path: &Path, _name: &str) -> Option<Self::About> {
        self.noted(path, true).map(Some)
    }

    fn describe<'a>(&self, about: &'a Self::About) -> Cow<'a, str> {
        Cow::Borrowed(about.as_deref().unwrap_or(""))
    }

    fn many(&self) -> bool {
        self.many
    }
}

/// How long a folder may take to list before the browser shows it is
/// listing and carries on; a quicker one is shown at once. The crate's
/// own tests wait for the listing, so they do not depend on how busy the
/// machine is.
#[cfg(not(test))]
pub const LIST_GRACE: Duration = Duration::from_millis(100);
#[cfg(test)]
pub const LIST_GRACE: Duration = Duration::from_secs(60);

/// What the worker listing a folder finds, as it finds it.
enum Found<A> {
    /// The folders, by name, and how many files are left to describe.
    Names(Vec<String>, usize),
    /// What [`Wanted::folder`] said of a folder.
    Dir(String, A),
    /// A file described: wanted, or not.
    File(String, Option<A>),
}

/// A folder being listed on a worker thread, which stops at its next
/// entry once this is dropped. A stream of what it finds, not one answer,
/// so a channel of its own rather than a [`crate::work`] `Pending`.
struct Listing<A> {
    rx: mpsc::Receiver<Found<A>>,
    since: Instant,
    /// Files still to describe, once the names have come.
    left: usize,
}

impl<A: Send + 'static> Listing<A> {
    /// List `dir` for `want`: every folder and the files not hidden, or,
    /// when `hidden`, only the hidden files.
    fn spawn<W: Wanted<About = A>>(dir: PathBuf, want: W, hidden: bool) -> Self {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || list_dir(&dir, &want, hidden, &tx));
        Self {
            rx,
            since: Instant::now(),
            left: 0,
        }
    }
}

/// [`Listing::spawn`]'s work: the names first, then each folder's note and
/// each file described, until the browser stops listening.
fn list_dir<W: Wanted>(dir: &Path, want: &W, hidden: bool, tx: &mpsc::Sender<Found<W::About>>) {
    let (mut dirs, mut files) = (Vec::new(), Vec::new());
    for e in std::fs::read_dir(dir).into_iter().flatten().flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        let path = e.path();
        // The entry's own type costs no call; a link is followed.
        let is_dir = match e.file_type() {
            Ok(t) if !t.is_symlink() => t.is_dir(),
            _ => path.is_dir(),
        };
        let store = is_dir && STORES.iter().any(|s| name.ends_with(s));
        if is_dir && !store {
            // Hidden folders cost nothing to name: listed up front.
            if !hidden {
                dirs.push((name, path));
            }
        } else if name.starts_with('.') == hidden {
            files.push((name, path, store));
        }
    }
    let names = dirs.iter().map(|(n, _)| n.clone()).collect();
    if tx.send(Found::Names(names, files.len())).is_err() {
        return;
    }
    for (name, path) in dirs {
        if let Some(a) = want.folder(&path, &name) {
            if tx.send(Found::Dir(name, a)).is_err() {
                return;
            }
        }
    }
    for (name, path, store) in files {
        let about = if store {
            want.store(&path, &name)
        } else {
            want.file(&path, &name)
        };
        if tx.send(Found::File(name, about)).is_err() {
            return;
        }
    }
}

/// An entry and its name in lower case, to narrow by.
struct Item<A> {
    lower: String,
    entry: Entry<A>,
}

impl<A> Item<A> {
    fn new(entry: Entry<A>) -> Self {
        Self {
            lower: entry.name().to_lowercase(),
            entry,
        }
    }
}

/// The browser's state, independent of the terminal so it can be tested.
pub struct Browser<W: Wanted> {
    /// The folder listed.
    pub dir: PathBuf,
    pub want: W,
    /// `..` unless at the root, folders, then files, each by name.
    entries: Vec<Item<W::About>>,
    /// Typed text narrows the list to names holding it, any case; typed
    /// from a `/`, it is a path the browser follows.
    pub filter: String,
    /// The cursor, among the entries shown.
    pub row: usize,
    /// The file to start on, as the caller chose it.
    best: Option<String>,
    /// Files marked with space, in any folder.
    pub marks: Marks,
    /// Why the file just chosen or marked was refused.
    pub refused: Option<Cow<'static, str>>,
    /// List every file, not only those wanted (see [`Self::toggle_all`]).
    pub all: bool,
    /// The folder's listings still coming: its own, and its hidden files'.
    listings: Vec<Listing<W::About>>,
    /// Whether this folder's hidden files were asked for.
    hidden: bool,
    /// The entry to put the cursor on once it is listed.
    select: Option<String>,
    /// The cursor was moved by hand: arriving files leave it be.
    moved: bool,
    grace: Duration,
}

impl<W: Wanted> Browser<W> {
    /// A browser at `dir` (made absolute) listing it for `want`, the cursor
    /// on `select` when that is listed, else on the best file, else on the
    /// first entry after `..`.
    pub fn open(dir: impl AsRef<Path>, want: W, select: Option<&str>) -> Self {
        Self::open_within(dir, want, select, LIST_GRACE)
    }

    /// [`Self::open`], waiting at most `grace` for each folder's listing.
    pub fn open_within(
        dir: impl AsRef<Path>,
        want: W,
        select: Option<&str>,
        grace: Duration,
    ) -> Self {
        let mut b = Browser {
            dir: normalize(dir.as_ref()),
            want,
            entries: Vec::new(),
            filter: String::new(),
            row: 0,
            best: None,
            marks: Marks::default(),
            refused: None,
            all: false,
            listings: Vec::new(),
            hidden: false,
            select: None,
            moved: false,
            grace,
        };
        b.read(select);
        b
    }

    /// List `dir` instead, with nothing typed.
    pub fn go(&mut self, dir: impl AsRef<Path>) {
        self.dir = normalize(dir.as_ref());
        self.read(None);
    }

    /// List the current folder afresh with nothing typed, the cursor going
    /// to `select` as in [`Self::open`].
    pub fn read(&mut self, select: Option<&str>) {
        self.filter.clear();
        self.select = select.map(str::to_string);
        self.load();
    }

    /// List the folder again, the cursor and the typed text kept.
    pub fn refresh(&mut self) {
        self.select = self.current().map(|e| e.name().to_string());
        self.load();
        self.ask_hidden();
    }

    /// Start listing the current folder: shown at once when quick, else as
    /// [`Self::poll`] finds it.
    fn load(&mut self) {
        self.listings.clear();
        self.entries = self
            .dir
            .parent()
            .map(|_| Item::new(Entry::Up))
            .into_iter()
            .collect();
        self.best = None;
        self.hidden = false;
        self.moved = false;
        self.row = 0;
        let listing = Listing::spawn(self.dir.clone(), self.want.clone(), false);
        self.listings.push(listing);
        self.wait();
    }

    /// Once the typed text asks for hidden entries, list this folder's
    /// hidden files too: once a visit, as describing one may open it.
    fn ask_hidden(&mut self) {
        if self.hidden || !self.narrowing().starts_with('.') {
            return;
        }
        self.hidden = true;
        let listing = Listing::spawn(self.dir.clone(), self.want.clone(), true);
        self.listings.push(listing);
        self.wait();
    }

    /// Take what the listings find for at most the grace period.
    fn wait(&mut self) {
        let until = Instant::now().checked_add(self.grace);
        let on = self.current().map(|e| e.name().to_string());
        let mut came = false;
        while let Some(l) = self.listings.first() {
            let got = match until {
                Some(t) => {
                    l.rx.recv_timeout(t.saturating_duration_since(Instant::now()))
                }
                None => {
                    l.rx.recv()
                        .map_err(|_| mpsc::RecvTimeoutError::Disconnected)
                }
            };
            match got {
                Ok(found) => {
                    self.take(0, found);
                    came = true;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => break,
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    self.listings.remove(0);
                }
            }
        }
        if came {
            self.settle(on);
        }
    }

    /// Take what the listings found since; whether anything came.
    pub fn poll(&mut self) -> bool {
        let on = self.current().map(|e| e.name().to_string());
        let mut came = false;
        let mut i = 0;
        while i < self.listings.len() {
            match self.listings[i].rx.try_recv() {
                Ok(found) => {
                    self.take(i, found);
                    came = true;
                }
                Err(mpsc::TryRecvError::Empty) => i += 1,
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.listings.remove(i);
                }
            }
        }
        if came {
            self.settle(on);
        }
        came
    }

    /// Whether the folder is still being listed, and for how long.
    #[must_use]
    pub fn listing(&self) -> Option<Duration> {
        self.listings.iter().map(|l| l.since.elapsed()).max()
    }

    /// Files found but still being described.
    #[must_use]
    pub fn reading(&self) -> usize {
        self.listings.iter().map(|l| l.left).sum()
    }

    /// Add what listing `i` found, out of order until [`Self::settle`].
    fn take(&mut self, i: usize, found: Found<W::About>) {
        match found {
            Found::Names(dirs, files) => {
                self.listings[i].left = files;
                let dirs = dirs.into_iter().map(|d| Item::new(Entry::Dir(d, None)));
                self.entries.extend(dirs);
            }
            Found::Dir(name, a) => {
                let dir = self.entries.iter_mut().find_map(|it| match &mut it.entry {
                    Entry::Dir(n, about) if *n == name => Some(about),
                    _ => None,
                });
                if let Some(about) = dir {
                    *about = Some(a);
                }
            }
            Found::File(name, about) => {
                let l = &mut self.listings[i].left;
                *l = l.saturating_sub(1);
                let entry = match about {
                    Some(a) => Entry::File(name, a),
                    None => Entry::Other(name),
                };
                self.entries.push(Item::new(entry));
            }
        }
    }

    /// Order what came, find the best file, and place the cursor: on the
    /// entry asked for, else the best file, else the first after `..`;
    /// once moved by hand, on `on`, the entry it was on.
    fn settle(&mut self, on: Option<String>) {
        self.entries.sort_by(|a, b| {
            (a.entry.rank(), a.entry.name()).cmp(&(b.entry.rank(), b.entry.name()))
        });
        let files: Vec<(&str, &W::About)> = self
            .entries
            .iter()
            .filter_map(|it| match &it.entry {
                Entry::File(n, a) => Some((n.as_str(), a)),
                _ => None,
            })
            .collect();
        self.best = self.want.best(&self.dir, &files);
        let shown = self.shown();
        let at = |name: &str| shown.iter().position(|e| e.name() == name);
        let row = if self.moved {
            on.as_deref().and_then(at)
        } else {
            self.select.as_deref().or(self.best.as_deref()).and_then(at)
        };
        self.row = row.unwrap_or_else(|| {
            if self.moved {
                self.row.min(shown.len().saturating_sub(1))
            } else {
                past_up(&shown)
            }
        });
    }

    /// The typed path's folder (up to its last `/`) and the name begun
    /// after it, when the typed text is a path.
    fn typed_path(&self) -> Option<(&str, &str)> {
        if !self.filter.starts_with('/') {
            return None;
        }
        let at = self.filter.rfind('/')? + 1;
        Some((&self.filter[..at], &self.filter[at..]))
    }

    /// Follow a typed path, which stays as typed: list its folder as soon
    /// as that folder exists, and the folder the name after it reaches
    /// once no other entry there begins with that name (`/data2` lists
    /// `/data2/` at once; `/data` beside `/data2` waits for a `/`).
    fn follow(&mut self) {
        let Some((folder, name)) = self.typed_path() else {
            return;
        };
        let folder: PathBuf = Path::new(folder).components().collect();
        if !folder.is_dir() {
            return;
        }
        let into = folder.join(name);
        let only = || {
            std::fs::read_dir(&folder).map_or(0, |d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().starts_with(name))
                    .count()
            }) == 1
        };
        let target = if !name.is_empty() && into.is_dir() && only() {
            into
        } else {
            folder
        };
        if target != self.dir {
            self.dir = target;
            self.select = None;
            self.load();
        }
    }

    /// Whether a typed path reached past what was typed: the folder its
    /// name was completed to.
    fn completed(&self) -> bool {
        self.typed_path().is_some_and(|(folder, name)| {
            !name.is_empty()
                && Path::new(folder)
                    .join(name)
                    .components()
                    .collect::<PathBuf>()
                    == self.dir
        })
    }

    /// What the shown names must hold, in lower case: the typed text, or a
    /// typed path's name begun after its last `/`; nothing once in the
    /// folder that name completed to.
    fn narrowing(&self) -> String {
        match self.typed_path() {
            Some(_) if self.completed() => String::new(),
            Some((_, name)) => name.to_lowercase(),
            None => self.filter.to_lowercase(),
        }
    }

    /// The entries the typed text lets through, `..` always first; hidden
    /// ones only when it starts with `.`, files not wanted only with
    /// [`Self::all`].
    #[must_use]
    pub fn shown(&self) -> Vec<&Entry<W::About>> {
        let f = self.narrowing();
        let hidden = f.starts_with('.');
        self.entries
            .iter()
            .filter(|it| match it.entry {
                Entry::Up => true,
                Entry::Other(_) if !self.all => false,
                _ => (hidden || !it.lower.starts_with('.')) && it.lower.contains(&f),
            })
            .map(|it| &it.entry)
            .collect()
    }

    /// The entry under the cursor.
    #[must_use]
    pub fn current(&self) -> Option<&Entry<W::About>> {
        self.shown().get(self.row).copied()
    }

    /// The path of the entry under the cursor: `..` is the parent.
    #[must_use]
    pub fn current_path(&self) -> Option<PathBuf> {
        match self.current()? {
            Entry::Up => self.dir.parent().map(Path::to_path_buf),
            e => Some(self.dir.join(e.name())),
        }
    }

    /// The file the browser starts on, if the caller named one here.
    #[must_use]
    pub fn best(&self) -> Option<&str> {
        self.best.as_deref()
    }

    /// Put the cursor on the entry shown named `name`; whether there is one.
    pub fn select(&mut self, name: &str) -> bool {
        let at = self.shown().iter().position(|e| e.name() == name);
        if let Some(at) = at {
            self.row = at;
            self.moved = true;
        }
        at.is_some()
    }

    /// List every file, or again only those wanted; the cursor kept on its
    /// entry where it still shows.
    pub fn toggle_all(&mut self) {
        let on = self.current().map(|e| e.name().to_string());
        self.all = !self.all;
        self.moved = true;
        self.settle(on);
    }

    pub fn go_up(&mut self) {
        let from = self
            .dir
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.dir = parent;
            self.read(from.as_deref());
        }
    }

    /// A pane title's tail naming the typed text, if any.
    #[must_use]
    pub fn tag(&self) -> String {
        if self.filter.is_empty() {
            String::new()
        } else {
            format!("· find: {} ", self.filter)
        }
    }

    /// Why `e` cannot be taken, if the caller refuses it or does not want it.
    fn refusal(&self, e: &Entry<W::About>) -> Option<Cow<'static, str>> {
        match e {
            Entry::File(name, about) => self.want.refuse(name, about),
            Entry::Other(name) => {
                Some(format!("{name} is not among the {}", self.want.header().what).into())
            }
            _ => None,
        }
    }

    /// Take the file under the cursor, unless it is refused: then say why
    /// and stay.
    fn take_here(&mut self) -> Outcome {
        let Some(e) = self.current().filter(|e| !e.is_dir()) else {
            return Outcome::Ignored;
        };
        if let Some(why) = self.refusal(e) {
            self.refused = Some(why);
            return Outcome::Moved;
        }
        Outcome::Chosen(Chosen::one(self.dir.join(e.name())))
    }

    /// A key: move, open a folder, narrow, mark, cancel, or choose.
    ///
    /// ↑↓, PgUp/PgDn by 10, Home/End move; → or Enter opens a folder, only
    /// Enter takes a file; ← goes up, as Backspace does with nothing typed.
    /// Letters type (not Space, nor chords): a name, a path from `/`, or
    /// a name then `/` to go into it; `~` with nothing typed goes home. Esc
    /// clears the typed text, else cancels. Where [`Wanted::many`], Space
    /// marks a file and moves on, ctrl+a marks every file shown, and Enter
    /// on a file takes the marked ones, or this one when none is.
    pub fn key(&mut self, k: KeyEvent) -> Outcome {
        // A refusal stays on screen until a key changes something.
        let said = self.refused.take();
        let out = self.handle(k);
        if out == Outcome::Ignored && self.refused.is_none() {
            self.refused = said;
        }
        out
    }

    fn handle(&mut self, k: KeyEvent) -> Outcome {
        if self.want.key(&k) {
            return Outcome::Moved;
        }
        if self.want.many() {
            if let Some(o) = self.mark_key(k) {
                return o;
            }
        }
        let chord = k
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        let last = self.shown().len().saturating_sub(1);
        let step = |d: isize| moved(self.row, d, last);
        match k.code {
            KeyCode::Esc if self.filter.is_empty() => return Outcome::Cancelled,
            KeyCode::Esc => {
                let on = self.current().map(|e| e.name().to_string());
                self.filter.clear();
                self.narrowed(on);
            }
            KeyCode::Up => self.moved_to(step(-1)),
            KeyCode::Down => self.moved_to(step(1)),
            KeyCode::PageUp => self.moved_to(step(-10)),
            KeyCode::PageDown => self.moved_to(step(10)),
            KeyCode::Home => self.moved_to(0),
            KeyCode::End => self.moved_to(last),
            KeyCode::Left => self.go_up(),
            KeyCode::Backspace => {
                let on = self.current().map(|e| e.name().to_string());
                if self.filter.pop().is_none() {
                    self.go_up();
                } else {
                    self.follow();
                    self.ask_hidden();
                    self.narrowed(on);
                }
            }
            KeyCode::Char('~') if self.filter.is_empty() && !chord => {
                if let Some(h) = home() {
                    self.go(h);
                }
            }
            KeyCode::Char('/')
                if !chord && !self.filter.is_empty() && self.typed_path().is_none() =>
            {
                self.enter_typed();
            }
            KeyCode::Enter | KeyCode::Right => match self.current() {
                None => return Outcome::Ignored,
                Some(Entry::Up) => self.go_up(),
                Some(Entry::Dir(name, _)) => {
                    let name = name.clone();
                    self.dir.push(name);
                    self.read(None);
                }
                Some(_) if k.code == KeyCode::Enter => return self.take_here(),
                Some(_) => return Outcome::Ignored,
            },
            KeyCode::Char(c) if c != ' ' && !chord => {
                let on = self.current().map(|e| e.name().to_string());
                self.filter.push(c);
                self.follow();
                self.ask_hidden();
                self.narrowed(on);
            }
            _ => return Outcome::Ignored,
        }
        Outcome::Moved
    }

    fn moved_to(&mut self, row: usize) {
        self.row = row;
        self.moved = true;
        self.select = None;
    }

    /// The cursor after the typed text changed: where it was if that still
    /// shows, else on the best file, else on the first entry after `..`.
    fn narrowed(&mut self, on: Option<String>) {
        let shown = self.shown();
        let at = |name: &str| shown.iter().position(|e| e.name() == name);
        let row = on
            .as_deref()
            .filter(|n| *n != "..")
            .and_then(at)
            .or_else(|| self.best.as_deref().and_then(at))
            .unwrap_or_else(|| past_up(&shown));
        self.moved_to(row);
    }

    /// `/` after a typed name: into the folder of that name, else the
    /// folder under the cursor; `../` goes up. Nothing to go into, the `/`
    /// is typed.
    fn enter_typed(&mut self) {
        if self.filter == ".." {
            return self.go_up();
        }
        let named = self.dir.join(&self.filter);
        let into = Some(named).filter(|p| p.is_dir()).or_else(|| {
            matches!(self.current(), Some(Entry::Dir(..)))
                .then(|| self.current_path())
                .flatten()
        });
        match into {
            Some(dir) => self.go(dir),
            None => {
                let on = self.current().map(|e| e.name().to_string());
                self.filter.push('/');
                self.narrowed(on);
            }
        }
    }

    /// Space marks or unmarks the file under the cursor and moves on;
    /// ctrl+a marks every file shown; enter on a file takes the marked
    /// ones, or this one when none is. On a folder enter still opens it, so
    /// files marked here and there can be taken together; space there is
    /// the caller's.
    fn mark_key(&mut self, k: KeyEvent) -> Option<Outcome> {
        let ctrl = k.modifiers.contains(KeyModifiers::CONTROL);
        match k.code {
            KeyCode::Char(' ') => {
                let e = self.current().filter(|e| !e.is_dir())?;
                if let Some(why) = self.refusal(e) {
                    self.refused = Some(why);
                } else {
                    let path = self.dir.join(e.name());
                    self.marks.toggle(path);
                    let last = self.shown().len().saturating_sub(1);
                    self.moved_to(moved(self.row, 1, last));
                }
                Some(Outcome::Moved)
            }
            KeyCode::Char('a') if ctrl => {
                // Refused files are left unmarked, the first reason shown.
                let mut files = Vec::new();
                let mut refused = Vec::new();
                for e in self.shown().into_iter().filter(|e| !e.is_dir()) {
                    match self.refusal(e) {
                        Some(why) => refused.push(why),
                        None => files.push(self.dir.join(e.name())),
                    }
                }
                if let Some(first) = refused.first() {
                    self.refused = Some(match refused.len() {
                        1 => first.clone(),
                        n => format!("{n} files not marked; {first}").into(),
                    });
                }
                for f in files {
                    self.marks.insert(f);
                }
                Some(Outcome::Moved)
            }
            KeyCode::Enter => {
                self.current().filter(|e| !e.is_dir())?;
                let mut marked = self.marks.take().into_iter();
                Some(match marked.next() {
                    Some(first) => Outcome::Chosen(Chosen {
                        first,
                        rest: marked.collect(),
                    }),
                    None => self.take_here(),
                })
            }
            _ => None,
        }
    }

    /// The widest name shown, a folder's `/` with it, at most 36.
    fn name_width(&self, shown: &[&Entry<W::About>]) -> usize {
        shown
            .iter()
            .map(|e| e.name().chars().count() + 1)
            .max()
            .unwrap_or(0)
            .min(36)
    }

    /// Whether rows have a column for marks and the star.
    fn mark_column(&self, header: &Header) -> bool {
        self.want.many() || (header.star.is_some() && self.best.is_some())
    }

    /// Entry `e`'s text with `name` for its name, and the name as shown.
    fn text(&self, e: &Entry<W::About>, name: &str, name_w: usize) -> (String, String) {
        match e {
            Entry::Up => ("../".into(), "../".into()),
            Entry::Dir(_, about) => {
                let shown = format!("{name}/");
                let note = about.as_ref().map(|a| self.want.describe(a));
                let text = name_then(&shown, note.as_deref().unwrap_or(""), name_w);
                (text, shown)
            }
            Entry::File(_, about) => (self.want.row(name, about, name_w), name.to_string()),
            Entry::Other(_) => (name.to_string(), name.to_string()),
        }
    }

    /// Entry `e`'s text in `room` columns, its name cut in the middle when
    /// the row is too wide, so names sharing a start still differ; whether
    /// the name was cut.
    fn fitted(&self, e: &Entry<W::About>, name_w: usize, room: usize) -> (String, String, bool) {
        let (full, shown) = self.text(e, e.name(), name_w);
        let over = full.chars().count().saturating_sub(room);
        if over == 0 {
            return (full, shown, false);
        }
        let keep = e.name().chars().count().saturating_sub(over).max(1);
        let (text, shown) = self.text(e, &cut_middle(e.name(), keep), name_w.min(keep + 1));
        (fit(&text, room), shown, true)
    }

    /// The `rows` entries around the cursor, `width` wide: `▸` by the
    /// cursor's, then, where files are marked or one is starred, `●` by a
    /// marked file and `★` by the best one; a file's row as
    /// [`Wanted::row`] makes it, a folder's with `/` and its note. Only
    /// the rows shown are drawn.
    #[must_use]
    pub fn rows(&self, rows: usize, width: usize, look: &Look) -> Vec<Line<'static>> {
        let header = self.want.header();
        let shown = self.shown();
        let name_w = self.name_width(&shown);
        let marks = self.mark_column(&header);
        let room = width.saturating_sub(2 + if marks { 2 } else { 0 });
        let first = first_visible(self.row, shown.len(), rows);
        shown
            .iter()
            .enumerate()
            .skip(first)
            .take(rows)
            .map(|(i, e)| {
                let at = i == self.row;
                let mut spans = vec![Span::styled(if at { "▸ " } else { "  " }, look.bold)];
                if marks {
                    let path = || self.dir.join(e.name());
                    let m = match e {
                        Entry::File(n, _) | Entry::Other(n) => {
                            if self.want.many() && self.marks.contains(&path()) {
                                "● "
                            } else if header.star.is_some() && self.best.as_deref() == Some(n) {
                                "★ "
                            } else {
                                "  "
                            }
                        }
                        _ => "  ",
                    };
                    spans.push(Span::styled(m, look.bold));
                }
                let (text, name, _) = self.fitted(e, name_w, room);
                let style = match e {
                    _ if at => look.selected,
                    Entry::File(..) => look.page,
                    _ => look.hint,
                };
                // The name in the row's style, what follows it as a hint.
                match text.strip_prefix(name.as_str()) {
                    Some(rest) if !at && !rest.is_empty() => {
                        spans.push(Span::styled(name, style));
                        spans.push(Span::styled(rest.to_string(), look.hint));
                    }
                    _ => spans.push(Span::styled(text, style)),
                }
                Line::from(spans)
            })
            .collect()
    }

    /// What the listing says under its rows, if anything: that it is still
    /// being listed, that nothing matches, or that there is nothing wanted.
    #[must_use]
    pub fn status(&self) -> Option<Cow<'static, str>> {
        if let Some(t) = self.listing() {
            let spin = SPINNER[(t.as_millis() / 100) as usize % SPINNER.len()];
            let reading = match self.reading() {
                0 => String::new(),
                1 => " · 1 file to read".into(),
                n => format!(" · {n} files to read"),
            };
            return Some(format!("{spin} listing this folder… {}s{reading}", t.as_secs()).into());
        }
        let shown = self.shown();
        if shown.iter().any(|e| e.is_file()) {
            return None;
        }
        if !self.filter.is_empty() && self.entries.iter().any(|it| it.entry.is_file()) {
            return Some(NOTHING_MATCHES.into());
        }
        Some(format!("no {} here", self.want.header().what).into())
    }

    /// The browser's lines, at most `height` tall and `width` wide: a side
    /// panel's or the inside of a popup. A header (the title, its notes,
    /// the folder, what was typed, the columns) and a footer (the status,
    /// a refusal in the accent, what the star means, the marks, the keys)
    /// keep their room; the entries get the rest, at least 3. A name cut
    /// under the cursor is shown whole above the footer.
    #[must_use]
    pub fn lines(&self, height: usize, width: usize, look: &Look) -> Vec<Line<'static>> {
        let (head, foot) = self.frame(width, look);
        let rows = height.saturating_sub(head.len() + foot.len()).max(3);
        let mut out = head;
        out.extend(self.rows(rows, width, look));
        out.extend(foot);
        out
    }

    /// The entry (an index into [`Self::shown`]) drawn on line `line` of
    /// [`Self::lines`]`(height, width, ..)`, for a click; none on the
    /// header, the footer or past the last entry. Where
    /// [`Self::has_mark_column`], columns 2 and 3 of a row are its mark.
    #[must_use]
    pub fn row_at(&self, height: usize, width: usize, line: usize) -> Option<usize> {
        // The layout does not depend on the styles.
        let (head, foot) = self.frame(width, &Look::terminal());
        let rows = height.saturating_sub(head.len() + foot.len()).max(3);
        let len = self.shown().len();
        let first = first_visible(self.row, len, rows);
        let k = line.checked_sub(head.len()).filter(|&k| k < rows)?;
        Some(first + k).filter(|&i| i < len)
    }

    /// Whether rows have a mark column (`●`, `★`) after the cursor's `▸`.
    #[must_use]
    pub fn has_mark_column(&self) -> bool {
        self.mark_column(&self.want.header())
    }

    /// [`Self::lines`]' header and footer, `width` wide.
    fn frame(&self, width: usize, look: &Look) -> (Vec<Line<'static>>, Vec<Line<'static>>) {
        let h = self.want.header();
        let line = |s: &str, style: Style| Line::from(Span::styled(fit(s, width), style));
        let mut head = vec![line(&format!(" {}", h.title), look.bold)];
        head.extend(h.notes.iter().map(|n| line(&format!(" {n}"), look.hint)));
        let here = shown(&self.dir);
        head.push(line(
            &format!(" {}", tail(&here, width.saturating_sub(1))),
            look.hint,
        ));
        if self.completed() {
            head.push(line(
                &format!(" go to “{}” → {here}/", self.filter),
                look.page,
            ));
        } else if self.typed_path().is_some() {
            head.push(line(&format!(" go to “{}”", self.filter), look.page));
        } else if !self.filter.is_empty() {
            head.push(line(&format!(" names with “{}”", self.filter), look.page));
        }
        if let Some(c) = &h.columns {
            head.push(line(c, look.hint));
        }

        let mut foot = Vec::new();
        // The name under the cursor whole when its row cuts it.
        let shown = self.shown();
        let marks = if self.mark_column(&h) { 2 } else { 0 };
        if let Some(e) = shown.get(self.row) {
            let room = width.saturating_sub(2 + marks);
            if self.fitted(e, self.name_width(&shown), room).2 {
                let w = width.saturating_sub(3);
                for (k, piece) in wrap(e.name(), w).into_iter().enumerate() {
                    let lead = if k == 0 { " ▸ " } else { "   " };
                    foot.push(line(&format!("{lead}{piece}"), look.bold));
                }
            }
        }
        if let Some(s) = self.status() {
            foot.push(line(&format!("  {s}"), look.hint));
        }
        foot.push(Line::from(""));
        if let Some(why) = &self.refused {
            foot.push(line(&format!(" {why}"), look.accent));
        }
        if let (Some(s), Some(_)) = (&h.star, &self.best) {
            foot.push(line(&format!(" ★ {s}"), look.hint));
        }
        if self.want.many() {
            if !self.marks.is_empty() {
                foot.push(line(&format!(" {} marked", self.marks.len()), look.page));
            }
            foot.push(line(
                " space mark  ctrl+a mark all shown  enter take the marked, or the one here",
                look.hint,
            ));
        }
        foot.push(line(
            &format!(" ↑↓ choose  enter {}  → open  ← up", h.verb),
            look.hint,
        ));
        foot.push(line(
            " type to narrow  / a path  ~ home  esc cancel",
            look.hint,
        ));
        (head, foot)
    }
}

/// The first entry after `..`, if there is one.
fn past_up<A>(shown: &[&Entry<A>]) -> usize {
    usize::from(shown.len() > 1 && matches!(shown.first(), Some(Entry::Up)))
}

/// A listing's spinner, a frame each tenth of a second.
const SPINNER: [char; 8] = ['⠁', '⠂', '⠄', '⡀', '⢀', '⠠', '⠐', '⠈'];

/// `s` in at most `n` characters.
fn fit(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// The last `n` characters of `s`, `…` marking a cut at the start.
fn tail(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        return s.to_string();
    }
    let mut out = String::from("…");
    out.extend(s.chars().skip(len + 1 - n.max(1)));
    out
}

/// `s` in pieces of at most `n` characters.
fn wrap(s: &str, n: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    chars.chunks(n.max(1)).map(|c| c.iter().collect()).collect()
}

/// `s` in at most `n` characters, `…` marking a cut in the middle, so
/// names sharing a prefix still differ.
fn cut_middle(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let end = n.saturating_sub(1) / 2;
    let start = n.saturating_sub(1) - end;
    let mut out: String = chars[..start].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - end..]);
    out
}

#[cfg(test)]
#[path = "tests/browse.rs"]
mod tests;
