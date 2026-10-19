//! A child's progress bar as a screen draws it: where the bar stands, the
//! scores it has reported (its trend), and whether it is still on the
//! child's own screen.
//!
//! A fit reports a score as the first `key=value` of its bar's message
//! ([`Progress::score`]), once a step, an epoch or a sweep, and the bar
//! redraws it until the next. Each message, up to its score, is one point
//! of the trend: a fit that reports the same score twice tells the two
//! apart before it (`sweep 3: moves=0`), and what follows the score (a
//! rate) changes no point.
//!
//! Whether a bar is on the child's screen is told by order: indicatif
//! clears a bar to redraw it or to print a log line above it, and draws it
//! again at once, while a bar finished and cleared is not drawn again. So
//! a clear ([`Said::Cleared`]) or a log line hides the bar, and its next
//! frame shows it. Where a read of the child's output ends can split a
//! redraw, so a bar is taken off the screen ([`Bar::visible`]) only once
//! it has stayed gone for a [`GRACE`].

use super::{number, scored, Progress, Said};
use std::time::{Duration, Instant};
use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

/// Scores kept in a bar's trend, at most drawn in its sparkline.
pub const TREND: usize = 64;

/// How long a bar stays visible once said gone: longer than a redraw
/// split between reads, and than indicatif's redraw tick.
pub const GRACE: Duration = Duration::from_millis(250);

/// A child's progress bar, followed through what the child says.
#[derive(Clone, Debug)]
pub struct Bar {
    /// Where it stands.
    pub now: Progress,
    /// One point per score it reported, the newest last.
    pub trend: Vec<f64>,
    /// Whether it is on the child's screen: no log line, and no clear
    /// ([`Said::Cleared`]), since its last frame. [`Bar::visible`] waits a
    /// [`GRACE`] before taking it off.
    pub shown: bool,
    /// The message, up to its score, the newest point came from.
    scored: String,
    /// When it was first said gone since its last frame.
    gone: Option<Instant>,
}

impl Bar {
    /// Follow `said`, a child's next word, into `bar`. A frame moves the
    /// bar, or starts a new one (none before, restarted, another length
    /// or name: its scores are not on the last one's scale), and a new
    /// score is a new point. A log line or a clear hides the bar.
    pub fn track(bar: &mut Option<Bar>, said: &Said) {
        let p = match said {
            Said::Progress(p) => p,
            Said::Line(_) | Said::Cleared => {
                if let Some(b) = bar {
                    b.shown = false;
                    b.gone.get_or_insert_with(Instant::now);
                }
                return;
            }
        };
        let same = |b: &Bar| p.pos >= b.now.pos && same_bar(p, &b.now);
        let b = match bar {
            Some(b) if same(b) => {
                b.now.clone_from(p);
                b.shown = true;
                b.gone = None;
                b
            }
            _ => bar.insert(Bar {
                now: p.clone(),
                trend: Vec::new(),
                shown: true,
                scored: String::new(),
                gone: None,
            }),
        };
        if let Some((said, v)) = scored(&p.what).filter(|(said, _)| *said != b.scored) {
            if b.trend.len() >= TREND {
                b.trend.remove(0);
            }
            b.trend.push(v);
            b.scored = said.to_string();
        }
    }

    /// Whether to draw it, as a screen should: on the child's screen, or
    /// gone for less than a [`GRACE`] (a redraw split between reads, about
    /// to be whole). It changes with time alone, so a screen drawing from
    /// it repaints within a [`GRACE`] of the bar's last word.
    #[must_use]
    pub fn visible(&self) -> bool {
        self.gone.is_none_or(|t| t.elapsed() < GRACE)
    }
}

/// Whether two frames are of one bar: one length, one name.
pub(super) fn same_bar(a: &Progress, b: &Progress) -> bool {
    a.len == b.len && label(&a.what).eq(label(&b.what))
}

/// A bar message's name: its words that carry no number, so a sweep's
/// count or score does not make it another bar (`phase=fit` stays).
fn label(what: &str) -> impl Iterator<Item = &str> {
    what.split_whitespace()
        .filter(|w| !w.chars().any(|c| c.is_ascii_digit()) && number(w).is_none())
}

/// `s` cut to at most `cells` terminal cells, an ellipsis where it is cut.
fn cut(s: &str, cells: usize) -> String {
    if s.width() <= cells {
        return s.to_string();
    }
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = c.width().unwrap_or(0);
        if used + w + 1 > cells {
            break;
        }
        out.push(c);
        used += w;
    }
    if cells > 0 {
        out.push('…');
    }
    out
}

/// A progress bar `width` cells wide with its count and what it counts;
/// a spinner, with no count, as what it is doing. What it counts is cut to
/// `room` cells, and the trend is drawn in what is left of them.
#[must_use]
pub fn gauge(p: &Progress, width: usize, trend: &[f64], room: usize) -> String {
    let head = if p.len == 0 {
        "…".to_string()
    } else {
        let filled = (width as u128 * u128::from(p.pos.min(p.len)) / u128::from(p.len)) as usize;
        format!(
            "{}{} {}/{}",
            "█".repeat(filled),
            "░".repeat(width - filled),
            p.pos,
            p.len
        )
    };
    let what = cut(&p.what, room.saturating_sub(head.width() + 1));
    let line = format!("{head} {what}");
    let s = spark(trend, room.saturating_sub(line.width() + 2));
    if s.is_empty() {
        line
    } else {
        format!("{line}  {s}")
    }
}

/// The last `n` (at most [`TREND`]) scores as blocks, lowest to highest
/// over their own range; nothing until there are two.
#[must_use]
pub fn spark(v: &[f64], n: usize) -> String {
    const BLOCKS: [char; 8] = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let v = &v[v.len().saturating_sub(n.min(TREND))..];
    if v.len() < 2 {
        return String::new();
    }
    let lo = v.iter().copied().fold(f64::INFINITY, f64::min);
    let range = v.iter().copied().fold(f64::NEG_INFINITY, f64::max) - lo;
    v.iter()
        .map(|&x| {
            let r = if range > 0.0 { (x - lo) / range } else { 0.0 };
            BLOCKS[((r * 7.0).round() as usize).min(7)]
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/bar.rs"]
mod tests;
