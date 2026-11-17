//! Small pieces the tools' views share, drawn in a [`Look`]: where each
//! thing was drawn, for the mouse ([`Hits`]); pop-ups of a given size or
//! sized to their lines, with buttons; a tab line; progress bars and a
//! clock; marks for what a scrolled run hides; the session keys over a
//! title row; and a numbered choice [`Menu`].
//!
//! Ported from faba (`src/tui.rs`) and lupin (`src/tui/menu.rs`), their
//! styles a [`Look`]'s. A pop-up sized to its wrapped lines, untitled, is
//! [`crate::chrome::Chrome::popup`].

use crate::look::Look;
pub use crate::text::{bar, filled};
use ratatui::crossterm::event::KeyCode;
use ratatui::layout::{Position, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Paragraph};
use ratatui::Frame;
use std::cell::RefCell;

/// Where a view drew what, for the mouse: rectangles, each with what a
/// click there means. Behind a `RefCell`, so a draw that only reads the
/// view can write it.
#[derive(Debug)]
pub struct Hits<T> {
    spots: RefCell<Vec<(Rect, T)>>,
}

impl<T> Default for Hits<T> {
    fn default() -> Self {
        Self {
            spots: RefCell::default(),
        }
    }
}

impl<T> Hits<T> {
    /// Forget the last draw's; call at the top of a draw.
    pub fn clear(&self) {
        self.spots.borrow_mut().clear();
    }

    /// `what` was drawn in `area`, over what was drawn before.
    pub fn add(&self, area: Rect, what: T) {
        self.spots.borrow_mut().push((area, what));
    }

    /// The draw's spots all at once, in place of the last draw's.
    pub fn set(&self, spots: Vec<(Rect, T)>) {
        *self.spots.borrow_mut() = spots;
    }

    /// A list drawn in `area` from item `first`: `what(i)` for item `i` on
    /// each of its rows, up to `len` items.
    pub fn rows(&self, area: Rect, first: usize, len: usize, what: impl Fn(usize) -> T) {
        for (k, i) in (first..len).take(usize::from(area.height)).enumerate() {
            let row = Rect::new(area.x, area.y + k as u16, area.width, 1);
            self.add(row, what(i));
        }
    }

    /// Whether nothing was drawn to click.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.spots.borrow().is_empty()
    }

    /// A cell where `what` was last drawn, to click in tests.
    #[must_use]
    pub fn spot(&self, what: &T) -> Option<(u16, u16)>
    where
        T: PartialEq,
    {
        let spots = self.spots.borrow();
        let (r, _) = spots.iter().rev().find(|(_, t)| t == what)?;
        Some((r.x, r.y))
    }

    /// What is at a cell: the last drawn there, which is on top.
    #[must_use]
    pub fn at(&self, column: u16, row: u16) -> Option<T>
    where
        T: Clone,
    {
        let pos = Position::new(column, row);
        self.spots
            .borrow()
            .iter()
            .rev()
            .find(|(r, _)| r.contains(pos))
            .map(|(_, t)| t.clone())
    }
}

/// A `w` x `h` rectangle centred in `area`, shrunk to fit it.
#[must_use]
pub fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let (w, h) = (w.min(area.width), h.min(area.height));
    Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    )
}

/// A pop-up's frame: rounded, its title in bold clear of the border's
/// colour.
fn frame_block(title: &str, look: &Look) -> Block<'static> {
    // A title takes the border's style; start clear of it.
    let style = Style::reset().patch(look.page).patch(look.bold);
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(look.border)
        .style(look.page)
        .title(Line::styled(title.to_string(), style))
}

/// Clear a `w` x `h` pop-up centred in `area`, frame it titled `title`,
/// and return the inside.
pub fn popup_frame(
    frame: &mut Frame,
    area: Rect,
    w: u16,
    h: u16,
    title: &str,
    look: &Look,
) -> Rect {
    let rect = centered(area, w, h);
    frame.render_widget(Clear, rect);
    let block = frame_block(title, look);
    let inner = block.inner(rect);
    frame.render_widget(block, rect);
    inner
}

