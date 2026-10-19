//! The screen around a map: text, hint and menu styles from a map
//! [`Theme`] or the terminal's own colours, windows that keep a row in
//! sight, names cut to fit, and bordered popups.
//!
//! senna's styles and popups (`src/tui/style.rs`, `wrapped_rows` from
//! `src/tui/mod.rs`) on its page's colours, and pinto's
//! (`src/tui/style.rs`, with `short`, `middle`, `tail` and `wrap`) on the
//! terminal's: `Chrome { theme: None }` is pinto's, so both read on a dark
//! or a light terminal. [`first_row`] was the same in both.

use crate::map::colour::Srgb;
use crate::map::theme::Theme;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};

/// An sRGB colour for ratatui.
#[must_use]
pub fn rgb(c: Srgb) -> Color {
    Color::Rgb(c[0], c[1], c[2])
}

/// The styles of a screen: a theme's colours, or none for the terminal's.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Chrome {
    pub theme: Option<Theme>,
}

/// Where a popup sits over its area.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum At {
    Top,
    Middle,
}

impl Chrome {
    /// The terminal's own colours, with bold for what matters, grey for
    /// hints, and the line under a cursor reversed.
    pub const TERMINAL: Chrome = Chrome { theme: None };

    /// A theme's colours.
    #[must_use]
    pub const fn themed(theme: Theme) -> Chrome {
        Chrome { theme: Some(theme) }
    }

    /// Bold text.
    #[must_use]
    pub fn bold(&self) -> Style {
        Style::default().add_modifier(Modifier::BOLD)
    }

    /// The page's colours: ink on the map's background.
    #[must_use]
    pub fn page(&self) -> Style {
        match self.theme {
            Some(t) => Style::default().bg(rgb(t.background)).fg(rgb(t.text)),
            None => Style::default(),
        }
    }

    /// Key hints and other secondary lines: a step lighter than the text.
    #[must_use]
    pub fn hint(&self) -> Style {
        match self.theme {
            Some(t) => Style::default().fg(rgb(t.hint)),
            None => Style::default().fg(Color::DarkGray),
        }
    }

    /// The line under a menu's cursor: a dark bar on a page, so where you
    /// are is plain; reversed on the terminal.
    #[must_use]
    pub fn selected(&self) -> Style {
        match self.theme {
            Some(t) => Style::default()
                .bg(rgb(t.text))
                .fg(rgb(t.background))
                .patch(self.bold()),
            None => Style::default().add_modifier(Modifier::REVERSED),
        }
    }

    /// A popup's border.
    #[must_use]
    pub fn border(&self) -> Style {
        match self.theme {
            Some(t) => Style::default().fg(rgb(t.text)),
            None => self.hint(),
        }
    }

