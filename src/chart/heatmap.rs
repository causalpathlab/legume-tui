//! Top features × groups heatmap.
//!
//! A feature marks the group it leads by the widest margin: its level
//! there minus its highest level in any other group. Ranking by margin,
//! not by fold over the mean of the rest, makes each row peak in its own
//! column, so the map reads as a diagonal; a tie peaks nowhere and is left
//! out. With a single group there is no runner-up, and rows rank by their
//! level there.
//!
//! Features are chosen in two stages. A cheap first matrix (the model's
//! levels) proposes [`CANDIDATES`] × N per group ([`candidates`]); the
//! viewer reads the values it shows for those alone, and they choose the
//! top N ([`top_by_margin`] with a [`Rerank`]). Without such values the
//! first matrix does both. Rows are z-scored across the groups and clipped
//! ([`zscore_rows`]), so a few outliers do not wash out the rest.
//!
//! Ported from pinto (`src/view/heatmap.rs`: `by_margin`, the two stages
//! of `Heatmap::build`, `tsv`; `src/view/tui/plots.rs`: `heat_picture` as
//! [`thumbnail`]) and senna (`src/view/chart.rs`: `zscore_rows`, and
//! `draw_heatmap` as [`draw_heat`], its constants made parameters). On
//! two or more groups pinto's one pass of peaks gives what senna's
//! `pick_top(own_peaks(margins))` gives.

use super::note;
use crate::map::colour::{diverging, diverging_lin, to_linear, to_srgb, Lin};
use crate::map::raster::Frame;
use crate::map::text::{self, Font};
use crate::map::theme::Theme;

/// Largest |z| drawn; beyond it colours saturate.
pub const CLIP: f32 = 2.5;
/// Candidates the first matrix proposes per feature finally kept.
pub const CANDIDATES: usize = 5;

const NOTHING: &str = "no features stand out in these groups";
const TOO_SMALL: &str = "too small to draw";

/// A features × groups matrix of levels. Its columns are the groups shown,
/// in display order; a level that is not a number is missing.
pub trait Levels {
    fn rows(&self) -> usize;
    fn cols(&self) -> usize;
    fn at(&self, r: usize, c: usize) -> f32;
}

/// Levels held row-major; missing past the end.
#[derive(Clone, Copy, Debug)]
pub struct Matrix<'a> {
    pub rows: usize,
    pub cols: usize,
    pub v: &'a [f32],
}

impl Levels for Matrix<'_> {
    fn rows(&self) -> usize {
        self.rows
    }
    fn cols(&self) -> usize {
        self.cols
    }
    fn at(&self, r: usize, c: usize) -> f32 {
        self.v.get(r * self.cols + c).copied().unwrap_or(f32::NAN)
    }
}

/// Levels worked out when asked, as pinto reads its model's rates.
#[derive(Clone, Copy, Debug)]
pub struct ByFn<F: Fn(usize, usize) -> f32> {
    pub rows: usize,
    pub cols: usize,
    pub f: F,
}

impl<F: Fn(usize, usize) -> f32> Levels for ByFn<F> {
    fn rows(&self) -> usize {
        self.rows
    }
    fn cols(&self) -> usize {
        self.cols
    }
    fn at(&self, r: usize, c: usize) -> f32 {
        (self.f)(r, c)
    }
}

/// Row `r`'s peak column and its lead over the runner-up, kept only when
/// it is a number above zero: a tie, a row with fewer than two levels
/// among two or more columns, or no level at all peaks nowhere. Missing
/// levels never win. With one column the lead is the level itself.
#[must_use]
pub fn peak_margin<L: Levels + ?Sized>(m: &L, r: usize) -> Option<(usize, f32)> {
    let cols = m.cols();
    let (mut top, mut first, mut second) = (0, f32::NEG_INFINITY, f32::NEG_INFINITY);
    for c in 0..cols {
        let v = m.at(r, c);
        if v > first {
            (top, second, first) = (c, first, v);
        } else if v > second {
            second = v;
        }
    }
    let margin = if cols > 1 { first - second } else { first };
    (margin > 0. && margin.is_finite()).then_some((top, margin))
}

