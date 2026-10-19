//! A map's frame on the terminal, through ratatui-image: how many frame
//! pixels a cell holds, the thinnest line worth drawing, and the frame
//! fitted to an area. senna's `cell`, `stroke_floor` and `protocol`
//! (`src/view/tui/mod.rs`); lupin's `fit` is the same call.

use image::DynamicImage;
use ratatui::layout::{Rect, Size};
use ratatui_image::errors::Errors;
use ratatui_image::picker::{Picker, ProtocolType};
use ratatui_image::protocol::Protocol;
use ratatui_image::Resize;

/// Frame pixels per terminal cell, across and down: the font's size, at
/// least one each way.
#[must_use]
pub fn cell(picker: &Picker) -> (f32, f32) {
    let f = picker.font_size();
    (f32::from(f.width.max(1)), f32::from(f.height.max(1)))
}

/// The thinnest line drawn on the map under `picker`: one pixel of the
/// screen when it shows the frame in coarser pixels (half blocks), as a
/// thinner line averages away.
#[must_use]
pub fn stroke_floor(picker: &Picker) -> f32 {
    if picker.protocol_type() == ProtocolType::Halfblocks {
        cell(picker).0
    } else {
        1.0
    }
}

/// `img` ready to draw into `area`, scaled to fit it.
///
/// # Errors
/// When ratatui-image cannot encode the picture for the protocol.
pub fn fit_protocol(
    picker: &Picker,
    img: image::RgbaImage,
    area: Rect,
) -> Result<Protocol, Errors> {
    picker.new_protocol(
        DynamicImage::ImageRgba8(img),
        Size::new(area.width, area.height),
        Resize::Fit(Some(image::imageops::FilterType::Triangle)),
    )
}

#[cfg(test)]
#[path = "../tests/display.rs"]
mod tests;
