//! What both charts share: a note in place of a chart, and the steps of
//! `+` / `-` through how many rows a heatmap shows.

use super::*;
use crate::map::raster::Frame;
use crate::map::theme::Theme;

#[test]
fn step_top_both_rules() {
    // senna: up by a quarter, down by a fifth, rounded up, within range.
    let q = |now, more| step_top(now, more, Step::Quarter, (1, 100));
    assert_eq!((q(10, true), q(10, false)), (13, 8));
    assert_eq!((q(1, true), q(1, false)), (2, 1));
    assert_eq!((q(90, true), q(100, true)), (100, 100));
    // pinto: one at a time.
    let one = |now, more| step_top(now, more, Step::One, (1, 100));
    assert_eq!((one(3, true), one(3, false)), (4, 2));
    assert_eq!((one(1, false), one(100, true)), (1, 100));
    // Out of range to begin with: brought into it.
    assert_eq!(step_top(0, false, Step::One, (1, 100)), 1);
    assert_eq!(step_top(500, true, Step::Quarter, (1, 100)), 100);
}

#[test]
fn a_note_is_one_line_in_the_page_s_text() {
    let theme = Theme::PAPER;
    let font = Font::for_cell_height(16.0, false);
    let mut f = Frame::blank(200, 50, theme.background);
    f.keep_text();
    note(&mut f, "drawing the heatmap…", font, &theme);
    let runs = f.take_text();
    assert_eq!(runs.len(), 1);
    let r = &runs[0];
    assert_eq!(
        (r.text.as_str(), r.x, r.y),
        ("drawing the heatmap…", 16.0, 16.0)
    );
    assert_eq!(r.ink, crate::map::colour::to_linear(theme.text));
    // Drawn, it changes the pixels.
    let mut g = Frame::blank(200, 50, theme.background);
    let blank = g.to_image();
    note(&mut g, "x", font, &theme);
    assert_ne!(g.to_image(), blank);
}
