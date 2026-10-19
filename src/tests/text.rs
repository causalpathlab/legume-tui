//! Text: senna's noto bitmaps, kept as runs for a vector page, and pinto's
//! 5×7 scale-bar font.

use super::*;
use crate::map::colour::{to_linear, Srgb};
use crate::map::raster::Frame;

const PAGE: Srgb = [246, 245, 242];

fn lit(f: &Frame) -> Vec<(usize, usize)> {
    let img = f.to_image();
    let bg = img.get_pixel(0, 0).0;
    img.enumerate_pixels()
        .filter(|(_, _, p)| p.0 != bg)
        .map(|(x, y, _)| (x as usize, y as usize))
        .collect()
}

#[test]
fn fonts_take_the_largest_raster_that_fits_then_scale_up() {
    let f = Font::for_cell_height(10.0, false);
    assert_eq!((f.line_height(), f.scale), (16, 1.0));
    let f = Font::for_cell_height(23.0, true);
    assert_eq!(f.line_height(), 20);
    let f = Font::for_cell_height(48.0, false);
    assert_eq!((f.line_height(), f.scale), (48, 1.5));
    assert_eq!(f.width("abc"), 3 * f.advance());
    assert_eq!(f.width("µé"), 2 * f.advance());
}

#[test]
fn text_is_drawn_inside_its_box_over_a_halo() {
    let font = Font::for_cell_height(16.0, false);
    let mut f = Frame::blank(120, 40, PAGE);
    draw(&mut f, font, "CT1", 10, 8, [0.0; 3], to_linear(PAGE));
    let px = lit(&f);
    assert!(!px.is_empty());
    let halo = 2;
    for (x, y) in px {
        assert!(x + halo >= 10 && x < 10 + font.width("CT1") + halo, "{x}");
        assert!(y + halo >= 8 && y < 8 + font.line_height() + halo, "{y}");
    }
}

#[test]
fn vertical_text_stands_in_its_column() {
    let font = Font::for_cell_height(16.0, false);
    let mut f = Frame::blank(40, 120, PAGE);
    draw_vertical(&mut f, font, "CT1", (10, 100), [0.0; 3], to_linear(PAGE));
    for (x, y) in lit(&f) {
        assert!(x + 2 >= 10 && x < 10 + font.line_height() + 2, "{x}");
        assert!(y < 100 + 2 && y + 2 + font.width("CT1") >= 100, "{y}");
    }
    let mut g = Frame::blank(40, 120, PAGE);
    g.keep_text();
    draw_vertical(&mut g, font, "CT1", (10, 100), [0.0; 3], to_linear(PAGE));
    let runs = g.take_text();
    assert!(runs[0].vertical && !runs[0].centred);
    assert_eq!(runs[0].w, font.width("CT1") as f32);
}

#[test]
fn a_glyph_past_the_font_falls_back_to_a_question_mark() {
    let font = Font::for_cell_height(16.0, false);
    let draw_one = |s: &str| {
        let mut f = Frame::blank(40, 30, PAGE);
        draw(&mut f, font, s, 4, 4, [0.0; 3], to_linear(PAGE));
        f.to_image().into_raw()
    };
    assert_eq!(draw_one("\u{4e00}"), draw_one("?"));
}

#[test]
fn every_scale_bar_character_has_a_tiny_glyph() {
    for label in ["0123456789", "µm mm px", "0.5"] {
        for c in label.chars() {
            assert!(tiny_glyph(c).is_some(), "{c:?}");
        }
    }
    assert!(tiny_glyph('Q').is_none());
    assert_eq!(tiny_width("50 µm", 2), 5 * (TINY_W + 1) * 2);
}

#[test]
fn tiny_text_is_filled_and_outlined() {
    let (white, black) = ([1.0; 3], [0.0; 3]);
    let mut f = Frame::blank(60, 30, [128, 128, 128]);
    draw_tiny(&mut f, (5, 5), 2, (white, black), "1");
    let at = |x: usize, y: usize| f.to_image().get_pixel(x as u32, y as u32).0;
    // '1' row 0 is "  #  ": its pixel at column 2, 2 px a glyph pixel.
    assert_eq!(at(5 + 4, 5), [255, 255, 255, 255]);
    // Beside it, the outline.
    assert_eq!(at(5 + 2, 5), [0, 0, 0, 255]);
    // Far from it, the page.
    let page = crate::map::colour::to_srgb_senna(to_linear([128; 3]));
    assert_eq!(at(50, 25)[..3], page);
    // Off the frame: clipped, no panic.
    draw_tiny(&mut f, (-20, 25), 3, (white, black), "0.5 mm");
}