/// `lines` in a pop-up titled `title`, sized to them and at least 40 wide,
/// centred in `area`; the inside, where they went.
pub fn popup(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    lines: Vec<Line<'static>>,
    look: &Look,
) -> Rect {
    let width = lines.iter().map(Line::width).max().unwrap_or(0);
    let width = u16::try_from(width).unwrap_or(u16::MAX).saturating_add(4);
    let height = u16::try_from(lines.len())
        .unwrap_or(u16::MAX)
        .saturating_add(2);
    let inner = popup_frame(frame, area, width.max(40), height, title, look);
    frame.render_widget(Paragraph::new(lines).style(look.page), inner);
    inner
}

/// A row of buttons to click, `[ label ]` each, two apart: the line, and
/// each button's column (from the line's start) and width.
#[must_use]
pub fn buttons(labels: &[&str], look: &Look) -> (Line<'static>, Vec<(u16, u16)>) {
    let style = look.bold.add_modifier(Modifier::REVERSED);
    let mut spans = vec![Span::raw(" ")];
    let mut spots = Vec::new();
    let mut x = 1u16;
    for (k, label) in labels.iter().enumerate() {
        if k > 0 {
            spans.push(Span::raw("  "));
            x = x.saturating_add(2);
        }
        let span = Span::styled(format!("[ {label} ]"), style);
        let w = u16::try_from(span.width()).unwrap_or(u16::MAX);
        spans.push(span);
        spots.push((x, w));
        x = x.saturating_add(w);
    }
    (Line::from(spans), spots)
}

/// A pop-up with `buttons` on its first line (where a long body cannot push
/// them out of sight), then `body`; each button's spot goes into `hits`.
/// The inside, as [`popup`].
pub fn button_popup<T: Clone>(
    frame: &mut Frame,
    area: Rect,
    title: &str,
    buttons_: &[(&str, T)],
    body: Vec<Line<'static>>,
    hits: &Hits<T>,
    look: &Look,
) -> Rect {
    let labels: Vec<&str> = buttons_.iter().map(|(l, _)| *l).collect();
    let (line, spots) = buttons(&labels, look);
    let mut lines = vec![line, Line::raw("")];
    lines.extend(body);
    let inner = popup(frame, area, title, lines, look);
    for (&(dx, w), (_, what)) in spots.iter().zip(buttons_) {
        hits.add(
            Rect::new(inner.x.saturating_add(dx), inner.y, w, 1),
            what.clone(),
        );
    }
    inner
}

/// A tab line for `area`'s top row: the names, `active` in bold and the
/// rest as hints, then `button` if any; each name's spot (`tab(i)`) and the
/// button's go into `hits`.
pub fn tab_bar<T: Clone>(
    area: Rect,
    names: &[&str],
    active: usize,
    button: Option<(&str, T)>,
    hits: &Hits<T>,
    tab: impl Fn(usize) -> T,
    look: &Look,
) -> Line<'static> {
    let mut spans = vec![Span::raw(" ")];
    let mut x = area.x.saturating_add(1);
    for (i, name) in names.iter().enumerate() {
        let style = if i == active { look.bold } else { look.hint };
        let w = u16::try_from(Span::raw(*name).width()).unwrap_or(u16::MAX);
        hits.add(Rect::new(x, area.y, w, 1), tab(i));
        spans.push(Span::styled(format!("{name}  "), style));
        x = x.saturating_add(w).saturating_add(2);
    }
    if let Some((label, what)) = button {
        let (line, spots) = buttons(&[label], look);
        if let Some(&(dx, w)) = spots.first() {
            hits.add(Rect::new(x.saturating_add(dx), area.y, w, 1), what);
        }
        spans.extend(line.spans);
    }
    Line::from(spans)
}

/// Keys and what they do as a hint line: each key in the accent in bold
/// (as the screen loop's chrome draws keys, typed values and the value in
/// focus), what it does as a hint. [`crate::view::help_line`] is this.
#[must_use]
pub fn key_line<K: AsRef<str>, W: AsRef<str>>(keys: &[(K, W)], look: &Look) -> Line<'static> {
    let key_style = look.highlight();
    let mut spans = vec![Span::raw(" ")];
    for (key, what) in keys {
        spans.push(Span::styled(key.as_ref().to_string(), key_style));
        spans.push(Span::styled(format!(" {}  ", what.as_ref()), look.hint));
    }
    Line::from(spans)
}

