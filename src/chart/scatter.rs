//! A scatter plot drawn into a [`Frame`]: points coloured by series, each
//! at its own opacity, on linear axes with their ends marked, reference
//! lines (labelled where they meet an axis), a key of the series, some
//! points named and some ringed.
//!
//! Points are drawn in the order given, later on top: put the ones that
//! matter last. A point past an axis's range is drawn on its edge, so a
//! long tail stays in view; a point with a NaN coordinate is not drawn.
//! Text is kept as text when the frame keeps it.
//!
//! [`Scatter`] and [`Point`] are built with `new` and `with_*`: more may
//! come, so not field by field.

use super::note;
use crate::map::colour::{to_linear, Lin};
use crate::map::raster::{edge, ring, stamp, Frame, Shape};
use crate::map::text::{self, Font};
use crate::map::theme::Theme;

/// One point: where, which series (an index into the plot's series), and
/// how much of its colour to draw, in `[0, 1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Point {
    pub x: f32,
    pub y: f32,
    pub series: usize,
    pub alpha: f32,
}

impl Point {
    #[must_use]
    pub fn new(x: f32, y: f32, series: usize, alpha: f32) -> Self {
        Self {
            x,
            y,
            series,
            alpha,
        }
    }
}

/// What a scatter plot shows.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct Scatter<'a> {
    pub title: &'a str,
    pub x_label: &'a str,
    pub y_label: &'a str,
    pub points: &'a [Point],
    /// The series' names, drawn as a key above the plot; each series in
    /// the theme's palette colour of its index.
    pub series: &'a [String],
    /// The axes' ranges, `(lo, hi)`; `None` spans the points' finite values.
    pub x_range: Option<(f32, f32)>,
    pub y_range: Option<(f32, f32)>,
    /// Reference lines across the plot, at these y and x values, muted.
    pub h_lines: &'a [f32],
    pub v_lines: &'a [f32],
    /// Points named beside them: (index into `points`, text).
    pub named: &'a [(usize, String)],
    /// Points ringed in ink: indices into `points`.
    pub ringed: &'a [usize],
}

impl<'a> Scatter<'a> {
    /// `points` of `series`, untitled, the axes spanning the points.
    #[must_use]
    pub fn new(points: &'a [Point], series: &'a [String]) -> Self {
        Self {
            title: "",
            x_label: "",
            y_label: "",
            points,
            series,
            x_range: None,
            y_range: None,
            h_lines: &[],
            v_lines: &[],
            named: &[],
            ringed: &[],
        }
    }

    #[must_use]
    pub fn with_title(self, title: &'a str) -> Self {
        Self { title, ..self }
    }

    /// The axes' names.
    #[must_use]
    pub fn with_labels(self, x_label: &'a str, y_label: &'a str) -> Self {
        Self {
            x_label,
            y_label,
            ..self
        }
    }

    /// The axes' ranges, `(lo, hi)`, in place of the points' span.
    #[must_use]
    pub fn with_ranges(self, x_range: Option<(f32, f32)>, y_range: Option<(f32, f32)>) -> Self {
        Self {
            x_range,
            y_range,
            ..self
        }
    }

    /// Reference lines across the plot at these y (`h_lines`) and x values.
    #[must_use]
    pub fn with_lines(self, h_lines: &'a [f32], v_lines: &'a [f32]) -> Self {
        Self {
            h_lines,
            v_lines,
            ..self
        }
    }

    /// Points named beside them: (index into the points, text).
    #[must_use]
    pub fn with_named(self, named: &'a [(usize, String)]) -> Self {
        Self { named, ..self }
    }

    /// Points ringed in ink: indices into the points.
    #[must_use]
    pub fn with_ringed(self, ringed: &'a [usize]) -> Self {
        Self { ringed, ..self }
    }
}

/// How a scatter plot draws its points.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct ScatterStyle {
    /// A point's radius, in line heights.
    pub radius: f32,
    /// The opacity a point at `alpha` 0 is still drawn at, so faint points
    /// stay visible.
    pub floor: f32,
}

impl Default for ScatterStyle {
    /// Points a sixth of a line high, never fainter than a third.
    fn default() -> Self {
        Self {
            radius: 1.0 / 6.0,
            floor: 0.35,
        }
    }
}

/// Where a drawn scatter plot is: its plot's rectangle (`x0, y0, x1, y1`)
/// and each axis's range, to map between pixels and data.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ScatterLayout {
    pub plot: [f32; 4],
    pub x_range: (f32, f32),
    pub y_range: (f32, f32),
}

impl ScatterLayout {
    /// Pixel of data point `(x, y)`, clamped onto the plot's edge (±∞ onto
    /// the far and near edges); `None` for a NaN coordinate.
    #[must_use]
    pub fn px(&self, x: f32, y: f32) -> Option<(f32, f32)> {
        let [x0, y0, x1, y1] = self.plot;
        let t = |v: f32, (lo, hi): (f32, f32)| -> Option<f32> {
            match v {
                v if v.is_nan() => None,
                v if v.is_finite() => Some(((v - lo) / (hi - lo)).clamp(0.0, 1.0)),
                v if v > 0.0 => Some(1.0),
                _ => Some(0.0),
            }
        };
        Some((
            x0 + t(x, self.x_range)? * (x1 - x0),
            y1 - t(y, self.y_range)? * (y1 - y0),
        ))
    }
}

const NOTHING: &str = "nothing to plot";
const TOO_SMALL: &str = "too small to plot";

/// `(lo, hi)` in order and wider than nothing: a range of one value, or
/// given backwards, is made one an axis can be drawn on.
fn widened((lo, hi): (f32, f32)) -> (f32, f32) {
    let (lo, hi) = (lo.min(hi), lo.max(hi));
    if hi - lo < 1e-12 {
        (lo - 0.5, hi + 0.5)
    } else {
        (lo, hi)
    }
}

