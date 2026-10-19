//! The viewport: which part of the data a `w × h` frame shows, as a centre
//! and a scale in pixels per data unit.
//!
//! senna's parameterisation (`src/view/render.rs` `Viewport`, and
//! `src/view/deck.rs` `camera` for [`Viewport::resized`]), with pinto's y
//! running down the screen (`src/view/render.rs`) and its scale limits,
//! and lupin's view kept inside the data (`src/figures/mod.rs`
//! `View::clamp`) as [`Limits::keep_inside`]. A zoom factor above one
//! always zooms in.

/// Which way data y runs on screen.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum YAxis {
    /// Up, as on a plot (senna, lupin).
    Up,
    /// Down, as in an image (pinto).
    Down,
}

/// `[x0, y0, x1, y1]` in data units.
pub type Bounds = [f32; 4];

/// How far a viewport may zoom and pan.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Limits {
    /// Smallest and largest scale, in pixels per data unit.
    pub scale: Option<(f32, f32)>,
    /// Keep the window inside these bounds: no panning past the data. On
    /// an axis the window is wider than them, they sit in its middle.
    pub keep_inside: Option<Bounds>,
}

/// Maps data coordinates to pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Viewport {
    pub cx: f32,
    pub cy: f32,
    /// Pixels per data unit.
    pub scale: f32,
    pub w: usize,
    pub h: usize,
    pub y: YAxis,
}

impl Viewport {
    /// Fit `bounds` into `w × h`, the tighter axis filled to `margin` of
    /// the frame (senna takes 0.92, pinto 1).
    #[must_use]
    pub fn fit(bounds: Bounds, w: usize, h: usize, margin: f32, y: YAxis) -> Self {
        let [x0, y0, x1, y1] = bounds;
        let (dw, dh) = ((x1 - x0).max(1e-6), (y1 - y0).max(1e-6));
        let scale = margin * (w as f32 / dw).min(h as f32 / dh);
        Self {
            cx: 0.5 * (x0 + x1),
            cy: 0.5 * (y0 + y1),
            scale,
            w,
            h,
            y,
        }
    }

    /// Screen y of data y measured from the centre, before scaling.
    #[inline]
    fn dy(self, y: f32) -> f32 {
        match self.y {
            YAxis::Up => self.cy - y,
            YAxis::Down => y - self.cy,
        }
    }

    #[inline]
    #[must_use]
    pub fn to_px(self, [x, y]: [f32; 2]) -> (f32, f32) {
        (
            (x - self.cx) * self.scale + 0.5 * self.w as f32,
            self.dy(y) * self.scale + 0.5 * self.h as f32,
        )
    }

    #[must_use]
    pub fn to_data(self, px: f32, py: f32) -> [f32; 2] {
        let dy = (py - 0.5 * self.h as f32) / self.scale;
        [
            (px - 0.5 * self.w as f32) / self.scale + self.cx,
            match self.y {
                YAxis::Up => self.cy - dy,
                YAxis::Down => self.cy + dy,
            },
        ]
    }

    /// Zoom in by `factor` (below one zooms out), the scale kept within
    /// `lim`, keeping the data point under `(px, py)` fixed.
    pub fn zoom_at(&mut self, factor: f32, px: f32, py: f32, lim: &Limits) {
        let anchor = self.to_data(px, py);
        self.scale *= factor;
        if let Some((min, max)) = lim.scale {
            self.scale = self.scale.clamp(min, max);
        }
        let moved = self.to_data(px, py);
        self.cx += anchor[0] - moved[0];
        self.cy += anchor[1] - moved[1];
        self.keep(lim);
    }

    /// Move the picture by `(dx, dy)` pixels, as a drag does.
    pub fn pan_px(&mut self, dx: f32, dy: f32, lim: &Limits) {
        self.cx -= dx / self.scale;
        match self.y {
            YAxis::Up => self.cy += dy / self.scale,
            YAxis::Down => self.cy -= dy / self.scale,
        }
        self.keep(lim);
    }

    /// The window back inside `lim.keep_inside`.
    fn keep(&mut self, lim: &Limits) {
        let Some([x0, y0, x1, y1]) = lim.keep_inside else {
            return;
        };
        let inside = |c: f32, half: f32, lo: f32, hi: f32| {
            if 2.0 * half >= hi - lo {
                0.5 * (lo + hi)
            } else {
                c.clamp(lo + half, hi - half)
            }
        };
        let (hw, hh) = (
            0.5 * self.w as f32 / self.scale,
            0.5 * self.h as f32 / self.scale,
        );
        self.cx = inside(self.cx, hw, x0, x1);
        self.cy = inside(self.cy, hh, y0, y1);
    }

    /// Data units per pixel.
    #[must_use]
    pub fn upp(&self) -> f32 {
        1.0 / self.scale
    }

    /// The data the frame covers, `x0 < x1` and `y0 < y1` either way y runs.
    #[must_use]
    pub fn window(&self) -> Bounds {
        let (hw, hh) = (
            0.5 * self.w as f32 / self.scale,
            0.5 * self.h as f32 / self.scale,
        );
        [self.cx - hw, self.cy - hh, self.cx + hw, self.cy + hh]
    }

    /// The same view in a `w × h` frame: its centre kept, scaled so what
    /// it showed still fits. A view never drawn (no pixels) is fitted to
    /// `bounds` afresh, with senna's margin.
    #[must_use]
    pub fn resized(self, bounds: Bounds, w: usize, h: usize) -> Self {
        if self.w > 0 && self.h > 0 {
            let k = (w as f32 / self.w as f32).min(h as f32 / self.h as f32);
            Self {
                scale: self.scale * k,
                w,
                h,
                ..self
            }
        } else {
            Self::fit(bounds, w, h, 0.92, self.y)
        }
    }
}

#[cfg(test)]
#[path = "../tests/viewport.rs"]
mod tests;