/// The session keys ([`key_line`]) drawn over the right end of `area`'s
/// top row, the title there giving way, so the detach key is always on
/// screen. Nothing for no keys.
pub fn draw_session_keys<K: AsRef<str>, W: AsRef<str>>(
    frame: &mut Frame,
    area: Rect,
    keys: &[(K, W)],
    look: &Look,
) {
    if keys.is_empty() || area.height == 0 {
        return;
    }
    let line = key_line(keys, look);
    let width = u16::try_from(line.width())
        .unwrap_or(u16::MAX)
        .min(area.width);
    let at = Rect::new(area.right() - width, area.y, width, 1);
    frame.render_widget(Clear, at);
    frame.render_widget(Paragraph::new(line), at);
}

/// A `width`-cell bar for `done` of `total` as [`bar`] draws it: the filled
/// part in `on`, the rest as a hint.
#[must_use]
pub fn bar_spans(
    done: u64,
    total: u64,
    width: usize,
    on: Style,
    look: &Look,
) -> [Span<'static>; 2] {
    let full = filled(done, total, width);
    [
        Span::styled("█".repeat(full), on),
        Span::styled("░".repeat(width - full), look.hint),
    ]
}

/// `d` as `hh:mm:ss`.
#[must_use]
pub fn clock(d: std::time::Duration) -> String {
    let s = d.as_secs();
    format!("{:02}:{:02}:{:02}", s / 3600, s / 60 % 60, s % 60)
}

/// How many characters `n` takes written out, its sign included.
#[must_use]
pub fn digits(n: i64) -> usize {
    let width = n
        .unsigned_abs()
        .checked_ilog10()
        .map_or(1, |d| d as usize + 1);
    width + usize::from(n < 0)
}

/// How many of a scrolled run are hidden before and after, as `‹2 5›`;
/// empty when none are.
#[must_use]
pub fn hidden_marks(left: usize, right: usize) -> String {
    let left = (left > 0).then(|| format!("‹{left}"));
    let right = (right > 0).then(|| format!("{right}›"));
    [left, right]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" ")
}

/// The key besides Esc that closes a [`Menu`], which no option is keyed by.
pub const CLOSES: char = 'q';

/// The rows PageUp and PageDown move a [`Menu`]'s selection.
const PAGE: usize = 10;

/// A menu's keys, along its bottom border.
const MENU_HINT: &str = " ↑↓ choose · enter or its key takes it · esc or q cancels ";

/// One option of a [`Menu`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Choice<A> {
    /// The key that takes it; none for options past the keys there are.
    pub key: Option<char>,
    /// What the menu lists.
    pub label: String,
    /// What choosing it does, shown while it is selected; a paragraph a
    /// line.
    pub detail: String,
    /// What the menu gives back when it is chosen.
    pub action: A,
}

/// What a key did to a [`Menu`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome<A> {
    /// Still open: moved, or a key it does not take.
    Open,
    /// Closed without a choice (Esc or [`CLOSES`]).
    Cancel,
    /// This option's action was taken.
    Chosen(A),
}

/// A choice pop-up: a question, keyed options with the selected one's
/// detail under them, chosen with the arrows and Enter or the option's key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Menu<A> {
    /// What is asked, over the options.
    pub question: String,
    /// The options, in order.
    pub choices: Vec<Choice<A>>,
    /// The selected option.
    pub sel: usize,
}

impl<A: Clone> Menu<A> {
    /// `question` over `choices`, the first selected.
    pub fn new(question: impl Into<String>, choices: Vec<Choice<A>>) -> Self {
        Self {
            question: question.into(),
            choices,
            sel: 0,
        }
    }

    /// ↑↓, PageUp/PageDown, Home and End move; Enter or an option's key
    /// takes it; Esc or [`CLOSES`] cancels.
    pub fn key(&mut self, code: KeyCode) -> Outcome<A> {
        let last = self.choices.len().saturating_sub(1);
        match code {
            KeyCode::Esc | KeyCode::Char(CLOSES) => return Outcome::Cancel,
            KeyCode::Enter => return self.take(self.sel),
            KeyCode::Char(c) => {
                return match self.choices.iter().position(|o| o.key == Some(c)) {
                    Some(i) => self.take(i),
                    None => Outcome::Open,
                }
            }
            KeyCode::Up => self.sel = self.sel.saturating_sub(1),
            KeyCode::Down => self.sel = (self.sel + 1).min(last),
            KeyCode::PageUp => self.sel = self.sel.saturating_sub(PAGE),
            KeyCode::PageDown => self.sel = (self.sel + PAGE).min(last),
            KeyCode::Home => self.sel = 0,
            KeyCode::End => self.sel = last,
            _ => {}
        }
        Outcome::Open
    }

