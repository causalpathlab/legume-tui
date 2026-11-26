//! Named ramps a map colours a number per point on: the viewer's own
//! activity ramp, viridis, magma and inferno, ColorBrewer's greys, reds and
//! blues, and two diverging ramps for signed values about a midpoint.
//!
//! The ColorBrewer ramps leave out their palest stop, which is all but
//! white and would vanish on a white page.

use super::colour::{activity_ramp, diverging_lin, magma, viridis, Lin, Ramp};

/// A ramp by name, low to high.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize,
)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Palette {
    /// The viewer's light-to-deep blue into violet ([`activity_ramp`]).
    #[default]
    Activity,
    Viridis,
    Magma,
    Inferno,
    /// Light grey to black.
    Greys,
    /// Pale to deep red.
    Reds,
    /// Pale to deep blue.
    Blues,
    /// Diverging: blue, near-white at the midpoint, red ([`diverging_lin`]).
    BlueRed,
    /// Diverging: purple, near-white at the midpoint, green (ColorBrewer PRGn).
    PurpleGreen,
}

const INFERNO: [u32; 9] = [
    0x000004, 0x1b0c41, 0x4a0c6b, 0x781c6d, 0xa52c60, 0xcf4446, 0xed6925, 0xfb9b06, 0xfcffa4,
];
const GREYS: [u32; 8] = [
    0xf0f0f0, 0xd9d9d9, 0xbdbdbd, 0x969696, 0x737373, 0x525252, 0x252525, 0x000000,
];
const REDS: [u32; 8] = [
    0xfee0d2, 0xfcbba1, 0xfc9272, 0xfb6a4a, 0xef3b2c, 0xcb181d, 0xa50f15, 0x67000d,
];
const BLUES: [u32; 8] = [
    0xdeebf7, 0xc6dbef, 0x9ecae1, 0x6baed6, 0x4292c6, 0x2171b5, 0x08519c, 0x08306b,
];
const PURPLE_GREEN: [u32; 11] = [
    0x40004b, 0x762a83, 0x9970ab, 0xc2a5cf, 0xe7d4e8, 0xf7f7f7, 0xd9f0d3, 0xa6dba0, 0x5aae61,
    0x1b7837, 0x00441b,
];

impl Palette {
    /// Every palette, sequential ones first.
    pub const ALL: [Self; 9] = [
        Self::Activity,
        Self::Viridis,
        Self::Magma,
        Self::Inferno,
        Self::Greys,
        Self::Reds,
        Self::Blues,
        Self::BlueRed,
        Self::PurpleGreen,
    ];

    /// The name it is saved and asked for by (`blue_red`).
    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Self::Activity => "activity",
            Self::Viridis => "viridis",
            Self::Magma => "magma",
            Self::Inferno => "inferno",
            Self::Greys => "greys",
            Self::Reds => "reds",
            Self::Blues => "blues",
            Self::BlueRed => "blue_red",
            Self::PurpleGreen => "purple_green",
        }
    }

    /// The palette named `name`, as [`Palette::name`] gives it.
    #[must_use]
    pub fn from_name(name: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|p| p.name() == name)
    }

    /// Whether it runs from one colour through a light middle to another,
    /// for values either side of a midpoint.
    #[must_use]
    pub fn is_diverging(self) -> bool {
        matches!(self, Self::BlueRed | Self::PurpleGreen)
    }

    /// The ColorBrewer and matplotlib stops a ramp is made of, for those
    /// not drawn by a function.
    fn stops(self) -> Option<&'static [u32]> {
        match self {
            Self::Inferno => Some(&INFERNO),
            Self::Greys => Some(&GREYS),
            Self::Reds => Some(&REDS),
            Self::Blues => Some(&BLUES),
            Self::PurpleGreen => Some(&PURPLE_GREEN),
            _ => None,
        }
    }

    /// `n` colours from low to high in linear light, high to low when
    /// `reverse`.
    #[must_use]
    pub fn lin(self, n: usize, reverse: bool) -> Vec<Lin> {
        let mut out = match (self, self.stops()) {
            (_, Some(stops)) => Ramp::from_stops(stops).steps(n),
            (Self::Viridis, _) => viridis().steps(n),
            (Self::Magma, _) => magma().steps(n),
            (Self::BlueRed, _) => (0..n)
                .map(|i| diverging_lin(2.0 * i as f32 / (n - 1).max(1) as f32 - 1.0))
                .collect(),
            _ => activity_ramp(n),
        };
        if reverse {
            out.reverse();
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_palette_is_named_and_sampled_low_to_high() {
        for p in Palette::ALL {
            assert_eq!(Palette::from_name(p.name()), Some(p));
            let ramp = p.lin(256, false);
            assert_eq!(ramp.len(), 256);
            let mut back = p.lin(256, true);
            back.reverse();
            assert_eq!(back, ramp);
        }
        // Greys darken: the luminance of the low end is above the high's.
        let g = Palette::Greys.lin(2, false);
        assert!(g[0][1] > g[1][1]);
        // A diverging ramp is light in the middle.
        let d = Palette::PurpleGreen.lin(3, false);
        assert!(d[1][1] > d[0][1] && d[1][1] > d[2][1]);
        assert!(Palette::BlueRed.is_diverging() && !Palette::Viridis.is_diverging());
    }
}
