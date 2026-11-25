//! A map as a figure panel: an exact size in inches, points a size in
//! points, a white page or none, every label one size in points, and the
//! names of its groups on the map, in a key to its right, alone in a file
//! of their own (a key shared by a row of panels), or nowhere.
//!
//! A [`Panel`] says how the panel looks and does the arithmetic: its
//! pixels ([`Panel::check`]), where the map and its key go
//! ([`Panel::areas`]), the camera that fits the data in the map's box
//! ([`Panel::viewport`]), the points' radius ([`Panel::point_radius_px`]),
//! the fonts ([`Panel::fonts`]), and the key ([`Panel::draw_key`],
//! [`Panel::key_frame`]). The viewer draws its points and labels with
//! them as it always does; [`Panel::page`] turns the frame into a PDF
//! page whose text is the panel's size exactly, [`Panel::write_pdf`]
//! writes it in the panel's [`Face`].

use super::font::Face;
use super::{fits, write_pdf_in, Page, EM_PER_LINE};
use crate::map::colour::{Lin, Srgb};
use crate::map::labels::Rect;
use crate::map::raster::{blend_round, Frame};
use crate::map::text::{self, Font};
use crate::map::viewport::{Bounds, Viewport, YAxis};
use std::io;
use std::path::Path;

/// What is under the points.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Background {
    /// White.
    #[default]
    White,
    /// Nothing: transparent in a PNG, no fill in a PDF. Labels keep a
    /// white halo.
    None,
    /// The viewer's own page colour, as on screen.
    Theme,
}

/// Where the groups are named.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Labels {
    /// On the map, each at its group, over a halo.
    #[default]
    Map,
    /// In a key to the right of the map.
    Legend,
    /// Nowhere.
    None,
}

/// One line of a key: the name, the dot's colour and the name's ink.
pub type KeyEntry = (String, Lin, Lin);

/// How a map looks as a figure panel. Built with [`Panel::new`] and the
/// `with_*` methods; the fields can also be set one by one.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Panel {
    /// The panel's size in inches: the map fits it, its aspect kept.
    pub width_in: f32,
    pub height_in: f32,
    /// Raster pixels to the inch.
    pub dpi: u32,
    /// The points' diameter in points; `None` sizes them by how crowded
    /// the map is ([`Panel::point_radius_px`]).
    pub point_pt: Option<f32>,
    /// The points' opacity, 0 to 1; `None` keeps the viewer's.
    pub point_alpha: Option<f32>,
    pub background: Background,
    /// Every label's size in points; `None` fits it to the panel
    /// ([`Panel::font_pt`]).
    pub font_pt: Option<f32>,
    /// The face a PDF's text is set in.
    pub face: Face,
    pub labels: Labels,
    /// Groups left out, by name: neither drawn nor named.
    pub hidden: Vec<String>,
}

/// Where a panel's map and key go, in pixels `[x0, y0, x1, y1]`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Areas {
    pub map: Rect,
    /// The key's box, `None` without a key or room for one.
    pub key: Option<Rect>,
    /// How many of the key's entries it has room for.
    pub shown: usize,
}

impl Panel {
    /// Raster resolution unless one is given: fine dots, and text that
    /// stays crisp in a PNG.
    pub const DPI: u32 = 300;

    /// A `width_in × height_in` inch panel: white, points sized for how
    /// crowded they are, labels on the map at a size fitting the panel, in
    /// Arial.
    #[must_use]
    pub fn new(width_in: f32, height_in: f32) -> Self {
        Self {
            width_in,
            height_in,
            dpi: Self::DPI,
            point_pt: None,
            point_alpha: None,
            background: Background::White,
            font_pt: None,
            face: Face::Arial,
            labels: Labels::Map,
            hidden: Vec::new(),
        }
    }

    #[must_use]
    pub fn with_dpi(mut self, dpi: u32) -> Self {
        self.dpi = dpi;
        self
    }