/// Each column's top `per_col` rows by [`peak_margin`], among the rows
/// that peak there: as (row, column), columns in order, best margin first,
/// ties by row.
#[must_use]
pub fn by_margin<L: Levels + ?Sized>(m: &L, per_col: usize) -> Vec<(usize, usize)> {
    let mut lead: Vec<Vec<(usize, f32)>> = vec![Vec::new(); m.cols()];
    for r in 0..m.rows() {
        if let Some((c, margin)) = peak_margin(m, r) {
            lead[c].push((r, margin));
        }
    }
    let mut out = Vec::new();
    for (c, mut rows) in lead.into_iter().enumerate() {
        rows.sort_by(|a, b| b.1.total_cmp(&a.1));
        out.extend(rows.into_iter().take(per_col).map(|(r, _)| (r, c)));
    }
    out
}

/// The rows [`by_margin`] proposes as candidates for `per_col` a column,
/// [`CANDIDATES`] each, in its order: those to read the shown values for.
#[must_use]
pub fn candidates<L: Levels + ?Sized>(m: &L, per_col: usize) -> Vec<usize> {
    by_margin(m, per_col * CANDIDATES)
        .into_iter()
        .map(|(r, _)| r)
        .collect()
}

/// A second matrix over some rows of the first: its row `i` holds the
/// values of the first matrix's row `rows[i]`, in the same columns, missing
/// where they could not be read.
#[derive(Clone, Copy, Debug)]
pub struct Rerank<'a, L: Levels + ?Sized> {
    pub rows: &'a [usize],
    pub m: &'a L,
}

/// Each column's top `per_col` rows, as (row of `first`, column). With no
/// `second`, `first` chooses; with one, only its rows compete, ranked on
/// its values.
///
/// # Panics
/// When `second` has more rows than it names.
#[must_use]
pub fn top_by_margin<A: Levels + ?Sized, B: Levels + ?Sized>(
    first: &A,
    per_col: usize,
    second: Option<Rerank<'_, B>>,
) -> Vec<(usize, usize)> {
    match second {
        None => by_margin(first, per_col),
        Some(Rerank { rows, m }) => by_margin(m, per_col)
            .into_iter()
            .map(|(i, c)| (rows[i], c))
            .collect(),
    }
}

/// z-score each row of `cols` values across its finite entries (their
/// population spread), clipped to `±clip`; a row with no spread is all
/// zeros, and what is not a number stays so.
pub fn zscore_rows(v: &mut [f32], cols: usize, clip: f32) {
    if cols == 0 {
        return;
    }
    for row in v.chunks_mut(cols) {
        let finite: Vec<f32> = row.iter().copied().filter(|x| x.is_finite()).collect();
        if finite.is_empty() {
            continue;
        }
        let n = finite.len() as f32;
        let mean = finite.iter().sum::<f32>() / n;
        let sd = (finite.iter().map(|x| (x - mean).powi(2)).sum::<f32>() / n).sqrt();
        for x in row.iter_mut().filter(|x| x.is_finite()) {
            *x = if sd > 1e-9 {
                ((*x - mean) / sd).clamp(-clip, clip)
            } else {
                0.0
            };
        }
    }
}

/// What a heatmap shows.
#[derive(Clone, Copy, Debug)]
pub struct Heat<'a> {
    pub title: &'a str,
    /// Feature names, a row each.
    pub rows: &'a [String],
    /// Group names, a column each.
    pub cols: &'a [String],
    /// A swatch of each group's colour over its column.
    pub col_swatch: Option<&'a [Lin]>,
    /// Row-major z-scores, `rows × cols`; missing where not a number.
    pub z: &'a [f32],
    /// The |z| the colours saturate at.
    pub clip: f32,
    /// A row whose name is set in bold.
    pub bold_row: Option<usize>,
}

/// How a heatmap is laid out and coloured.
#[derive(Clone, Copy, Debug)]
pub struct HeatStyle {
    /// Characters of a row's name drawn.
    pub row_chars: usize,
    /// Characters of a column's name drawn.
    pub col_chars: usize,
    /// The colour of a missing value.
    pub missing: Lin,
    /// Draw the ramp's key at the right.
    pub key: bool,
}

impl HeatStyle {
    /// senna's: 16 and 18 characters, missing values in the theme's muted
    /// colour, with a key. pinto passes its theme's dimmed colour instead.
    #[must_use]
    pub fn for_theme(theme: &Theme) -> Self {
        Self {
            row_chars: 16,
            col_chars: 18,
            missing: to_linear(theme.muted),
            key: true,
        }
    }
}

/// Where a drawn heatmap is: its cells' rectangle (`x0, y0, x1, y1`), a
/// cell's width and height, and the top of each row's name where it was
/// drawn (rows too close to the one before get none), a `line` high.
#[derive(Clone, Debug, PartialEq)]
pub struct HeatLayout {
    pub plot: [usize; 4],
    pub cell: (f32, f32),
    pub row_y: Vec<Option<f32>>,
    pub line: f32,
}

