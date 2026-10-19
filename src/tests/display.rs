//! A map's frame on the terminal: the cell in frame pixels, the thinnest
//! line, and the picture fitted to an area.

use super::*;
use ratatui_image::picker::ProtocolType;
use ratatui_image::FontSize;

/// A picker of a known font, as `graphics` makes one from the terminal's
/// answer.
#[allow(deprecated)]
fn sized(width: u16, height: u16) -> Picker {
    Picker::from_fontsize(FontSize { width, height })
}

fn picker(protocol: ProtocolType) -> Picker {
    let mut p = sized(9, 18);
    p.set_protocol_type(protocol);
    p
}

#[test]
fn a_cell_is_the_font_size_and_never_nothing() {
    assert_eq!(cell(&picker(ProtocolType::Kitty)), (9.0, 18.0));
    assert_eq!(cell(&sized(0, 0)), (1.0, 1.0));
}

#[test]
fn half_blocks_draw_no_line_thinner_than_a_cell() {
    assert_eq!(stroke_floor(&picker(ProtocolType::Halfblocks)), 9.0);
    for p in [
        ProtocolType::Kitty,
        ProtocolType::Sixel,
        ProtocolType::Iterm2,
    ] {
        assert_eq!(stroke_floor(&picker(p)), 1.0);
    }
}

#[test]
fn a_frame_fits_inside_its_area() {
    let p = picker(ProtocolType::Halfblocks);
    let area = Rect::new(0, 0, 20, 10);
    // Twice as wide as the area's pixels: fitted to its width.
    let img = image::RgbaImage::from_pixel(360, 90, image::Rgba([10, 20, 30, 255]));
    let proto = fit_protocol(&p, img, area).unwrap();
    let got = proto.size();
    assert!(got.width <= area.width && got.height <= area.height);
    assert_eq!(got.width, 20);
    // A frame the size of the area takes all of it.
    let img = image::RgbaImage::from_pixel(180, 180, image::Rgba([0, 0, 0, 255]));
    let proto = fit_protocol(&p, img, area).unwrap();
    assert_eq!(proto.size(), ratatui::layout::Size::new(20, 10));
}