    fn take(&mut self, i: usize) -> Outcome<A> {
        match self.choices.get(i) {
            Some(o) => {
                self.sel = i;
                Outcome::Chosen(o.action.clone())
            }
            None => Outcome::Open,
        }
    }
}

impl<A> Menu<A> {
    /// The menu in a pop-up centred in `over`, at most 100 wide unless an
    /// option needs more, which takes the frame's whole width: the question
    /// in bold, the options scrolled to keep the selected one in sight, and
    /// its detail as hints; the keys along the bottom border.
    pub fn draw(&self, f: &mut Frame, over: Rect, look: &Look) {
        let key = |o: &Choice<A>| o.key.unwrap_or(' ');
        let option = |o: &Choice<A>| Span::raw(format!(" {}  {}", key(o), o.label)).width();
        let longest = self
            .choices
            .iter()
            .map(option)
            .chain(std::iter::once(Span::raw(MENU_HINT).width()))
            .max()
            .unwrap_or(0);
        let need = u16::try_from(longest).unwrap_or(u16::MAX).saturating_add(2);
        let mut width = over.width.saturating_sub(2).clamp(20.min(over.width), 100);
        let over = if need > width {
            let screen = f.area();
            width = need.min(screen.width.saturating_sub(2));
            Rect {
                x: screen.x,
                width: screen.width,
                ..over
            }
        } else {
            over
        };
        let inner = usize::from(width.saturating_sub(2).max(1));
        let mut lines: Vec<Line> = crate::text::wrap_at_most(&self.question, inner, 6)
            .into_iter()
            .map(|l| Line::styled(l, look.bold))
            .collect();
        lines.push(Line::default());
        // A detail's lines are its paragraphs; the options get the room the
        // question, the detail and the frame leave.
        let detail: Vec<String> = self.choices.get(self.sel).map_or_else(Vec::new, |o| {
            o.detail
                .lines()
                .flat_map(|p| crate::text::wrap_at_most(p, inner, 4))
                .take(12)
                .collect()
        });
        let fixed = lines.len() + detail.len() + 4;
        let room = usize::from(over.height).saturating_sub(fixed).max(3);
        let first = self.sel.saturating_sub(room.saturating_sub(1));
        for (i, o) in self.choices.iter().enumerate().skip(first).take(room) {
            let text = format!(" {}  {}", key(o), o.label);
            let style = if i == self.sel {
                look.selected
            } else {
                look.page
            };
            lines.push(Line::styled(text, style));
        }
        if self.choices.len() > first + room {
            let more = self.choices.len() - first - room;
            lines.push(Line::styled(format!("    … {more} more"), look.hint));
        }
        lines.push(Line::default());
        lines.extend(detail.into_iter().map(|l| Line::styled(l, look.hint)));
        let height = u16::try_from(lines.len())
            .unwrap_or(u16::MAX)
            .saturating_add(2)
            .min(over.height);
        let area = centered(over, width, height);
        f.render_widget(Clear, area);
        let block = frame_block("", look).title_bottom(Line::styled(MENU_HINT, look.hint));
        f.render_widget(Paragraph::new(lines).style(look.page).block(block), area);
    }
}

/// `label` and `detail` for each action, keyed `1` to `9`, then `a` to `z`,
/// then `A` to `Z`, never by [`CLOSES`]; past those, keyed by none.
#[must_use]
pub fn numbered<A>(items: Vec<(String, String, A)>) -> Vec<Choice<A>> {
    numbered_but(items, "")
}

/// As [`numbered`], skipping the keys in `taken`, which the menu's own
/// options have.
#[must_use]
pub fn numbered_but<A>(items: Vec<(String, String, A)>, taken: &str) -> Vec<Choice<A>> {
    let mut keys = ('1'..='9')
        .chain('a'..='z')
        .chain('A'..='Z')
        .filter(|&c| c != CLOSES && !taken.contains(c));
    items
        .into_iter()
        .map(|(label, detail, action)| Choice {
            key: keys.next(),
            label,
            detail,
            action,
        })
        .collect()
}

#[cfg(test)]
#[path = "tests/widgets.rs"]
mod tests;
