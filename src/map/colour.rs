//! Colour: linear light and 8-bit sRGB, OKLCH, categorical palettes and
//! 256-entry ramps.
//!
//! A renderer blends in linear light ([`Lin`], `[0, 1]`) and converts to
//! sRGB bytes ([`Srgb`]) once a pixel, through tables: [`to_linear`] reads
//! a byte back exactly, and [`to_srgb`] gives it again, so colours a tool
//! keeps on disk in sRGB survive the trip.
//!
//! Ported from senna (`src/view/color.rs`: OKLCH, OKLab, the categorical
//! palette, label ink, the activity ramp) and pinto (`src/view/color.rs`:
//! the decode and encode tables, its palettes, [`Ramp`], [`viridis`],
//! [`magma`], [`diverging`]). The maths is theirs, operation for
//! operation, so each tool draws the colours it drew before.

use std::sync::LazyLock;

/// Linear light, each channel in `[0, 1]`.
pub type Lin = [f32; 3];
/// 8-bit sRGB.
pub type Srgb = [u8; 3];

/// sRGB byte → linear light, by the formula.
#[must_use]
pub fn decode(v: u8) -> f32 {
    let c = f32::from(v) / 255.;
    if c <= 0.040_45 {
        c / 12.92
    } else {
        ((c + 0.055) / 1.055).powf(2.4)
    }
}

/// [`decode`] by table lookup, for per-pixel use.
#[must_use]
pub fn linear(v: u8) -> f32 {
    static TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| std::array::from_fn(|v| decode(v as u8)));
    TABLE[usize::from(v)]
}

/// An sRGB colour in linear light.
#[must_use]
pub fn to_linear(c: Srgb) -> Lin {
    c.map(linear)
}

/// Linear light → sRGB byte, by the formula.
#[must_use]
pub fn encode_exact(c: f32) -> u8 {
    let c = c.clamp(0., 1.);
    let v = if c <= 0.003_130_8 {
        12.92 * c
    } else {
        1.055 * c.powf(1. / 2.4) - 0.055
    };
    (v * 255.).round() as u8
}

/// Steps of the linear-light table behind [`encode`]: finer than a byte
/// of sRGB needs anywhere but the darkest few codes.
const ENCODE_STEPS: usize = 4096;

/// [`encode_exact`] by table lookup, for per-pixel use: within one step of
/// it, and exact for every byte [`linear`] gives.
#[must_use]
pub fn encode(c: f32) -> u8 {
    static TABLE: LazyLock<Vec<u8>> = LazyLock::new(|| {
        (0..=ENCODE_STEPS)
            .map(|i| encode_exact(i as f32 / ENCODE_STEPS as f32))
            .collect()
    });
    TABLE[(c.clamp(0., 1.) * ENCODE_STEPS as f32).round() as usize]
}

/// A linear colour in sRGB, by table.
#[must_use]
pub fn to_srgb(c: Lin) -> Srgb {
    c.map(encode)
}

/// Relative luminance of an sRGB colour: 0 for black, 1 for white.
#[must_use]
pub fn luminance([r, g, b]: Srgb) -> f32 {
    0.2126 * linear(r) + 0.7152 * linear(g) + 0.0722 * linear(b)
}

/// OKLCH (lightness, chroma, hue in degrees) → linear sRGB, clamped.
#[must_use]
pub fn oklch(l: f32, c: f32, h_deg: f32) -> Lin {
    let h = h_deg.to_radians();
    let (a, b) = (c * h.cos(), c * h.sin());
    let l_ = l + 0.396_337_78 * a + 0.215_803_76 * b;
    let m_ = l - 0.105_561_346 * a - 0.063_854_17 * b;
    let s_ = l - 0.089_484_18 * a - 1.291_485_5 * b;
    let (l3, m3, s3) = (l_ * l_ * l_, m_ * m_ * m_, s_ * s_ * s_);
    [
        4.076_741_7 * l3 - 3.307_711_6 * m3 + 0.230_969_94 * s3,
        -1.268_438 * l3 + 2.609_757_4 * m3 - 0.341_319_38 * s3,
        -0.004_196_086_3 * l3 - 0.703_418_6 * m3 + 1.707_614_7 * s3,
    ]
    .map(|v| v.clamp(0.0, 1.0))
}

