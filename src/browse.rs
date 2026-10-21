//! A file browser for a pop-up or a pane: one folder's subfolders, then the
//! files it keeps, with a cursor, narrowed to what has been typed, and typed
//! paths walked (`/` and `~` jump, a name then `/` goes into it).
//!
//! Ported from faba (`src/tui/browser.rs`, with `Find` and `FindList` from
//! `src/tui.rs`). The note beside each file (a matrix's shape, "indexed")
//! and the box before it (`[x]`, a role) are the caller's, and its styles a
//! [`Look`]'s.

use crate::look::Look;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::text::{Line, Span};
use std::path::{Path, PathBuf};

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

/// `at` moved by `d`, kept within `0..=last`.
#[must_use]
pub fn moved(at: usize, d: isize, last: usize) -> usize {
    at.saturating_add_signed(d).min(last)
}

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

/// One listed name.
#[derive(Debug)]
pub struct Entry {
    pub name: String,
    /// The folder's own path, or the file's; `..` is the parent.
    pub path: PathBuf,
    pub dir: bool,
    /// What the browser's noter said of it: a matrix's shape, "indexed".
    pub note: Option<String>,
}

impl AsRef<str> for Entry {
    fn as_ref(&self) -> &str {
        &self.name
    }
}

/// What a key did in the browser.
#[derive(Debug, PartialEq, Eq)]
pub enum Nav {
    /// It moved, opened a folder or narrowed the listing.
    Moved,
    /// Enter on a file.
    Picked(PathBuf),
    /// Not a browser key; the caller's to handle.
    Ignored,
}

/// The suffixes of folders listed as files: a zarr store is a folder, but
/// it is one matrix.
pub const STORES: [&str; 1] = [".zarr"];

/// What a browser says beside each file it lists.
type Noter = Box<dyn FnMut(&Path) -> Option<String>>;

/// The browser's state, independent of the terminal so it can be tested.
pub struct Browser {
    pub cwd: PathBuf,
    /// The listing: `..` first unless at the root, then subfolders, then
    /// files, each sorted, narrowed by what has been typed.
    pub list: FindList<Entry>,
    keep: Box<dyn Fn(&str) -> bool>,
    note: Noter,
}

impl Browser {
    /// A browser at `cwd` that has not listed it yet: files `keep` accepts
    /// by name (folders are always listed), each with what `note` says of
    /// it.
    pub fn new(
        cwd: PathBuf,
        keep: impl Fn(&str) -> bool + 'static,
        note: impl FnMut(&Path) -> Option<String> + 'static,
    ) -> Self {
        Self {
            cwd,
            list: FindList::default(),
            keep: Box::new(keep),
            note: Box::new(note),
        }
    }

    /// Listed at its folder.
    #[must_use]
    pub fn opened(mut self) -> Self {
        self.open(self.cwd.clone());
        self
    }

