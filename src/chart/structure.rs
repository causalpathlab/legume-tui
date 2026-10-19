//! Structure plot: each row's mixture over `k` components as a stacked
//! bar, rows side by side in one panel per group: the admixture plot of
//! population genetics, a topic model's topic shares, pinto's community
//! propensities.
//!
//! Components stack in order of overall prevalence, the most prevalent at
//! the bottom. Inside a panel, rows go by their dominant component's rank,
//! then by its weight, highest first; on a tie for the dominant component
//! the more prevalent one takes it. A pixel column sums the rows that fall
//! on it and renormalises, so weights in any non-negative scale (shares in
//! `f32`, pinto's `u8` of 255) draw alike.
//!
//! Ported from pinto (`src/view/structure.rs`: `build`, `sort_cells`,
//! `widths`, `render`) and senna (`src/view/chart.rs`: `build_structure`'s
//! panels and `draw_structure`'s widths, title, names and key), with pinto's plot
//! names in terminal rows (`src/view/tui/plots.rs`: `plot_names`) as
//! [`name_rows`]. [`bars`] draws the bars alone, as pinto does under its
//! map; [`figure`] draws senna's whole chart around them. Hits are kept as
//! per-column cuts ([`Drawn::at`]) instead of pinto's map of every pixel.

use super::note;
use crate::map::colour::{to_linear, Lin};
use crate::map::raster::Frame;
use crate::map::text::{self, Font};
use crate::map::theme::Theme;
use std::cmp::Ordering;

/// Pixels between panels.
pub const GAP: usize = 2;
/// Characters of a panel's name drawn under it.
pub const NAME_CHARS: usize = 18;
/// The panel of rows no group claims, drawn last.
pub const UNASSIGNED: &str = "unassigned";

/// `n × k` weights, row-major, in any non-negative scale: senna's `f32`
/// shares, pinto's `u8` propensities of 255.
#[derive(Clone, Copy, Debug)]
pub struct Weights<'a, W> {
    pub k: usize,
    pub w: &'a [W],
    /// Which rows count. With none, a row counts when its first weight is
    /// a number and its weights sum above zero.
    pub valid: Option<&'a [bool]>,
}

impl<W: Copy + Into<f32>> Weights<'_, W> {
    /// Rows in the weights.
    #[must_use]
    pub fn n(&self) -> usize {
        self.w.len().checked_div(self.k).unwrap_or(0)
    }

    fn row(&self, i: usize) -> &[W] {
        &self.w[i * self.k..(i + 1) * self.k]
    }

    fn counts(&self, i: usize) -> bool {
        match self.valid {
            Some(v) => v.get(i).copied().unwrap_or(false),
            None => {
                let row = self.row(i);
                row.first().is_some_and(|&v| v.into().is_finite())
                    && row.iter().map(|&v| f64::from(v.into())).sum::<f64>() > 0.0
            }
        }
    }
}

/// The group of each row, by index into `names`. An id past the names
/// (`u32::MAX`, pinto's `NO_CLUSTER`) puts the row in [`UNASSIGNED`].
#[derive(Clone, Copy, Debug)]
pub struct Grouping<'a, G> {
    pub of: &'a [G],
    pub names: &'a [String],
    /// The panels' order, by group; groups left out are not drawn. All of
    /// them in their own order with none.
    pub order: Option<&'a [usize]>,
}

/// A panel: its name and its rows in drawing order.
#[derive(Clone, Debug, PartialEq)]
pub struct Panel {
    pub name: String,
    pub rows: Vec<u32>,
}

/// What a structure plot shows: its panels, and the components from the
/// bottom of a bar to its top.
#[derive(Clone, Debug, PartialEq)]
pub struct Layout {
    pub panels: Vec<Panel>,
    pub stack: Vec<usize>,
}

