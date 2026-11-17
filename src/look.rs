//! The styles the save dialog, the saved-figures strip, the form's rows and
//! the file browser draw in, so they need no map theme: the terminal's own
//! colours by default, or a map viewer's [`crate::chrome::Chrome`] with
//! feature `map`.

use ratatui::style::{Color, Modifier, Style};

/// Styles for a popup or a strip. Built with [`Look::new`] or
/// [`Look::terminal`]: more styles may come, so not field by field.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
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
    /// What wants attention: a warning, a refusal, clap's complaint.
    pub accent: Style,
}

/// [`Look::terminal`]'s accent: bold yellow.
const ACCENT: Style = Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD);

impl Look {
    /// A look of these styles.
    #[must_use]
    pub const fn new(
        page: Style,
        bold: Style,
        hint: Style,
        selected: Style,
        border: Style,
        accent: Style,
    ) -> Self {
        Self {
            page,
            bold,
            hint,
            selected,
            border,
            accent,
        }
    }

    /// The terminal's colours: bold, grey hints, the cursor's line
    /// reversed (pinto's), warnings bold yellow.
    #[must_use]
    pub const fn terminal() -> Self {
        let hint = Style::new().fg(Color::DarkGray);
        Self::new(
            Style::new(),
            Style::new().add_modifier(Modifier::BOLD),
            hint,
            Style::new().add_modifier(Modifier::REVERSED),
            hint,
            ACCENT,
        )
    }

    /// Key names, typed values, the value in focus: the accent in bold.
    #[must_use]
    pub const fn highlight(&self) -> Style {
        self.accent.add_modifier(Modifier::BOLD)
    }

    /// This look with `accent` for what wants attention.
    #[must_use]
    pub const fn with_accent(mut self, accent: Style) -> Self {
        self.accent = accent;
        self
    }

    /// This look with `border` for borders and rules: a look cannot be
    /// built field by field, so a change of one style goes through these.
    #[must_use]
    pub const fn with_border(mut self, border: Style) -> Self {
        self.border = border;
        self
    }

    /// This look with `page` for the page.
    #[must_use]
    pub const fn with_page(mut self, page: Style) -> Self {
        self.page = page;
        self
    }

    /// This look with `hint` for key hints and secondary lines.
    #[must_use]
    pub const fn with_hint(mut self, hint: Style) -> Self {
        self.hint = hint;
        self
    }
}

impl Default for Look {
    fn default() -> Self {
        Self::terminal()
    }
}

#[cfg(feature = "map")]
impl From<crate::chrome::Chrome> for Look {
    /// A screen's chrome: a theme's colours, or the terminal's; the
    /// terminal's accent either way, as a chrome has none of its own.
    fn from(c: crate::chrome::Chrome) -> Self {
        Self::new(
            c.page(),
            c.bold(),
            c.hint(),
            c.selected(),
            c.border(),
            ACCENT,
        )
    }
}

#[cfg(test)]
#[path = "tests/look.rs"]
mod tests;
