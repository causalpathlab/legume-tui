//! The strip of saved figures on the left of a viewer: each save's
//! thumbnail, file name, how long ago and what, newest first; the wheel
//! scrolls it, and a save chosen there takes the keys that move or remove
//! it.
//!
//! senna's (`src/view/tui/grid.rs`: `strip`, `content`, `focus_saved`,
//! `rows_per_thumb`, `saved_shown`, `scroll_saved`, `choose_saved`,
//! `saved_at_row`, `strip_mouse`, `strip_key`, `after_change`,
//! `prepare_thumbs`, `saved_block`, `draw_saved`, `thumbs_that_fit`), as a
//! widget over a [`Gallery`] the viewer owns. A key that changes the
//! gallery comes back as an [`Action`] for the viewer to carry out, so it
//! can word what it says and ask first where it wants. Thumbnails are
//! drawn by a [`ThumbPainter`]: [`ProtocolPainter`] is senna's, through
//! the terminal's image protocol; pinto's block characters can be another.

use super::{ago, Gallery};
use crate::look::Look;
use image::RgbaImage;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::{Position, Rect};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use std::collections::HashMap;
use std::path::PathBuf;

/// Columns of the strip.
pub const WIDTH: u16 = 28;

/// What draws a thumbnail into the terminal.
pub trait ThumbPainter {
    /// A thumbnail made ready to draw.
    type Thumb;
    /// `img` made ready to draw in `cols` × `rows` cells; `None` when it
    /// cannot be.
    fn make(&mut self, img: RgbaImage, cols: u16, rows: u16) -> Option<Self::Thumb>;
    /// Draw `t` into `area`.
    fn draw(&self, t: &Self::Thumb, f: &mut ratatui::Frame, area: Rect);
}

/// Thumbnails in the terminal's image protocol, scaled to fit (senna's).
#[cfg(feature = "graphics")]
pub struct ProtocolPainter {
    pub picker: ratatui_image::picker::Picker,
}

#[cfg(feature = "graphics")]
impl ThumbPainter for ProtocolPainter {
    type Thumb = ratatui_image::protocol::Protocol;
    fn make(&mut self, img: RgbaImage, cols: u16, rows: u16) -> Option<Self::Thumb> {
        self.picker
            .new_protocol(
                image::DynamicImage::ImageRgba8(img),
                ratatui::layout::Size::new(cols, rows),
                ratatui_image::Resize::Fit(Some(image::imageops::FilterType::Triangle)),
            )
            .ok()
    }
    fn draw(&self, t: &Self::Thumb, f: &mut ratatui::Frame, area: Rect) {
        f.render_widget(ratatui_image::Image::new(t), area);
    }
}

/// The letters the strip answers to; arrows, `j` `k`, page keys, Home,
/// End, Esc, Tab, Enter, Delete and Ctrl+C are fixed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Keymap {
    /// Hands the keys back (as Esc and Tab do).
    pub leave: char,
    /// Hides the strip.
    pub hide: char,
    /// Moves the chosen save (as Enter does).
    pub relocate: char,
    /// Takes it off the list (as Delete does).
    pub drop: char,
    /// Takes it off the list and deletes its files.
    pub delete: char,
    /// Quits the viewer (as Ctrl+C does).
    pub quit: Option<char>,
}

impl Default for Keymap {
    /// senna's: `F` leaves, `f` hides, `m` moves, `x` drops, `X` deletes,
    /// `q` quits.
    fn default() -> Self {
        Self {
            leave: 'F',
            hide: 'f',
            relocate: 'm',
            drop: 'x',
            delete: 'X',
            quit: Some('q'),
        }
    }
}

/// What a key in the strip asks of the viewer.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Action {
    /// Nothing for the viewer to do.
    None,
    /// The strip changed: draw it again.
    Redraw,
    /// The strip showed or went: lay the screen out again.
    Relayout,
    /// The keys went back to the viewer.
    Leave,
    Quit,
    /// Ask where to move save `.0`.
    Move(usize),
    /// Take save `at` off the list; with `delete`, its files too. Then
    /// call [`Strip::after_change`].
    Remove {
        at: usize,
        delete: bool,
    },
}

