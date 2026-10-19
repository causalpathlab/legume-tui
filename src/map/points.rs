//! Progressive point rasterizer.
//!
//! Points are drawn as opaque, anti-aliased marks (circle, square, diamond,
//! triangle, cross) straight onto a linear-light canvas; only a mark given
//! an opacity below one is translucent. With opaque marks the draw order
//! says what is seen, so each layer is drawn in the order its viewer ranks
//! it ([`ranked`], [`by_level`]): muted points first, the focused group last
//! so it pops on top.
//!
//! Within a rank points keep a fixed shuffled order, so every prefix is a
//! uniform subsample. A [`Job`] draws in chunks of doubling size and the
//! viewer shows the canvas after each chunk: the picture appears at once,
//! then fills in.
//!
//! Ported from senna (`src/view/render.rs`: `Job`, `auto_radius`,
//! `visible_count`, `group_medians`, and `Paint::draw_order` as [`ranked`]
//! and [`by_level`]). A viewer's points come through [`PointLayer`], which
//! resolves each point's mark itself, so the job knows nothing of groups,
//! focus or styles.

use super::colour::{to_linear, Lin, Srgb};
use super::raster::{stamp, Frame, Shape};
use super::viewport::Viewport;
use image::RgbaImage;
use std::sync::Arc;

/// First progressive chunk; each later chunk doubles.
const FIRST_CHUNK: usize = 1 << 15;

/// How one point is drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Mark {
    pub colour: Lin,
    pub alpha: f32,
    pub shape: Shape,
    /// Radius multiplier over the layer's.
    pub size: f32,
}

/// One layer of points, as its viewer has them.
pub trait PointLayer: Sync {
    /// Where point `i` is, in data units.
    fn xy(&self, i: usize) -> [f32; 2];
    /// Every point once, in a fixed random order: each prefix is a uniform
    /// subsample.
    fn shuffled(&self) -> &[u32];
    /// The order to draw in, typically [`ranked`] over [`Self::shuffled`]
    /// and cached by the viewer across pans and zooms.
    fn order(&self) -> Arc<Vec<u32>>;
    /// Point `i`'s mark, muted or in ink as the viewer decides; `None`
    /// hides it.
    fn mark(&self, i: usize) -> Option<Mark>;
    /// Radius multiplier for the whole layer.
    fn size(&self) -> f32 {
        1.0
    }
}

/// The draw order: `shuffled` bucketed by `rank` (below `buckets`, higher
/// drawn later, on top), keeping the shuffled order within a rank.
///
/// # Panics
/// When `rank` gives `buckets` or more.
#[must_use]
pub fn ranked(shuffled: &[u32], rank: impl Fn(usize) -> u8, buckets: usize) -> Vec<u32> {
    let mut by_rank: Vec<Vec<u32>> = vec![Vec::new(); buckets];
    for &i in shuffled {
        by_rank[rank(i as usize) as usize].push(i);
    }
    by_rank.concat()
}

/// The draw order for activity: `shuffled` by level `t`, low to high,
/// keeping the shuffled order between equal levels.
#[must_use]
pub fn by_level(shuffled: &[u32], t: impl Fn(usize) -> f32) -> Vec<u32> {
    let n = shuffled.iter().map(|&i| i as usize + 1).max().unwrap_or(0);
    let t: Vec<f32> = (0..n).map(t).collect();
    let mut o = shuffled.to_vec();
    o.sort_by(|&a, &b| t[a as usize].total_cmp(&t[b as usize]));
    o
}

/// A point radius for `n_visible` points spread over the viewport: small
/// marks when crowded, a little larger when sparse.
#[must_use]
pub fn auto_radius(n_visible: usize, vp: &Viewport) -> f32 {
    let per_px = n_visible as f32 / (vp.w * vp.h).max(1) as f32;
    (0.35 / per_px.max(1e-9).sqrt()).clamp(0.6, 2.4)
}

/// Estimated count of a layer's points inside the viewport, from a prefix
/// of the shuffled order.
#[must_use]
pub fn visible_count<L: PointLayer + ?Sized>(l: &L, vp: &Viewport) -> usize {
    let order = l.shuffled();
    let probe = order.len().min(4096);
    if probe == 0 {
        return 0;
    }
    let inside = order[..probe]
        .iter()
        .filter(|&&i| {
            let (x, y) = vp.to_px(l.xy(i as usize));
            x >= 0.0 && y >= 0.0 && x < vp.w as f32 && y < vp.h as f32
        })
        .count();
    inside * order.len() / probe
}

