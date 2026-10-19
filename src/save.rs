//! Saving a viewer's figure: a one-line [`Field`] whose suggested text
//! starts selected, names that never replace a file unasked
//! ([`free_stem`], [`taken`]), and the [`SaveDialog`] popup that asks what
//! to save, how wide, how fine and under what name.
//!
//! The dialog is senna's (`src/view/tui/grid.rs`: `SaveDialog`, its
//! fields, sizes and `free_stem`), over a [`SaveTarget`] the viewer
//! implements in place of senna's `Scope` and `Deck`. [`Field`] is
//! pinto's (`src/tui/field.rs`), and from pinto's save dialog
//! (`src/view/tui/save.rs`) come numbered names free of every file a save
//! writes, `stem_of`, the check for a missing folder, and the rule that a
//! confirmed replace holds until the name changes, not until any key.

use crate::look::Look;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use std::path::{Path, PathBuf};

/// Page widths offered, in inches: journal single and double column, then
/// larger.
pub const WIDTHS_IN: [f32; 5] = [3.5, 5.0, 7.0, 10.0, 14.0];
/// Raster resolutions offered for the points.
pub const DPIS: [u32; 4] = [150, 300, 600, 1200];
/// The most pixels a page may take.
pub const MAX_PIXELS: usize = 80_000_000;

/// A one-line text field whose text starts selected: typing replaces a
/// suggested value at once, and an arrow key keeps it to edit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub text: String,
    /// The whole text is selected: the next character replaces it and
    /// backspace clears it.
    selected: bool,
}

impl Field {
    /// `text`, all of it selected.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let selected = !text.is_empty();
        Self { text, selected }
    }

    /// `text` to go on typing at, nothing selected.
    #[must_use]
    pub fn unselected(text: String) -> Self {
        Self {
            text,
            selected: false,
        }
    }

    /// Whether the whole text is selected.
    #[must_use]
    pub fn is_selected(&self) -> bool {
        self.selected
    }

    /// Set the text, unselected, as a completion does.
    pub fn set(&mut self, text: String) {
        self.text = text;
        self.selected = false;
    }

    /// Edit with `key`; whether the text changed. Ctrl-A selects
    /// everything again, ctrl-U clears, and an arrow drops the selection.
    pub fn key(&mut self, key: KeyEvent) -> bool {
        let chord = key
            .modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT | KeyModifiers::SUPER);
        let ctrl = key.modifiers.contains(KeyModifiers::CONTROL);
        let before = self.text.clone();
        match key.code {
            KeyCode::Char('a') if ctrl => self.selected = !self.text.is_empty(),
            KeyCode::Char('u') if ctrl => self.set(String::new()),
            KeyCode::Char(_) if chord => {}
            KeyCode::Char(c) => {
                if self.selected {
                    self.text.clear();
                }
                self.text.push(c);
                self.selected = false;
            }
            KeyCode::Backspace | KeyCode::Delete if self.selected => self.set(String::new()),
            KeyCode::Backspace => {
                self.text.pop();
            }
            KeyCode::Left | KeyCode::Right | KeyCode::Home | KeyCode::End => {
                self.selected = false;
            }
            _ => {}
        }
        self.text != before
    }

    /// The text wrapped at `width`, one span a line, in `style`: in
    /// `selected` while selected where `on`, else with a bar for the cursor
    /// at its end.
    #[must_use]
    pub fn lines(
        &self,
        width: usize,
        on: bool,
        style: Style,
        selected: Style,
    ) -> Vec<Span<'static>> {
        let chars: Vec<char> = self.text.chars().collect();
        let mut pieces: Vec<String> = chars
            .chunks(width.max(1))
            .map(|c| c.iter().collect())
            .collect();
        if pieces.is_empty() {
            pieces.push(String::new());
        }
        let n = pieces.len();
        pieces
            .into_iter()
            .enumerate()
            .map(|(i, p)| match (on, self.selected) {
                (true, true) => Span::styled(p, selected),
                (true, false) if i + 1 == n => Span::styled(format!("{p}▏"), style),
                _ => Span::styled(p, style),
            })
            .collect()
    }
}

