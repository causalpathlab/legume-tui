//! A map, a chart or a grid of them as a PDF page: the raster as one
//! image, with the text over it kept as text (Helvetica, one of the
//! standard fonts every reader has), so labels stay sharp, selectable and
//! editable. A [`Frame`] drawn with [`Frame::keep_text`] gives both halves
//! of a [`Page`]. [`PageSize`] is a page's width and resolution, the sizes
//! a save offers, and [`PdfArgs`] the flags that ask for one.
//!
//! Ported from senna: [`write_pdf`] and [`Page`] from `src/view/pdf.rs`
//! (its maths unchanged), [`font`] from the same file, [`shape`],
//! [`MAX_PIXELS`] and [`grid_page`] (senna's `render_grid`, over frames
//! the viewer drew rather than its scenes) from `src/view/deck.rs`, the
//! widths and resolutions from `src/view/tui/grid.rs`, and the sizing and
//! flags of `--pdf` from `src/view/mod.rs` ([`PageSize::px`] its single
//! map's page, [`PageSize::grid_px`] its grid's).

use crate::map::colour::{to_linear, to_srgb_senna, Lin, Srgb};
use crate::map::raster::Frame;
use crate::map::text::{self, Font, TextRun};
use crate::map::theme::Theme;
use image::RgbaImage;
use pdf_writer::types::{LineJoinStyle, TextRenderingMode};
use pdf_writer::{Content, Filter, Finish, Name, Pdf, Rect, Ref, Str};
use rayon::prelude::*;
use std::io;
use std::path::Path;

pub mod font;

use font::{helvetica_width, win_ansi};

/// One page: the raster, and the text to set over it (pixel coordinates).
pub struct Page {
    pub img: RgbaImage,
    pub text: Vec<TextRun>,
}

impl Page {
    /// A `w × h` page drawn by `f`: a blank frame in `bg` that keeps its
    /// text, handed to `f` with the regular and bold fonts for `cell_px`;
    /// its pixels and text become the page, beside what `f` returned.
    pub fn draw<T>(
        (w, h): (usize, usize),
        bg: Srgb,
        cell_px: f32,
        f: impl FnOnce(&mut Frame, Font, Font) -> T,
    ) -> (Self, T) {
        let mut frame = Frame::blank(w, h, bg);
        frame.keep_text();
        let out = f(
            &mut frame,
            Font::for_cell_height(cell_px, false),
            Font::for_cell_height(cell_px, true),
        );
        let img = frame.to_image();
        let text = frame.take_text();
        (Self { img, text }, out)
    }
}

/// Pixels past which a picture is refused (its canvas is 12 bytes a pixel).
pub const MAX_PIXELS: usize = 80_000_000;

/// Point size of text whose raster line is `line` pixels tall, and where its
/// baseline sits below the top of that line, as fractions of it.
const EM_PER_LINE: f32 = 0.74;
const BASELINE: f32 = 0.78;

/// Write `pages` to `path`, `dpi` raster pixels to the inch.
///
/// # Errors
/// As [`pdf_bytes`], before the file is touched, or when it cannot be
/// written.
pub fn write_pdf(pages: &[Page], dpi: f32, path: &Path) -> io::Result<()> {
    std::fs::write(path, pdf_bytes(pages, dpi)?)
}

