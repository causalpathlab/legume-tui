//! The styles the screen loop's chrome and the histograms draw in beyond
//! a [`Look`]'s own: the accent, and the accent in bold for keys and the
//! value in focus.

use crate::look::Look;
use ratatui::style::{Modifier, Style};

/// Accent bars and marks.
pub(crate) fn accent(look: &Look) -> Style {
    look.accent
}

/// Key names, typed values, the value in focus: the accent in bold.
pub(crate) fn highlight(look: &Look) -> Style {
    accent(look).add_modifier(Modifier::BOLD)
}
