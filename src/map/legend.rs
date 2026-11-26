//! Keys drawn into the frame: a legend of groups in its top-left corner,
//! and for a number per point a ramp under its title, with labelled ticks
//! and a swatch for points with no value, in a corner it is given. Each returns the area it
//! covers, for labels to keep clear of.
//!
//! Ported from senna (`src/view/render.rs`: `draw_legend`,
//! `draw_ramp_key`), with lupin's `+k more` line past a number of rows
//! (`src/figures/mod.rs` `legend_svg`). The ramp key's title ink is a
//! parameter here, not senna's constant.

use super::colour::Lin;
use super::labels::Rect;
use super::raster::{blend_round, Canvas, Frame};
use super::text::{self, Font};

/// A key in the top-left corner: a dot in the point colour and the name in
/// the label ink, one per line, no frame. With `max_rows` set to `(m, ink)`
/// and more than `m` entries, the first `m - 1` are listed and a last line
/// in `ink` counts the rest (`+k more`). Returns the area it covers, so map
/// labels can keep clear of it.
pub fn draw_legend(
    frame: &mut Frame,
    entries: &[(String, Lin, Lin)],
    font: Font,
    max_rows: Option<(usize, Lin)>,
) -> Option<Rect> {
    if entries.is_empty() || max_rows.is_some_and(|(m, _)| m == 0) {
        return None;
    }
    let (shown, more) = match max_rows {
        Some((m, ink)) if entries.len() > m => (
            &entries[..m - 1],
            Some((format!("+{} more", entries.len() - (m - 1)), ink)),
        ),
        _ => (entries, None),
    };
    let bg = frame.background();
    let lh = font.line_height() as i32;
    let r = lh as f32 * 0.22;
    let (x0, mut y) = (lh, lh / 2);
    let text_x = x0 + lh / 2 + 2;
    let widest = shown
        .iter()
        .map(|(n, ..)| font.width(n))
        .chain(more.iter().map(|(n, _)| font.width(n)))
        .max()
        .unwrap_or(0) as i32;
    let rows = shown.len() + usize::from(more.is_some());
    let area = [
        0.0,
        0.0,
        (text_x + widest + lh / 2) as f32,
        (y + lh * rows as i32 + lh / 2) as f32,
    ];
    for (name, dot, ink) in shown {
        if y + lh > frame.h as i32 {
            return Some(area);
        }
        let (cx, cy) = (x0 as f32, y as f32 + 0.5 * lh as f32);
        blend_round(frame, cx, cy, r + 1.0, *dot, |d| r + 0.5 - d);
        text::draw(frame, font, name, text_x, y, *ink, bg);
        y += lh;
    }
    if let Some((line, ink)) = more {
        if y + lh <= frame.h as i32 {
            text::draw(frame, font, &line, text_x, y, ink, bg);
        }
    }
    Some(area)
}

/// Where a ramp key goes. In a frame the viewer draws, a corner of it; in
/// a figure panel ([`crate::export::Panel`]) `Right` and `Below` put it
/// outside the map, beside it or under it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum KeyPlace {
    #[default]
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
    /// Beside the map at its right; a frame's top-right corner.
    Right,
    /// Under the map; a frame's bottom-left corner.
    Below,
}

impl KeyPlace {
    /// Whether at the right of its box, and whether at the bottom.
    pub(crate) fn sides(self) -> (bool, bool) {
        match self {
            Self::TopLeft => (false, false),
            Self::TopRight | Self::Right => (true, false),
            Self::BottomLeft | Self::Below => (false, true),
            Self::BottomRight => (true, true),
        }
    }
}