    #[must_use]
    pub fn with_point_pt(mut self, pt: f32) -> Self {
        self.point_pt = Some(pt);
        self
    }

    #[must_use]
    pub fn with_point_alpha(mut self, alpha: f32) -> Self {
        self.point_alpha = Some(alpha);
        self
    }

    #[must_use]
    pub fn with_background(mut self, background: Background) -> Self {
        self.background = background;
        self
    }

    #[must_use]
    pub fn with_font_pt(mut self, pt: f32) -> Self {
        self.font_pt = Some(pt);
        self
    }

    #[must_use]
    pub fn with_face(mut self, face: Face) -> Self {
        self.face = face;
        self
    }

    #[must_use]
    pub fn with_labels(mut self, labels: Labels) -> Self {
        self.labels = labels;
        self
    }

    #[must_use]
    pub fn with_hidden(mut self, hidden: Vec<String>) -> Self {
        self.hidden = hidden;
        self
    }

    /// Whether group `name` is left out.
    #[must_use]
    pub fn hides(&self, name: &str) -> bool {
        self.hidden.iter().any(|h| h == name)
    }

    /// Pixels to a point.
    #[must_use]
    pub fn px_per_pt(&self) -> f32 {
        self.dpi as f32 / 72.0
    }

    /// The panel's pixels, or why it cannot be drawn: a size or opacity
    /// that is not a number in range, or a page [`fits`] refuses.
    ///
    /// # Errors
    /// The reason, to show the user.
    pub fn check(&self) -> Result<(usize, usize), String> {
        let positive = |v: f32| v.is_finite() && v > 0.0;
        if !(positive(self.width_in) && positive(self.height_in) && self.dpi > 0) {
            return Err("the width, height and dpi must be positive".into());
        }
        if !self.point_pt.is_none_or(positive) || !self.font_pt.is_none_or(positive) {
            return Err("point and font sizes must be positive".into());
        }
        if !self.point_alpha.is_none_or(|a| a > 0.0 && a <= 1.0) {
            return Err("point alpha must be above 0 and at most 1".into());
        }
        let px = |inches: f32| (inches * self.dpi as f32).round() as usize;
        let (w, h) = (px(self.width_in), px(self.height_in));
        fits((w, h), self.dpi as f32)?;
        Ok((w, h))
    }

    /// The labels' size in points: as asked, else 4 points to the inch of
    /// the panel's shorter side, between 5 and 8 (7.2 on a 1.8 inch panel).
    #[must_use]
    pub fn font_pt(&self) -> f32 {
        self.font_pt
            .unwrap_or_else(|| (4.0 * self.width_in.min(self.height_in)).clamp(5.0, 8.0))
    }

    /// The raster line text of [`Panel::font_pt`] fills, in pixels.
    #[must_use]
    pub fn line_px(&self) -> f32 {
        self.font_pt() / EM_PER_LINE * self.px_per_pt()
    }

    /// The regular and bold fonts of every label.
    #[must_use]
    pub fn fonts(&self) -> (Font, Font) {
        super::fonts(self.line_px())
    }

    /// The page colour points are drawn on: the theme's `page` for
    /// [`Background::Theme`], else white (under no fill, the halo's).
    #[must_use]
    pub fn page_colour(&self, page: Srgb) -> Srgb {
        match self.background {
            Background::Theme => page,
            Background::White | Background::None => [255; 3],
        }
    }

    /// Whether the page has no fill: start the frame with
    /// [`Frame::transparent`].
    #[must_use]
    pub fn is_transparent(&self) -> bool {
        self.background == Background::None
    }

