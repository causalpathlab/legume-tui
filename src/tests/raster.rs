//! The raster: frames, marks, rings and edges, as senna drew them.

use super::*;
use crate::map::colour::{encode_senna, linear, to_linear, to_srgb_senna};
use crate::map::text::{self, Font};

const PAGE: Srgb = [246, 245, 242];

fn pixel(f: &Frame, x: usize, y: usize) -> Lin {
    f.px[y * f.w + x]
}

#[test]
fn every_shape_covers_its_centre_and_not_far_outside() {
    for s in Shape::ALL {
        assert!(s.coverage(0.0, 0.0, 3.0) > 0.99, "{s:?} centre");
        assert!(s.coverage(6.0, 6.0, 3.0) < 0.01, "{s:?} outside");
    }
}

#[test]
fn shapes_keep_their_names_on_disk() {
    for s in Shape::ALL {
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(json, format!("\"{}\"", s.name()));
        assert_eq!(serde_json::from_str::<Shape>(&json).unwrap(), s);
    }
    assert_eq!(Shape::default(), Shape::Circle);
}

#[test]
fn a_blank_page_encodes_through_senna_s_table() {
    let f = Frame::blank(3, 2, PAGE);
    assert_eq!(f.background(), to_linear(PAGE));
    let img = f.to_image();
    assert_eq!(img.dimensions(), (3, 2));
    let want = to_srgb_senna(to_linear(PAGE));
    for p in img.pixels() {
        assert_eq!(p.0, [want[0], want[1], want[2], 255]);
    }
}

#[test]
fn a_frame_from_srgb_bytes_gives_every_byte_back() {
    // senna's table loses a few dark bytes; a frame entered from bytes
    // keeps them all, so pinto's pixels do not change.
    assert_ne!(encode_senna(linear(3)), 3);
    let rgba: Vec<u8> = (0..=255u8).flat_map(|v| [v, 255 - v, v / 2, 7]).collect();
    let f = Frame::from_srgba(16, 16, &rgba, [0, 0, 0]);
    let back = f.to_image().into_raw();
    for (i, (a, b)) in rgba.chunks(4).zip(back.chunks(4)).enumerate() {
        assert_eq!(a[..3], b[..3], "pixel {i}");
        assert_eq!(b[3], 255);
    }
    assert_eq!(f.background(), [0.0; 3]);
}

#[test]
fn fill_is_clipped_to_the_frame() {
    let mut f = Frame::blank(4, 3, PAGE);
    f.fill([2, 1, 10, 10], [1.0, 0.0, 0.0]);
    assert_eq!(pixel(&f, 3, 2), [1.0, 0.0, 0.0]);
    assert_eq!(pixel(&f, 2, 1), [1.0, 0.0, 0.0]);
    assert_eq!(pixel(&f, 1, 1), to_linear(PAGE));
    assert_eq!(pixel(&f, 3, 0), to_linear(PAGE));
    // A rectangle past the frame is nothing.
    f.fill([5, 5, 2, 2], [0.0; 3]);
}

#[test]
fn a_mark_covers_its_centre_and_stops_at_the_frame() {
    let mut f = Frame::blank(10, 10, PAGE);
    let red = [1.0, 0.0, 0.0];
    stamp(&mut f, 5.0, 5.0, 2.0, Shape::Circle, red, 1.0);
    assert_eq!(pixel(&f, 5, 5), red);
    assert_eq!(pixel(&f, 0, 0), to_linear(PAGE));
    // Half opacity lands halfway.
    let mut g = Frame::blank(10, 10, [0, 0, 0]);
    stamp(&mut g, 5.0, 5.0, 2.0, Shape::Square, [1.0; 3], 0.5);
    assert!((pixel(&g, 5, 5)[0] - 0.5).abs() < 1e-6);
    // Off the frame, or past its corner, draws nothing and does not panic.
    stamp(&mut f, -20.0, -20.0, 2.0, Shape::Cross, red, 1.0);
    stamp(&mut f, 30.0, 30.0, 2.0, Shape::Triangle, red, 1.0);
    stamp(&mut f, 9.9, 9.9, 3.0, Shape::Diamond, red, 1.0);
}

#[test]
fn a_ring_leaves_its_centre_alone() {
    let mut f = Frame::blank(40, 40, PAGE);
    let ink = [0.0; 3];
    ring(&mut f, 20.0, 20.0, 8.0, ink);
    assert_eq!(pixel(&f, 20, 20), to_linear(PAGE));
    assert_ne!(pixel(&f, 27, 19), to_linear(PAGE));
    assert_eq!(pixel(&f, 35, 19), to_linear(PAGE));
}

#[test]
fn an_edge_runs_between_its_trims() {
    let mut f = Frame::blank(100, 40, PAGE);
    let ink = [0.0; 3];
    edge(&mut f, (10.0, 20.5), (90.0, 20.5), (10.0, 10.0), ink, 1.0);
    // Halfway along, drawn; inside either trim, not.
    assert_ne!(pixel(&f, 50, 20), to_linear(PAGE));
    assert_eq!(pixel(&f, 12, 20), to_linear(PAGE));
    assert_eq!(pixel(&f, 88, 20), to_linear(PAGE));
    // Steep, reversed and off the frame: no panic, and the middle drawn.
    edge(&mut f, (50.5, 39.0), (52.5, -10.0), (0.0, 0.0), ink, 0.5);
    assert_ne!(pixel(&f, 51, 10), to_linear(PAGE));
    // Shorter than its trims: nothing.
    let before = f.clone();
    edge(&mut f, (5.0, 5.0), (8.0, 5.0), (2.0, 2.0), ink, 1.0);
    assert_eq!(before.px, f.px);
}

#[test]
fn kept_text_is_text_and_leaves_the_pixels() {
    let mut f = Frame::blank(200, 40, PAGE);
    let font = Font::for_cell_height(16.0, true);
    f.keep_text();
    text::draw_aligned(
        &mut f,
        font,
        "L1-0",
        (4, 5),
        true,
        [0.0; 3],
        to_linear(PAGE),
    );
    assert!(f.px.iter().all(|&p| p == to_linear(PAGE)));
    let runs = f.take_text();
    assert_eq!(runs.len(), 1);
    assert_eq!(runs[0].text, "L1-0");
    assert!(runs[0].centred && runs[0].bold && !runs[0].vertical);
    assert_eq!((runs[0].x, runs[0].y), (4.0, 5.0));
    assert!(f.take_text().is_empty());
    // Without keep_text, text is pixels.
    let mut g = Frame::blank(200, 40, PAGE);
    text::draw(&mut g, font, "L1-0", 4, 5, [0.0; 3], to_linear(PAGE));
    assert!(g.px.iter().any(|&p| p != to_linear(PAGE)));
    assert!(g.take_text().is_empty());
}