/// Linear sRGB → OKLab.
#[must_use]
pub fn oklab(c: Lin) -> [f32; 3] {
    let [r, g, b] = c;
    let l = (0.412_221_46 * r + 0.536_332_55 * g + 0.051_445_995 * b).cbrt();
    let m = (0.211_903_5 * r + 0.680_699_5 * g + 0.107_396_96 * b).cbrt();
    let s = (0.088_302_46 * r + 0.281_718_85 * g + 0.629_978_7 * b).cbrt();
    [
        0.210_454_26 * l + 0.793_617_8 * m - 0.004_072_047 * s,
        1.977_998_5 * l - 2.428_592_2 * m + 0.450_593_7 * s,
        0.025_904_037 * l + 0.782_771_77 * m - 0.808_675_77 * s,
    ]
}

/// Label ink for points drawn in `c`: the same hue at `lightness`, and a
/// little less saturated, so text stays legible on the page (senna takes
/// 0.42 on its light page).
#[must_use]
pub fn ink_for(c: Lin, lightness: f32) -> Lin {
    let [_, a, b] = oklab(c);
    oklch(lightness, a.hypot(b).min(0.12), b.atan2(a).to_degrees())
}

/// The golden angle: neighbouring ids never get neighbouring hues however
/// many categories there are.
const GOLDEN_ANGLE: f32 = 137.507_76;

/// A categorical palette in OKLCH: golden-angle hues from `hue0`, at a few
/// lightness and chroma steps.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Categorical {
    /// Hue of category 0, in degrees.
    pub hue0: f32,
    /// Lightness steps, taken in turn.
    pub l: &'static [f32],
    /// Chroma steps, taken in turn with the lightness.
    pub c: &'static [f32],
    /// Up to this many categories all take the first step; past it the
    /// steps alternate. 0: they always alternate.
    pub cycle_after: usize,
    /// Whether the hue is taken modulo 360° before it is turned (senna's);
    /// pinto's is not. The two differ in the last bits of a few colours.
    pub wrap_hue: bool,
}

impl Categorical {
    /// senna's: equal lightness keeps any one group from shouting; past a
    /// dozen groups lightness alternates to buy separation.
    pub const SENNA: Self = Self {
        hue0: 25.0,
        l: &[0.64, 0.76, 0.52],
        c: &[0.15],
        cycle_after: 12,
        wrap_hue: true,
    };
    /// pinto's on a light background: darker steps, a little more
    /// saturated, so neighbouring ids never share a hue or a lightness.
    pub const PINTO_LIGHT: Self = Self {
        hue0: 20.0,
        l: &[0.58, 0.47, 0.68],
        c: &[0.15, 0.14, 0.14],
        cycle_after: 0,
        wrap_hue: false,
    };
    /// pinto's on a dark background.
    pub const PINTO_DARK: Self = Self {
        hue0: 20.0,
        l: &[0.72, 0.60, 0.82],
        c: &[0.13, 0.13, 0.10],
        cycle_after: 0,
        wrap_hue: false,
    };

    /// Lightness, chroma and hue of category `i` of `n`.
    fn lch(&self, i: usize, n: usize) -> (f32, f32, f32) {
        let step = if n <= self.cycle_after {
            0
        } else {
            i % self.l.len()
        };
        let hue = self.hue0 + i as f32 * GOLDEN_ANGLE;
        let hue = if self.wrap_hue { hue % 360.0 } else { hue };
        (self.l[step], self.c[step % self.c.len()], hue)
    }

    /// Colour of category `i` of `n`, in linear light.
    #[must_use]
    pub fn colour(&self, i: usize, n: usize) -> Lin {
        let (l, c, h) = self.lch(i, n);
        oklch(l, c, h)
    }

    /// Colour of category `i` of `n` in sRGB, by the formula (pinto's).
    #[must_use]
    pub fn srgb(&self, i: usize, n: usize) -> Srgb {
        self.colour(i, n).map(encode_exact)
    }

    /// The `k` colours of `k` categories in sRGB: pinto's `palette(k)`.
    #[must_use]
    pub fn srgb_all(&self, k: usize) -> Vec<Srgb> {
        (0..k).map(|i| self.srgb(i, k)).collect()
    }
}

