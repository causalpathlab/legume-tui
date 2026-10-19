//! Labels on a finished frame: centred on their anchors, the more wanted
//! first, and none over another, over a key or off the frame. A label the
//! viewer has moved ([`Offsets`]) sits where it was put instead.
//!
//! Ported from senna (`src/view/render.rs` `draw_labels`, `Label`; the
//! label hit test `Scene::group_at` from `src/view/paint.rs` as [`Hits`];
//! `TEXT_SCALES` from `src/view/mod.rs`). senna's `Tier` is a `u8` here,
//! its order kept (`Map` 0 up to `Picked` 4), and a label's group is any
//! key. With no offsets, placement and pixels are senna's.

use super::colour::Lin;
use super::offsets::Offsets;
use super::raster::{edge, Frame};
use super::text::{self, Font};
use super::viewport::Viewport;
use std::hash::Hash;

/// `[x0, y0, x1, y1]` in frame pixels.
pub type Rect = [f32; 4];

/// The label sizes a viewer steps through, as multiples of its text size.
pub const TEXT_SCALES: [f32; 4] = [1.0, 1.4, 1.8, 2.4];

/// A label candidate in pixel space.
#[derive(Clone, Debug)]
pub struct Label<K> {
    pub text: String,
    /// The anchor: where the label is centred unless moved.
    pub x: f32,
    pub y: f32,
    pub ink: Lin,
    /// Which labels win a collision, before `priority` is compared:
    /// higher first (what was just asked for over the map's own labels).
    pub tier: u8,
    /// Within a tier, larger goes first when labels collide.
    pub priority: f32,
    pub font: Font,
    /// What the label names, so a click or a drag can find it, and its
    /// offset be looked up.
    pub key: Option<K>,
}

/// Where a keyed label was drawn.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placed<K> {
    pub key: K,
    /// Its box, text and padding.
    pub rect: Rect,
    /// Its anchor.
    pub anchor: (f32, f32),
    /// Drawn at an offset from its anchor.
    pub moved: bool,
}

/// Place labels centred on their anchors, dropping any that would overlap one
/// already placed or a `reserved` rectangle, or run off the frame. Higher
/// tier, then higher priority, wins. Returns where each keyed label was
/// drawn.
///
/// A label with an offset in `offsets` goes first, centred that many line
/// heights from its anchor; it is never dropped for an overlap, it is kept
/// inside the frame, and when the gap between its box and its anchor is
/// over half a line a thin leader in its ink joins them.
pub fn place<K: Copy + Eq + Hash>(
    frame: &mut Frame,
    mut labels: Vec<Label<K>>,
    reserved: &[Rect],
    offsets: &Offsets<K>,
) -> Vec<Placed<K>> {
    let offset = |l: &Label<K>| l.key.and_then(|k| offsets.get(&k));
    let mut drawn = Vec::new();
    labels.sort_by(|a, b| {
        offset(b)
            .is_some()
            .cmp(&offset(a).is_some())
            .then(b.tier.cmp(&a.tier))
            .then(b.priority.total_cmp(&a.priority))
    });
    let bg = frame.background();
    let (w, h) = (frame.w as f32, frame.h as f32);
    let mut placed: Vec<Rect> = reserved.to_vec();
    for l in labels {
        let (tw, th) = (l.font.width(&l.text) as f32, l.font.line_height() as f32);
        let moved = offset(&l);
        let (cx, cy) = match moved {
            Some((dx, dy)) => (l.x + dx * th, l.y + dy * th),
            None => (l.x, l.y),
        };
        let (mut x0, mut y0) = (cx - 0.5 * tw, cy - 0.5 * th);
        if moved.is_some() {
            // Inside the frame, its left and top kept when it is too big.
            x0 = x0.min(w - tw - 3.0).max(3.0);
            y0 = y0.min(h - th - 1.0).max(1.0);
        }
        let rect = [x0 - 3.0, y0 - 1.0, x0 + tw + 3.0, y0 + th + 1.0];
        if moved.is_none() {
            if rect[0] < 0.0 || rect[1] < 0.0 || rect[2] > w || rect[3] > h {
                continue;
            }
            let hit = placed
                .iter()
                .any(|p| rect[0] < p[2] && p[0] < rect[2] && rect[1] < p[3] && p[1] < rect[3]);
            if hit {
                continue;
            }
        }
        placed.push(rect);
        if moved.is_some() {
            leader(frame, rect, (l.x, l.y), th, l.ink);
        }
        if let Some(key) = l.key {
            drawn.push(Placed {
                key,
                rect,
                anchor: (l.x, l.y),
                moved: moved.is_some(),
            });
        }
        text::draw_aligned(
            frame,
            l.font,
            &l.text,
            (x0 as i32, y0 as i32),
            true,
            l.ink,
            bg,
        );
    }
    drawn
}

/// A thin line from the nearest point of `rect` to `anchor`, when they are
/// more than half of `line` apart.
fn leader(frame: &mut Frame, rect: Rect, anchor: (f32, f32), line: f32, ink: Lin) {
    let near = (
        anchor.0.clamp(rect[0], rect[2]),
        anchor.1.clamp(rect[1], rect[3]),
    );
    if (anchor.0 - near.0).hypot(anchor.1 - near.1) > 0.5 * line {
        edge(frame, near, anchor, (0.0, 0.0), ink, 0.7);
    }
}

/// Where the keyed labels of the last frame were placed, and with which
/// camera, so a click finds the label it is on.
#[derive(Clone, Debug)]
pub struct Hits<K> {
    at: Option<Viewport>,
    drawn: Vec<(K, Rect)>,
}

impl<K> Default for Hits<K> {
    fn default() -> Self {
        Self {
            at: None,
            drawn: Vec::new(),
        }
    }
}

impl<K: Copy> Hits<K> {
    /// Keep what `place` gave for the frame drawn with camera `vp`.
    pub fn record(&mut self, vp: Viewport, p: &[Placed<K>]) {
        self.at = Some(vp);
        self.drawn = p.iter().map(|p| (p.key, p.rect)).collect();
    }

    /// The label under pixel `(px, py)` in the frame last drawn with camera
    /// `vp`, the one whose centre is nearest when boxes overlap: only
    /// labels that frame showed count.
    #[must_use]
    pub fn at(&self, vp: &Viewport, px: f32, py: f32) -> Option<K> {
        if self.at.as_ref() != Some(vp) {
            return None;
        }
        self.drawn
            .iter()
            .filter(|(_, [x0, y0, x1, y1])| (*x0..=*x1).contains(&px) && (*y0..=*y1).contains(&py))
            .min_by(|a, b| {
                let d = |r: &Rect| (0.5 * (r[0] + r[2]) - px).hypot(0.5 * (r[1] + r[3]) - py);
                d(&a.1).total_cmp(&d(&b.1))
            })
            .map(|(k, _)| *k)
    }

    /// Where label `k` was placed, if it was.
    #[must_use]
    pub fn rect(&self, k: K) -> Option<Rect>
    where
        K: PartialEq,
    {
        self.drawn.iter().find(|(d, _)| *d == k).map(|(_, r)| *r)
    }
}

#[cfg(test)]
#[path = "../tests/labels.rs"]
mod tests;
