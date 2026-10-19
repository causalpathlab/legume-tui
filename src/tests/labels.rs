//! Label placement: senna's rule with no offsets, and labels a viewer has
//! moved.

use super::*;
use crate::map::colour::{to_linear, Srgb};
use crate::map::raster::Frame;
use crate::map::viewport::{Viewport, YAxis};

const PAGE: Srgb = [246, 245, 242];

fn font() -> Font {
    Font::for_cell_height(16.0, true)
}

fn label(text: &str, (x, y): (f32, f32), tier: u8, priority: f32, key: Option<u32>) -> Label<u32> {
    Label {
        text: text.to_string(),
        x,
        y,
        ink: [0.0; 3],
        tier,
        priority,
        font: font(),
        key,
    }
}

fn none() -> Offsets<u32> {
    Offsets::default()
}

fn keys(p: &[Placed<u32>]) -> Vec<u32> {
    p.iter().map(|p| p.key).collect()
}

#[test]
fn labels_go_by_tier_then_priority_and_drop_what_they_would_cover() {
    let mut f = Frame::blank(200, 100, PAGE);
    let placed = place(
        &mut f,
        vec![
            label("CT1", (100.0, 50.0), 0, 9.0, Some(1)),
            label("CT2", (104.0, 52.0), 0, 5.0, Some(2)),
            // A higher tier wins over any priority.
            label("CT3", (96.0, 48.0), 1, 0.0, Some(3)),
            // Off the frame, and over a reserved corner: not drawn.
            label("CT4", (2.0, 50.0), 0, 99.0, Some(4)),
            label("CT5", (180.0, 15.0), 0, 99.0, Some(5)),
            // Drawn, but with no key, not reported.
            label("L1-0", (40.0, 80.0), 0, 1.0, None),
        ],
        &[[160.0, 0.0, 200.0, 30.0]],
        &none(),
    );
    assert_eq!(keys(&placed), [3]);
    let p = &placed[0];
    assert_eq!(p.anchor, (96.0, 48.0));
    assert!(!p.moved);
    let (tw, th) = (font().width("CT3") as f32, font().line_height() as f32);
    let (x0, y0) = (96.0 - 0.5 * tw, 48.0 - 0.5 * th);
    assert_eq!(p.rect, [x0 - 3.0, y0 - 1.0, x0 + tw + 3.0, y0 + th + 1.0]);
}

#[test]
fn a_moved_label_goes_first_and_is_kept_over_an_overlap() {
    let labels = || {
        vec![
            label("CT1", (100.0, 50.0), 2, 9.0, Some(1)),
            label("CT2", (100.0, 50.0), 0, 1.0, Some(2)),
        ]
    };
    let mut f = Frame::blank(200, 100, PAGE);
    assert_eq!(keys(&place(&mut f, labels(), &[], &none())), [1]);
    // Moved a little, still over CT1: kept, and placed first, so CT1,
    // which now overlaps it, goes.
    let mut o = none();
    o.nudge(2, (0.1, 0.0));
    let mut f = Frame::blank(200, 100, PAGE);
    let placed = place(&mut f, labels(), &[], &o);
    assert_eq!(keys(&placed), [2]);
    assert!(placed[0].moved);
    // Over a reserved rectangle, too.
    let mut f = Frame::blank(200, 100, PAGE);
    let placed = place(&mut f, labels(), &[[0.0, 0.0, 200.0, 100.0]], &o);
    assert_eq!(keys(&placed), [2]);
}

#[test]
fn offsets_are_in_line_heights() {
    let mut o = none();
    o.nudge(1, (1.0, -0.5));
    for px in [16.0, 40.0] {
        let mut l = label("CT1", (100.0, 60.0), 0, 0.0, Some(1));
        l.font = Font::for_cell_height(px, true);
        let lh = l.font.line_height() as f32;
        let mut f = Frame::blank(300, 200, PAGE);
        let still = place(
            &mut f,
            vec![label("CT1", (100.0, 60.0), 0, 0.0, Some(1))],
            &[],
            &none(),
        );
        let mut f = Frame::blank(300, 200, PAGE);
        let moved = place(&mut f, vec![l], &[], &o)[0];
        let centre = |r: Rect| (0.5 * (r[0] + r[2]), 0.5 * (r[1] + r[3]));
        let (cx, cy) = centre(moved.rect);
        assert!((cx - (100.0 + lh)).abs() < 1e-3, "{px}");
        assert!((cy - (60.0 - 0.5 * lh)).abs() < 1e-3, "{px}");
        assert_eq!(moved.anchor, still[0].anchor);
    }
}