/// `pages` as the bytes of a PDF, `dpi` raster pixels to the inch. Each
/// run is set in its own `ink` over a halo of its `bg`, so a caller with
/// fixed greys of its own passes them in the runs.
///
/// # Errors
/// [`io::ErrorKind::InvalidInput`] for a `dpi` that is not a positive
/// number, which would size the page in infinities, or an empty page,
/// which readers refuse as an image.
pub fn pdf_bytes(pages: &[Page], dpi: f32) -> io::Result<Vec<u8>> {
    let invalid = |e: String| io::Error::new(io::ErrorKind::InvalidInput, e);
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(invalid(format!("{dpi} dpi is not a resolution")));
    }
    if let Some(k) = pages
        .iter()
        .position(|p| p.img.width() == 0 || p.img.height() == 0)
    {
        return Err(invalid(format!("page {} is empty", k + 1)));
    }
    let (catalog, tree, regular, bold) = (Ref::new(1), Ref::new(2), Ref::new(3), Ref::new(4));
    // Page, content and image of page k.
    let ids = |k: usize| {
        let b = 5 + 3 * k as i32;
        (Ref::new(b), Ref::new(b + 1), Ref::new(b + 2))
    };
    let mut pdf = Pdf::new();
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree)
        .kids((0..pages.len()).map(|k| ids(k).0))
        .count(pages.len() as i32);
    for (id, name) in [(regular, &b"Helvetica"[..]), (bold, b"Helvetica-Bold")] {
        pdf.type1_font(id)
            .base_font(Name(name))
            .encoding_predefined(Name(b"WinAnsiEncoding"));
    }

    // The rasters are the bulk of the file: pack them side by side. Level 3
    // is several times faster than the default and barely larger on points.
    let packed: Vec<Vec<u8>> = pages
        .par_iter()
        .map(|page| {
            let mut rgb = Vec::with_capacity(page.img.as_raw().len() / 4 * 3);
            rgb.extend(page.img.pixels().flat_map(|p| [p[0], p[1], p[2]]));
            miniz_oxide::deflate::compress_to_vec_zlib(&rgb, 3)
        })
        .collect();
    let pt = 72.0 / dpi;
    for (k, page) in pages.iter().enumerate() {
        let (page_id, content_id, image_id) = ids(k);
        let (w, h) = (page.img.width(), page.img.height());
        let (wpt, hpt) = (w as f32 * pt, h as f32 * pt);

        let mut p = pdf.page(page_id);
        p.media_box(Rect::new(0.0, 0.0, wpt, hpt))
            .parent(tree)
            .contents(content_id);
        let mut res = p.resources();
        res.x_objects().pair(Name(b"Im"), image_id);
        res.fonts()
            .pair(Name(b"F1"), regular)
            .pair(Name(b"F2"), bold);
        res.finish();
        p.finish();

        let mut im = pdf.image_xobject(image_id, &packed[k]);
        im.filter(Filter::FlateDecode);
        im.width(w as i32).height(h as i32).bits_per_component(8);
        im.color_space().device_rgb();
        im.finish();

        let mut c = Content::new();
        c.save_state()
            .transform([wpt, 0.0, 0.0, hpt, 0.0, 0.0])
            .x_object(Name(b"Im"))
            .restore_state();
        c.begin_text().set_line_join(LineJoinStyle::RoundJoin);
        for run in &page.text {
            let size = run.line * EM_PER_LINE * pt;
            let bytes = win_ansi(&run.text);
            let width = helvetica_width(&bytes, run.bold) * size;
            let mut x = run.x * pt;
            if run.centred {
                x += 0.5 * (run.w * pt - width);
            }
            // Turned a quarter left, the baseline is a vertical line
            // `BASELINE` of a line in from the column's left.
            let matrix = if run.vertical {
                [
                    0.0,
                    1.0,
                    -1.0,
                    0.0,
                    (run.x + BASELINE * run.line) * pt,
                    hpt - run.y * pt,
                ]
            } else {
                let y = hpt - (run.y + BASELINE * run.line) * pt;
                [1.0, 0.0, 0.0, 1.0, x, y]
            };
            let font = Name(if run.bold { b"F2" } else { b"F1" });
            c.set_font(font, size);
            // The halo: a stroke in the page colour under the letters.
            let [r, g, b] = srgb(run.bg);
            c.set_stroke_rgb(r, g, b)
                .set_line_width(0.22 * size)
                .set_text_rendering_mode(TextRenderingMode::Stroke)
                .set_text_matrix(matrix)
                .show(Str(&bytes));
            let [r, g, b] = srgb(run.ink);
            c.set_fill_rgb(r, g, b)
                .set_text_rendering_mode(TextRenderingMode::Fill)
                .set_text_matrix(matrix)
                .show(Str(&bytes));
        }
        c.end_text();
        pdf.stream(content_id, &c.finish());
    }
    Ok(pdf.finish())
}

/// A linear colour as the PDF's sRGB fractions, by senna's table.
fn srgb(c: Lin) -> [f32; 3] {
    to_srgb_senna(c).map(|v| f32::from(v) / 255.0)
}

/// Columns and rows for `n` tiles in a `w × h` area: the shape whose tiles
/// have the longest shorter side (maps are roughly square), fewer columns on
/// a tie.
#[must_use]
pub fn shape(n: usize, w: f32, h: f32) -> (usize, usize) {
    let n = n.max(1);
    let side = |c: usize| (w / c as f32).min(h / n.div_ceil(c) as f32);
    let c = (1..=n).fold(1, |best, c| if side(c) > side(best) { c } else { best });
    (c, n.div_ceil(c))
}

/// `tiles` in one `w × h` page, a grid of the [`shape`] for their number,
/// each named at its bottom left in bold at `cell_px`, with hairlines
/// between them in the theme's muted colour on its background.
///
/// Each frame is drawn by the viewer at its tile's size, `w / cols ×
/// h / rows`, and with [`Frame::keep_text`] its text, title included,
/// comes back as runs for a PDF rather than drawn into the pixels.
#[must_use]
pub fn grid_page(
    tiles: Vec<(Frame, String)>,
    (w, h): (usize, usize),
    cell_px: f32,
    theme: &Theme,
) -> Page {
    let (cols, rows) = shape(tiles.len(), w as f32, h as f32);
    let (tw, th) = (w / cols, h / rows);

    let [r, g, b] = theme.background;
    let mut out = RgbaImage::from_pixel(w as u32, h as u32, image::Rgba([r, g, b, 255]));
    let font = Font::for_cell_height(cell_px, true);
    let lh = font.line_height() as i32;
    let ink = to_linear(theme.ink);
    let mut kept = Vec::new();
    for (k, (mut frame, title)) in tiles.into_iter().enumerate() {
        let bg = frame.background();
        text::draw(
            &mut frame,
            font,
            &title,
            lh / 2,
            th as i32 - lh - lh / 4,
            ink,
            bg,
        );
        let (x, y) = (k % cols * tw, k / cols * th);
        kept.extend(frame.take_text().into_iter().map(|mut t| {
            t.x += x as f32;
            t.y += y as f32;
            t
        }));
        image::imageops::replace(&mut out, &frame.to_image(), x as i64, y as i64);
    }
    // Hairlines between tiles, on a page that has pixels to draw them in.
    if w == 0 || h == 0 {
        return Page {
            img: out,
            text: kept,
        };
    }
    let [r, g, b] = theme.muted;
    let rule = image::Rgba([r, g, b, 255]);
    for c in 1..cols {
        let x = (c * tw) as u32;
        (0..h as u32).for_each(|y| out.put_pixel(x.min(w as u32 - 1), y, rule));
    }
    for r in 1..rows {
        let y = (r * th) as u32;
        (0..w as u32).for_each(|x| out.put_pixel(x, y.min(h as u32 - 1), rule));
    }
    Page {
        img: out,
        text: kept,
    }
}