    /// `rows` scrolled to keep row `at` in sight within `room` rows; the
    /// first and last rows shown say how many are cut off above and below.
    #[must_use]
    pub fn scrolled(&self, rows: Vec<Line<'static>>, at: usize, room: usize) -> Vec<Line<'static>> {
        let room = room.max(1);
        let first = first_row(at, room, rows.len());
        let below = rows.len().saturating_sub(first + room);
        let mut shown: Vec<Line> = rows.into_iter().skip(first).take(room).collect();
        let more = |n: usize, arrow: &str| {
            Line::from(Span::styled(format!(" {arrow} {n} more"), self.hint()))
        };
        if shown.len() > 1 {
            if first > 0 {
                shown[0] = more(first + 1, "↑");
            }
            if below > 0 {
                let last = shown.len() - 1;
                shown[last] = more(below + 1, "↓");
            }
        }
        shown
    }

    /// A bordered popup of `lines` over `area`, at most `max_w` columns wide
    /// and as tall as its lines once wrapped. When they do not fit, the last
    /// line (a popup's keys) keeps the bottom row, so it is never cut off.
    pub fn popup(
        &self,
        f: &mut ratatui::Frame,
        area: Rect,
        mut lines: Vec<Line<'_>>,
        max_w: u16,
        at: At,
    ) {
        let w = max_w.min(area.width);
        let inner = usize::from(w.saturating_sub(2)).max(1);
        let rows: usize = lines
            .iter()
            .map(|l| {
                let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
                wrapped_rows(&text, inner)
            })
            .sum();
        let h = (rows.min(usize::from(u16::MAX)) as u16)
            .saturating_add(2)
            .min(area.height);
        let y = match at {
            At::Top => area.y + 1.min(area.height - h),
            At::Middle => area.y + (area.height - h) / 2,
        };
        let r = Rect::new(area.x + (area.width - w) / 2, y, w, h);
        f.render_widget(Clear, r);
        let block = Block::bordered()
            .border_style(self.border())
            .style(self.page());
        let body = block.inner(r);
        f.render_widget(block, r);
        let wrap = Wrap { trim: false };
        if rows > usize::from(body.height) && lines.len() > 1 && body.height > 1 {
            let last = lines.pop().unwrap_or_default();
            let top = Rect {
                height: body.height - 1,
                ..body
            };
            let bottom = Rect {
                y: body.y + body.height - 1,
                height: 1,
                ..body
            };
            f.render_widget(Paragraph::new(lines).wrap(wrap), top);
            f.render_widget(Paragraph::new(last), bottom);
        } else {
            f.render_widget(Paragraph::new(lines).wrap(wrap), body);
        }
    }

    /// A one-line notice at the top of `area`: an answer came, a file saved.
    pub fn toast(&self, f: &mut ratatui::Frame, area: Rect, text: &str) {
        let line = Line::from(Span::styled(text.to_string(), self.bold())).centered();
        let w = text.chars().count() as u16 + 4;
        self.popup(f, area, vec![line], w, At::Top);
    }
}

/// The first of `n` rows to show in a window `rows` tall so that `row`
/// sits in its middle where it can.
#[must_use]
pub fn first_row(row: usize, rows: usize, n: usize) -> usize {
    row.saturating_sub(rows / 2).min(n.saturating_sub(rows))
}

/// The rows of a page that fit `room` from `scroll`, its furthest scroll put
/// in `max` for the keys that move it, and where it stands for its footer:
/// `"more below · "`, `"end · "`, or `""` when it all fits.
pub fn page_window(
    body: Vec<Line<'static>>,
    room: usize,
    scroll: usize,
    max: &std::cell::Cell<usize>,
) -> (Vec<Line<'static>>, &'static str) {
    let room = room.max(1);
    let furthest = body.len().saturating_sub(room);
    max.set(furthest);
    let skip = scroll.min(furthest);
    let stands = if furthest == 0 {
        ""
    } else if skip < furthest {
        "more below · "
    } else {
        "end · "
    };
    (body.into_iter().skip(skip).take(room).collect(), stands)
}

/// How many rows `text` takes wrapped at word boundaries to `width`
/// columns, as a wrapping paragraph lays it out; a word longer than a row
/// is broken across rows of its own.
#[must_use]
pub fn wrapped_rows(text: &str, width: usize) -> usize {
    let width = width.max(1);
    let (mut rows, mut used) = (1, 0usize);
    for word in text.split(' ') {
        let n = word.chars().count();
        let need = if used == 0 { n } else { used + 1 + n };
        if need <= width {
            used = need;
        } else if n <= width {
            rows += 1;
            used = n;
        } else {
            // Broken across rows of its own.
            rows += usize::from(used > 0) + (n - 1) / width;
            used = n % width;
        }
    }
    rows
}

/// `s` in at most `n` characters, `…` marking a cut at the end.
#[must_use]
pub fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// `s` in at most `n` characters, `…` marking a cut in the middle: the
/// start and the end kept, so names sharing a prefix still differ.
#[must_use]
pub fn middle(s: &str, n: usize) -> String {
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

/// The last `n` characters of `s`, `…` marking a cut at the start.
#[must_use]
pub fn tail(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        s.to_string()
    } else {
        let mut out = String::from("…");
        out.extend(s.chars().skip(len + 1 - n.max(1)));
        out
    }
}

/// `s` in pieces of at most `first` characters, then `rest` for each
/// piece after the first; none for an empty `s`.
#[must_use]
pub fn wrap(s: &str, first: usize, rest: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let (head, tail) = chars.split_at(first.max(1).min(chars.len()));
    std::iter::once(head)
        .filter(|h| !h.is_empty())
        .chain(tail.chunks(rest.max(1)))
        .map(|c| c.iter().collect())
        .collect()
}

#[cfg(test)]
#[path = "tests/chrome.rs"]
mod tests;
