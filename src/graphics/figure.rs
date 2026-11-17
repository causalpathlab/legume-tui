//! Figures drawn as images inside the full-screen views, in terminals that
//! speak an image protocol (kitty, iTerm2, WezTerm, Ghostty, sixel). The
//! image is the same SVG a save writes, rasterised (resvg, with the
//! system's fonts) to the plot area's pixel size; elsewhere a view keeps
//! its text plot, which beats half-block images.
//!
//! Ported from data-beans and faba (`src/figure/term.rs`, and their SVG
//! rasterising): an image that could not be made is not tried again
//! until the view, the area or the terminal changes.

use super::Fallback;
use crate::Session;
use image::{DynamicImage, RgbaImage};
use ratatui::layout::{Rect, Size};
use ratatui::Frame;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::{Image, Resize};
use resvg::tiny_skia;
use std::io;
use std::sync::{Arc, OnceLock};

/// Figure units (points) per terminal cell height, so 9-point text reads
/// at about three quarters of a text line.
pub const UNITS_PER_CELL: f32 = 12.0;

/// Whether the environment names a terminal with an image protocol. The
/// query is only sent then, so a terminal without one never waits for its
/// answer.
fn likely_images(var: impl Fn(&str) -> Option<String>) -> bool {
    let has = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let term = var("TERM").unwrap_or_default();
    let program = var("TERM_PROGRAM").unwrap_or_default();
    has("KITTY_WINDOW_ID")
        || has("WEZTERM_EXECUTABLE")
        || matches!(
            program.as_str(),
            "iTerm.app" | "WezTerm" | "ghostty" | "vscode"
        )
        || var("LC_TERMINAL").is_some_and(|v| v == "iTerm2")
        || ["kitty", "ghostty", "wezterm", "foot"]
            .iter()
            .any(|t| term.contains(t))
}

/// Whether to ask the terminal for its image protocol, by the override's
/// value (`images`) and the terminal's environment (`var`): never with
/// `0`, always with `1`, else where the environment names a terminal that
/// has one.
fn asks(images: Option<&str>, var: impl Fn(&str) -> Option<String>) -> bool {
    match images {
        Some("0") => false,
        Some("1") => true,
        _ => likely_images(var),
    }
}

/// The image support of the terminal `session` is attached to now, asked
/// of it ([`super::pick_protocol`], with a timeout). `None` without an
/// image protocol (block characters, where text plots do better), in a
/// terminal not known to have one, or as the environment variable
/// `override_var` (`TOOL_TUI_IMAGES`, say) says: `0` never asks, `1`
/// always does. Call it with the screen up, and again after every attach.
pub fn picker(session: &Session, override_var: &str) -> Option<Picker> {
    let images = std::env::var(override_var).ok();
    if !asks(images.as_deref(), |k| session.terminal_env(k)) {
        return None;
    }
    let (p, _) = super::pick_protocol(session, None, Fallback::Probe);
    (p.protocol_type() != ProtocolType::Halfblocks).then_some(p)
}

/// The system's fonts, loaded once.
fn fontdb() -> Arc<usvg::fontdb::Database> {
    static DB: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    DB.get_or_init(|| {
        let mut db = usvg::fontdb::Database::new();
        db.load_system_fonts();
        Arc::new(db)
    })
    .clone()
}

/// SVG parsing options, with the system's fonts loaded once.
pub fn svg_options() -> usvg::Options<'static> {
    usvg::Options {
        fontdb: fontdb(),
        ..usvg::Options::default()
    }
}

/// `svg` parsed with [`svg_options`].
pub fn svg_tree(svg: &str) -> io::Result<usvg::Tree> {
    usvg::Tree::from_str(svg, &svg_options()).map_err(io::Error::other)
}

/// `tree` rendered at `scale` pixels per unit, on white.
pub fn pixmap(tree: &usvg::Tree, scale: f32) -> io::Result<tiny_skia::Pixmap> {
    let size = tree
        .size()
        .to_int_size()
        .scale_by(scale)
        .ok_or_else(|| io::Error::other("figure has no size"))?;
    let mut pixmap = tiny_skia::Pixmap::new(size.width(), size.height())
        .ok_or_else(|| io::Error::other("figure too large to rasterise"))?;
    pixmap.fill(tiny_skia::Color::WHITE);
    resvg::render(
        tree,
        tiny_skia::Transform::from_scale(scale, scale),
        &mut pixmap.as_mut(),
    );
    Ok(pixmap)
}