/// How a name is numbered past the files it would replace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Numbering {
    /// `stem`, then `stem-2`, `stem-3`, … (senna).
    Suffix,
    /// `stem-001`, `stem-002`, … in `digits` digits at least (pinto).
    Counter { digits: usize },
}

/// The files of `stem` that exist: `stem` and each of `suffixes` (`.pdf`,
/// `-structure.png`).
#[must_use]
pub fn taken(stem: &str, suffixes: &[impl AsRef<str>]) -> Vec<PathBuf> {
    suffixes
        .iter()
        .map(|s| PathBuf::from(format!("{stem}{}", s.as_ref())))
        .filter(|p| p.exists())
        .collect()
}

/// The first name from `stem`, numbered as `numbering` says, none of whose
/// files (`stem` and each of `suffixes`) exists yet.
#[must_use]
pub fn free_stem(stem: &str, suffixes: &[impl AsRef<str>], numbering: Numbering) -> String {
    let free = |s: &str| taken(s, suffixes).is_empty();
    match numbering {
        Numbering::Suffix if free(stem) => stem.to_string(),
        Numbering::Suffix => (2..)
            .map(|k| format!("{stem}-{k}"))
            .find(|s| free(s))
            .expect("some number is free"),
        Numbering::Counter { digits } => (1..)
            .map(|k| format!("{stem}-{k:0digits$}"))
            .find(|s| free(s))
            .expect("some number is free"),
    }
}

/// `name` trimmed, `~` expanded, and without any of `exts` it ends in.
#[must_use]
pub fn stem_of(name: &str, exts: &[&str]) -> String {
    let name = name.trim();
    let rest = match name {
        "~" => Some(""),
        _ => name.strip_prefix("~/"),
    };
    let name = match (rest, std::env::var_os("HOME")) {
        (Some(rest), Some(home)) => {
            let home = home.to_string_lossy();
            let home = home.trim_end_matches('/');
            if rest.is_empty() {
                // Home itself is a folder: keep the slash, so the dialog
                // asks for a file name.
                format!("{home}/")
            } else {
                format!("{home}/{rest}")
            }
        }
        _ => name.to_string(),
    };
    for ext in exts {
        if let Some(s) = name.strip_suffix(&format!(".{ext}")) {
            return s.to_string();
        }
    }
    name
}

/// Why `stem` cannot be saved to, or `None`: no name, a folder (ending in
/// `/` or one that exists), or a folder that is not there.
#[must_use]
pub fn name_problem(stem: &str) -> Option<String> {
    let path = Path::new(stem);
    if stem.is_empty() || stem.ends_with('/') || path.is_dir() {
        return Some("type a file name".into());
    }
    let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
    parent
        .filter(|d| !d.is_dir())
        .map(|d| format!("no folder {}", d.display()))
}

/// What a viewer offers to save, for its [`SaveDialog`].
pub trait SaveTarget {
    /// What a save takes in: this view, every view in a grid, …
    type Scope: Copy + PartialEq;
    /// The scopes offered, in the order ← → steps through them.
    fn scopes(&self) -> Vec<Self::Scope>;
    /// `s` as the dialog names it.
    fn scope_name(&self, s: Self::Scope) -> String;
    /// The name offered for `s`, before it is numbered past saved files.
    fn default_stem(&self, s: Self::Scope) -> String;
    /// The extensions a save of `s` writes beside its stem, without the
    /// dot; the first is the file the dialog names and the gallery lists.
    /// A typed name loses any of them.
    fn exts(&self, s: Self::Scope) -> Vec<String>;
    /// What each file a save of `s` adds to its stem, the main file first:
    /// `.` and each of [`exts`](Self::exts), unless the save writes
    /// companions as well (pinto's `-structure.png`). A name is offered,
    /// and saved over without asking, only while none of them exists.
    fn suffixes(&self, s: Self::Scope) -> Vec<String> {
        self.exts(s).iter().map(|e| format!(".{e}")).collect()
    }
    /// The picture's width over its height.
    fn aspect(&self, s: Self::Scope) -> f32;
    /// Said after the size: `(first of 3 pages)`.
    fn size_note(&self, s: Self::Scope) -> String {
        let _ = s;
        String::new()
    }
}