/// One progressive rendering of a scene.
pub struct Job {
    pub vp: Viewport,
    canvas: Frame,
    orders: Vec<Arc<Vec<u32>>>,
    radius: Vec<f32>,
    /// Layer being drawn and position within its order.
    layer: usize,
    cursor: usize,
    chunk: usize,
}

impl Job {
    /// A job drawing `layers` in turn, bottom first, on a page `bg`.
    #[must_use]
    pub fn new<L: PointLayer>(vp: Viewport, bg: Srgb, layers: &[L]) -> Self {
        let radius = layers
            .iter()
            .map(|l| l.size() * auto_radius(visible_count(l, &vp), &vp))
            .collect();
        let bg = to_linear(bg);
        Self {
            vp,
            canvas: Frame::from_canvas(vp.w, vp.h, vec![bg; vp.w * vp.h], bg),
            orders: layers.iter().map(PointLayer::order).collect(),
            radius,
            layer: 0,
            cursor: 0,
            chunk: FIRST_CHUNK,
        }
    }

    /// The radius layer `layer` is drawn at, before each mark's size.
    #[must_use]
    pub fn radius(&self, layer: usize) -> f32 {
        self.radius[layer]
    }

    fn done(&self) -> bool {
        self.layer >= self.orders.len()
    }

    /// Draw the next chunk. Returns `true` once every layer is drawn.
    pub fn step<L: PointLayer>(&mut self, layers: &[L]) -> bool {
        let mut budget = self.chunk;
        while budget > 0 && self.layer < layers.len() {
            let paint = &layers[self.layer];
            let order = self.orders[self.layer].clone();
            let end = (self.cursor + budget).min(order.len());
            let base = self.radius[self.layer];
            let (w, h) = (self.vp.w as f32, self.vp.h as f32);
            for &i in &order[self.cursor..end] {
                let i = i as usize;
                let (x, y) = self.vp.to_px(paint.xy(i));
                let pad = 4.0 * base + 2.0;
                if x < -pad || y < -pad || x > w + pad || y > h + pad {
                    continue;
                }
                if let Some(m) = paint.mark(i) {
                    stamp(
                        &mut self.canvas,
                        x,
                        y,
                        base * m.size,
                        m.shape,
                        m.colour,
                        m.alpha,
                    );
                }
            }
            budget -= end - self.cursor;
            self.cursor = end;
            let finished = self.cursor >= order.len();
            if finished {
                self.layer += 1;
                self.cursor = 0;
            }
        }
        self.chunk *= 2;
        self.done()
    }

    /// The canvas so far, encoded for display (intermediate frames).
    #[must_use]
    pub fn image(&self) -> RgbaImage {
        self.canvas.to_image()
    }

    /// The finished canvas, handed over for labels and encoding.
    #[must_use]
    pub fn finish(self) -> Frame {
        self.canvas
    }
}

/// Each group's median position and size, `None` for an empty group.
pub type Medians = Vec<Option<([f32; 2], usize)>>;

/// Median position of each of `n` groups, from a prefix of the `shuffled`
/// order (a uniform subsample, so the estimate is unbiased), with group
/// sizes. `group` gives a point's group, `None` (or an id past `n`) for
/// none.
#[must_use]
pub fn group_medians(
    xy: impl Fn(usize) -> [f32; 2],
    shuffled: &[u32],
    group: impl Fn(usize) -> Option<u32>,
    n: usize,
) -> Medians {
    const SAMPLE: usize = 50_000;
    let mut xs: Vec<Vec<f32>> = vec![Vec::new(); n];
    let mut ys: Vec<Vec<f32>> = vec![Vec::new(); n];
    let take = shuffled.len().min(SAMPLE);
    for &i in &shuffled[..take] {
        let Some(g) = group(i as usize).filter(|&g| (g as usize) < n) else {
            continue;
        };
        let [x, y] = xy(i as usize);
        xs[g as usize].push(x);
        ys[g as usize].push(y);
    }
    let scale = shuffled.len() as f32 / take.max(1) as f32;
    let median = |v: &mut Vec<f32>| {
        let m = v.len() / 2;
        *v.select_nth_unstable_by(m, f32::total_cmp).1
    };
    xs.iter_mut()
        .zip(ys.iter_mut())
        .map(|(x, y)| {
            if x.is_empty() {
                None
            } else {
                let n = (x.len() as f32 * scale).round() as usize;
                Some(([median(x), median(y)], n))
            }
        })
        .collect()
}

#[cfg(test)]
#[path = "../tests/points.rs"]
mod tests;