/// A key for a number per point: its title, the ramp as a bar, labelled
/// ticks under the bar, and a swatch for points with no value.
#[derive(Clone, Debug, Default)]
pub struct RampKey<'a> {
    pub title: &'a str,
    /// The bar's colours, low end at the left.
    pub ramp: &'a [Lin],
    /// Ticks under the bar: where along it, 0 to 1, and the label.
    pub ticks: &'a [(f32, String)],
    /// The bar's length in line heights; `None`: eight, or half the
    /// title's width when that is longer.
    pub length: Option<f32>,
    /// A swatch after the bar for points with no value: its label and
    /// colour.
    pub missing: Option<(&'a str, Lin)>,
}

/// A ramp key's layout in pixels, its top left at the origin, and its
/// title as it fits.
struct RampLayout {
    title: String,
    w: f32,
    h: f32,
    /// The bar's left, top, width and height.
    bar: [f32; 4],
    tick_h: f32,
    /// Where the swatch's dot is centred, when there is one.
    missing_at: Option<(f32, f32)>,
}

impl RampKey<'_> {
    fn layout(&self, font: Font, max_w: f32) -> RampLayout {
        let lh = font.line_height() as f32;
        let title = self.title_within(font, max_w);
        let title_w = font.width(&title) as f32;
        let bw = self
            .length
            .map_or_else(|| (8.0 * lh).max(title_w / 2.0), |l| (l * lh).max(2.0))
            .round();
        let bh = ((lh as i32 / 4).max(3)) as f32;
        // Tick labels may stand out half their width past either end.
        let overhang = |k: usize| {
            self.ticks
                .get(k)
                .map_or(0.0, |(_, l)| 0.5 * font.width(l) as f32)
        };
        let left = overhang(0);
        let right = overhang(self.ticks.len().wrapping_sub(1));
        let bar_x = left.ceil();
        let (bar_y, tick_h) = (
            if self.title.is_empty() { 0.0 } else { lh + 2.0 },
            (bh / 2.0).max(2.0).round(),
        );
        let missing_at = self.missing.map(|_| (bar_x + bw + lh, bar_y + 0.5 * bh));
        let missing_w = self
            .missing
            .map_or(0.0, |(l, _)| lh * 1.5 + font.width(l) as f32);
        let ticks_h = if self.ticks.is_empty() {
            0.0
        } else {
            tick_h + 1.0 + lh
        };
        RampLayout {
            title,
            w: title_w.max(bar_x + bw + right.max(missing_w)).ceil(),
            h: (bar_y + bh + ticks_h).ceil(),
            bar: [bar_x, bar_y, bw, bh],
            tick_h,
            missing_at,
        }
    }

    /// Its width and height in pixels in `font`, margins left out.
    #[must_use]
    pub fn size(&self, font: Font) -> (f32, f32) {
        self.size_within(font, f32::INFINITY)
    }

    /// [`RampKey::size`] with the title shortened to `max_w` pixels.
    #[must_use]
    pub fn size_within(&self, font: Font, max_w: f32) -> (f32, f32) {
        let l = self.layout(font, max_w);
        (l.w, l.h)
    }

    /// The title, cut short with `...` to fit `max_w` pixels (the raster
    /// font has no ellipsis).
    fn title_within(&self, font: Font, max_w: f32) -> String {
        if font.width(self.title) as f32 <= max_w {
            return self.title.to_string();
        }
        // The longest start that fits with the dots, by bisection over the
        // characters' ends.
        let ends: Vec<usize> = self.title.char_indices().map(|(i, _)| i).collect();
        let fits = |k: usize| font.width(&format!("{}...", &self.title[..ends[k]])) as f32 <= max_w;
        let (mut lo, mut hi) = (0, ends.len() - 1);
        while lo < hi {
            let mid = (lo + hi).div_ceil(2);
            if fits(mid) {
                lo = mid;
            } else {
                hi = mid - 1;
            }
        }
        format!("{}...", self.title[..ends[lo]].trim_end())
    }
}

