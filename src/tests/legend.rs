//! Keys: senna's legend and ramp key, with lupin's `+k more`.

use super::*;
use crate::map::colour::{to_linear, Srgb};

const PAGE: Srgb = [246, 245, 242];

fn entries(n: usize) -> Vec<(String, Lin, Lin)> {
    (0..n)
        .map(|i| (format!("CT{i}"), [0.8, 0.1, 0.1], [0.3, 0.05, 0.05]))
        .collect()
}

fn kept(f: &mut Frame) -> Vec<String> {
    f.take_text().into_iter().map(|r| r.text).collect()
}

#[test]
fn an_empty_legend_draws_nothing() {
    let mut f = Frame::blank(100, 100, PAGE);
    let font = Font::for_cell_height(16.0, false);
    assert_eq!(draw_legend(&mut f, &[], font, None), None);
    assert_eq!(
        draw_legend(&mut f, &entries(3), font, Some((0, [0.0; 3]))),
        None
    );
}

#[test]
fn a_legend_lists_every_entry_and_covers_them() {
    let mut f = Frame::blank(300, 300, PAGE);
    f.keep_text();
    let font = Font::for_cell_height(16.0, false);
    let lh = font.line_height() as f32;
    let area = draw_legend(&mut f, &entries(4), font, None).unwrap();
    assert_eq!(kept(&mut f), ["CT0", "CT1", "CT2", "CT3"]);
    let text_x = lh + (lh / 2.0).floor() + 2.0;
    assert_eq!(
        area,
        [
            0.0,
            0.0,
            text_x + font.width("CT0") as f32 + lh / 2.0,
            lh / 2.0 + 4.0 * lh + lh / 2.0
        ]
    );
    // The dots are pixels even while text is kept.
    let img = f.to_image();
    let dot = img.get_pixel(lh as u32, (lh / 2.0 + lh / 2.0) as u32).0;
    assert_ne!(dot, img.get_pixel(299, 299).0);
}

#[test]
fn past_max_rows_the_rest_are_counted() {
    let font = Font::for_cell_height(16.0, false);
    let lh = font.line_height() as f32;
    let ink = [0.1; 3];
    let mut f = Frame::blank(300, 300, PAGE);
    f.keep_text();
    let area = draw_legend(&mut f, &entries(5), font, Some((3, ink))).unwrap();
    let runs = f.take_text();
    let texts: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
    assert_eq!(texts, ["CT0", "CT1", "+3 more"]);
    assert_eq!(runs[2].ink, ink);
    assert_eq!(area[3], lh / 2.0 + 3.0 * lh + lh / 2.0);
    let text_x = lh + (lh / 2.0).floor() + 2.0;
    assert_eq!(area[2], text_x + font.width("+3 more") as f32 + lh / 2.0);
    // Entries that fit are all listed, with no count.
    let mut g = Frame::blank(300, 300, PAGE);
    g.keep_text();
    draw_legend(&mut g, &entries(3), font, Some((3, ink)));
    assert_eq!(kept(&mut g), ["CT0", "CT1", "CT2"]);
}

#[test]
fn a_legend_stops_at_the_frame_bottom() {
    let font = Font::for_cell_height(16.0, false);
    let mut f = Frame::blank(300, 60, PAGE);
    f.keep_text();
    draw_legend(&mut f, &entries(10), font, None);
    assert_eq!(kept(&mut f), ["CT0", "CT1", "CT2"]);
}

#[test]
fn a_ramp_key_titles_a_bar_of_the_ramp() {
    let font = Font::for_cell_height(16.0, false);
    let lh = font.line_height();
    let ink = to_linear([92, 92, 92]);
    let ramp = [[0.0, 0.0, 1.0], [0.5, 0.5, 0.5], [1.0, 0.0, 0.0]];
    let mut f = Frame::blank(300, 100, PAGE);
    f.keep_text();
    let area = draw_ramp_key(&mut f, "GENE1", &ramp, font, ink);
    let runs = f.take_text();
    assert_eq!(runs.len(), 1);
    assert_eq!((runs[0].text.as_str(), runs[0].ink), ("GENE1", ink));
    let (x0, by) = (lh / 2, lh / 2 + lh + 2);
    let img = f.to_image();
    assert_eq!(img.get_pixel(x0 as u32, by as u32).0, [0, 0, 255, 255]);
    assert_eq!(
        img.get_pixel((x0 + 8 * lh - 1) as u32, by as u32).0,
        [255, 0, 0, 255]
    );
    let bh = (lh / 4).max(3);
    assert_eq!(
        area,
        [
            0.0,
            0.0,
            (x0 + 8 * lh + lh / 2) as f32,
            (by + bh + lh / 2) as f32
        ]
    );
}