    /// Show `dir`'s visible entries, with nothing typed; going up lands on
    /// the folder just left.
    pub fn open(&mut self, dir: PathBuf) {
        let dir = normalize(&dir);
        let (mut dirs, mut files): (Vec<String>, Vec<String>) = (Vec::new(), Vec::new());
        for e in std::fs::read_dir(&dir).into_iter().flatten().flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let is_dir = e.file_type().is_ok_and(|t| t.is_dir()) || e.path().is_dir();
            let store = STORES.iter().any(|s| name.ends_with(s));
            if is_dir && !store {
                dirs.push(name);
            } else if (self.keep)(&name) {
                files.push(name);
            }
        }
        dirs.sort();
        files.sort();
        let mut all: Vec<Entry> = dir
            .parent()
            .map(|p| Entry {
                name: "..".into(),
                path: p.to_path_buf(),
                dir: true,
                note: None,
            })
            .into_iter()
            .collect();
        for (names, is_dir) in [(dirs, true), (files, false)] {
            for name in names {
                let path = dir.join(&name);
                all.push(Entry {
                    note: if is_dir { None } else { (self.note)(&path) },
                    name,
                    path,
                    dir: is_dir,
                });
            }
        }
        let left = (Some(dir.as_path()) == self.cwd.parent())
            .then(|| self.cwd.file_name())
            .flatten()
            .map(|n| n.to_string_lossy().into_owned());
        self.list.set(all, 0);
        if let Some(l) = left {
            self.list.select(&l);
        }
        self.cwd = dir;
    }

    /// List the folder again, the cursor kept on the entry it was on.
    pub fn refresh(&mut self) {
        let at = self.highlighted().map(|e| e.name.clone());
        let cwd = self.cwd.clone();
        self.open(cwd);
        if let Some(at) = at {
            self.list.select(&at);
        }
    }

    pub fn up(&mut self) {
        if let Some(p) = self.cwd.parent().map(Path::to_path_buf) {
            self.open(p);
        }
    }

    /// The entry under the cursor.
    #[must_use]
    pub fn highlighted(&self) -> Option<&Entry> {
        self.list.highlighted()
    }

    /// Walk a typed path, narrow by typed text, move, open a folder or go
    /// up; Enter on a file picks it. Letters type, so only arrows move.
    pub fn key(&mut self, key: KeyEvent) -> Nav {
        let into = self
            .highlighted()
            .filter(|e| e.dir && !self.list.find.text.is_empty())
            .map(|e| e.path.clone());
        if let Some(to) = self.list.find.jump(&key, &self.cwd, into) {
            self.open(to);
            return Nav::Moved;
        }
        if self.list.key(&key) {
            return Nav::Moved;
        }
        match key.code {
            KeyCode::Enter | KeyCode::Right => match self.highlighted() {
                Some(e) if e.dir => self.open(e.path.clone()),
                Some(e) if key.code == KeyCode::Enter => return Nav::Picked(e.path.clone()),
                _ => {}
            },
            KeyCode::Left | KeyCode::Backspace => self.up(),
            _ => return Nav::Ignored,
        }
        Nav::Moved
    }

    /// `rows` lines of the listing around the cursor, `width` wide: each
    /// file with the box `mark` puts before it (a folder gets as many
    /// blanks as `mark_width`, none when 0) and its note after it; an empty
    /// listing says `empty`.
    #[must_use]
    pub fn lines(
        &self,
        rows: usize,
        width: usize,
        look: &Look,
        mark_width: usize,
        mark: impl Fn(&Entry) -> Option<String>,
        empty: &str,
    ) -> Vec<Line<'static>> {
        if self.list.is_empty() {
            let why = self.list.empty_note(empty);
            return vec![Line::from(Span::styled(format!("  {why}"), look.hint))];
        }
        let lines = self
            .list
            .shown()
            .enumerate()
            .map(|(i, e)| {
                let selected = i == self.list.at;
                let note = e
                    .note
                    .as_ref()
                    .map_or_else(String::new, |n| format!("  {n}"));
                let boxed = match (mark_width, e.dir) {
                    (0, _) => String::new(),
                    (w, true) => " ".repeat(w + 1),
                    (w, false) => {
                        let m = mark(e).unwrap_or_default();
                        format!("{m:<w$} ")
                    }
                };
                let used = 2 + boxed.chars().count() + note.chars().count();
                let name_w = width.saturating_sub(used).max(1);
                let name = if e.dir {
                    format!("{}/", e.name)
                } else {
                    e.name.clone()
                };
                let name = cut_middle(&name, name_w);
                let style = match (selected, e.dir) {
                    (true, _) => look.selected,
                    (false, true) => look.hint,
                    (false, false) => look.page,
                };
                Line::from(vec![
                    Span::styled(if selected { "▸ " } else { "  " }, look.bold),
                    Span::styled(boxed, look.bold),
                    Span::styled(format!("{name:<name_w$}"), style),
                    Span::styled(note, look.hint),
                ])
            })
            .collect();
        window(lines, self.list.at, rows)
    }
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
