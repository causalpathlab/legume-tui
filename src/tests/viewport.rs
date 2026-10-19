//! The viewport between data and pixels: fitted, zoomed about the pointer
//! and panned with it, y up or down the screen, within its limits.

use super::*;

const SQUARE: Bounds = [0.0, 0.0, 10.0, 10.0];
const WIDE: Bounds = [0.0, -5.0, 10.0, 5.0];
const BOTH: [YAxis; 2] = [YAxis::Up, YAxis::Down];

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-3
}

fn near(a: [f32; 2], b: [f32; 2]) -> bool {
    close(a[0], b[0]) && close(a[1], b[1])
}

fn width(v: &Viewport) -> f32 {
    let [x0, _, x1, _] = v.window();
    x1 - x0
}

#[test]
fn fit_centres_the_bounds_with_a_margin() {
    for y in BOTH {
        let v = Viewport::fit(WIDE, 200, 100, 0.92, y);
        assert_eq!((v.cx, v.cy, v.w, v.h, v.y), (5.0, 0.0, 200, 100, y));
        assert!(close(v.scale, 0.92 * 10.0));
        assert_eq!(v.to_px([5.0, 0.0]), (100.0, 50.0));
        // Without a margin the tighter axis is filled exactly (pinto's).
        let v = Viewport::fit(WIDE, 200, 100, 1.0, y);
        let [_, y0, _, y1] = v.window();
        assert!(close(y0, -5.0) && close(y1, 5.0), "{y:?}");
    }
    // An empty extent still gives a finite scale.
    assert!(Viewport::fit([1.0, 1.0, 1.0, 1.0], 10, 10, 1.0, YAxis::Up)
        .scale
        .is_finite());
}

#[test]
fn y_runs_up_or_down_the_screen() {
    let up = Viewport::fit(WIDE, 200, 100, 1.0, YAxis::Up);
    let down = Viewport::fit(WIDE, 200, 100, 1.0, YAxis::Down);
    let (ux, uy) = up.to_px([7.0, 4.0]);
    let (dx, dy) = down.to_px([7.0, 4.0]);
    assert_eq!(ux, dx, "x is the same either way");
    assert!(uy < 50.0, "up: higher data is higher on screen");
    assert!(dy > 50.0, "down: higher data is lower on screen");
    assert!(close(uy + dy, 100.0));
}

#[test]
fn to_data_inverts_to_px() {
    for y in BOTH {
        let v = Viewport::fit(WIDE, 300, 120, 0.92, y);
        for p in [[0.0, 0.0], [10.0, -5.0], [3.5, 4.25], [-20.0, 7.0]] {
            let (px, py) = v.to_px(p);
            assert!(near(v.to_data(px, py), p), "{y:?} {p:?}");
        }
    }
}

#[test]
fn zooming_keeps_the_point_under_the_pointer() {
    for y in BOTH {
        let mut v = Viewport::fit(WIDE, 200, 100, 0.92, y);
        let before = v.to_data(30.0, 70.0);
        let scale = v.scale;
        v.zoom_at(2.0, 30.0, 70.0, &Limits::default());
        assert!(close(v.scale, 2.0 * scale));
        assert!(near(v.to_data(30.0, 70.0), before), "{y:?}");
    }
}

#[test]
fn a_factor_above_one_always_zooms_in() {
    for y in BOTH {
        for f in [1.25, 2.0, 10.0] {
            let mut v = Viewport::fit(WIDE, 200, 100, 1.0, y);
            let shown = width(&v);
            v.zoom_at(f, 10.0, 90.0, &Limits::default());
            assert!(width(&v) < shown, "{y:?} {f}");
            assert!(close(width(&v), shown / f));
            v.zoom_at(1.0 / f, 10.0, 90.0, &Limits::default());
            assert!(close(width(&v), shown), "{y:?} {f}: and back out");
        }
    }
}

#[test]
fn panning_moves_the_picture_with_the_pointer() {
    for y in BOTH {
        let mut v = Viewport::fit(WIDE, 200, 100, 0.92, y);
        let p = [6.0, 2.0];
        let (x0, y0) = v.to_px(p);
        v.pan_px(7.0, -3.0, &Limits::default());
        let (x1, y1) = v.to_px(p);
        assert!(close(x1, x0 + 7.0) && close(y1, y0 - 3.0), "{y:?}");
    }
}

#[test]
fn scale_limits_hold_and_the_anchor_stays() {
    let lim = Limits {
        scale: Some((5.0, 20.0)),
        keep_inside: None,
    };
    for y in BOTH {
        let mut v = Viewport::fit(SQUARE, 100, 100, 1.0, y);
        let before = v.to_data(20.0, 30.0);
        v.zoom_at(100.0, 20.0, 30.0, &lim);
        assert_eq!(v.scale, 20.0);
        assert!(near(v.to_data(20.0, 30.0), before), "{y:?}");
        v.zoom_at(0.001, 20.0, 30.0, &lim);
        assert_eq!(v.scale, 5.0);
        assert!(near(v.to_data(20.0, 30.0), before), "{y:?}");
    }
}