/// `svg` as an RGBA image, `scale` pixels per unit, on white.
pub fn raster(svg: &str, scale: f32) -> io::Result<RgbaImage> {
    let pixmap = pixmap(&svg_tree(svg)?, scale)?;
    let (w, h) = (pixmap.width(), pixmap.height());
    RgbaImage::from_raw(w, h, pixmap.take())
        .ok_or_else(|| io::Error::other("pixel buffer size mismatch"))
}

/// `svg` as PNG bytes, `scale` pixels per unit, on white.
pub fn png(svg: &str, scale: f32) -> io::Result<Vec<u8>> {
    pixmap(&svg_tree(svg)?, scale)?
        .encode_png()
        .map_err(io::Error::other)
}

/// What an image made with a picker depends on: its protocol and cell size.
type PickerKind = (ProtocolType, u16, u16);

/// `picker`'s [`PickerKind`].
fn kind(picker: &Picker) -> PickerKind {
    let font = picker.font_size();
    (picker.protocol_type(), font.width, font.height)
}

/// One plot area drawn as an image: rebuilt only when marked stale, when
/// the area changes size, or after a terminal attached or the picker
/// changed (its protocol or cell size may be another, and kitty's images
/// were deleted by the attach). An image that could not be made is not
/// tried again until then either.
#[derive(Default)]
pub struct PlotImage {
    stale: bool,
    size: Option<(u16, u16)>,
    protocol: Option<Protocol>,
    /// The attach ([`crate::attaches`]) the image was made at, and the
    /// picker's [`kind`].
    made: Option<(u64, PickerKind)>,
}

impl PlotImage {
    /// The view changed: rebuild on the next frame.
    pub fn invalidate(&mut self) {
        self.stale = true;
    }

    /// Whether the next frame rebuilds it, for a view that marked it.
    pub fn is_stale(&self) -> bool {
        self.stale
    }

    /// Draw into `area`. `draw(width, height)` returns the SVG, in figure
    /// units, for an area that size. False when no image could be made,
    /// so the caller can fall back to text.
    pub fn render(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        picker: &Picker,
        draw: impl FnOnce(f64, f64) -> String,
    ) -> bool {
        self.render_at(frame, area, picker, crate::attaches(), draw)
    }

    /// [`PlotImage::render`] as after `attach` attaches
    /// ([`crate::attaches`]): an image made at another, or with a picker
    /// of another protocol or cell size, is made again. For tests.
    pub fn render_at(
        &mut self,
        frame: &mut Frame,
        area: Rect,
        picker: &Picker,
        attach: u64,
        draw: impl FnOnce(f64, f64) -> String,
    ) -> bool {
        if area.width == 0 || area.height == 0 {
            return true;
        }
        let made = (attach, kind(picker));
        if self.made != Some(made) {
            self.invalidate();
            self.made = Some(made);
        }
        let size = (area.width, area.height);
        if self.stale || self.size != Some(size) {
            let font = picker.font_size();
            let scale = font.height.max(1) as f32 / UNITS_PER_CELL;
            let (px, py) = (
                f32::from(area.width) * f32::from(font.width),
                f32::from(area.height) * f32::from(font.height),
            );
            let svg = draw(f64::from(px / scale), f64::from(py / scale));
            let made = raster(&svg, scale).and_then(|img| {
                picker
                    .new_protocol(
                        DynamicImage::ImageRgba8(img),
                        Size::new(area.width, area.height),
                        Resize::Fit(None),
                    )
                    .map_err(io::Error::other)
            });
            #[cfg(feature = "logs")]
            if let Err(e) = &made {
                log::debug!("no image for a {}x{} plot area: {e}", size.0, size.1);
            }
            self.protocol = made.ok();
            self.size = Some(size);
            self.stale = false;
        }
        match &self.protocol {
            Some(p) => {
                frame.render_widget(Image::new(p), area);
                true
            }
            None => false,
        }
    }
}

#[cfg(test)]
#[path = "../tests/figure.rs"]
mod tests;
