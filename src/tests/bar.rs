//! A child's progress bar, followed through what it says: where it stands,
//! the scores it reports, and whether it is on the child's screen.

use super::*;

/// What a child says, frame by frame, into `bar`.
fn say(bar: &mut Option<Bar>, said: Said) {
    Bar::track(bar, &said);
}

fn frame(bar: &mut Option<Bar>, pos: u64, len: u64, what: &str) {
    say(bar, Said::Progress(Progress::new(pos, len, what)));
}

fn trend(bar: &Option<Bar>) -> Vec<f64> {
    bar.as_ref().map_or_else(Vec::new, |b| b.trend.clone())
}

#[test]
fn each_score_a_bar_says_is_one_point_of_its_trend() {
    // A bar that counts steps but scores once an epoch redraws each score
    // many times: the trend is the epochs, not the redraws.
    let mut bar = None;
    for (pos, what) in [
        (0, "epoch 1 loss=3"),
        (1, "epoch 1 loss=3"),
        // The next score is drawn before the step counts it.
        (1, "epoch 2 loss=2"),
        (2, "epoch 2 loss=2"),
        (3, "epoch 3 loss=1"),
    ] {
        frame(&mut bar, pos, 10, what);
    }
    assert_eq!(trend(&bar), vec![3., 2., 1.]);
    assert_eq!(bar.as_ref().unwrap().now.pos, 3);
}

#[test]
fn a_score_that_holds_is_a_point_each_time_it_is_said() {
    // A converged sweep moves nothing, sweep after sweep: a plateau, not
    // one point.
    let mut bar = None;
    for pos in 0..4 {
        frame(&mut bar, pos, 10, &format!("sweeps {}: moves=0", pos + 1));
    }
    assert_eq!(trend(&bar), vec![0.; 4]);
}

#[test]
fn a_new_bar_starts_a_new_trend() {
    let mut bar = None;
    frame(&mut bar, 0, 10, "sweeps 1: score=5");
    frame(&mut bar, 1, 10, "sweeps 2: score=6");
    // Another length: another bar, on another scale.
    frame(&mut bar, 0, 4, "epochs 1: loss=0.5");
    assert_eq!(trend(&bar), vec![0.5]);
    // Back to its start.
    frame(&mut bar, 1, 4, "epochs 2: loss=0.4");
    frame(&mut bar, 0, 4, "epochs 1: loss=0.9");
    assert_eq!(trend(&bar), vec![0.9]);
    // Same length, further on, but another bar by its name.
    frame(&mut bar, 2, 4, "greedy 1: moves=7");
    assert_eq!(trend(&bar), vec![7.]);
}

#[test]
fn a_trend_keeps_its_newest_points() {
    let mut bar = None;
    for pos in 0..(TREND as u64 + 5) {
        frame(&mut bar, pos, 1000, &format!("x={pos}"));
    }
    let t = trend(&bar);
    assert_eq!(t.len(), TREND);
    assert_eq!(t[0], 5.);
}

#[test]
fn a_log_line_hides_the_bar_until_it_is_drawn_again() {
    // A bar is cleared to print a line above it and redrawn at once; a bar
    // finished and cleared is not drawn again. Only the order tells them
    // apart, so a line hides the bar and its next frame shows it.
    let mut bar = None;
    say(&mut bar, Said::Line("reading".into()));
    assert!(bar.is_none(), "a line before any bar is only a line");
    frame(&mut bar, 1, 10, "sweeps 1: moves=9");
    assert!(bar.as_ref().unwrap().shown);
    say(&mut bar, Said::Line("level 1 done".into()));
    assert!(!bar.as_ref().unwrap().shown);
    frame(&mut bar, 2, 10, "sweeps 2: moves=4");
    let b = bar.as_ref().unwrap();
    assert!(b.shown);
    assert_eq!(b.trend, vec![9., 4.], "hidden, the bar keeps its trend");
}

#[test]
fn a_trend_is_drawn_over_its_own_range() {
    assert_eq!(spark(&[1.0], 9), "");
    assert_eq!(spark(&[3.0, 3.0], 9), "▁▁");
    assert_eq!(spark(&[0.0, 0.5, 1.0], 9), "▁▅█");
    let long: Vec<f64> = (0..100).map(f64::from).collect();
    assert_eq!(spark(&long, 10).chars().count(), 10);
    assert_eq!(spark(&long, 500).chars().count(), TREND);
    assert_eq!(spark(&long, 1), "");
}

#[test]
fn a_gauge_shows_the_bar_and_its_trend_in_the_room_it_has() {
    let p = Progress::new(5, 10, "sweeps score=1");
    let g = gauge(&p, 10, &[3., 2., 1.], 80);
    assert!(g.starts_with("█████░░░░░ 5/10 sweeps score=1"), "{g}");
    assert!(g.ends_with("█▅▁"), "{g}");
    // No room left: the bar alone.
    assert!(!gauge(&p, 10, &[3., 2., 1.], 20).contains('▁'));
    // A spinner says what it does.
    assert_eq!(
        gauge(&Progress::new(0, 0, "reading"), 10, &[], 80),
        "… reading"
    );
}

#[test]
fn what_a_message_says_after_its_score_is_not_a_new_point() {
    // A rate that changes every redraw, after the score: one point.
    let mut bar = None;
    frame(&mut bar, 1, 10, "epoch 1 loss=2 it/s=13");
    frame(&mut bar, 2, 10, "epoch 1 loss=2 it/s=14");
    frame(&mut bar, 3, 10, "epoch 2 loss=1 it/s=12");
    assert_eq!(trend(&bar), vec![2., 1.]);
}

#[test]
fn a_bar_s_name_keeps_the_words_that_are_not_numbers() {
    // Two phases of one length are two bars.
    let mut bar = None;
    frame(&mut bar, 1, 10, "phase=init 1: loss=5");
    frame(&mut bar, 2, 10, "phase=fit 1: loss=1");
    assert_eq!(trend(&bar), vec![1.]);
}

#[test]
fn a_spinner_draws_its_trend_too() {
    let g = gauge(&Progress::new(0, 0, "fitting loss=1"), 10, &[3., 1.], 80);
    assert_eq!(g, "… fitting loss=1  █▁");
}

#[test]
fn a_gauge_keeps_to_its_room() {
    use ratatui::text::Span;
    // A long message is cut, wide characters counted by the cells they take.
    let p = Progress::new(1, 2, "読み込み中のファイルがとても多いです loss=1");
    for room in [20, 30, 45] {
        let g = gauge(&p, 10, &[2., 1.], room);
        assert!(Span::raw(g.as_str()).width() <= room, "{g:?} in {room}");
    }
    // A count past what a u64 product holds draws, it does not overflow.
    let huge = Progress::new(u64::MAX / 2, u64::MAX, "bytes");
    assert!(gauge(&huge, 80, &[], 200).starts_with(&"█".repeat(39)));
}