#[test]
fn keep_inside_stops_panning_at_the_edges() {
    let lim = Limits {
        scale: None,
        keep_inside: Some(SQUARE),
    };
    for y in BOTH {
        let whole = Viewport::fit(SQUARE, 100, 100, 1.0, y);
        let mut v = whole;
        v.pan_px(40.0, 0.0, &lim);
        assert_eq!(v, whole, "{y:?}: the whole does not pan");
        v.zoom_at(2.0, 50.0, 50.0, &lim);
        for _ in 0..50 {
            v.pan_px(-20.0, -20.0, &lim);
        }
        let [x0, y0, x1, y1] = v.window();
        assert!(
            close(x1, 10.0) && close(x0, 5.0),
            "{y:?}: stops at the right edge"
        );
        assert!(
            y0 >= -1e-3 && y1 <= 10.0 + 1e-3,
            "{y:?}: and inside top and bottom"
        );
        assert!(close(y1 - y0, 5.0));
        // Zoomed out past the whole, the whole sits in the middle.
        v.zoom_at(0.25, 0.0, 0.0, &lim);
        assert!(close(v.cx, 5.0) && close(v.cy, 5.0), "{y:?}");
    }
}

#[test]
fn down_zooms_as_pinto_s_camera_did() {
    // pinto kept a world corner and units per pixel; its `zoomed(f)`
    // zoomed out for f > 1, so its call sites now pass 1 / f.
    let v = Viewport::fit(WIDE, 160, 90, 1.0, YAxis::Down);
    let [x0, y0, ..] = v.window();
    let (upp, f, (ax, ay)) = (v.upp(), 0.5, (40.0, 20.0));
    let (wx, wy) = (x0 + ax * upp, y0 + ay * upp);
    let pinto = [wx - (wx - x0) * f, wy - (wy - y0) * f, upp * f];
    let mut ours = v;
    ours.zoom_at(1.0 / f, ax, ay, &Limits::default());
    let [ox0, oy0, ..] = ours.window();
    assert!(close(ox0, pinto[0]) && close(oy0, pinto[1]));
    assert!(close(ours.upp(), pinto[2]));
    // And its pixels: (world - corner) / upp.
    let (px, py) = ours.to_px([3.0, 1.0]);
    assert!(close(px, (3.0 - ox0) / ours.upp()) && close(py, (1.0 - oy0) / ours.upp()));
}

#[test]
fn units_per_pixel_and_the_window() {
    let v = Viewport::fit(SQUARE, 100, 50, 1.0, YAxis::Up);
    assert!(close(v.upp(), 1.0 / v.scale));
    let [x0, y0, x1, y1] = v.window();
    assert!(close(y0, 0.0) && close(y1, 10.0));
    assert!(close(x1 - x0, 20.0) && close(0.5 * (x0 + x1), 5.0));
}

#[test]
fn resized_keeps_the_centre_and_what_fits() {
    let mut v = Viewport::fit(SQUARE, 200, 100, 0.92, YAxis::Up);
    v.pan_px(13.0, 4.0, &Limits::default());
    let wider = v.resized(SQUARE, 400, 100);
    assert_eq!((wider.cx, wider.cy, wider.scale), (v.cx, v.cy, v.scale));
    assert_eq!((wider.w, wider.h), (400, 100));
    let larger = v.resized(SQUARE, 400, 200);
    assert!(close(larger.scale, 2.0 * v.scale));
    // A view never drawn is fitted afresh.
    let empty = Viewport { w: 0, h: 0, ..v };
    assert_eq!(
        empty.resized(SQUARE, 50, 50),
        Viewport::fit(SQUARE, 50, 50, 0.92, YAxis::Up)
    );
}

#[test]
fn scale_limits_given_either_way_round_hold_and_never_panic() {
    let lim = |a: f32, b: f32| Limits {
        scale: Some((a, b)),
        keep_inside: None,
    };
    for (a, b) in [(2.0, 8.0), (8.0, 2.0)] {
        let mut vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 100, 100, 1.0, YAxis::Up);
        vp.zoom_at(100.0, 50.0, 50.0, &lim(a, b));
        assert_eq!(vp.scale, 8.0, "({a}, {b})");
        vp.zoom_at(0.001, 50.0, 50.0, &lim(a, b));
        assert_eq!(vp.scale, 2.0, "({a}, {b})");
    }
    // Limits that are not numbers are no limits.
    let mut vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 100, 100, 1.0, YAxis::Up);
    let before = vp.scale;
    vp.zoom_at(2.0, 50.0, 50.0, &lim(f32::NAN, f32::NAN));
    assert_eq!(vp.scale, before * 2.0);
}
