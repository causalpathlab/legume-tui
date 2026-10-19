//! The page a map is drawn on: its background, text and the colours of
//! points out of focus, the palette its groups take, and the ramps that
//! fade into it.
//!
//! [`Theme::PAPER`] is senna's page (`src/tui/style.rs` and
//! `src/view/color.rs`); [`Theme::LIGHT`] and [`Theme::DARK`] are pinto's
//! map themes, with [`Theme::for_background`] picking one for the
//! terminal's own background (`src/view/color.rs`).

use super::colour::{self, luminance, Categorical, Lin, Ramp, Srgb, MAGMA};
use std::sync::LazyLock;

/// A map's colours.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Theme {
    /// The page.
    pub background: Srgb,
    /// Rules, and points with no group or outside the focused one.
    pub muted: Srgb,
    /// Menus, popups and the status line: to read at a glance.
    pub text: Srgb,
    /// Key hints and other secondary lines beside `text`.
    pub hint: Srgb,
    /// Text drawn on the map (labels, titles).
    pub ink: Srgb,
    /// Points outside the focused groups: visible as tissue, not as
    /// colour.
    pub dimmed: Srgb,
    /// Points with no group.
    pub no_group: Srgb,
    /// Scale bar fill and the outline that keeps it readable over points.
    pub bar: (Srgb, Srgb),
    /// The groups' colours.
    pub palette: Categorical,
    /// OKLCH lightness of a label's ink beside its group's colour.
    pub ink_lightness: f32,
}

impl Theme {
    /// senna's page: a warm light grey, near-black text, muted points a
    /// step darker than the page.
    pub const PAPER: Theme = Theme {
        background: [246, 245, 242],
        muted: [200, 199, 196],
        text: [30, 30, 32],
        hint: [104, 104, 104],
        ink: [92, 92, 92],
        dimmed: [200, 199, 196],
        no_group: [200, 199, 196],
        bar: ([0; 3], [255; 3]),
        palette: Categorical::SENNA,
        ink_lightness: 0.42,
    };
    /// pinto's light map.
    pub const LIGHT: Theme = Theme {
        background: [238, 238, 236],
        muted: [212, 212, 216],
        text: [30, 30, 32],
        hint: [104, 104, 104],
        ink: [92, 92, 92],
        dimmed: [212, 212, 216],
        no_group: [175, 175, 181],
        bar: ([0; 3], [255; 3]),
        palette: Categorical::PINTO_LIGHT,
        ink_lightness: 0.42,
    };
    /// pinto's dark map.
    pub const DARK: Theme = Theme {
        background: [17, 17, 20],
        muted: [40, 40, 46],
        text: [230, 230, 232],
        hint: [150, 150, 150],
        ink: [190, 190, 190],
        dimmed: [40, 40, 46],
        no_group: [70, 70, 76],
        bar: ([255; 3], [0; 3]),
        palette: Categorical::PINTO_DARK,
        ink_lightness: 0.80,
    };

    /// The theme for a background colour: light when its luminance is above
    /// middle grey (18% in linear light).
    #[must_use]
    pub fn for_background(bg: Srgb) -> Theme {
        if luminance(bg) > 0.18 {
            Theme::LIGHT
        } else {
            Theme::DARK
        }
    }

    /// Whether the page is dark, by the same rule.
    #[must_use]
    pub fn is_dark(&self) -> bool {
        luminance(self.background) <= 0.18
    }

    /// The other one of pinto's two, for the theme key.
    #[must_use]
    pub fn other(&self) -> Theme {
        if self.is_dark() {
            Theme::LIGHT
        } else {
            Theme::DARK
        }
    }

    /// Label ink for points drawn in `c`.
    #[must_use]
    pub fn ink_for(&self, c: Lin) -> Lin {
        colour::ink_for(c, self.ink_lightness)
    }

    /// Ramp for one group's propensity: from the background's side up to
    /// the strongest colour, so low values fade into the page.
    #[must_use]
    pub fn magma(&self) -> &'static Ramp {
        // Magma run backwards from near-white, without its black and its
        // pale ends.
        static LIGHT: LazyLock<Ramp> = LazyLock::new(|| {
            let stops: Vec<u32> = std::iter::once(0xf4f4f0)
                .chain(MAGMA[1..MAGMA.len() - 1].iter().rev().copied())
                .collect();
            Ramp::from_stops(&stops)
        });
        if self.is_dark() {
            colour::magma()
        } else {
            &LIGHT
        }
    }

    /// Ramp for feature levels: from the dimmed tissue grey at zero (of
    /// [`Theme::LIGHT`] or [`Theme::DARK`]), so points without the feature
    /// still show the tissue, to the colour furthest from the background.
    #[must_use]
    pub fn expression(&self) -> &'static Ramp {
        fn hex([r, g, b]: Srgb) -> u32 {
            u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b)
        }
        static DARK: LazyLock<Ramp> = LazyLock::new(|| {
            Ramp::from_stops(&[
                hex(Theme::DARK.dimmed),
                0x4f127b,
                0xb5367a,
                0xfb8761,
                0xfcfdbf,
            ])
        });
        static LIGHT: LazyLock<Ramp> = LazyLock::new(|| {
            Ramp::from_stops(&[
                hex(Theme::LIGHT.dimmed),
                0xfb8761,
                0xe55064,
                0x812581,
                0x1c1044,
            ])
        });
        if self.is_dark() {
            &DARK
        } else {
            &LIGHT
        }
    }
}

#[cfg(test)]
#[path = "../tests/theme.rs"]
mod tests;