impl Layout {
    /// Panels by group in the grouping's order, rows with no group last;
    /// components by their total weight over the rows that count (in
    /// `f64`, ties by id); rows in each panel by their dominant
    /// component's rank, then its weight. Empty panels are left out.
    #[must_use]
    pub fn build<W: Copy + Into<f32>, G: Copy + Into<u32>>(
        w: &Weights<W>,
        g: &Grouping<G>,
    ) -> Self {
        let k = w.k;
        let n = w.n().min(g.of.len());
        let rows: Vec<usize> = (0..n).filter(|&i| w.counts(i)).collect();

        let mut total = vec![0f64; k];
        for &i in &rows {
            for (t, &v) in total.iter_mut().zip(w.row(i)) {
                *t += f64::from(v.into());
            }
        }
        let mut stack: Vec<usize> = (0..k).collect();
        stack.sort_by(|&a, &b| total[b].total_cmp(&total[a]).then(a.cmp(&b)));
        let rank = rank_of(&stack);

        let groups = g.names.len();
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); groups];
        let mut unassigned = Vec::new();
        for &i in &rows {
            match members.get_mut(g.of[i].into() as usize) {
                Some(m) => m.push(i as u32),
                None => unassigned.push(i as u32),
            }
        }
        let all: Vec<usize>;
        let order = match g.order {
            Some(o) => o,
            None => {
                all = (0..groups).collect();
                &all
            }
        };
        let mut panels: Vec<Panel> = order
            .iter()
            .filter(|&&p| p < groups)
            .filter_map(|&p| {
                let rows = std::mem::take(&mut members[p]);
                (!rows.is_empty()).then(|| Panel {
                    name: g.names[p].clone(),
                    rows,
                })
            })
            .collect();
        // Rows no group claims go last.
        if !unassigned.is_empty() {
            panels.push(Panel {
                name: UNASSIGNED.into(),
                rows: unassigned,
            });
        }
        for p in &mut panels {
            sort_rows(&mut p.rows, w, &rank);
        }
        Layout { panels, stack }
    }

    /// Each panel as a plot of its own, by name, on the same stack (pinto's
    /// grid tiles).
    #[must_use]
    pub fn into_panels(self) -> Vec<(String, Layout)> {
        let stack = self.stack;
        self.panels
            .into_iter()
            .map(|p| {
                let name = p.name.clone();
                let one = Layout {
                    panels: vec![p],
                    stack: stack.clone(),
                };
                (name, one)
            })
            .collect()
    }

    /// Rows in all panels.
    #[must_use]
    pub fn n_rows(&self) -> usize {
        self.panels.iter().map(|p| p.rows.len()).sum()
    }

    /// Each component's place in the stack, by id: 0 at the bottom.
    #[must_use]
    pub fn rank(&self) -> Vec<usize> {
        rank_of(&self.stack)
    }
}

fn rank_of(stack: &[usize]) -> Vec<usize> {
    let mut rank = vec![0usize; stack.len()];
    for (r, &c) in stack.iter().enumerate() {
        rank[c] = r;
    }
    rank
}

/// Dominant component's rank, then its weight, highest first; a tie for
/// the dominant component goes to the lower rank.
fn sort_rows<W: Copy + Into<f32>>(rows: &mut [u32], w: &Weights<W>, rank: &[usize]) {
    let dominant = |i: u32| {
        let mut best: Option<(usize, f32)> = None;
        for (c, &q) in w.row(i as usize).iter().enumerate() {
            let q = q.into();
            let wins = best.is_none_or(|(b, bq)| match q.total_cmp(&bq) {
                Ordering::Greater => true,
                Ordering::Equal => rank[c] < rank[b],
                Ordering::Less => false,
            });
            if wins {
                best = Some((c, q));
            }
        }
        best.map_or((0, 0.0), |(c, q)| (rank[c], q))
    };
    let mut keyed: Vec<(u32, (usize, f32))> = rows.iter().map(|&i| (i, dominant(i))).collect();
    keyed.sort_by(|(_, (ra, qa)), (_, (rb, qb))| ra.cmp(rb).then(qb.total_cmp(qa)));
    for (r, (i, _)) in rows.iter_mut().zip(keyed) {
        *r = i;
    }
}

/// Panel widths for `w` pixels with `gap` between panels: in proportion
/// to their `sizes`, the pixels left over to the largest remainders, so
/// they sum to `w - gap × (panels - 1)`; a panel too small for a pixel
/// borrows one from the widest while it can spare one.
#[must_use]
pub fn widths(sizes: &[usize], w: usize, gap: usize) -> Vec<usize> {
    let p = sizes.len();
    let n = sizes.iter().sum::<usize>().max(1);
    let avail = w.saturating_sub(gap * p.saturating_sub(1));
    let exact: Vec<f64> = sizes
        .iter()
        .map(|&c| c as f64 * avail as f64 / n as f64)
        .collect();
    let mut out: Vec<usize> = exact.iter().map(|&x| x.floor() as usize).collect();
    // Largest remainders take the pixels left over.
    let mut left = avail.saturating_sub(out.iter().sum());
    let mut by_rest: Vec<usize> = (0..p).collect();
    by_rest
        .sort_by(|&a, &b| (exact[b] - exact[b].floor()).total_cmp(&(exact[a] - exact[a].floor())));
    for &i in &by_rest {
        if left == 0 {
            break;
        }
        out[i] += 1;
        left -= 1;
    }
    // A panel too small for a pixel borrows one from the widest.
    for i in 0..p {
        if out[i] == 0 {
            let widest = (0..p).max_by_key(|&j| out[j]).unwrap_or(i);
            if out[widest] > 1 {
                out[widest] -= 1;
                out[i] = 1;
            }
        }
    }
    out
}