/// What a [`SaveDialog`] made of a key.
#[derive(Clone, Debug, PartialEq)]
pub enum SaveKey<S> {
    /// It stays open.
    Open,
    /// Esc: nothing saved.
    Cancelled,
    /// Enter, on a name that may be written: save this.
    Save(SaveRequest<S>),
}

/// A save the user asked for.
#[derive(Clone, Debug, PartialEq)]
pub struct SaveRequest<S> {
    pub scope: S,
    /// The page's width in inches, and the points' resolution.
    pub width_in: f32,
    pub dpi: u32,
    /// The picture's size in pixels.
    pub px: (usize, usize),
    /// The name without an extension, `~` expanded.
    pub stem: String,
    /// The main file: the stem and the first of its suffixes.
    pub path: PathBuf,
    /// Every file the save writes, the main one first.
    pub files: Vec<PathBuf>,
}

/// The dialog's rows, in order.
const FIELDS: [&str; 4] = ["what", "width", "dpi", "file"];
const FILE: usize = 3;

/// The save popup: what, how large, how fine, and where.
#[derive(Clone, Debug)]
pub struct SaveDialog<S> {
    scope: S,
    width: usize,
    dpi: usize,
    name: Field,
    /// The name was typed, so a change of scope keeps it.
    typed: bool,
    field: usize,
    numbering: Numbering,
    status: Option<String>,
    /// Enter was pressed once on a name whose files exist: the next one
    /// replaces them, until the name changes.
    replace: bool,
}

impl<S: Copy + PartialEq> SaveDialog<S> {
    /// The dialog for `scope` of `t`, at 7 in and 300 dpi, the name offered
    /// numbered past saved files and selected.
    pub fn new<T: SaveTarget<Scope = S>>(t: &T, scope: S, numbering: Numbering) -> Self {
        Self {
            scope,
            width: 2,
            dpi: 1,
            name: Field::new(offered(t, scope, numbering)),
            typed: false,
            field: 0,
            numbering,
            status: None,
            replace: false,
        }
    }

    #[must_use]
    pub fn scope(&self) -> S {
        self.scope
    }

    #[must_use]
    pub fn width_in(&self) -> f32 {
        WIDTHS_IN[self.width]
    }

    #[must_use]
    pub fn dpi(&self) -> u32 {
        DPIS[self.dpi]
    }