/// A 256-entry colour ramp.
#[derive(Clone, Debug, PartialEq)]
pub struct Ramp(Vec<Srgb>);

impl Ramp {
    /// Evenly spaced stops (`0xrrggbb`), interpolated in sRGB.
    ///
    /// # Panics
    /// With fewer than two stops.
    #[must_use]
    pub fn from_stops(stops: &[u32]) -> Self {
        assert!(stops.len() >= 2, "a ramp needs two stops");
        let rgb = |h: u32| {
            [
                (h >> 16) as f32,
                ((h >> 8) & 0xff) as f32,
                (h & 0xff) as f32,
            ]
        };
        let segments = (stops.len() - 1) as f32;
        let lut = (0..256)
            .map(|i| {
                let t = i as f32 / 255. * segments;
                let j = (t.floor() as usize).min(stops.len() - 2);
                let f = t - j as f32;
                let (a, b) = (rgb(stops[j]), rgb(stops[j + 1]));
                [0, 1, 2].map(|c| (a[c] + f * (b[c] - a[c])).round() as u8)
            })
            .collect();
        Ramp(lut)
    }

    /// `f(t)` for `t` in `[0, 1]`, sampled 256 times.
    #[must_use]
    pub fn from_fn(f: impl Fn(f32) -> Lin) -> Self {
        Ramp((0..256).map(|i| to_srgb(f(i as f32 / 255.))).collect())
    }

    /// Colour for `t` in `[0, 1]`; outside it the nearer end, and the low
    /// end for NaN.
    #[must_use]
    pub fn at(&self, t: f32) -> Srgb {
        let t = if t.is_nan() { 0. } else { t };
        self.0[(t.clamp(0., 1.) * 255.).round() as usize]
    }

    #[must_use]
    pub fn at_u8(&self, q: u8) -> Srgb {
        self.0[usize::from(q)]
    }

    /// `n` evenly spaced colours from the low end to the high, in linear
    /// light: a key's swatches.
    #[must_use]
    pub fn steps(&self, n: usize) -> Vec<Lin> {
        (0..n)
            .map(|i| to_linear(self.at(i as f32 / (n - 1).max(1) as f32)))
            .collect()
    }
}

#[must_use]
pub fn viridis() -> &'static Ramp {
    static RAMP: LazyLock<Ramp> = LazyLock::new(|| {
        Ramp::from_stops(&[
            0x440154, 0x472d7b, 0x3b528b, 0x2c728e, 0x21918c, 0x28ae80, 0x5ec962, 0xaddc30,
            0xfde725,
        ])
    });
    &RAMP
}

/// Magma's stops, black to pale yellow.
pub(crate) const MAGMA: [u32; 9] = [
    0x000004, 0x1c1044, 0x4f127b, 0x812581, 0xb5367a, 0xe55064, 0xfb8761, 0xfec287, 0xfcfdbf,
];

#[must_use]
pub fn magma() -> &'static Ramp {
    static RAMP: LazyLock<Ramp> = LazyLock::new(|| Ramp::from_stops(&MAGMA));
    &RAMP
}

/// A diverging colour for `t` in -1..1: blue below 0, near-white at 0,
/// red above, equally light at equal distances (OKLCH, hues 255° and 27°).
#[must_use]
pub fn diverging(t: f32) -> Srgb {
    let t = if t.is_finite() { t.clamp(-1., 1.) } else { 0. };
    let (l, c) = (0.97 - 0.45 * t.abs(), 0.16 * t.abs());
    let hue = if t < 0. { 255. } else { 27. };
    oklch(l, c, hue).map(encode_exact)
}

/// Sequential ramp for feature activity, light to deep, one hue family
/// (cool blue into violet) so it reads as "more" on a light page.
#[must_use]
pub fn activity_ramp(n: usize) -> Vec<Lin> {
    (0..n)
        .map(|i| {
            let t = i as f32 / (n - 1).max(1) as f32;
            oklch(0.88 - 0.52 * t, 0.04 + 0.13 * t, 235.0 + 75.0 * t)
        })
        .collect()
}

/// Ink for a highlighted single feature.
#[must_use]
pub fn highlight_ink() -> Lin {
    oklch(0.36, 0.17, 310.0)
}

#[cfg(test)]
#[path = "../tests/colour.rs"]
mod tests;
