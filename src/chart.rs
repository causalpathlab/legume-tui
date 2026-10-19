//! Charts the map viewers draw in place of a map, into the same [`Frame`]:
//! a structure plot of each row's mixture as stacked bars, in panels by
//! group ([`structure`]), and a heatmap of the features that lead each
//! group by the widest margin ([`heatmap`]). Both keep their text as text
//! when the frame does, for a vector page.
//!
//! Ported from senna (`src/view/chart.rs`) and pinto
//! (`src/view/{structure,heatmap}.rs`, `src/view/tui/plots.rs`). What
//! follows is shared by both: a [`note`] where a chart cannot be drawn,
//! and [`step_top`], the `+` / `-` steps through how many features a
//! heatmap shows.

pub mod heatmap;
pub mod structure;

use crate::map::colour::to_linear;
use crate::map::raster::Frame;
use crate::map::text::{self, Font};
use crate::map::theme::Theme;

/// One line of text where the chart would be, in the page's text colour.
pub fn note(frame: &mut Frame, s: &str, font: Font, theme: &Theme) {
    let bg = frame.background();
    let lh = font.line_height() as i32;
    text::draw(frame, font, s, lh, lh, to_linear(theme.text), bg);
}

/// How [`step_top`] steps.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Step {
    /// Up by a quarter, down by a fifth, each rounded up.
    Quarter,
    /// One at a time.
    One,
}

/// How many features per group after one `+` (`more`) or `-` from `now`,
/// kept within `range` (inclusive).
#[must_use]
pub fn step_top(now: usize, more: bool, step: Step, (lo, hi): (usize, usize)) -> usize {
    let next = match (step, more) {
        (Step::Quarter, true) => now + now.div_ceil(4),
        (Step::Quarter, false) => now.saturating_sub(now.div_ceil(5)),
        (Step::One, true) => now + 1,
        (Step::One, false) => now.saturating_sub(1),
    };
    next.clamp(lo, hi)
}

#[cfg(test)]
#[path = "tests/chart.rs"]
mod tests;
