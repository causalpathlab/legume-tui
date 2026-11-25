//! A figure panel: its size, a page with no fill, and its key alone.

use super::*;
use crate::export::pdf_bytes_in;

fn entries() -> Vec<KeyEntry> {
    (0..12)
        .map(|k| (format!("C{k}"), [0.2, 0.4, 0.8], [0.0; 3]))
        .collect()
}

#[test]
fn a_panel_is_its_size_and_fits_the_map_in_its_box() {
    let p = Panel::new(1.8, 1.8).with_font_pt(7.0);
    assert_eq!(p.check(), Ok((540, 540)));
    assert!(Panel::new(1.8, 0.0).check().is_err());
    assert!(Panel::new(1.8, 1.8).with_point_alpha(0.0).check().is_err());
    // A legend takes the right of the panel; the map is centred in the rest.
    let p = p.with_labels(Labels::Legend);
    let a = p.areas(&entries(), (540, 540));
    let key = a.key.unwrap();
    assert_eq!(a.map[2], key[0]);
    assert!(key[0] > 270.0 && a.shown == 12);
    let vp = p.viewport([0.0, 0.0, 2.0, 1.0], a.map, 2.0, (540, 540), YAxis::Up);
    let ((x0, y0), (x1, y1)) = (vp.to_px([0.0, 1.0]), vp.to_px([2.0, 0.0]));
    assert!(x0 >= 0.0 && x1 <= key[0] && y0 >= 0.0 && y1 <= 540.0);
    assert!((0.5 * (x0 + x1) - 0.5 * key[0]).abs() < 0.5);
    assert!((0.5 * (y0 + y1) - 270.0).abs() < 0.5);
    // 7 pt on the page, exactly, whatever the raster line.
    let mut frame = Frame::blank(540, 540, [255; 3]);
    frame.keep_text();
    let (font, _) = p.fonts();
    text::draw(&mut frame, font, "C1", 10, 10, [0.0; 3], [1.0; 3]);
    let page = p.page(frame);
    let size = page.text[0].line * EM_PER_LINE * 72.0 / p.dpi as f32;
    assert!((size - 7.0).abs() < 1e-4);
}

#[test]
fn no_background_is_transparent_and_the_key_stands_alone() {
    let p = Panel::new(1.0, 1.0).with_background(Background::None);
    let frame = p.key_frame(&entries(), [246, 245, 242], false).unwrap();
    assert_eq!(frame.h, 300);
    let img = frame.to_image();
    // Untouched pixels see through; a dot is opaque in its own colour.
    assert_eq!(img.get_pixel(img.width() - 1, 0)[3], 0);
    assert!(img.pixels().any(|p| p[3] == 255 && p[2] > p[0]));
    let page = p.page(p.key_frame(&entries(), [0; 3], true).unwrap());
    assert_eq!(page.text.len(), 12);
    let pdf = pdf_bytes_in(&[page], p.dpi as f32, p.face).unwrap();
    let s = String::from_utf8_lossy(&pdf);
    assert!(s.contains("/SMask") && s.contains("/ArialMT"));
    assert!(p.key_frame(&[], [0; 3], false).is_err());
    // The PNG says 300 dpi (11811 px a metre) and keeps its alpha.
    let png = p.png(&frame).unwrap();
    assert_eq!(&png[37..41], b"pHYs");
    assert_eq!(u32::from_be_bytes(png[41..45].try_into().unwrap()), 11811);
    let back = image::load_from_memory(&png).unwrap().to_rgba8();
    assert_eq!(back.get_pixel(back.width() - 1, 0)[3], 0);
}
