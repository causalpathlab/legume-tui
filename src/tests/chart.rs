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

/// The least of `from..=to` that is `ok`, or `to`, one at a time.
fn least_linear(from: usize, to: usize, ok: impl Fn(usize) -> bool) -> usize {
    (from..=to).find(|&v| ok(v)).unwrap_or(to)
}

#[test]
fn least_is_the_linear_search_on_monotone_tests() {
    let mut s = 3u64;
    let mut next = |n: usize| {
        s = s
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (s >> 33) as usize % n
    };
    for case in 0..5000 {
        let from = next(1000);
        let span = [1, 2, 10, 5000][next(4)];
        let to = from + next(span);
        // A threshold anywhere: before the range, in it, or past it.
        let at = from.saturating_sub(5) + next(to - from + 20);
        let ok = |v: usize| v >= at;
        assert_eq!(
            least(from, to, ok),
            least_linear(from, to, ok),
            "case {case}: {from}..={to} from {at}"
        );
    }
    // An empty range gives `to`, as the linear search does.
    assert_eq!(least(5, 3, |_| true), 3);
    assert_eq!(least(5, 5, |_| false), 5);
    assert_eq!(least(0, usize::MAX, |v| v >= 1 << 40), 1 << 40);
}

#[test]
fn least_asks_few_times() {
    use std::cell::Cell;
    let asked = Cell::new(0usize);
    let v = least(10, 1_000_000, |v| {
        asked.set(asked.get() + 1);
        v >= 123_456
    });
    assert_eq!(v, 123_456);
    assert!(asked.get() <= 2 * 21, "{} tests", asked.get());
}