/// senna's panel widths for `w` pixels with `gap` between panels: each
/// its share of the room, rounded down, at least a pixel. The pixels left
/// over stay blank at the right.
#[must_use]
pub fn senna_widths(sizes: &[usize], w: usize, gap: usize) -> Vec<usize> {
    let n = sizes.iter().sum::<usize>().max(1);
    let room = w.saturating_sub(gap * sizes.len().saturating_sub(1));
    sizes.iter().map(|&c| (room * c / n).max(1)).collect()
}

/// How bars are coloured and spaced.
#[derive(Clone, Copy, Debug)]
pub struct BarStyle<'a> {
    /// Colour by component id.
    pub palette: &'a [Lin],
    /// The components in focus, and the colour of the rest (pinto passes
    /// its theme's dimmed colour).
    pub focus: Option<(&'a [bool], Lin)>,
    /// Pixels between panels.
    pub gap: usize,
    /// Keep where each component is drawn, for [`Drawn::at`].
    pub hits: bool,
}

/// A panel's place in a drawn plot: its first and last pixel column.
#[derive(Clone, Debug, PartialEq)]
pub struct Span {
    /// Its place in [`Layout::panels`]: a panel too narrow for a pixel
    /// has no span.
    pub panel: usize,
    pub x0: usize,
    /// Inclusive.
    pub x1: usize,
    pub name: String,
    /// Rows in the panel.
    pub rows: usize,
}

/// A drawn plot: the rectangle of its bars, where its panels are, and
/// (with [`BarStyle::hits`]) where each component is in each column.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// `x0, y0, x1, y1` of the bars, ends exclusive.
    pub rect: [usize; 4],
    pub spans: Vec<Span>,
    /// Per column of `rect`, `stack.len() + 1` y's from the bottom: the
    /// component at stack place `s` covers `cuts[s + 1]..cuts[s]`.
    cuts: Vec<u32>,
    stack: Vec<usize>,
}

impl Drawn {
    fn empty(rect: [usize; 4]) -> Self {
        Self {
            rect,
            spans: Vec::new(),
            cuts: Vec::new(),
            stack: Vec::new(),
        }
    }

    /// The panel (by its place in [`Layout::panels`]) and component at
    /// pixel `(x, y)` of the frame.
    #[must_use]
    pub fn at(&self, x: usize, y: usize) -> Option<(usize, usize)> {
        let [x0, y0, x1, y1] = self.rect;
        if self.cuts.is_empty() || x < x0 || x >= x1 || y < y0 || y >= y1 {
            return None;
        }
        let panel = self.spans.iter().find(|s| s.x0 <= x && x <= s.x1)?.panel;
        let per = self.stack.len() + 1;
        let cuts = &self.cuts[(x - x0) * per..(x - x0 + 1) * per];
        (0..self.stack.len())
            .find(|&s| cuts[s + 1] as usize <= y && y < cuts[s] as usize)
            .map(|s| (panel, self.stack[s]))
    }
}

/// The bars alone, into `rect` (`x0, y0, x1, y1`) of `frame`: panels side
/// by side by [`widths`], each pixel column the rows that fall on it,
/// summed and renormalised, stacked from the bottom. A column whose rows
/// weigh nothing stays the page.
///
/// For pinto's bytes, draw into a frame made by
/// [`Frame::from_srgba`](crate::map::raster::Frame::from_srgba): its
/// colours come back as the bytes they were.
pub fn bars<W: Copy + Into<f32>>(
    frame: &mut Frame,
    rect: [usize; 4],
    l: &Layout,
    w: &Weights<W>,
    s: &BarStyle,
) -> Drawn {
    let sizes: Vec<usize> = l.panels.iter().map(|p| p.rows.len()).collect();
    let room = rect[2].saturating_sub(rect[0]);
    bars_in(frame, rect, l, w, s, &widths(&sizes, room, s.gap))
}