    /// The points' radius in pixels for `n` of them in a map box of
    /// `(w, h)` pixels: half of [`Panel::point_pt`], or, unsized, a dot
    /// a little wider than the room it has to itself, 0.5 to 3 points across.
    #[must_use]
    pub fn point_radius_px(&self, n: usize, (w, h): (f32, f32)) -> f32 {
        let pt = self.px_per_pt();
        let diameter = self.point_pt.unwrap_or_else(|| {
            let area_pt = (w * h).max(1.0) / (pt * pt);
            let per_pt = n.max(1) as f32 / area_pt;
            (1.2 / per_pt.sqrt()).clamp(0.5, 3.0)
        });
        0.5 * diameter * pt
    }

    /// A key's rows and columns, line height and column width in pixels,
    /// for `entries` in a box at most `max_h` pixels tall and `max_w` wide
    /// (with its half-line margin): as few rows as the columns allow, and
    /// with too few columns, every row there is room for. No columns when
    /// not even one fits.
    fn key_grid(&self, entries: &[KeyEntry], max_h: f32, max_w: f32) -> (usize, usize, f32, f32) {
        let font = self.fonts().0;
        let lh = font.line_height() as f32;
        let widest = entries.iter().map(|(n, ..)| font.width(n)).max();
        let col_w = lh * 1.5 + widest.unwrap_or(0) as f32;
        let fit = (((max_h - lh) / lh + 1e-3).floor() as usize).max(1);
        let room = ((max_w - 0.5 * lh) / col_w).floor().max(0.0) as usize;
        let cols = entries.len().div_ceil(fit).max(1).min(room);
        let rows = entries.len().div_ceil(cols.max(1)).min(fit);
        (rows, cols, lh, col_w)
    }

    /// Where the map and its key go in a `(w, h)` pixel panel: the whole
    /// panel for the map, or, with [`Labels::Legend`], `entries` in as
    /// many columns as the height needs at the right, at most half the
    /// width, and the map in what is left. Names too long for a column in
    /// half the width go under the map instead, in rows across it, at
    /// most half the height; names too long for the panel's width, nowhere.
    #[must_use]
    pub fn areas(&self, entries: &[KeyEntry], (w, h): (usize, usize)) -> Areas {
        let (w, h) = (w as f32, h as f32);
        let whole = Areas {
            map: [0.0, 0.0, w, h],
            key: None,
            shown: 0,
        };
        if self.labels != Labels::Legend || entries.is_empty() {
            return whole;
        }
        let (rows, cols, lh, col_w) = self.key_grid(entries, h, 0.5 * w);
        if cols > 0 {
            let x0 = w - (cols as f32 * col_w + 0.5 * lh);
            let key_h = rows as f32 * lh;
            let y0 = (0.5 * (h - key_h)).max(0.0);
            return Areas {
                map: [0.0, 0.0, x0, h],
                key: Some([x0, y0, w, y0 + key_h]),
                shown: (rows * cols).min(entries.len()),
            };
        }
        // Under the map: as many columns as the width holds, then rows.
        let room = ((w - 0.5 * lh) / col_w).floor().max(0.0) as usize;
        if room == 0 {
            return whole;
        }
        let cols = room.min(entries.len());
        let fit = ((0.5 * h / lh).floor() as usize).max(1);
        let rows = entries.len().div_ceil(cols).min(fit);
        let (key_w, key_h) = (cols as f32 * col_w + 0.5 * lh, rows as f32 * lh);
        let (x0, y0) = (0.5 * (w - key_w), h - key_h - 0.25 * lh);
        Areas {
            map: [0.0, 0.0, w, y0],
            key: Some([x0, y0, x0 + key_w, y0 + key_h]),
            shown: (rows * cols).min(entries.len()),
        }
    }