/// `(lo, hi)` of the finite values, `None` without any.
fn span(values: impl Iterator<Item = f32>) -> Option<(f32, f32)> {
    let (lo, hi) = values
        .filter(|v| v.is_finite())
        .fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    (lo <= hi).then_some((lo, hi))
}

/// A number short enough for an axis mark.
fn tick(v: f32) -> String {
    if v != 0.0 && (v.abs() >= 1e4 || v.abs() < 1e-2) {
        format!("{v:.1e}")
    } else if v.fract() == 0.0 {
        format!("{v:.0}")
    } else {
        format!("{v:.2}")
    }
}

/// Draw `s` into `frame`: the title, the series' key, the plot with its
/// axes and lines, the points in order, the named points' text and the
/// rings.
///
/// # Errors
/// Nothing to plot (no finite point and no range given), or a frame too
/// small for a plot; the reason is drawn in its place.
pub fn draw_scatter(
    frame: &mut Frame,
    s: &Scatter,
    font: Font,
    bold: Font,
    theme: &Theme,
    style: &ScatterStyle,
) -> Result<ScatterLayout, &'static str> {
    let bg = frame.background();
    let ink = to_linear(theme.text);
    let muted = to_linear(theme.muted);
    let lh = font.line_height() as f32;
    let put = |frame: &mut Frame, font: Font, s: &str, x: f32, y: f32| {
        text::draw(frame, font, s, x as i32, y as i32, ink, bg);
    };
    put(frame, bold, s.title, lh / 2.0, lh / 2.0);
    let ranges = (
        s.x_range.or_else(|| span(s.points.iter().map(|p| p.x))),
        s.y_range.or_else(|| span(s.points.iter().map(|p| p.y))),
    );
    let (Some(x_range), Some(y_range)) = ranges else {
        note(frame, NOTHING, font, theme);
        return Err(NOTHING);
    };
    // The series' key: a dot and a name each, in a row under the title.
    let n = s.series.len();
    let colours: Vec<Lin> = (0..n).map(|i| theme.palette.colour(i, n)).collect();
    let mut x = 1.5 * lh;
    for (name, &c) in s.series.iter().zip(&colours) {
        stamp(frame, x, 2.2 * lh, lh / 3.0, Shape::Circle, c, 1.0);
        put(frame, font, name, x + lh / 2.0, 1.7 * lh);
        x += 2.0 * lh + font.width(name) as f32;
    }
    let plot = [
        4.0 * lh,
        3.5 * lh,
        frame.w as f32 - lh,
        frame.h as f32 - 2.5 * lh,
    ];
    let [x0, y0, x1, y1] = plot;
    if x1 - x0 < 4.0 * lh || y1 - y0 < 4.0 * lh {
        note(frame, TOO_SMALL, font, theme);
        return Err(TOO_SMALL);
    }
    let layout = ScatterLayout {
        plot,
        x_range: widened(x_range),
        y_range: widened(y_range),
    };
    let (x_range, y_range) = (layout.x_range, layout.y_range);
    // Axes along the bottom and left, then the reference lines.
    edge(frame, (x0, y1), (x1, y1), (0.0, 0.0), ink, 0.8);
    edge(frame, (x0, y0), (x0, y1), (0.0, 0.0), ink, 0.8);
    let below = y1 + lh / 3.0;
    let left = lh / 2.0;
    // The axes' ends, and each line's value where it meets its axis.
    let x_marks = [x_range.0, x_range.1]
        .into_iter()
        .chain(s.v_lines.iter().copied());
    for (k, v) in x_marks.enumerate() {
        let Some((px, _)) = layout.px(v, y_range.0) else {
            continue;
        };
        if k >= 2 {
            edge(frame, (px, y0), (px, y1), (0.0, 0.0), muted, 0.8);
        }
        let t = tick(v);
        let w = font.width(&t) as f32;
        put(frame, font, &t, (px - w / 2.0).clamp(x0, x1 - w), below);
    }
    for (k, v) in [y_range.0, y_range.1]
        .into_iter()
        .chain(s.h_lines.iter().copied())
        .enumerate()
    {
        let Some((_, py)) = layout.px(x_range.0, v) else {
            continue;
        };
        if k >= 2 {
            edge(frame, (x0, py), (x1, py), (0.0, 0.0), muted, 0.8);
        }
        put(
            frame,
            font,
            &tick(v),
            left,
            (py - lh / 2.0).clamp(y0, y1 - lh),
        );
    }
    let w = font.width(s.x_label) as f32;
    put(frame, font, s.x_label, x1 - w, below + lh);
    put(frame, font, s.y_label, left, y0 + lh);
    let r = (style.radius * lh).max(1.5);
    for p in s.points {
        let Some((px, py)) = layout.px(p.x, p.y) else {
            continue;
        };
        let a = style.floor + (1.0 - style.floor) * p.alpha.clamp(0.0, 1.0);
        let c = colours.get(p.series).copied().unwrap_or(muted);
        stamp(frame, px, py, r, Shape::Circle, c, a);
    }
    for (i, name) in s.named {
        if let Some((px, py)) = s.points.get(*i).and_then(|p| layout.px(p.x, p.y)) {
            put(frame, font, name, px + r + 2.0, py - lh / 2.0);
        }
    }
    for &i in s.ringed {
        if let Some((px, py)) = s.points.get(i).and_then(|p| layout.px(p.x, p.y)) {
            ring(frame, px, py, 2.0 * r, ink);
        }
    }
    Ok(layout)
}

#[cfg(test)]
#[path = "../tests/chart_scatter.rs"]
mod tests;