/// [`bars`] in panels of the given widths.
fn bars_in<W: Copy + Into<f32>>(
    frame: &mut Frame,
    rect: [usize; 4],
    l: &Layout,
    w: &Weights<W>,
    s: &BarStyle,
    widths: &[usize],
) -> Drawn {
    let [x0, y0, x1, y1] = rect;
    let (room, height) = (x1.saturating_sub(x0), y1.saturating_sub(y0) as f32);
    let colour = |c: usize| match s.focus {
        Some((f, dim)) if !f.get(c).copied().unwrap_or(false) => dim,
        _ => s.palette[c],
    };
    let per = l.stack.len() + 1;
    let mut cuts = if s.hits {
        vec![y1 as u32; room * per]
    } else {
        Vec::new()
    };
    let mut spans = Vec::new();
    let mut x = x0;
    let mut mix = vec![0f32; w.k];
    for (panel, (p, &width)) in l.panels.iter().zip(widths).enumerate() {
        if width == 0 {
            continue;
        }
        let n = p.rows.len();
        for col in 0..width {
            // The rows under this column; stretched when a panel has fewer
            // rows than pixels.
            let a = col * n / width;
            let b = ((col + 1) * n / width).max(a + 1).min(n);
            mix.fill(0.);
            for &i in &p.rows[a..b] {
                for (m, &q) in mix.iter_mut().zip(w.row(i as usize)) {
                    *m += q.into();
                }
            }
            let total: f32 = mix.iter().sum();
            if total.is_nan() || total <= 0. || x + col >= x1 {
                continue;
            }
            // Bottom up, each component a run of rows.
            let mut from = 0f32;
            for (place, &c) in l.stack.iter().enumerate() {
                let to = from + mix[c] / total;
                let ya = y0 + (height * (1. - to)).round() as usize;
                let yb = y0 + (height * (1. - from)).round() as usize;
                frame.fill([x + col, ya, x + col + 1, yb.min(y1)], colour(c));
                if s.hits {
                    cuts[(x + col - x0) * per + place + 1] = ya.min(y1) as u32;
                }
                from = to;
            }
        }
        spans.push(Span {
            panel,
            x0: x,
            x1: x + width - 1,
            name: p.name.clone(),
            rows: n,
        });
        x += width + s.gap;
    }
    Drawn {
        rect,
        spans,
        cuts,
        stack: l.stack.clone(),
    }
}

/// Where panel names go under a [`figure`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Names {
    /// Across, unless one is wider than its panel: then all turned.
    Auto,
    Horizontal,
    /// Read bottom to top.
    Turned,
    None,
}

/// What a [`figure`] puts around its bars.
#[derive(Clone, Copy, Debug)]
pub struct Figure<'a> {
    pub title: Option<&'a str>,
    /// The components' names by id, listed at the right in stack order
    /// beside their colours.
    pub key: Option<&'a [String]>,
    pub names: Names,
    pub font: Font,
    pub bold: Font,
    pub theme: &'a Theme,
}