/// A page's width in inches and the resolution of its raster: its height
/// follows the picture's aspect.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PageSize {
    pub width_in: f32,
    pub dpi: u32,
}

impl PageSize {
    /// Page widths offered, in inches: journal single and double column,
    /// then larger.
    pub const WIDTHS_IN: [f32; 5] = [3.5, 5.0, 7.0, 10.0, 14.0];
    /// Raster resolutions offered for the points.
    pub const DPIS: [u32; 4] = [150, 300, 600, 1200];

    /// The raster's pixels for a picture `aspect` (height over width) tall,
    /// held between 0.4 and 2.5.
    #[must_use]
    pub fn px(self, aspect: f32) -> (usize, usize) {
        let w = (self.width_in * self.dpi as f32).round() as usize;
        (w, (w as f32 * aspect.clamp(0.4, 2.5)).round() as usize)
    }

    /// The raster's pixels for a grid of `n` tiles, as senna sizes one: 4:3
    /// tiles in the [`shape`] they take in a 4:3 page, the height as many
    /// rows of them as the shape has, the aspect not held.
    #[must_use]
    pub fn grid_px(self, n: usize) -> (usize, usize) {
        let w = (self.width_in * self.dpi as f32).round() as usize;
        let (cols, rows) = shape(n, 4.0, 3.0);
        (
            w,
            (w as f32 * 0.75 * rows as f32 / cols as f32).round() as usize,
        )
    }

    /// The cell height text is sized for: labels about 8 pt on the page,
    /// whatever its size.
    #[must_use]
    pub fn cell_px(self) -> f32 {
        self.dpi as f32 * 11.0 / 72.0
    }

    /// [`PageSize::px`], or why the page cannot be drawn: a width that is
    /// not a positive number, a dpi of 0, or more than [`MAX_PIXELS`].
    ///
    /// # Errors
    /// The reason, to show the user.
    pub fn check(self, aspect: f32) -> Result<(usize, usize), String> {
        self.fits(|p| p.px(aspect))
    }

    /// [`PageSize::grid_px`], or why the page cannot be drawn, as
    /// [`PageSize::check`].
    ///
    /// # Errors
    /// The reason, to show the user.
    pub fn check_grid(self, n: usize) -> Result<(usize, usize), String> {
        self.fits(|p| p.grid_px(n))
    }

    fn fits(self, px: impl FnOnce(Self) -> (usize, usize)) -> Result<(usize, usize), String> {
        if !(self.width_in.is_finite() && self.width_in > 0.0 && self.dpi > 0) {
            return Err("the width and dpi must be positive".into());
        }
        let (w, h) = px(self);
        if w == 0 || h == 0 {
            return Err(format!("{w} × {h} px is empty; raise the width or dpi"));
        }
        if w.checked_mul(h).is_none_or(|n| n > MAX_PIXELS) {
            return Err(format!("{w} × {h} px is too large; lower the width or dpi"));
        }
        Ok((w, h))
    }
}

/// The flags of a headless PDF, as senna names them: `--pdf`, `--width`
/// and `--dpi`. A tool flattens them into its own arguments.
#[cfg(feature = "clap")]
#[derive(clap::Args, Clone, Debug)]
pub struct PdfArgs {
    #[arg(
        long,
        help = "Save the starting view to this PDF and exit (no terminal needed)",
        long_help = "Save the starting view to a PDF and exit: the points as an image\n\
                     at --dpi, the labels as text, --width inches wide."
    )]
    pub pdf: Option<std::path::PathBuf>,

    #[arg(
        long,
        default_value_t = 7.0,
        help = "Page width in inches (with --pdf); the height follows the view"
    )]
    pub width: f32,

    #[arg(
        long,
        default_value_t = 300,
        help = "Resolution of the points in dots per inch (with --pdf); text stays text"
    )]
    pub dpi: u32,
}

#[cfg(feature = "clap")]
impl PdfArgs {
    /// The page these flags ask for.
    #[must_use]
    pub fn size(&self) -> PageSize {
        PageSize {
            width_in: self.width,
            dpi: self.dpi,
        }
    }
}

#[cfg(test)]
#[path = "tests/export.rs"]
mod tests;