/// The strip: whether it shows, how far it is scrolled, the save chosen,
/// and the thumbnails made for it.
pub struct Strip<P: ThumbPainter> {
    painter: P,
    /// Pixels per terminal cell.
    cell: (f32, f32),
    show: bool,
    /// The first save shown, as it scrolls.
    top: usize,
    /// The save chosen, while keys go to the strip.
    at: Option<usize>,
    /// Thumbnails made, by thumbnail file, at `thumb_rows` rows; `None`
    /// for one that could not be read, so it is not read again.
    thumbs: HashMap<PathBuf, Option<P::Thumb>>,
    thumb_rows: u16,
}

impl<P: ThumbPainter> Strip<P> {
    /// A strip, showing, whose thumbnails `painter` makes, on a terminal of
    /// `cell` pixels per cell.
    pub fn new(painter: P, cell: (f32, f32)) -> Self {
        Self {
            painter,
            cell,
            show: true,
            top: 0,
            at: None,
            thumbs: HashMap::new(),
            thumb_rows: 0,
        }
    }

    /// A new painter or cell size, after an attach: every thumbnail is made
    /// again.
    pub fn repaint(&mut self, painter: P, cell: (f32, f32)) {
        self.painter = painter;
        self.cell = cell;
        self.thumbs.clear();
    }

    /// Drop the thumbnails made, so the next [`Self::prepare`] makes them
    /// again.
    pub fn forget(&mut self) {
        self.thumbs.clear();
    }

    /// How many thumbnails are made.
    #[must_use]
    pub fn made(&self) -> usize {
        self.thumbs.len()
    }

    #[must_use]
    pub fn painter(&self) -> &P {
        &self.painter
    }

    /// Whether the strip shows when there is room and something saved.
    #[must_use]
    pub fn is_shown(&self) -> bool {
        self.show
    }

    /// The save chosen, while keys go to the strip.
    #[must_use]
    pub fn chosen(&self) -> Option<usize> {
        self.at
    }

    /// The first save shown.
    #[must_use]
    pub fn top(&self) -> usize {
        self.top
    }

    /// Where the strip goes in `body`, when it shows: on the left, full
    /// height. It shows when not hidden, with a save to show, on a screen
    /// three strips wide.
    #[must_use]
    pub fn area<X>(&self, body: Rect, g: &Gallery<X>) -> Option<Rect> {
        let shows = self.show && !g.entries().is_empty() && body.width >= 3 * WIDTH;
        shows.then(|| Rect::new(body.x, body.y, WIDTH, body.height))
    }

    /// What is left of `body` beside the strip.
    #[must_use]
    pub fn content<X>(&self, body: Rect, g: &Gallery<X>) -> Rect {
        match self.area(body, g) {
            Some(s) => Rect {
                x: body.x + s.width,
                width: body.width - s.width,
                ..body
            },
            None => body,
        }
    }

    /// Show or hide the strip; hidden, it hands the keys back.
    pub fn toggle(&mut self) {
        self.show = !self.show;
        if !self.show {
            self.at = None;
        }
    }

    /// Keys go to the strip, on the save at its top; it shows if hidden.
    /// False when nothing is saved.
    pub fn focus<X>(&mut self, g: &Gallery<X>) -> bool {
        let n = g.entries().len();
        if n == 0 {
            return false;
        }
        self.show = true;
        self.at = Some(self.top.min(n - 1));
        true
    }

    /// Hand the keys back.
    pub fn leave(&mut self) {
        self.at = None;
    }

    /// Rows a thumbnail takes, at the typical 4:3 of a map.
    #[must_use]
    pub fn rows_per_thumb(&self) -> u16 {
        let px_wide = f32::from(WIDTH - 2) * self.cell.0;
        (px_wide * 0.75 / self.cell.1.max(1.0))
            .round()
            .clamp(3.0, 12.0) as u16
    }

