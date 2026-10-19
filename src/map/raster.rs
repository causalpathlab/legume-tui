//! The raster a map is drawn on: a frame in linear light, the marks points
//! are stamped as, rings and thin edges.
//!
//! Ported from senna: `Frame`, its sRGB encoding and `draw_ring`,
//! `draw_edge` from `src/view/render.rs`, `Canvas` from
//! `src/view/text.rs`, and `Shape` with its coverage from
//! `src/view/style.rs`. The page colour is a parameter here, not senna's
//! constant, and the order of every float operation is senna's, so senna
//! keeps its pixels. [`Frame::from_srgba`] is where a renderer writing
//! sRGB bytes (pinto's) enters; such a frame gives every byte back.

use super::colour::{encode_senna, to_linear, to_srgb, Lin, Srgb};
use super::text::TextRun;
use image::RgbaImage;
use rayon::prelude::*;
use serde::{Deserialize, Serialize};

/// A linear-light canvas that marks and text are blended into.
pub trait Canvas {
    fn size(&self) -> (usize, usize);
    /// Blend `c` over pixel `(x, y)` at opacity `a`.
    fn blend(&mut self, x: usize, y: usize, c: Lin, a: f32);
    /// Where text goes instead of the pixels, when it is kept as text.
    fn text_sink(&mut self) -> Option<&mut Vec<TextRun>> {
        None
    }
}

/// How a frame turns linear light into bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Encoding {
    /// senna's table, for frames drawn from scratch.
    Senna,
    /// The table every byte comes back through, for frames entered from
    /// bytes.
    Exact,
}

/// A composited linear-light image, ready for labels and encoding.
#[derive(Clone)]
pub struct Frame {
    pub w: usize,
    pub h: usize,
    px: Vec<Lin>,
    bg: Lin,
    /// Text kept as text, when recording; see `keep_text`.
    text: Option<Vec<TextRun>>,
    encoding: Encoding,
}

impl Frame {
    /// A `w × h` page in the colour `bg`, for points or a chart drawn
    /// from scratch.
    #[must_use]
    pub fn blank(w: usize, h: usize, bg: Srgb) -> Self {
        let bg = to_linear(bg);
        Self::from_canvas(w, h, vec![bg; w * h], bg)
    }

    /// A frame holding the `w × h` sRGBA bytes `rgba` (alpha ignored) on
    /// the page `bg`, for labels and keys over a picture drawn in bytes.
    /// Pixels nothing is drawn over come back as the bytes they were.
    ///
    /// # Panics
    /// When `rgba` holds fewer than `w × h × 4` bytes.
    #[must_use]
    pub fn from_srgba(w: usize, h: usize, rgba: &[u8], bg: Srgb) -> Self {
        assert!(rgba.len() >= w * h * 4, "w × h × 4 bytes");
        let px = rgba[..w * h * 4]
            .par_chunks_exact(4)
            .map(|p| to_linear([p[0], p[1], p[2]]))
            .collect();
        Self {
            encoding: Encoding::Exact,
            ..Self::from_canvas(w, h, px, to_linear(bg))
        }
    }

    /// A frame over a finished canvas of `w × h` pixels on the page `bg`,
    /// as a point job hands it over.
    pub(crate) fn from_canvas(w: usize, h: usize, px: Vec<Lin>, bg: Lin) -> Self {
        Self {
            w,
            h,
            px,
            bg,
            text: None,
            encoding: Encoding::Senna,
        }
    }

    /// Fill the pixels `x0..x1 × y0..y1` (clipped to the frame) with `c`.
    pub fn fill(&mut self, [x0, y0, x1, y1]: [usize; 4], c: Lin) {
        let (x1, y1) = (x1.min(self.w), y1.min(self.h));
        for y in y0.min(y1)..y1 {
            self.px[y * self.w + x0.min(x1)..y * self.w + x1].fill(c);
        }
    }

    /// The pixels in linear light, row-major.
    pub(crate) fn pixels(&self) -> &[Lin] {
        &self.px
    }

    /// The page colour.
    #[must_use]
    pub fn background(&self) -> Lin {
        self.bg
    }

    /// From now on keep text as runs instead of drawing it, for a vector
    /// export that sets it in a real font over the raster.
    pub fn keep_text(&mut self) {
        self.text.get_or_insert_with(Vec::new);
    }