/// senna's whole chart: the title in bold at the top left, the bars in
/// panels of [`senna_widths`], panel names under them (cut to
/// [`NAME_CHARS`]) and the key at the right, in the theme's text colour.
/// Text is kept as text when the frame keeps it. Too small for bars, it
/// says so in a note and nothing is drawn. Unlike senna, bars stop at the
/// key when panels given a pixel they had no room for would pass it.
pub fn figure<W: Copy + Into<f32>>(
    frame: &mut Frame,
    l: &Layout,
    w: &Weights<W>,
    s: &BarStyle,
    fig: &Figure,
) -> Drawn {
    let bg = frame.background();
    let ink = to_linear(fig.theme.text);
    let font = fig.font;
    let lh = font.line_height();
    let adv = font.advance();
    if let Some(title) = fig.title {
        text::draw(
            frame,
            fig.bold,
            title,
            lh as i32 / 2,
            lh as i32 / 2,
            ink,
            bg,
        );
    }
    // Right: one line per component; bottom: panel names, turned when long.
    let keys: Vec<&str> = fig.key.map_or_else(Vec::new, |k| {
        l.stack
            .iter()
            .map(|&c| k.get(c).map_or("", String::as_str))
            .collect()
    });
    let key_w = (keys.iter().map(|n| n.chars().count()).max().unwrap_or(2) + 3) * adv;
    let right = if fig.key.is_some() {
        key_w + lh
    } else {
        lh / 2
    };
    let (x0, y0) = (lh / 2, if fig.title.is_some() { 2 * lh } else { lh / 2 });
    let x1 = frame.w.saturating_sub(right);
    let sizes: Vec<usize> = l.panels.iter().map(|p| p.rows.len()).collect();
    let room = senna_widths(&sizes, x1.saturating_sub(x0), s.gap);
    let widest = l
        .panels
        .iter()
        .map(|p| p.name.chars().count())
        .max()
        .unwrap_or(0);
    let turned = match fig.names {
        Names::Auto => l
            .panels
            .iter()
            .zip(&room)
            .any(|(p, &w)| font.width(&p.name) + adv > w),
        Names::Turned => true,
        Names::Horizontal | Names::None => false,
    };
    let label_h = match fig.names {
        Names::None => lh / 2,
        _ if turned => (widest.min(NAME_CHARS) * adv + lh / 2).min(frame.h / 3),
        _ => lh + lh / 2,
    };
    let y1 = frame.h.saturating_sub(label_h);
    if y1 <= y0 + 4 || x1 <= x0 + 4 {
        note(frame, "too small to draw", font, fig.theme);
        return Drawn::empty([x0, y0, x1, y1]);
    }
    let drawn = bars_in(frame, [x0, y0, x1, y1], l, w, s, &room);
    // Each panel's name under it.
    if fig.names != Names::None {
        for sp in &drawn.spans {
            let (x, w) = (sp.x0, sp.x1 + 1 - sp.x0);
            let shown: String = sp.name.chars().take(NAME_CHARS).collect();
            if turned {
                let cx = (x + w / 2).saturating_sub(lh / 2) as i32;
                if w + s.gap >= lh / 2 {
                    let bottom = (y1 + label_h - lh / 4) as i32;
                    text::draw_vertical(frame, font, &shown, (cx, bottom), ink, bg);
                }
            } else {
                let tx = (x + w / 2).saturating_sub(font.width(&shown) / 2) as i32;
                text::draw(frame, font, &shown, tx, (y1 + lh / 4) as i32, ink, bg);
            }
        }
    }
    // The key: one swatch and name per component, in the stack's order.
    if fig.key.is_some() {
        let kx = frame.w.saturating_sub(key_w);
        for (i, (name, &c)) in keys.iter().zip(&l.stack).enumerate() {
            let y = y0 + i * lh;
            if y + lh > frame.h {
                break;
            }
            frame.fill([kx, y + lh / 4, kx + lh / 2, y + 3 * lh / 4], s.palette[c]);
            text::draw(frame, font, name, (kx + lh) as i32, y as i32, ink, bg);
        }
    }
    drawn
}

/// Panel names for the terminal rows under a plot drawn at `px_per_cell`
/// pixels a column, `cols` columns wide: alternating between `rows` rows,
/// each name from its panel's first column (or past the name before it on
/// its row), at most its panel's width or 8 columns, whichever is wider.
#[must_use]
pub fn name_rows(spans: &[Span], cols: usize, px_per_cell: f32, rows: usize) -> Vec<String> {
    if rows == 0 {
        return Vec::new();
    }
    let mut out = vec![vec![' '; cols]; rows];
    let mut next_free = vec![0usize; rows];
    for (i, p) in spans.iter().enumerate() {
        let c0 = (p.x0 as f32 / px_per_cell) as usize;
        let c1 = ((p.x1 + 1) as f32 / px_per_cell) as usize;
        let r = i % rows;
        let start = c0.max(next_free[r]);
        if start >= cols {
            continue;
        }
        // At most the panel's width, or up to the next panel on the row.
        let room = c1.saturating_sub(start).max(8).min(cols - start);
        let name: Vec<char> = p.name.chars().take(room.saturating_sub(1)).collect();
        if name.is_empty() {
            continue;
        }
        out[r][start..start + name.len()].copy_from_slice(&name);
        next_free[r] = start + name.len() + 1;
    }
    out.into_iter().map(|r| r.into_iter().collect()).collect()
}

#[cfg(test)]
#[path = "../tests/chart_structure.rs"]
mod tests;