impl HeatLayout {
    /// What is at pixel `(px, py)`: a cell as (row, Some(column)), or a
    /// row's name left of the cells as (row, None).
    #[must_use]
    pub fn at(&self, px: f32, py: f32) -> Option<(usize, Option<usize>)> {
        let [x0, y0, x1, y1] = self.plot.map(|v| v as f32);
        let (cw, rh) = self.cell;
        let rows = self.row_y.len();
        if (0.0..x0).contains(&px) {
            return self
                .row_y
                .iter()
                .position(|y| y.is_some_and(|y| (y..y + self.line).contains(&py)))
                .map(|r| (r, None));
        }
        if !(x0..x1).contains(&px) || !(y0..y1).contains(&py) || rows == 0 {
            return None;
        }
        let cols = ((x1 - x0) / cw).round() as usize;
        let r = (((py - y0) / rh) as usize).min(rows - 1);
        let c = (((px - x0) / cw) as usize).min(cols.saturating_sub(1));
        Some((r, Some(c)))
    }
}

/// senna's heatmap: the title in bold, a cell per row and column in the
/// diverging colour of `z / clip`, row names at the left (as many as fit a
/// line each), column names turned under the columns, and the ramp from
/// `+clip` to `-clip` at the right, in the theme's text colour. Text is
/// kept as text when the frame keeps it.
///
/// # Errors
/// With no rows or columns, or too little room, a note says so where the
/// heatmap would be, and the note is returned.
pub fn draw_heat(
    frame: &mut Frame,
    h: &Heat,
    font: Font,
    bold: Font,
    theme: &Theme,
    s: &HeatStyle,
) -> Result<HeatLayout, &'static str> {
    let bg = frame.background();
    let ink = to_linear(theme.text);
    let lh = font.line_height();
    let adv = font.advance();
    text::draw(frame, bold, h.title, lh as i32 / 2, lh as i32 / 2, ink, bg);
    let (rows, cols) = (h.rows.len(), h.cols.len());
    if rows == 0 || cols == 0 {
        note(frame, NOTHING, font, theme);
        return Err(NOTHING);
    }
    let longest = |names: &[String], most: usize| {
        names
            .iter()
            .map(|n| n.chars().count())
            .max()
            .unwrap_or(0)
            .min(most)
            * adv
    };
    let name_w = longest(h.rows, s.row_chars);
    let col_label = longest(h.cols, s.col_chars);
    let key_w = if s.key { 6 * adv + lh } else { 0 };
    let (x0, y0) = (lh / 2 + name_w + adv, 2 * lh);
    let x1 = frame.w.saturating_sub(key_w + lh);
    let y1 = frame
        .h
        .saturating_sub((col_label + lh / 2).min(frame.h / 3));
    if x1 <= x0 + cols || y1 <= y0 + 4 {
        note(frame, TOO_SMALL, font, theme);
        return Err(TOO_SMALL);
    }
    let (cw, rh) = (
        (x1 - x0) as f32 / cols as f32,
        (y1 - y0) as f32 / rows as f32,
    );
    let col_x = |c: usize| (x0 as f32 + c as f32 * cw) as usize;
    for r in 0..rows {
        let (ya, yb) = (
            (y0 as f32 + r as f32 * rh) as usize,
            (y0 as f32 + (r + 1) as f32 * rh) as usize,
        );
        for c in 0..cols {
            let v = h.z.get(r * cols + c).copied().unwrap_or(f32::NAN);
            let colour = if v.is_finite() {
                diverging_lin(v / h.clip)
            } else {
                s.missing
            };
            frame.fill([col_x(c), ya, col_x(c + 1), yb.max(ya + 1)], colour);
        }
    }
    // Each group's colour over its column.
    if let Some(swatch) = h.col_swatch {
        for (c, &colour) in swatch.iter().enumerate().take(cols) {
            frame.fill(
                [col_x(c), y0 - 3 * lh / 8, col_x(c + 1), y0 - lh / 8],
                colour,
            );
        }
    }
    // Row names down the left, as many as fit a line each.
    let mut row_y = vec![None; rows];
    let mut next = 0.0;
    for (r, name) in h.rows.iter().enumerate() {
        let y = y0 as f32 + r as f32 * rh;
        if y + 0.5 * rh < next {
            continue;
        }
        let f = if h.bold_row == Some(r) { bold } else { font };
        let shown: String = name.chars().take(s.row_chars).collect();
        let ty = (y + 0.5 * rh - lh as f32 / 2.0) as i32;
        let tx = x0.saturating_sub(adv + f.width(&shown)) as i32;
        text::draw(frame, f, &shown, tx, ty, ink, bg);
        row_y[r] = Some(ty as f32);
        next = y + 0.5 * rh + lh as f32;
    }
    // Column names under the columns, turned, as many as fit.
    let mut next = 0.0;
    for (c, name) in h.cols.iter().enumerate() {
        let x = x0 as f32 + (c as f32 + 0.5) * cw;
        if x < next {
            continue;
        }
        let shown: String = name.chars().take(s.col_chars).collect();
        let lx = (x - lh as f32 / 2.0) as i32;
        let bottom = (y1 + lh / 4 + font.width(&shown)) as i32;
        text::draw_vertical(frame, font, &shown, (lx, bottom), ink, bg);
        next = x + lh as f32;
    }
    if s.key {
        // The ramp, top (+clip) to bottom (-clip).
        let kx = frame.w.saturating_sub(key_w);
        let (ka, kb) = (y0 + lh, y1.saturating_sub(lh).max(y0 + lh + 2));
        for y in ka..kb {
            let t = 1.0 - 2.0 * (y - ka) as f32 / (kb - ka) as f32;
            frame.fill([kx, y, kx + lh / 2, y + 1], diverging_lin(t));
        }
        let label_x = (kx + lh / 2 + adv / 2) as i32;
        let half = lh as i32 / 2;
        for (s, y) in [
            (format!("+{}", h.clip), ka as i32 - half),
            ("0".to_string(), ((ka + kb) / 2) as i32 - half),
            (format!("-{}", h.clip), kb as i32 - half),
            ("z".to_string(), y0 as i32 - half),
        ] {
            text::draw(frame, font, &s, label_x, y, ink, bg);
        }
    }
    Ok(HeatLayout {
        plot: [x0, y0, x1, y1],
        cell: (cw, rh),
        row_y,
        line: lh as f32,
    })
}

