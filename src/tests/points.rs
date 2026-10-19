//! Points: draw order, radius, the progressive job and group medians, as
//! senna drew them.

use super::*;
use crate::map::colour::to_linear;
use crate::map::raster::{stamp, Shape};
use crate::map::viewport::YAxis;

const PAGE: Srgb = [246, 245, 242];

struct Layer {
    xy: Vec<[f32; 2]>,
    shuffled: Vec<u32>,
    order: Arc<Vec<u32>>,
    marks: Vec<Option<Mark>>,
    size: f32,
}

impl PointLayer for Layer {
    fn xy(&self, i: usize) -> [f32; 2] {
        self.xy[i]
    }
    fn shuffled(&self) -> &[u32] {
        &self.shuffled
    }
    fn order(&self) -> Arc<Vec<u32>> {
        self.order.clone()
    }
    fn mark(&self, i: usize) -> Option<Mark> {
        self.marks[i]
    }
    fn size(&self) -> f32 {
        self.size
    }
}

fn plain(colour: Lin) -> Mark {
    Mark {
        colour,
        alpha: 1.0,
        shape: Shape::Circle,
        size: 1.0,
    }
}

/// `n` points on a grid over `[0, 1)²`, shuffled by a fixed stride.
fn grid(n: usize) -> Layer {
    let side = (n as f32).sqrt().ceil() as usize;
    let xy = (0..n)
        .map(|i| {
            [
                (i % side) as f32 / side as f32,
                (i / side) as f32 / side as f32,
            ]
        })
        .collect();
    let shuffled: Vec<u32> = (0..n).map(|i| ((i * 7919) % n) as u32).collect();
    let marks = (0..n)
        .map(|i| Some(plain([(i % 3) as f32 / 3.0, 0.2, 0.7])))
        .collect();
    Layer {
        xy,
        order: Arc::new(shuffled.clone()),
        shuffled,
        marks,
        size: 1.0,
    }
}

#[test]
fn the_merge_cursor_group_is_drawn_last() {
    // senna: group 0 chosen, the merge cursor on group 2 (not chosen).
    let groups = [0, 1, 2, 0, 1, 2];
    let (selected, cursor) = ([true, false, false], 2);
    let rank = |i: usize| {
        let g = groups[i];
        if g == cursor {
            3
        } else if selected[g] {
            2
        } else {
            0
        }
    };
    let order = ranked(&[0, 1, 2, 3, 4, 5], rank, 4);
    let last: Vec<usize> = order[4..].iter().map(|&i| groups[i as usize]).collect();
    assert_eq!(last, [2, 2]);
    assert_eq!(order, [1, 4, 0, 3, 2, 5]);
}

#[test]
fn a_rank_keeps_the_shuffled_order_within_it() {
    let shuffled = [5, 3, 0, 4, 1, 2];
    assert_eq!(ranked(&shuffled, |i| (i % 2) as u8, 2), [0, 4, 2, 5, 3, 1]);
    // Activity: low to high, ties in shuffled order.
    let t = [0.5, 0.1, 0.5, -1.0, 0.9, 0.1];
    assert_eq!(by_level(&shuffled, |i| t[i]), [3, 5, 1, 0, 2, 4]);
}

#[test]
fn many_equal_levels_keep_the_shuffled_order() {
    // Enough points that an unstable sort would reorder the ties.
    let n = 500u32;
    let shuffled: Vec<u32> = (0..n).map(|i| (i * 211 + 7) % n).collect();
    let t: Vec<f32> = (0..n).map(|i| (i % 3) as f32).collect();
    let order = by_level(&shuffled, |i| t[i]);
    let want: Vec<u32> = (0..3)
        .flat_map(|l| shuffled.iter().copied().filter(move |&i| i % 3 == l))
        .collect();
    assert_eq!(order, want);
}

#[test]
fn the_radius_shrinks_with_crowding_within_bounds() {
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 100, 100, 0.92, YAxis::Up);
    assert_eq!(auto_radius(0, &vp), 2.4);
    assert_eq!(auto_radius(10_000_000, &vp), 0.6);
    assert!(auto_radius(2000, &vp) < auto_radius(200, &vp));
}