    /// The camera for a `(w, h)` pixel panel that fits `bounds` (data
    /// units) centred in the box `area`, a point's `radius` clear of its
    /// edges, the data's aspect kept.
    #[must_use]
    pub fn viewport(
        &self,
        bounds: Bounds,
        area: Rect,
        radius: f32,
        (w, h): (usize, usize),
        y: YAxis,
    ) -> Viewport {
        let [x0, y0, x1, y1] = bounds;
        let pad = 2.0 * radius + 1.0;
        let (bw, bh) = (
            (area[2] - area[0] - 2.0 * pad).max(1.0),
            (area[3] - area[1] - 2.0 * pad).max(1.0),
        );
        let scale = (bw / (x1 - x0).max(1e-6)).min(bh / (y1 - y0).max(1e-6));
        // The box's centre, from the frame's, in data units.
        let dx = (0.5 * (area[0] + area[2]) - 0.5 * w as f32) / scale;
        let dy = (0.5 * (area[1] + area[3]) - 0.5 * h as f32) / scale;
        let cy = 0.5 * (y0 + y1);
        Viewport {
            cx: 0.5 * (x0 + x1) - dx,
            cy: match y {
                YAxis::Up => cy + dy,
                YAxis::Down => cy - dy,
            },
            scale,
            w,
            h,
            y,
        }
    }

    /// Draw `entries` into `area` of `frame`, from [`Panel::areas`] or
    /// [`Panel::key_frame`]: a dot in each colour and its name in its ink,
    /// column after column, as many as there is room for.
    pub fn draw_key(&self, frame: &mut Frame, entries: &[KeyEntry], area: Rect) {
        let font = self.fonts().0;
        let lh = font.line_height() as f32;
        let (rows, _, lh, col_w) = self.key_grid(entries, area[3] - area[1] + lh, f32::INFINITY);
        let bg = frame.background();
        let r = 0.28 * lh;
        for (k, (name, dot, ink)) in entries.iter().enumerate() {
            let (col, row) = (k / rows, k % rows);
            let x = area[0] + col as f32 * col_w;
            if x + col_w > area[2] + 1.0 {
                break;
            }
            let y = area[1] + row as f32 * lh;
            blend_round(frame, x + 0.5 * lh, y + 0.5 * lh, r + 1.0, *dot, |d| {
                r + 0.5 - d
            });
            text::draw(frame, font, name, (x + lh) as i32, y as i32, *ink, bg);
        }
    }

    /// The key alone, for a panel of its own beside a row of maps: the
    /// panel's height, as many columns as that needs and as wide as they
    /// are, on the panel's background. With `keep_text`, its text is kept
    /// for [`Panel::page`].
    ///
    /// # Errors
    /// As [`Panel::check`], or when there is nothing to name.
    pub fn key_frame(
        &self,
        entries: &[KeyEntry],
        page: Srgb,
        keep_text: bool,
    ) -> Result<Frame, String> {
        let (_, h) = self.check()?;
        if entries.is_empty() {
            return Err("no groups to name in a key".into());
        }
        let (rows, cols, lh, col_w) = self.key_grid(entries, h as f32, f32::INFINITY);
        let w = (cols as f32 * col_w + 0.5 * lh).ceil() as usize;
        let key_h = rows as f32 * lh;
        let mut frame = Frame::blank(w, h, self.page_colour(page));
        if self.is_transparent() {
            frame = frame.transparent();
        }
        if keep_text {
            frame.keep_text();
        }
        let y0 = (0.5 * (h as f32 - key_h)).max(0.0);
        self.draw_key(&mut frame, entries, [0.5 * lh, y0, w as f32, y0 + key_h]);
        Ok(frame)
    }

    /// `frame`, drawn with its text kept, as a PDF page: the text set at
    /// [`Panel::font_pt`] exactly, not at the raster font's whole pixels.
    #[must_use]
    pub fn page(&self, mut frame: Frame) -> Page {
        let k = self.line_px() / self.fonts().0.line_height() as f32;
        let text = frame
            .take_text()
            .into_iter()
            .map(|mut run| {
                run.line *= k;
                run
            })
            .collect();
        Page {
            img: frame.to_image(),
            text,
        }
    }