/// Draw `key` in `font` and `ink` inside `bounds` of `frame`, half a line
/// in from the corner `place` says. Tick labels that would run into the
/// one before are left out. Returns the area it covers, for labels to keep
/// clear of.
pub fn draw_ramp_key_in(
    frame: &mut Frame,
    key: &RampKey<'_>,
    font: Font,
    ink: Lin,
    bounds: Rect,
    place: KeyPlace,
) -> Rect {
    let bg = frame.background();
    let lh = font.line_height() as f32;
    // Half a line in whole pixels, as the legend's margins are.
    let half = (0.5 * lh).floor();
    // A title wider than the box is cut short to fit it.
    let max_w = (bounds[2] - bounds[0] - 2.0 * half).max(lh);
    let l = key.layout(font, max_w);
    let title = &l.title;
    let (right, bottom) = place.sides();
    let x0 = if right {
        bounds[2] - half - l.w
    } else {
        bounds[0] + half
    };
    let y0 = if bottom {
        bounds[3] - half - l.h
    } else {
        bounds[1] + half
    };
    let (x0, y0) = (x0.max(bounds[0]).round(), y0.max(bounds[1]).round());
    if !title.is_empty() {
        text::draw(frame, font, title, x0 as i32, y0 as i32, ink, bg);
    }
    let [bx, by, bw, bh] = l.bar;
    let (bx, by) = (x0 + bx, y0 + by);
    let put = |frame: &mut Frame, x: f32, y: f32, c: Lin| {
        let (x, y) = (x as i64, y as i64);
        if x >= 0 && y >= 0 && (x as usize) < frame.w && (y as usize) < frame.h {
            frame.blend(x as usize, y as usize, c, 1.0);
        }
    };
    let n = bw as usize;
    if !key.ramp.is_empty() {
        for dx in 0..n {
            let k = (dx * (key.ramp.len() - 1)) / n.saturating_sub(1).max(1);
            for dy in 0..bh as usize {
                put(frame, bx + dx as f32, by + dy as f32, key.ramp[k]);
            }
        }
    }
    // Ticks: a mark under the bar and its label centred under the mark,
    // unless it would touch the label before.
    let tick_w = (lh / 12.0).max(1.0).round() as usize;
    let mut last_right = f32::NEG_INFINITY;
    for (t, label) in key.ticks {
        let x = (bx + t.clamp(0.0, 1.0) * (bw - tick_w as f32)).round();
        for dx in 0..tick_w {
            for dy in 0..l.tick_h as usize {
                put(frame, x + dx as f32, by + bh + dy as f32, ink);
            }
        }
        let w = font.width(label) as f32;
        let lx = (x - 0.5 * w).round();
        if lx < last_right + 0.4 * lh {
            continue;
        }
        last_right = lx + w;
        let ly = (by + bh + l.tick_h + 1.0) as i32;
        text::draw_aligned(frame, font, label, (lx as i32, ly), true, ink, bg);
    }
    if let (Some((label, colour)), Some((cx, cy))) = (key.missing, l.missing_at) {
        let (cx, cy) = (x0 + cx, y0 + cy);
        let r = (0.3 * lh).max(bh / 2.0);
        blend_round(frame, cx, cy, r + 1.0, colour, |d| r + 0.5 - d);
        let ty = (cy - 0.5 * lh).round() as i32;
        text::draw(frame, font, label, (cx + r + 0.4 * lh) as i32, ty, ink, bg);
    }
    [x0 - half, y0 - half, x0 + l.w + half, y0 + l.h + half]
}

/// A key for an activity view in the top-left corner: the title in `ink`,
/// then the ramp as a thin bar. Returns the area it covers.
pub fn draw_ramp_key(frame: &mut Frame, title: &str, ramp: &[Lin], font: Font, ink: Lin) -> Rect {
    let bounds = [0.0, 0.0, frame.w as f32, frame.h as f32];
    let key = RampKey {
        title,
        ramp,
        ..RampKey::default()
    };
    draw_ramp_key_in(frame, &key, font, ink, bounds, KeyPlace::TopLeft)
}

#[cfg(test)]
#[path = "../tests/legend.rs"]
mod tests;