#[test]
fn visible_points_are_counted_from_a_prefix() {
    let l = grid(10_000);
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 100, 100, 0.92, YAxis::Up);
    assert_eq!(visible_count(&l, &vp), 10_000);
    // The left half of the data only.
    let half = Viewport { cx: 0.0, ..vp };
    let n = visible_count(&l, &half);
    assert!((4_000..6_000).contains(&n), "{n}");
    let empty = Layer {
        shuffled: Vec::new(),
        ..grid(1)
    };
    assert_eq!(visible_count(&empty, &vp), 0);
}

#[test]
fn a_job_fills_in_by_doubling_chunks_and_ends_as_drawn_at_once() {
    let n = 100_000;
    let l = grid(n);
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 64, 48, 0.92, YAxis::Up);
    let layers = [l];
    let mut job = Job::new(vp, PAGE, &layers);
    // The first chunk does not finish a hundred thousand points.
    assert!(!job.step(&layers));
    let early = job.image();
    let mut steps = 1;
    while !job.step(&layers) {
        steps += 1;
    }
    assert_eq!(steps, 2);
    assert_ne!(early, job.image());
    let done = job.finish();

    // The same marks stamped one by one, in order, at the job's radius.
    let mut want = Frame::blank(64, 48, PAGE);
    let r = auto_radius(visible_count(&layers[0], &vp), &vp);
    for &i in layers[0].order.iter() {
        let (x, y) = vp.to_px(layers[0].xy[i as usize]);
        let m = layers[0].marks[i as usize].unwrap();
        stamp(&mut want, x, y, r * m.size, m.shape, m.colour, m.alpha);
    }
    assert_eq!(done.to_image(), want.to_image());
    assert_eq!(done.background(), to_linear(PAGE));
}

#[test]
fn hidden_points_and_layers_past_the_first_are_drawn_in_turn() {
    let red = [1.0, 0.0, 0.0];
    let blue = [0.0, 0.0, 1.0];
    let one = |colour: Lin, hidden: bool, size: f32| Layer {
        xy: vec![[0.5, 0.5]],
        shuffled: vec![0],
        order: Arc::new(vec![0]),
        marks: vec![(!hidden).then(|| plain(colour))],
        size,
    };
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 20, 20, 0.92, YAxis::Up);
    let centre = |layers: &[Layer]| {
        let mut job = Job::new(vp, PAGE, layers);
        while !job.step(layers) {}
        job.finish().to_image().get_pixel(10, 10).0
    };
    // The later layer on top.
    assert_eq!(
        centre(&[one(red, false, 1.0), one(blue, false, 1.0)]),
        [0, 0, 255, 255]
    );
    // A hidden mark is not drawn.
    assert_eq!(
        centre(&[one(red, false, 1.0), one(blue, true, 1.0)]),
        [255, 0, 0, 255]
    );
    // A layer's size scales its radius.
    let small = Job::new(vp, PAGE, &[one(red, false, 1.0)]);
    let big = Job::new(vp, PAGE, &[one(red, false, 2.0)]);
    assert_eq!(big.radius(0), 2.0 * small.radius(0));
}

#[test]
fn points_far_off_the_frame_are_skipped() {
    let l = Layer {
        xy: vec![[100.0, 100.0]],
        shuffled: vec![0],
        order: Arc::new(vec![0]),
        marks: vec![Some(plain([0.0; 3]))],
        size: 1.0,
    };
    let vp = Viewport::fit([0.0, 0.0, 1.0, 1.0], 20, 20, 0.92, YAxis::Up);
    let layers = [l];
    let mut job = Job::new(vp, PAGE, &layers);
    assert!(job.step(&layers));
    assert_eq!(
        job.finish().to_image(),
        Frame::blank(20, 20, PAGE).to_image()
    );
}

#[test]
fn group_medians_come_from_the_shuffled_prefix() {
    let xy = [[0.0, 0.0], [1.0, 5.0], [2.0, 1.0], [9.0, 9.0], [4.0, 4.0]];
    let group = [Some(0), Some(0), Some(0), None, Some(2)];
    let m = group_medians(|i| xy[i], &[4, 3, 2, 1, 0], |i| group[i], 3);
    assert_eq!(m, vec![Some(([1.0, 1.0], 3)), None, Some(([4.0, 4.0], 1))]);
    // Ids past n are no group.
    let m = group_medians(|i| xy[i], &[0, 1], |_| Some(7), 1);
    assert_eq!(m, vec![None]);
}