/// A heatmap as a small picture, for a gallery thumbnail (pinto's
/// `heat_picture`): with `swatch`, a row of the groups' colours on top,
/// then a `px`-square block per value of `z` (`cols` a row), in pinto's
/// bytes; missing values in the theme's dimmed colour.
#[must_use]
pub fn thumbnail(
    z: &[f32],
    cols: usize,
    swatch: Option<&[Lin]>,
    px: usize,
    theme: &Theme,
    clip: f32,
) -> Frame {
    let cols = cols.max(1);
    let top = usize::from(swatch.is_some());
    let rows = z.len().div_ceil(cols) + top;
    let (w, h) = (cols * px, rows * px);
    let bg = theme.background;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        for x in 0..w {
            let (r, c) = (y / px, x / px);
            let [red, g, b] = match swatch {
                Some(s) if r == 0 => s.get(c).map_or(bg, |&l| to_srgb(l)),
                _ => {
                    let v = z.get((r - top) * cols + c).copied().unwrap_or(0.);
                    if v.is_finite() {
                        diverging(v / clip)
                    } else {
                        theme.dimmed
                    }
                }
            };
            rgba.extend_from_slice(&[red, g, b, 255]);
        }
    }
    Frame::from_srgba(w, h, &rgba, bg)
}

/// A heatmap's values as a table (pinto's): a `# what` line, a header of
/// `head` (what the rows are; pinto's `gene`), `marks` and the columns,
/// then per row its name, the group it marks and its values to four
/// places, tab-separated.
#[must_use]
pub fn tsv(
    what: &str,
    head: &str,
    rows: &[String],
    marks: &[String],
    cols: &[String],
    values: &[f32],
) -> String {
    use std::fmt::Write as _;
    let mut out = format!("# {what}\n{head}\tmarks");
    for c in cols {
        out.push('\t');
        out.push_str(c);
    }
    out.push('\n');
    let n = cols.len();
    for (r, name) in rows.iter().enumerate() {
        let mark = marks.get(r).map_or("", String::as_str);
        write!(out, "{name}\t{mark}").ok();
        for v in values.iter().skip(r * n).take(n) {
            write!(out, "\t{v:.4}").ok();
        }
        out.push('\n');
    }
    out
}

#[cfg(test)]
#[path = "../tests/chart_heatmap.rs"]
mod tests;