    /// The strip's frame: a rule on the map side, its title, and its keys
    /// along the bottom.
    fn block(&self, n: usize, look: &Look, keys: &Keymap) -> Block<'static> {
        let (hint, style) = if self.at.is_some() {
            (
                format!(" ↑↓ {} move {} drop esc ", keys.relocate, keys.drop),
                look.bold,
            )
        } else {
            (
                format!(" {} choose · wheel scrolls ", keys.leave),
                look.hint,
            )
        };
        Block::new()
            .borders(Borders::RIGHT | Borders::BOTTOM)
            .border_style(look.border)
            .title(Span::styled(
                format!(" saved ({n}) · {} hides ", keys.hide),
                look.bold,
            ))
            .title_bottom(Span::styled(hint, style))
            .style(look.page)
    }

    /// How many saves `strip` shows at once.
    #[must_use]
    pub fn shown(&self, strip: Rect) -> usize {
        let inner = self
            .block(0, &Look::default(), &Keymap::default())
            .inner(strip);
        thumbs_that_fit(inner.height, self.rows_per_thumb())
    }

    /// Scroll so the strip starts at `top`, kept in range, `shown` saves
    /// showing at once. Returns whether it moved.
    pub fn scroll<X>(&mut self, top: usize, shown: usize, g: &Gallery<X>) -> bool {
        let top = top.min(last_top(g.entries().len(), shown));
        if top == self.top {
            return false;
        }
        self.top = top;
        true
    }

    /// Choose save `k`, scrolling to show it.
    pub fn choose<X>(&mut self, k: usize, shown: usize, g: &Gallery<X>) {
        let n = g.entries().len();
        if n == 0 {
            self.at = None;
            return;
        }
        let k = k.min(n - 1);
        self.at = Some(k);
        let shown = shown.max(1);
        if k < self.top {
            self.scroll(k, shown, g);
        } else if k >= self.top + shown {
            self.scroll(k + 1 - shown, shown, g);
        }
    }

    /// The save drawn at screen row `row` of `strip`, if any.
    #[must_use]
    pub fn at_row<X>(&self, strip: Rect, row: u16, g: &Gallery<X>) -> Option<usize> {
        let inner = self
            .block(0, &Look::default(), &Keymap::default())
            .inner(strip);
        let each = self.rows_per_thumb() + 3;
        let k = usize::from(row.checked_sub(inner.y + 1)? / each);
        (k < self.shown(strip))
            .then_some(self.top + k)
            .filter(|&k| k < g.entries().len())
    }

    /// The mouse over `strip`: the wheel scrolls it, a click chooses a
    /// save. `None` when the pointer is not on the strip, else whether it
    /// changed.
    pub fn mouse<X>(&mut self, strip: Rect, m: &MouseEvent, g: &Gallery<X>) -> Option<bool> {
        if !strip.contains(Position::new(m.column, m.row)) {
            return None;
        }
        let shown = self.shown(strip);
        Some(match m.kind {
            MouseEventKind::ScrollUp => self.scroll(self.top.saturating_sub(1), shown, g),
            MouseEventKind::ScrollDown => self.scroll(self.top + 1, shown, g),
            MouseEventKind::Down(MouseButton::Left) => match self.at_row(strip, m.row, g) {
                Some(k) => {
                    self.choose(k, shown, g);
                    true
                }
                None => false,
            },
            _ => false,
        })
    }

    /// A key while the strip has them (a save is chosen): choose, move or
    /// remove a save. With none chosen, [`Action::None`].
    pub fn key<X>(&mut self, strip: Rect, k: KeyEvent, g: &Gallery<X>, keys: &Keymap) -> Action {
        let Some(at) = self.at else {
            return Action::None;
        };
        if at >= g.entries().len() {
            self.at = None;
            return Action::Leave;
        }
        let shown = self.shown(strip);
        let page = shown.max(1);
        let before = (self.at, self.top);
        match k.code {
            KeyCode::Char('c') if k.modifiers.contains(KeyModifiers::CONTROL) => {
                return Action::Quit
            }
            KeyCode::Char(c) if Some(c) == keys.quit => return Action::Quit,
            KeyCode::Esc | KeyCode::Tab => {
                self.at = None;
                return Action::Leave;
            }
            KeyCode::Char(c) if c == keys.leave => {
                self.at = None;
                return Action::Leave;
            }
            KeyCode::Char(c) if c == keys.hide => {
                self.toggle();
                return Action::Relayout;
            }
            KeyCode::Char(c) if c == keys.relocate => return Action::Move(at),
            KeyCode::Enter => return Action::Move(at),
            KeyCode::Char(c) if c == keys.drop => return Action::Remove { at, delete: false },
            KeyCode::Delete => return Action::Remove { at, delete: false },
            KeyCode::Char(c) if c == keys.delete => return Action::Remove { at, delete: true },
            KeyCode::Up | KeyCode::Char('k') => self.choose(at.saturating_sub(1), shown, g),
            KeyCode::Down | KeyCode::Char('j') => self.choose(at + 1, shown, g),
            KeyCode::PageUp => self.choose(at.saturating_sub(page), shown, g),
            KeyCode::PageDown => self.choose(at + page, shown, g),
            KeyCode::Home => self.choose(0, shown, g),
            KeyCode::End => self.choose(usize::MAX, shown, g),
            _ => {}
        }
        if (self.at, self.top) == before {
            Action::None
        } else {
            Action::Redraw
        }
    }

    /// The list changed under the strip (a save removed or moved): keep the
    /// scroll and the choice in range. Lay the screen out again when the
    /// strip went with the last save.
    pub fn after_change<X>(&mut self, strip: Rect, g: &Gallery<X>) -> Action {
        let shown = self.shown(strip);
        self.top = self.top.min(last_top(g.entries().len(), shown));
        if let Some(at) = self.at {
            self.choose(at, shown, g);
        }
        if g.entries().is_empty() {
            Action::Relayout
        } else {
            Action::Redraw
        }
    }

    /// Make the thumbnails for the saves `strip` shows, once each (again
    /// when their size in rows changes); those no longer shown are dropped.
    pub fn prepare<X>(&mut self, strip: Rect, g: &Gallery<X>) {
        let rows = self.rows_per_thumb();
        if rows != self.thumb_rows {
            self.thumbs.clear();
            self.thumb_rows = rows;
        }
        let shown = self.shown(strip);
        // A smaller list, or a taller screen, may leave the top past the end.
        self.top = self.top.min(last_top(g.entries().len(), shown));
        let wanted: Vec<PathBuf> = g
            .entries()
            .iter()
            .skip(self.top)
            .take(shown)
            .filter_map(|e| g.thumb(e))
            .collect();
        self.thumbs.retain(|p, _| wanted.contains(p));
        for path in wanted {
            if !self.thumbs.contains_key(&path) {
                let t = super::thumb::read_png(&path)
                    .and_then(|img| self.painter.make(img, WIDTH - 2, rows));
                self.thumbs.insert(path, t);
            }
        }
    }

    /// The saved figures, newest first: thumbnail, file name, how long ago
    /// and what was saved; as many as fit from the top of the scroll, and
    /// how many more lie above and below. `now` is seconds since the epoch.
    pub fn draw<X>(
        &self,
        f: &mut ratatui::Frame,
        strip: Rect,
        g: &Gallery<X>,
        look: &Look,
        keys: &Keymap,
        now: u64,
    ) {
        let n = g.entries().len();
        let block = self.block(n, look, keys);
        let inner = block.inner(strip);
        f.render_widget(block, strip);
        let rows = self.rows_per_thumb();
        let shown = thumbs_that_fit(inner.height, rows);
        let mut y = inner.y + 1;
        for (k, e) in g.entries().iter().enumerate().skip(self.top).take(shown) {
            let pic = Rect::new(inner.x + 1, y, inner.width.saturating_sub(1), rows);
            if let Some(Some(t)) = g.thumb(e).and_then(|p| self.thumbs.get(&p)) {
                self.painter.draw(t, f, pic);
            }
            let chosen = self.at == Some(k);
            let text = vec![
                Line::from(Span::styled(
                    format!("{}{}", if chosen { "▸" } else { " " }, e.name()),
                    if chosen { look.selected } else { look.bold },
                )),
                Line::from(Span::styled(
                    format!(" {} · {}", ago(e.when, now), e.what),
                    look.hint,
                )),
            ];
            f.render_widget(
                Paragraph::new(text).style(look.page),
                Rect::new(inner.x, y + rows, inner.width, 2),
            );
            y += rows + 3;
        }
        // Above and below what fits, in the top row and under the last.
        let mut more = |text: String, y: u16| {
            f.render_widget(
                Paragraph::new(Line::from(Span::styled(text, look.hint))).style(look.page),
                Rect::new(inner.x, y, inner.width, 1),
            );
        };
        if self.top > 0 {
            more(format!(" ↑ {} more", self.top), inner.y);
        }
        let below = n.saturating_sub(self.top + shown);
        if below > 0 && y < inner.y + inner.height {
            more(format!(" ↓ {below} more"), y.saturating_sub(1));
        }
    }
}

/// The furthest the strip scrolls over `n` saves, `shown` at once.
fn last_top(n: usize, shown: usize) -> usize {
    n.saturating_sub(shown.max(1))
}

/// How many saves the strip shows in `height` rows, each a thumbnail
/// `rows` tall, two lines of text and a gap, below one row of space.
#[must_use]
pub fn thumbs_that_fit(height: u16, rows: u16) -> usize {
    let each = rows + 3;
    match height.checked_sub(each) {
        Some(rest) => usize::from(rest / each) + 1,
        None => 0,
    }
}

#[cfg(test)]
#[path = "../tests/strip.rs"]
mod tests;