#[test]
fn a_moved_label_is_clamped_inside_the_frame() {
    let (w, h) = (200.0, 100.0);
    // Which side it lands against: `rect[side] == at`.
    for (d, side, at) in [
        ((100.0, 0.0), 2, 200.0),
        ((-100.0, 0.0), 0, 0.0),
        ((0.0, 100.0), 3, 100.0),
        ((0.0, -100.0), 1, 0.0),
    ] {
        let mut o = none();
        o.nudge(1, d);
        let mut f = Frame::blank(w as usize, h as usize, PAGE);
        let placed = place(
            &mut f,
            vec![label("CT1", (100.0, 50.0), 0, 0.0, Some(1))],
            &[],
            &o,
        );
        let r = placed[0].rect;
        assert_eq!(r[side], at, "{d:?} {r:?}");
        assert!(
            r[0] >= 0.0 && r[1] >= 0.0 && r[2] <= w && r[3] <= h,
            "{d:?} {r:?}"
        );
    }
    // An unmoved label that would run off the frame is dropped instead.
    let mut f = Frame::blank(200, 100, PAGE);
    assert!(place(
        &mut f,
        vec![label("CT1", (1.0, 50.0), 0, 0.0, Some(1))],
        &[],
        &none()
    )
    .is_empty());
}

#[test]
fn a_leader_joins_a_label_moved_past_half_a_line() {
    let lh = font().line_height() as f32;
    let anchor = (100.0, 30.0);
    let column = |dy: f32| {
        let mut o = none();
        o.nudge(1, (0.0, dy));
        let mut f = Frame::blank(200, 150, PAGE);
        let placed = place(&mut f, vec![label("CT1", anchor, 0, 0.0, Some(1))], &[], &o);
        let top = placed[0].rect[1];
        let img = f.to_image();
        let page = img.get_pixel(0, 0).0;
        // Between the anchor and the label's box, a little clear of both.
        (anchor.1 as u32 + 1..top as u32 - 3)
            .filter(|&y| img.get_pixel(anchor.0 as u32, y).0 != page)
            .count()
    };
    // Three lines down: the gap is well past half a line, and joined.
    assert!(column(3.0) as f32 > lh, "{}", column(3.0));
    // Under a line down the box nearly touches the anchor: no leader.
    assert_eq!(column(0.9), 0);
}

#[test]
fn a_leader_to_an_anchor_far_off_the_frame_is_quick() {
    // Zoomed deep: the anchor is far off the frame, the label held inside.
    let mut o = none();
    o.nudge(1, (-1e7, 0.0));
    let mut f = Frame::blank(200, 100, PAGE);
    let t = std::time::Instant::now();
    let placed = place(
        &mut f,
        vec![label("CT1", (1e8, 50.0), 0, 0.0, Some(1))],
        &[],
        &o,
    );
    assert!(t.elapsed().as_secs_f32() < 1.0, "{:?}", t.elapsed());
    assert!(placed[0].moved);
    // The leader runs from the box to the frame's right edge.
    let img = f.to_image();
    let page = img.get_pixel(0, 0).0;
    assert_ne!(img.get_pixel(199, 50).0, page);
}

#[test]
fn hits_find_a_label_only_on_the_frame_it_was_placed_on() {
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 200, 100, 0.92, YAxis::Up);
    let mut f = Frame::blank(200, 100, PAGE);
    let mut o = none();
    // Both moved, apart but still overlapping: both are kept, and the
    // nearer centre wins.
    o.nudge(1, (-0.5, 0.0));
    o.nudge(2, (0.5, 0.0));
    let placed = place(
        &mut f,
        vec![
            label("CT1", (100.0, 50.0), 0, 0.0, Some(1)),
            label("CT2", (100.0, 50.0), 0, 0.0, Some(2)),
        ],
        &[],
        &o,
    );
    let mut hits = Hits::default();
    assert_eq!(hits.at(&vp, 100.0, 50.0), None);
    hits.record(vp, &placed);
    assert_eq!(hits.at(&vp, 98.0, 50.0), Some(1));
    assert_eq!(hits.at(&vp, 102.0, 50.0), Some(2));
    let lh = font().line_height() as f32;
    assert_eq!(hits.at(&vp, 100.0 + lh, 50.0), Some(2));
    assert_eq!(hits.at(&vp, 5.0, 5.0), None);
    assert_eq!(
        hits.rect(2),
        Some(placed.iter().find(|p| p.key == 2).unwrap().rect)
    );
    // Another camera's frame has no labels to click.
    let moved = Viewport {
        cx: vp.cx + 1.0,
        ..vp
    };
    assert_eq!(hits.at(&moved, 100.0, 50.0), None);
}

#[test]
fn text_scales_are_senna_s() {
    assert_eq!(TEXT_SCALES, [1.0, 1.4, 1.8, 2.4]);
}

#[test]
fn a_label_s_halo_is_the_page() {
    // The halo is drawn in the frame's own page colour.
    let mut f = Frame::blank(200, 100, [10, 20, 30]);
    f.keep_text();
    place(
        &mut f,
        vec![label("CT1", (100.0, 50.0), 0, 0.0, Some(1))],
        &[],
        &none(),
    );
    let runs = f.take_text();
    assert_eq!(runs[0].bg, to_linear([10, 20, 30]));
    assert!(runs[0].centred);
}
