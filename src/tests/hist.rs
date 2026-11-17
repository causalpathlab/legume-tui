//! Histograms: bins on every scale, the shared axis, and the two plots'
//! bars, ticks and labels.

use super::*;

#[test]
fn lower_edges_open_their_bin_on_every_scale() {
    for scale in [Scale::Log, Scale::Sqrt, Scale::Linear] {
        let b = Binning::new(scale, 5000.0, true);
        for k in 1..60 {
            let e = b.lower_edge(k);
            assert!(b.key(e as f64) >= k, "{scale:?} k={k} e={e}");
            assert!(e == 0 || b.key((e - 1) as f64) < k, "{scale:?} k={k} e={e}");
        }
    }
}

#[test]
fn continuous_bins_cover_the_range() {
    for scale in [Scale::Log, Scale::Sqrt, Scale::Linear] {
        let vals = [0.0f32, 0.5, 3.2, 18.0];
        let binned = Binned::new(&vals, scale);
        assert!(binned.counts.len() > 1, "{scale:?}");
        assert_eq!(binned.counts.iter().sum::<usize>(), vals.len(), "{scale:?}");
        let subset = binned.count([0.5f32, 18.0].into_iter());
        assert_eq!(subset.len(), binned.counts.len(), "{scale:?}");
        assert_eq!(subset.iter().sum::<usize>(), 2, "{scale:?}");
    }
}

/// The bins are made of finite values; a subset counted in them may hold
/// values that are not finite, which take no bin.
#[test]
fn counted_values_that_are_not_finite_take_no_bin() {
    for scale in [Scale::Log, Scale::Sqrt, Scale::Linear] {
        let binned = Binned::new(&[0.0f32, 0.5, 3.2, 18.0], scale);
        let clean = binned.count([0.5f32, 18.0].into_iter());
        let odd = [f32::NEG_INFINITY, 0.5, f32::NAN, 18.0, f32::INFINITY];
        assert_eq!(binned.count(odd.into_iter()), clean, "{scale:?}");
        assert_eq!(clean.iter().sum::<usize>(), 2, "{scale:?}");
    }
}

#[test]
fn compact_labels() {
    assert_eq!(compact(0.0), "0");
    assert_eq!(compact(0.25), "0.25");
    assert_eq!(compact(950.0), "950");
    assert_eq!(compact(1234.0), "1.2k");
    assert_eq!(compact(35_000.0), "35k");
    assert_eq!(compact(1_100_000.0), "1.1M");
}

#[test]
fn log_bins_read_values_on_the_shared_axis() {
    // Means mostly below 1 spread over the bins, not into the first few.
    let means: Vec<f32> = (1..=400).map(|i| 0.001 * 1.02f32.powi(i)).collect();
    let binned = Binned::new(&means, Scale::Log);
    let filled = binned.counts.iter().filter(|&&c| c > 0).count();
    assert!(filled >= 40, "{filled} of {} bins", binned.counts.len());
    // Ticks at round values.
    let ticks = binned.bins.ticks(binned.kmin, binned.kmax());
    assert!(ticks.iter().any(|&(_, v)| v == 0.01), "{ticks:?}");
    let (kmin, kmax) = (binned.kmin, binned.kmax());
    assert!(
        ticks.iter().all(|&(k, _)| (kmin..=kmax + 1).contains(&k)),
        "{ticks:?}"
    );
}

#[test]
fn whole_number_data_gets_whole_count_bins() {
    let whole = Binned::new(&[0.0, 3.0, 7.0, 400.0], Scale::Linear);
    let edges: Vec<usize> = (0..5).map(|k| whole.bins.lower_edge(k)).collect();
    assert!(edges.windows(2).all(|w| w[1] > w[0]), "{edges:?}");
    let fractional = Binned::new(&[0.0, 0.25, 1.5], Scale::Linear);
    assert!(fractional.counts.len() > 2);
}

#[test]
fn unit_width_bins_are_one_bar_per_index() {
    let b = Binning::with_width(Scale::Linear, 1.0);
    for k in 0..500 {
        assert_eq!(b.key(k as f64), k);
    }
}

fn axis_rows(plot: &HistPlot, width: u16, height: u16) -> (String, String) {
    let area = Rect::new(0, 0, width, height);
    let mut buf = Buffer::empty(area);
    plot.render(&mut buf, area);
    let row = |y: u16| -> String { (0..width).map(|x| buf[(x, y)].symbol()).collect() };
    (row(height - 2), row(height - 1))
}