    /// The name as typed.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name.text
    }

    /// What the dialog says under its fields, until the next key.
    #[must_use]
    pub fn status(&self) -> Option<&str> {
        self.status.as_deref()
    }

    /// Say `msg` in the dialog: "saving…", or why a save failed.
    pub fn say(&mut self, msg: impl Into<String>) {
        self.status = Some(msg.into());
    }

    /// The picture's size in pixels: the page's width at the dpi, the
    /// height at the scope's aspect.
    #[must_use]
    pub fn px<T: SaveTarget<Scope = S>>(&self, t: &T) -> (usize, usize) {
        let wp = (self.width_in() * self.dpi() as f32).round() as usize;
        let hp = (wp as f32 / t.aspect(self.scope).max(f32::MIN_POSITIVE)).round() as usize;
        (wp, hp.max(1))
    }

    /// The stem, the main file and every file a save as now would write.
    fn files<T: SaveTarget<Scope = S>>(&self, t: &T) -> (String, Vec<PathBuf>) {
        let exts = t.exts(self.scope);
        let refs: Vec<&str> = exts.iter().map(String::as_str).collect();
        let stem = stem_of(&self.name.text, &refs);
        let files = t
            .suffixes(self.scope)
            .iter()
            .map(|s| PathBuf::from(format!("{stem}{s}")))
            .collect();
        (stem, files)
    }

    /// A key: enter asks to save when the name may be written, esc
    /// cancels, the rest edit the dialog.
    pub fn key<T: SaveTarget<Scope = S>>(&mut self, k: KeyEvent, t: &T) -> SaveKey<S> {
        self.status = None;
        let before = self.name.text.clone();
        let step: isize = match k.code {
            KeyCode::Left => -1,
            KeyCode::Right => 1,
            _ => 0,
        };
        let cycle = |at: usize, len: usize| (at as isize + step).rem_euclid(len as isize) as usize;
        match k.code {
            KeyCode::Esc => return SaveKey::Cancelled,
            KeyCode::Enter => return self.enter(t),
            KeyCode::Up | KeyCode::BackTab => {
                self.field = (self.field + FIELDS.len() - 1) % FIELDS.len()
            }
            KeyCode::Down | KeyCode::Tab => self.field = (self.field + 1) % FIELDS.len(),
            KeyCode::Left | KeyCode::Right if self.field != FILE => match self.field {
                0 => {
                    let scopes = t.scopes();
                    if scopes.len() > 1 {
                        let at = scopes.iter().position(|&s| s == self.scope).unwrap_or(0);
                        self.scope = scopes[cycle(at, scopes.len())];
                        // A new scope brings its own name, unless one was typed.
                        if !self.typed {
                            self.name = Field::new(offered(t, self.scope, self.numbering));
                        }
                    }
                }
                1 => self.width = cycle(self.width, WIDTHS_IN.len()),
                _ => self.dpi = cycle(self.dpi, DPIS.len()),
            },
            _ => {
                if self.name.key(k) {
                    self.field = FILE;
                    self.typed = true;
                } else if matches!(k.code, KeyCode::Char(_) | KeyCode::Backspace) {
                    self.field = FILE;
                }
            }
        }
        // Any other name has to be confirmed again.
        if self.name.text != before {
            self.replace = false;
        }
        SaveKey::Open
    }

    /// Enter: the save, or what stands in its way.
    fn enter<T: SaveTarget<Scope = S>>(&mut self, t: &T) -> SaveKey<S> {
        let (stem, files) = self.files(t);
        if let Some(problem) = name_problem(&stem) {
            self.say(problem);
            return SaveKey::Open;
        }
        let (w, h) = self.px(t);
        if w.saturating_mul(h) > MAX_PIXELS {
            self.say("too large: choose a smaller width or dpi");
            return SaveKey::Open;
        }
        let exist: Vec<String> = files
            .iter()
            .filter(|f| f.exists())
            .map(|f| f.display().to_string())
            .collect();
        if !exist.is_empty() && !self.replace {
            self.replace = true;
            self.say(format!(
                "{} exists · enter again replaces it, or rename",
                exist.join(", ")
            ));
            return SaveKey::Open;
        }
        SaveKey::Save(SaveRequest {
            scope: self.scope,
            width_in: self.width_in(),
            dpi: self.dpi(),
            px: (w, h),
            path: files
                .first()
                .cloned()
                .unwrap_or_else(|| PathBuf::from(&stem)),
            stem,
            files,
        })
    }

    /// The dialog's lines: its title, the four fields, the size, what it
    /// says, and its keys.
    #[must_use]
    pub fn lines<T: SaveTarget<Scope = S>>(&self, t: &T, look: &Look) -> Vec<Line<'static>> {
        let exts = t.exts(self.scope);
        let main = exts.first().map_or(String::new(), |e| e.to_uppercase());
        let n_scopes = t.scopes().len();
        let (stem, files) = self.files(t);
        let shown_ext = match exts.first() {
            Some(e) if self.name.text.trim().ends_with(&format!(".{e}")) => String::new(),
            Some(e) => format!(".{e}"),
            None => String::new(),
        };
        let what = t.scope_name(self.scope);
        let values = [
            if n_scopes > 1 {
                format!("‹ {what} ›")
            } else {
                what
            },
            format!("‹ {} in ›", self.width_in()),
            format!(
                "‹ {} ›  the points' resolution; text stays text",
                self.dpi()
            ),
        ];
        let mut lines = vec![
            Line::from(Span::styled(format!(" Save as {main}"), look.bold)),
            Line::from(""),
        ];
        for (k, (field, value)) in FIELDS.iter().zip(values).enumerate() {
            let style = if k == self.field {
                look.selected
            } else {
                look.page
            };
            let cursor = if k == self.field { " ▸ " } else { "   " };
            lines.push(Line::from(Span::styled(
                format!("{cursor}{field:<6} {value} "),
                style,
            )));
        }
        let on = self.field == FILE;
        let cursor = if on { " ▸ " } else { "   " };
        let mut file = vec![Span::styled(
            format!("{cursor}{:<6} ", FIELDS[FILE]),
            look.page,
        )];
        let name = self.name.lines(usize::MAX, on, look.page, look.selected);
        file.extend(name);
        file.push(Span::styled(format!("{shown_ext} "), look.hint));
        lines.push(Line::from(file));
        lines.push(Line::from(""));
        let (w, h) = self.px(t);
        let dpi = self.dpi() as f32;
        let note = t.size_note(self.scope);
        let note = if note.is_empty() {
            note
        } else {
            format!(" {note}")
        };
        lines.push(Line::from(format!(
            " {w} × {h} px · {:.1} × {:.1} in{note}",
            w as f32 / dpi,
            h as f32 / dpi
        )));
        let exist = files.iter().find(|f| f.exists());
        let say = match &self.status {
            Some(s) => Line::from(Span::styled(format!(" {s}"), look.bold)),
            None if stem.is_empty() => Line::from(Span::styled(" type a file name", look.bold)),
            None if w.saturating_mul(h) > MAX_PIXELS => Line::from(Span::styled(
                " too large: choose a smaller width or dpi",
                look.bold,
            )),
            None => match exist {
                Some(p) => Line::from(Span::styled(
                    format!(
                        " {} exists · enter twice replaces it, or rename",
                        p.display()
                    ),
                    look.bold,
                )),
                None => Line::from(""),
            },
        };
        lines.push(say);
        lines.push(Line::from(Span::styled(
            " ↑ ↓ field   ← → change   type to rename (ctrl+u clears)   enter save   esc cancel",
            look.hint,
        )));
        lines
    }

    /// Draw the dialog in the middle of `area`.
    pub fn render<T: SaveTarget<Scope = S>>(
        &self,
        f: &mut ratatui::Frame,
        area: Rect,
        t: &T,
        look: &Look,
    ) {
        let lines = self.lines(t, look);
        let w = 84.min(area.width);
        let h = (lines.len() as u16 + 2).min(area.height);
        let r = Rect::new(
            area.x + (area.width - w) / 2,
            area.y + (area.height - h) / 2,
            w,
            h,
        );
        f.render_widget(Clear, r);
        let block = Block::bordered().border_style(look.border).style(look.page);
        let body = block.inner(r);
        f.render_widget(block, r);
        f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), body);
    }
}

/// The name offered for `scope`, numbered past the files a save would
/// write.
fn offered<T: SaveTarget>(t: &T, scope: T::Scope, numbering: Numbering) -> String {
    let suffixes = t.suffixes(scope);
    let refs: Vec<&str> = suffixes.iter().map(String::as_str).collect();
    free_stem(&t.default_stem(scope), &refs, numbering)
}

#[cfg(test)]
#[path = "tests/save.rs"]
mod tests;