    /// The text kept so far.
    pub fn take_text(&mut self) -> Vec<TextRun> {
        self.text.as_mut().map(std::mem::take).unwrap_or_default()
    }

    #[must_use]
    pub fn to_image(&self) -> RgbaImage {
        encode(self.w, self.h, &self.px, self.encoding)
    }
}

/// Linear-light pixels → 8-bit sRGBA, in parallel.
fn encode(w: usize, h: usize, px: &[Lin], encoding: Encoding) -> RgbaImage {
    let buf: Vec<u8> = match encoding {
        Encoding::Senna => px
            .par_iter()
            .flat_map_iter(|p| {
                [
                    encode_senna(p[0]),
                    encode_senna(p[1]),
                    encode_senna(p[2]),
                    255,
                ]
            })
            .collect(),
        Encoding::Exact => px
            .par_iter()
            .flat_map_iter(|&p| {
                let [r, g, b] = to_srgb(p);
                [r, g, b, 255]
            })
            .collect(),
    };
    RgbaImage::from_raw(w as u32, h as u32, buf).expect("w × h × 4 bytes")
}

impl Canvas for Frame {
    fn size(&self) -> (usize, usize) {
        (self.w, self.h)
    }
    #[inline]
    fn blend(&mut self, x: usize, y: usize, c: Lin, a: f32) {
        let p = &mut self.px[y * self.w + x];
        for k in 0..3 {
            p[k] += (c[k] - p[k]) * a;
        }
    }
    fn text_sink(&mut self) -> Option<&mut Vec<TextRun>> {
        self.text.as_mut()
    }
}

/// A point's mark. Saved by name (`"circle"`), as senna's style file
/// keeps it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Shape {
    #[default]
    Circle,
    Square,
    Diamond,
    Triangle,
    Cross,
}

impl Shape {
    pub const ALL: [Shape; 5] = [
        Shape::Circle,
        Shape::Square,
        Shape::Diamond,
        Shape::Triangle,
        Shape::Cross,
    ];

    #[must_use]
    pub fn glyph(self) -> &'static str {
        match self {
            Shape::Circle => "●",
            Shape::Square => "■",
            Shape::Diamond => "◆",
            Shape::Triangle => "▲",
            Shape::Cross => "✚",
        }
    }

    #[must_use]
    pub fn name(self) -> &'static str {
        match self {
            Shape::Circle => "circle",
            Shape::Square => "square",
            Shape::Diamond => "diamond",
            Shape::Triangle => "triangle",
            Shape::Cross => "cross",
        }
    }

    /// Coverage of pixel offset `(dx, dy)` by this shape at radius `r`, with a
    /// one-pixel anti-aliased edge. Shapes are scaled to about the area of the
    /// circle of radius `r`, so switching shape does not change visual weight.
    #[inline]
    #[must_use]
    pub fn coverage(self, dx: f32, dy: f32, r: f32) -> f32 {
        let (ax, ay) = (dx.abs(), dy.abs());
        let d = match self {
            Shape::Circle => (dx * dx + dy * dy).sqrt() - r,
            Shape::Square => ax.max(ay) - 0.886 * r,
            Shape::Diamond => (ax + ay) * std::f32::consts::FRAC_1_SQRT_2 - 0.886 * r,
            Shape::Triangle => {
                // Upward equilateral triangle (screen y grows downward).
                let s = 1.35 * r;
                let k = 3f32.sqrt();
                let (mut px, mut py) = (ax - s, -dy + s / k);
                if px + k * py > 0.0 {
                    (px, py) = ((px - k * py) * 0.5, (-k * px - py) * 0.5);
                }
                px -= px.clamp(-2.0 * s, 0.0);
                -(px * px + py * py).sqrt() * py.signum()
            }
            Shape::Cross => {
                let (a, w) = (1.15 * r, 0.42 * r);
                (ax - a).max(ay - w).min((ax - w).max(ay - a))
            }
        };
        (0.5 - d).clamp(0.0, 1.0)
    }

    /// How far past `r` the shape reaches.
    #[must_use]
    pub fn reach(self, r: f32) -> f32 {
        1.4 * r + 1.0
    }
}