#[test]
fn custom_labels_and_tick_spacing() {
    let counts = vec![3usize; 12];
    let label = |k: i32| (k % 4 == 0).then(|| format!("L{k}"));
    let plot = HistPlot {
        bins: Binning::with_width(Scale::Linear, 1.0),
        kmin: 0,
        counts: &counts,
        style: &|_| Style::new(),
        subset: None,
        y_scale: Scale::Linear,
        y_max: None,
        pointer: None,
        marks: Vec::new(),
        x_label: Some(&label),
        tick_every: Some(2),
        look: Look::default(),
    };
    let (axis, labels) = axis_rows(&plot, 6 + 24, 8);
    // Ticks every 2 bins, kept only where a label is given.
    assert_eq!(axis.matches('┴').count(), 3, "{axis}");
    assert!(labels.contains("L0") && labels.contains("L4") && labels.contains("L8"));
    assert!(!labels.contains("L2") && !labels.contains("L6"), "{labels}");
}

#[test]
fn default_labels_are_unchanged() {
    let counts = vec![1usize; 10];
    let base = |x_label, tick_every| HistPlot {
        bins: Binning::with_width(Scale::Linear, 1.0),
        kmin: 0,
        counts: &counts,
        style: &|_| Style::new(),
        subset: None,
        y_scale: Scale::Linear,
        y_max: None,
        pointer: None,
        marks: Vec::new(),
        x_label,
        tick_every,
        look: Look::default(),
    };
    let (_, labels) = axis_rows(&base(None, None), 26, 8);
    assert!(labels.trim_start().starts_with('0'), "{labels}");
}