    /// `frame`, its text drawn into the pixels, as PNG bytes that say the
    /// panel's resolution, so an editor places it at the panel's size.
    /// Alpha is kept: no fill stays transparent.
    ///
    /// # Errors
    /// When the image cannot be encoded.
    pub fn png(&self, frame: &Frame) -> io::Result<Vec<u8>> {
        let img = frame.to_image();
        let mut out = Vec::new();
        let mut enc = png::Encoder::new(&mut out, img.width(), img.height());
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        // Pixels to the metre, both ways.
        let ppm = (self.dpi as f32 / 0.0254).round() as u32;
        enc.set_pixel_dims(Some(png::PixelDimensions {
            xppu: ppm,
            yppu: ppm,
            unit: png::Unit::Meter,
        }));
        enc.write_header()
            .and_then(|mut w| w.write_image_data(img.as_raw()))
            .map_err(io::Error::other)?;
        Ok(out)
    }

    /// Write `pages` to `path` at the panel's resolution, in its face.
    ///
    /// # Errors
    /// As [`write_pdf_in`].
    pub fn write_pdf(&self, pages: &[Page], path: &Path) -> io::Result<()> {
        write_pdf_in(pages, self.dpi as f32, self.face, path)
    }
}

/// The flags of a figure panel, beside [`super::PdfArgs`]' `--pdf`,
/// `--width` and `--dpi`: any of them asks for a panel. A tool flattens
/// them into its own arguments.
#[cfg(feature = "clap")]
#[derive(clap::Args, Clone, Debug, Default)]
pub struct PanelArgs {
    #[arg(
        long,
        help = "Panel height in inches: with it, the map fits a --width × --height box exactly"
    )]
    pub height: Option<f32>,

    #[arg(long, help = "Save the starting view as a PNG panel too (or alone)")]
    pub png: Option<std::path::PathBuf>,

    #[arg(
        long,
        help = "Point diameter in points [default: by how crowded the map is]"
    )]
    pub point_pt: Option<f32>,

    #[arg(long, help = "Point opacity, above 0 to 1 [default: as styled]")]
    pub point_alpha: Option<f32>,

    #[arg(
        long,
        value_enum,
        help = "Under the points: white, none (transparent) or theme [default: white]"
    )]
    pub background: Option<Background>,

    #[arg(long, help = "Label size in points [default: fit to the panel]")]
    pub font_pt: Option<f32>,

    #[arg(
        long,
        value_enum,
        help = "Group names: on the map, in a legend at the right, or none"
    )]
    pub labels: Option<Labels>,

    #[arg(
        long,
        help = "Save the legend alone, a key for a row of panels (.png or .pdf)"
    )]
    pub legend: Option<std::path::PathBuf>,

    #[arg(long, num_args = 1.., value_delimiter = ',', help = "Groups left out of the panel, by name")]
    pub hide: Vec<String>,
}

#[cfg(feature = "clap")]
impl PanelArgs {
    /// Whether any panel flag is given.
    #[must_use]
    pub fn wanted(&self) -> bool {
        self.height.is_some()
            || self.png.is_some()
            || self.point_pt.is_some()
            || self.point_alpha.is_some()
            || self.background.is_some()
            || self.font_pt.is_some()
            || self.labels.is_some()
            || self.legend.is_some()
            || !self.hide.is_empty()
    }

    /// The panel these flags ask for, `width_in` wide at `dpi`, `height_in`
    /// tall unless `--height` says.
    #[must_use]
    pub fn panel(&self, width_in: f32, height_in: f32, dpi: u32) -> Panel {
        let mut p = Panel::new(width_in, self.height.unwrap_or(height_in)).with_dpi(dpi);
        p.point_pt = self.point_pt;
        p.point_alpha = self.point_alpha;
        p.font_pt = self.font_pt;
        p.background = self.background.unwrap_or_default();
        p.labels = self.labels.unwrap_or_default();
        p.hidden.clone_from(&self.hide);
        p
    }
}

#[cfg(test)]
#[path = "../tests/panel.rs"]
mod tests;