/// Stamp one mark of radius `r` centred on `(x, y)`, in `colour` at
/// opacity `alpha`, anti-aliased.
#[inline]
pub fn stamp<C: Canvas>(c: &mut C, x: f32, y: f32, r: f32, shape: Shape, colour: Lin, alpha: f32) {
    let (w, h) = c.size();
    let reach = shape.reach(r);
    let (x0, x1) = ((x - reach).floor().max(0.0), (x + reach).ceil());
    let (y0, y1) = ((y - reach).floor().max(0.0), (y + reach).ceil());
    let (x1, y1) = (x1.min(w as f32 - 1.0), y1.min(h as f32 - 1.0));
    if x0 > x1 || y0 > y1 {
        return;
    }
    for py in y0 as usize..=y1 as usize {
        let dy = py as f32 + 0.5 - y;
        for px in x0 as usize..=x1 as usize {
            let a = alpha * shape.coverage(px as f32 + 0.5 - x, dy, r);
            if a > 0.0 {
                c.blend(px, py, colour, a);
            }
        }
    }
}

/// A ring around `(x, y)`, to mark one point.
pub fn ring(frame: &mut Frame, x: f32, y: f32, r: f32, ink: Lin) {
    blend_round(frame, x, y, r + 2.0, ink, |d| 1.2 - (d - r).abs());
}

/// A thin line from `a` to `b` at opacity `alpha`, stopping `trim.0` and
/// `trim.1` short of its ends so it meets the rings drawn there.
pub fn edge(
    frame: &mut Frame,
    a: (f32, f32),
    b: (f32, f32),
    (trim_a, trim_b): (f32, f32),
    ink: Lin,
    alpha: f32,
) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len = dx.hypot(dy);
    if len <= trim_a + trim_b + 1.0 {
        return;
    }
    let (ux, uy) = (dx / len, dy / len);
    // Pixel centres at integer coordinates, the major axis first.
    let steep = dy.abs() > dx.abs();
    let ends = [
        (a.0 + ux * trim_a - 0.5, a.1 + uy * trim_a - 0.5),
        (b.0 - ux * trim_b - 0.5, b.1 - uy * trim_b - 0.5),
    ]
    .map(|(x, y)| if steep { (y, x) } else { (x, y) });
    let [p, q] = if ends[0].0 <= ends[1].0 {
        ends
    } else {
        [ends[1], ends[0]]
    };
    let slope = (q.1 - p.1) / (q.0 - p.0);
    // One pixel column per step; the line's minor coordinate is shared
    // between the two pixels it falls between. Only the columns on the
    // frame are stepped, so a far end costs nothing.
    let major = if steep { frame.h } else { frame.w } as i64;
    for m in (p.0.ceil() as i64).max(0)..=(q.0.floor() as i64).min(major - 1) {
        let minor = p.1 + (m as f32 - p.0) * slope;
        let lo = minor.floor();
        for (o, w) in [(lo as i64, 1.0 - (minor - lo)), (lo as i64 + 1, minor - lo)] {
            let (x, y) = if steep { (o, m) } else { (m, o) };
            if x >= 0 && y >= 0 && (x as usize) < frame.w && (y as usize) < frame.h {
                frame.blend(x as usize, y as usize, ink, alpha * w);
            }
        }
    }
}

/// Blend `c` over the pixels within `reach` of `(x, y)`, each at the opacity
/// `cov(distance)` clamped to `[0, 1]`: a disc, a ring, whatever `cov` draws.
pub(crate) fn blend_round(
    frame: &mut Frame,
    x: f32,
    y: f32,
    reach: f32,
    c: Lin,
    cov: impl Fn(f32) -> f32,
) {
    for py in (y - reach).floor() as i32..=(y + reach).ceil() as i32 {
        for px in (x - reach).floor() as i32..=(x + reach).ceil() as i32 {
            if px < 0 || py < 0 || px as usize >= frame.w || py as usize >= frame.h {
                continue;
            }
            let d = ((px as f32 + 0.5 - x).powi(2) + (py as f32 + 0.5 - y).powi(2)).sqrt();
            let a = cov(d).clamp(0.0, 1.0);
            if a > 0.0 {
                frame.blend(px as usize, py as usize, c, a);
            }
        }
    }
}

#[cfg(test)]
#[path = "../tests/raster.rs"]
mod tests;