/// The bar cells of a plot of `counts`, with no x labels.
fn bar_cells<T: BarValue>(counts: &[T]) -> String {
    let area = Rect::new(0, 0, 6 + counts.len() as u16 * 2, 10);
    let mut buf = Buffer::empty(area);
    HistPlot {
        bins: Binning::with_width(Scale::Linear, 1.0),
        kmin: 0,
        counts,
        style: &|_| Style::new(),
        subset: None,
        y_scale: Scale::Linear,
        y_max: None,
        pointer: None,
        marks: Vec::new(),
        x_label: Some(&|_| None),
        tick_every: None,
        look: Look::default(),
    }
    .render(&mut buf, area);
    (0..area.height - 2)
        .flat_map(|y| (6..area.width).map(move |x| (x, y)))
        .map(|(x, y)| buf[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn real_valued_bars_draw_like_counts() {
    // Same shapes; only the y labels in the gutter differ.
    assert_eq!(
        bar_cells(&[0usize, 2, 4, 8]),
        bar_cells(&[0.0f64, 0.2, 0.4, 0.8])
    );
}

#[test]
fn a_fixed_y_max_shares_the_scale() {
    // The same bars under a higher ceiling draw shorter.
    let draw = |y_max| {
        let counts = [2usize, 4];
        let area = Rect::new(0, 0, 6 + 4, 12);
        let mut buf = Buffer::empty(area);
        HistPlot {
            bins: Binning::with_width(Scale::Linear, 1.0),
            kmin: 0,
            counts: &counts,
            style: &|_| Style::new(),
            subset: None,
            y_scale: Scale::Linear,
            y_max,
            pointer: None,
            marks: Vec::new(),
            x_label: Some(&|_| None),
            tick_every: None,
            look: Look::default(),
        }
        .render(&mut buf, area);
        (0..10).filter(|&y| buf[(8, y)].symbol() == "█").count()
    };
    assert!(draw(Some(8.0)) < draw(None));
    assert_eq!(draw(Some(1.0)), draw(None), "never below the tallest bar");
}

fn draw_mirror(plot: &MirrorPlot<'_, f64>, w: u16, h: u16) -> Vec<String> {
    let area = Rect::new(0, 0, w, h);
    let mut buf = Buffer::empty(area);
    plot.render(&mut buf, area);
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn a_mirror_grows_one_side_up_and_the_other_down() {
    let (up, down) = ([4.0, 0.0, 2.0], [0.0, 4.0, 1.0]);
    let side = |counts| MirrorSide {
        counts,
        subset: None,
        style: Style::new(),
        name: "",
    };
    let label = |i: usize| (i == 0).then(|| "A".to_string());
    let plot = MirrorPlot {
        up: side(&up),
        down: side(&down),
        y_scale: Scale::Linear,
        y_max: None,
        y_labels: None,
        pointer: Some(1),
        x_label: Some(&label),
        look: Look::default(),
    };
    // 7 chart rows: 3 above the zero line, the line, 3 below.
    let rows = draw_mirror(&plot, 10, 9);
    let col = |x: usize| -> String {
        rows.iter()
            .take(7)
            .map(|r| r.chars().nth(x).unwrap())
            .collect()
    };
    assert_eq!(col(GUTTER as usize), "███─   ", "tallest up fills its side");
    assert_eq!(
        col(GUTTER as usize + 1),
        "┊┊┊─███",
        "down, under the pointer"
    );
    assert_eq!(col(GUTTER as usize + 2), " ▄█─█  ", "halves round to cells");
    assert!(rows[0].trim_start().starts_with('4'), "{rows:?}");
    assert!(rows[6].trim_start().starts_with('4'), "{rows:?}");
    assert!(rows[8].contains('A'));
    assert!(rows[7].contains('▲'));
}

#[test]
fn linear_ticks_of_a_range_with_no_span_end() {
    assert_eq!(Axis::linear(0.0, 0.0).ticks(), vec![0.0]);
    assert_eq!(Axis::linear(3.0, 3.0).ticks(), vec![3.0]);
    assert!(Axis::linear(1.0, 0.0).ticks().is_empty());
    assert!(Axis::linear(0.0, f64::INFINITY).ticks().is_empty());
    assert!(Axis::linear(0.0, f64::NAN).ticks().is_empty());
    assert!(Axis::linear(0.0, 5e-324).ticks().is_empty());
    assert_eq!(
        Axis::linear(0.0, 1.0).ticks(),
        vec![0.0, 0.2, 0.4, 0.6, 0.8, 1.0]
    );
}

#[test]
fn a_plain_axis_has_no_ticks() {
    let counts = vec![3usize; 12];
    let label = |k: i32| (k % 4 == 0).then(|| format!("L{k}"));
    let area = Rect::new(0, 0, 30, 8);
    let mut buf = Buffer::empty(area);
    HistPlot {
        bins: Binning::with_width(Scale::Linear, 1.0),
        kmin: 0,
        counts: &counts,
        style: &|_| Style::new(),
        subset: None,
        y_scale: Scale::Linear,
        y_max: None,
        pointer: None,
        marks: Vec::new(),
        x_label: Some(&label),
        tick_every: None,
        look: Look::default(),
    }
    .render(&mut buf, area);
    plain_axis(&mut buf, area);
    let axis: String = (0..30).map(|x| buf[(x, 6)].symbol()).collect();
    assert!(!axis.contains('┴') && axis.contains('─'), "{axis}");
}

#[test]
fn medians_of_odd_even_and_empty() {
    assert_eq!(median(&[]), 0.0);
    assert_eq!(median(&[1.0, 2.0, 9.0]), 2.0);
    assert_eq!(median(&[1.0, 2.0, 4.0, 9.0]), 3.0);
}

#[test]
fn negative_data_gets_bins_sized_to_its_range() {
    for scale in [Scale::Log, Scale::Sqrt, Scale::Linear] {
        for vals in [
            &[-1e6f32, 1.0][..],
            &[-999_999.5, 1.0],
            &[-0.5, 0.0],
            &[-0.5, -0.25],
            &[-3.0, 2.0],
            &[0.0],
            &[0.0, 0.0],
        ] {
            let binned = Binned::new(vals, scale);
            let n = binned.counts.len();
            assert!(
                (1..=MAX_BINS as usize).contains(&n),
                "{scale:?} {vals:?}: {n}"
            );
            assert_eq!(binned.counts.iter().sum::<usize>(), vals.len());
        }
    }
    let wide = Binned::new(&[-1e6, 1.0], Scale::Linear);
    assert!(wide.counts.len() <= 52, "{}", wide.counts.len());
    let small = Binned::new(&[-0.5, 0.0], Scale::Linear);
    assert!(
        (40..=52).contains(&small.counts.len()),
        "{}",
        small.counts.len()
    );
}

#[test]
fn non_negative_data_bins_as_from_zero() {
    for scale in [Scale::Log, Scale::Sqrt, Scale::Linear] {
        let binned = Binned::new(&[0.5f32, 3.2, 18.0], scale);
        let shift = Axis::fit(&[0.5, 3.2, 18.0], true).shift;
        let mut from_zero = Binning::new(scale, 18.0, false);
        from_zero.shift = shift;
        let want = from_zero.t(18.0) / TARGET_BINS;
        assert_eq!(binned.bins.width, want, "{scale:?}");
    }
}

#[test]
fn a_subset_on_other_bins_draws_without_panicking() {
    let draw_hist = |counts: &[usize], subset: &[usize]| {
        let area = Rect::new(0, 0, 30, 8);
        let mut buf = Buffer::empty(area);
        HistPlot {
            bins: Binning::with_width(Scale::Linear, 1.0),
            kmin: 0,
            counts,
            style: &|_| Style::new(),
            subset: Some(subset),
            y_scale: Scale::Linear,
            y_max: None,
            pointer: None,
            marks: Vec::new(),
            x_label: None,
            tick_every: None,
            look: Look::default(),
        }
        .render(&mut buf, area);
    };
    draw_hist(&[3, 1], &[1, 2, 4, 5]);
    draw_hist(&[3, 1, 2], &[1]);

    let (counts, subset) = ([2.0, 1.0], [1.0, 2.0, 3.0]);
    let side = || MirrorSide {
        counts: &counts,
        subset: Some(&subset),
        style: Style::new(),
        name: "",
    };
    let plot = MirrorPlot {
        up: side(),
        down: side(),
        y_scale: Scale::Linear,
        y_max: None,
        y_labels: None,
        pointer: None,
        x_label: None,
        look: Look::default(),
    };
    let rows = draw_mirror(&plot, 12, 9);
    assert!(rows[3].contains('─'), "{rows:?}");
}
