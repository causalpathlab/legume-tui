//! The styles the save dialog, the saved-figures strip, the form's rows and
//! the file browser draw in, so they need no map theme: the terminal's own
//! colours by default, or a map viewer's [`crate::chrome::Chrome`] with
//! feature `map`.

use ratatui::style::{Color, Modifier, Style};

/// Styles for a popup or a strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Look {
    /// The page: text on the background.
    pub page: Style,
    /// What matters.
    pub bold: Style,
    /// Key hints and other secondary lines.
    pub hint: Style,
    /// The line under a cursor.
    pub selected: Style,
    /// Borders and rules.
    pub border: Style,
}

impl Look {
    /// The terminal's colours: bold, grey hints, the cursor's line
    /// reversed (pinto's).
    #[must_use]
    pub fn terminal() -> Self {
        let hint = Style::default().fg(Color::DarkGray);
        Self {
            page: Style::default(),
            bold: Style::default().add_modifier(Modifier::BOLD),
            hint,
            selected: Style::default().add_modifier(Modifier::REVERSED),
            border: hint,
        }
    }
}

impl Default for Look {
    fn default() -> Self {
        Self::terminal()
    }
}

#[cfg(feature = "map")]
impl From<crate::chrome::Chrome> for Look {
    /// A screen's chrome: a theme's colours, or the terminal's.
    fn from(c: crate::chrome::Chrome) -> Self {
        Self {
            page: c.page(),
            bold: c.bold(),
            hint: c.hint(),
            selected: c.selected(),
            border: c.border(),
        }
    }
}
