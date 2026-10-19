//! Heatmaps: the top rows by margin (senna's and pinto's tests), z-scores,
//! and senna's drawing. The pinned numbers of the drawing are what senna's
//! own `draw_heatmap` (`src/view/chart.rs`, compiled apart from this crate)
//! gives for the same heatmap.

use super::*;
use crate::map::colour::{diverging, to_linear, Srgb};
use crate::map::raster::Frame;
use crate::map::text::Font;
use crate::map::theme::Theme;

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

#[test]
fn a_feature_scores_by_its_margin_over_the_next_group() {
    // senna: two features × three groups.
    let v = [1.0, 4.0, 2.0, 5.0, 5.0, f32::NAN];
    let m = Matrix {
        rows: 2,
        cols: 3,
        v: &v,
    };
    // Peaks in group 1, 2 over group 2.
    assert_eq!(peak_margin(&m, 0), Some((1, 2.0)));
    // A tie peaks nowhere.
    assert_eq!(peak_margin(&m, 1), None);
}

#[test]
fn each_group_takes_its_own_top_features() {
    // senna's `pick_top` case as levels: features 0 and 1 peak in group 0
    // (by 3 and 2), 3 and 2 in group 1 (by 4 and 2.5); a missing level
    // never peaks.
    let v = [3.0, 0.0, 2.0, 0.0, 0.0, 2.5, 0.0, 4.0, f32::NAN, f32::NAN];
    let m = Matrix {
        rows: 5,
        cols: 2,
        v: &v,
    };
    assert_eq!(by_margin(&m, 2), [(0, 0), (1, 0), (3, 1), (2, 1)]);
    assert_eq!(by_margin(&m, 1), [(0, 0), (3, 1)]);
    assert_eq!(candidates(&m, 1), [0, 1, 3, 2]);
}

#[test]
fn rows_are_z_scored_and_clipped() {
    // senna.
    let mut v = [1.0, 2.0, 3.0, 5.0, 5.0, 5.0, 0.0, f32::NAN, 10.0];
    zscore_rows(&mut v, 3, 1.0);
    let r = 1.0 / (2.0f32 / 3.0).sqrt();
    assert!((v[0] + r.min(1.0)).abs() < 1e-5 && v[1].abs() < 1e-6);
    assert_eq!(&v[3..6], [0.0, 0.0, 0.0]);
    assert!(v[7].is_nan() && v[6] == -1.0 && v[8] == 1.0);
}

#[test]
fn rows_are_z_scored_clipped_and_flat_rows_are_zero() {
    // pinto.
    let mut z = [1., 2., 3., 4., 4., 4., 0., f32::NAN, 2.];
    zscore_rows(&mut z, 3, CLIP);
    let s = (2f32 / 3.).sqrt();
    assert!((z[0] + 1. / s).abs() < 1e-5 && z[1].abs() < 1e-6);
    assert_eq!(&z[3..6], &[0., 0., 0.]);
    assert!(z[7].is_nan() && (z[6] + 1.).abs() < 1e-6);
    let mut wide: Vec<f32> = [0.; 99].iter().copied().chain([1000.]).collect();
    zscore_rows(&mut wide, 100, CLIP);
    assert_eq!(wide[99], CLIP);
    // A row with nothing in it stays as it was.
    let mut none = [f32::NAN; 3];
    zscore_rows(&mut none, 3, CLIP);
    assert!(none.iter().all(|v| v.is_nan()));
    zscore_rows(&mut none, 0, CLIP);
}

/// pinto's rates, five or three features × groups a, b, c.
const A_B_C: [&str; 3] = ["a", "b", "c"];
/// The columns, largest group first: c, a, b.
const SHOWN: [usize; 3] = [2, 0, 1];

fn model(rates: &[[f32; 3]]) -> ByFn<impl Fn(usize, usize) -> f32 + '_> {
    ByFn {
        rows: rates.len(),
        cols: SHOWN.len(),
        f: move |r, c| rates[r][SHOWN[c]].max(0.).ln_1p(),
    }
}

#[test]
fn genes_lead_their_own_group_by_a_margin() {
    let rates = [
        [9., 1., 8.], // g0: peaks in a but only just ahead of c
        [5., 1., 1.], // g1: peaks in a well ahead
        [1., 1., 7.], // g2: peaks in c
        [3., 3., 1.], // g3: a tie peaks nowhere
        [1., 6., 1.], // g4: peaks in b
    ];
    let m = model(&rates);
    let top = top_by_margin(&m, 1, None::<Rerank<'_, Matrix>>);
    // By margin g1 leads a, not g0 (which has the highest fold over the mean).
    let named: Vec<(usize, &str)> = top.iter().map(|&(r, c)| (r, A_B_C[SHOWN[c]])).collect();
    assert_eq!(named, [(2, "c"), (1, "a"), (4, "b")]);
    // Every row peaks on the diagonal.
    let m = &m;
    let mut z: Vec<f32> = top
        .iter()
        .flat_map(|&(r, _)| (0..3).map(move |c| m.at(r, c)))
        .collect();
    zscore_rows(&mut z, 3, CLIP);
    for (r, row) in z.chunks(3).enumerate() {
        let peak = (0..3).max_by(|&a, &b| row[a].total_cmp(&row[b])).unwrap();
        assert_eq!(peak, r);
    }
}

#[test]
fn observed_counts_rank_the_models_candidates() {
    let rates = [
        [9., 1., 1.], // g0: the model's best for a
        [5., 1., 1.], // g1: second for a
        [4., 1., 1.], // g2: third for a, missing from the counts
    ];
    let m = model(&rates);
    // Every candidate the model proposes is looked up.
    let pool = candidates(&m, 1);
    assert_eq!(pool, [0, 1, 2]);
    // Means over a, b, c: in the counts g0 is flat and g1 leads a; g2 is
    // not in them.
    let means = [Some([1., 1., 1.]), Some([3., 0., 1.]), None];
    let v: Vec<f32> = pool
        .iter()
        .flat_map(|&r| SHOWN.map(|g| means[r].map_or(f32::NAN, |m| m[g])))
        .collect();
    let counts = Matrix {
        rows: pool.len(),
        cols: 3,
        v: &v,
    };
    let top = top_by_margin(
        &m,
        1,
        Some(Rerank {
            rows: &pool,
            m: &counts,
        }),
    );
    assert_eq!(top, [(1, 1)]);
    // Columns in drawing order: c, a, b.
    let values: Vec<f32> = (0..3).map(|c| counts.at(1, c)).collect();
    assert_eq!(values, [1., 3., 0.]);
    let cols: Vec<String> = SHOWN.iter().map(|&g| A_B_C[g].to_string()).collect();
    let table = tsv(
        "mean ln(1+count)",
        "feature",
        &["g1".to_string()],
        &["a".to_string()],
        &cols,
        &values,
    );
    assert!(table.starts_with("# mean ln(1+count)\nfeature\tmarks\tc\ta\tb\ng1\ta\t"));
    assert_eq!(
        table,
        "# mean ln(1+count)\nfeature\tmarks\tc\ta\tb\ng1\ta\t1.0000\t3.0000\t0.0000\n"
    );
}

#[test]
fn without_counts_the_model_chooses() {
    let rates = [[5., 1., 1.]];
    let m = model(&rates);
    let top = top_by_margin(&m, 1, None::<Rerank<'_, Matrix>>);
    assert_eq!(top, [(0, 1)]);
}

#[test]
fn a_single_group_shows_its_top_levels() {
    // With one column there is no runner-up: rows rank by their level,
    // and those at or below zero, or missing, are left out.
    let v = [0.5, 2.0, 0.0, -1.0, f32::NAN, 1.0];
    let m = Matrix {
        rows: 6,
        cols: 1,
        v: &v,
    };
    assert_eq!(peak_margin(&m, 1), Some((0, 2.0)));
    assert_eq!(peak_margin(&m, 2), None);
    assert_eq!(by_margin(&m, 2), [(1, 0), (5, 0)]);
    assert_eq!(by_margin(&m, 10), [(1, 0), (5, 0), (0, 0)]);
    // No columns: nothing peaks.
    let none = Matrix {
        rows: 2,
        cols: 0,
        v: &[],
    };
    assert!(by_margin(&none, 3).is_empty());
}

#[test]
fn nan_rows_equal_dropped_rows_with_two_or_more_columns() {
    // A row with no level, or only one, never scores: the same rows come
    // out as when it is not there at all.
    let nan = f32::NAN;
    let with = [
        4.0, 1.0, 0.0, //
        nan, nan, nan, //
        nan, 9.0, nan, //
        0.0, 2.0, 5.0, //
        1.0, 3.0, 1.5,
    ];
    let without = [
        4.0, 1.0, 0.0, //
        0.0, 2.0, 5.0, //
        1.0, 3.0, 1.5,
    ];
    let kept = [0, 3, 4];
    let a = by_margin(
        &Matrix {
            rows: 5,
            cols: 3,
            v: &with,
        },
        2,
    );
    let b: Vec<(usize, usize)> = by_margin(
        &Matrix {
            rows: 3,
            cols: 3,
            v: &without,
        },
        2,
    )
    .into_iter()
    .map(|(r, c)| (kept[r], c))
    .collect();
    assert_eq!(a, b);
    assert_eq!(a, [(0, 0), (4, 1), (3, 2)]);
}

const PAGE: Srgb = [246, 245, 242];

fn senna_heat() -> (Vec<String>, Vec<String>, Vec<f32>) {
    let rows = [
        "GENE1",
        "GENE2",
        "GENE3",
        "A-LONGER-FEATURE-NAME",
        "GENE5",
        "GENE6",
    ];
    let cols = ["CT1", "CT2", "CT3", "CT4"];
    let z = vec![
        2.1,
        -0.4,
        -0.9,
        -0.8, //
        1.2,
        0.3,
        -1.0,
        -0.5, //
        -0.7,
        3.4,
        -1.1,
        -1.6, //
        -0.2,
        0.9,
        f32::NAN,
        -0.7, //
        -0.6,
        -0.6,
        2.4,
        -1.2, //
        -2.9,
        0.0,
        0.5,
        1.7,
    ];
    (
        rows.map(String::from).to_vec(),
        cols.map(String::from).to_vec(),
        z,
    )
}

const TITLE: &str =
    "top 2 features per cluster · mean ln(1 + count), z-scored per feature, clipped ±2.5";

fn draw_senna(
    f: &mut Frame,
    rows: &[String],
    cols: &[String],
    z: &[f32],
) -> Result<HeatLayout, &'static str> {
    let heat = Heat {
        title: TITLE,
        rows,
        cols,
        col_swatch: None,
        z,
        clip: CLIP,
        bold_row: None,
    };
    let theme = Theme::PAPER;
    draw_heat(
        f,
        &heat,
        Font::for_cell_height(16.0, false),
        Font::for_cell_height(16.0, true),
        &theme,
        &HeatStyle::for_theme(&theme),
    )
}

#[test]
fn draw_heat_matches_senna_draw_heatmap() {
    let (rows, cols, z) = senna_heat();
    let mut f = Frame::blank(420, 300, PAGE);
    draw_senna(&mut f, &rows, &cols, &z).unwrap();
    assert_eq!(fnv(f.to_image().as_raw()), 0x83f2_5abd_01db_a441);

    let mut f = Frame::blank(420, 300, PAGE);
    f.keep_text();
    draw_senna(&mut f, &rows, &cols, &z).unwrap();
    assert_eq!(fnv(f.to_image().as_raw()), 0x4558_3cf5_4a65_f852);
    let runs: Vec<(String, f32, f32, f32, bool, bool)> = f
        .take_text()
        .into_iter()
        .map(|r| (r.text, r.x, r.y, r.w, r.bold, r.vertical))
        .collect();
    let want: Vec<(String, f32, f32, f32, bool, bool)> = [
        (TITLE, 8., 8., 581., true, false),
        ("GENE1", 85., 43., 35., false, false),
        ("GENE2", 85., 83., 35., false, false),
        ("GENE3", 85., 123., 35., false, false),
        ("A-LONGER-FEATURE", 8., 163., 112., false, false),
        ("GENE5", 85., 203., 35., false, false),
        ("GENE6", 85., 243., 35., false, false),
        ("CT1", 146., 296., 21., false, true),
        ("CT2", 201., 296., 21., false, true),
        ("CT3", 255., 296., 21., false, true),
        ("CT4", 310., 296., 21., false, true),
        ("+2.5", 373., 40., 28., false, false),
        ("0", 373., 143., 7., false, false),
        ("-2.5", 373., 247., 28., false, false),
        ("z", 373., 24., 7., false, false),
    ]
    .iter()
    .map(|&(t, x, y, w, b, v)| (t.to_string(), x, y, w, b, v))
    .collect();
    assert_eq!(runs, want);

    // Too small: senna's title, then a note.
    let mut f = Frame::blank(60, 40, PAGE);
    assert_eq!(
        draw_senna(&mut f, &rows, &cols, &z).unwrap_err(),
        "too small to draw"
    );
    assert_eq!(fnv(f.to_image().as_raw()), 0x403a_fcaf_c3f4_a726);
    // Nothing to show.
    let mut f = Frame::blank(420, 300, PAGE);
    assert_eq!(
        draw_senna(&mut f, &[], &cols, &[]).unwrap_err(),
        "no features stand out in these groups"
    );
}

#[test]
fn heat_layout_at_maps_row_labels_and_cells() {
    let (rows, cols, z) = senna_heat();
    let mut f = Frame::blank(420, 300, PAGE);
    let l = draw_senna(&mut f, &rows, &cols, &z).unwrap();
    let [x0, y0, x1, y1] = l.plot;
    let (cw, rh) = l.cell;
    assert!((cw - (x1 - x0) as f32 / 4.0).abs() < 1e-4);
    assert!((rh - (y1 - y0) as f32 / 6.0).abs() < 1e-4);
    let (x0, y0) = (x0 as f32, y0 as f32);
    // Cells.
    assert_eq!(l.at(x0 + 0.5, y0 + 0.5), Some((0, Some(0))));
    assert_eq!(l.at(x0 + 2.5 * cw, y0 + 3.5 * rh), Some((3, Some(2))));
    assert_eq!(l.at(x0 + 3.99 * cw, y0 + 5.99 * rh), Some((5, Some(3))));
    // Row labels, left of the plot.
    assert_eq!(l.row_y.len(), 6);
    assert!(l.row_y.iter().all(Option::is_some));
    let label = l.row_y[2].unwrap();
    assert_eq!(l.at(x0 - 20.0, label + 1.0), Some((2, None)));
    // Outside.
    assert_eq!(l.at(x0 + 0.5, y0 - 1.0), None);
    assert_eq!(l.at(x1 as f32 + 1.0, y0 + 0.5), None);
    assert_eq!(l.at(x0 + 0.5, y1 as f32 + 0.5), None);
}

#[test]
fn rows_too_close_for_a_label_each_skip_some() {
    let rows: Vec<String> = (0..40).map(|i| format!("GENE{i}")).collect();
    let cols = vec!["CT1".to_string(), "CT2".to_string()];
    let z: Vec<f32> = (0..80).map(|i| (i % 5) as f32 - 2.0).collect();
    let mut f = Frame::blank(300, 200, PAGE);
    let l = draw_senna(&mut f, &rows, &cols, &z).unwrap();
    let shown: Vec<usize> = (0..40).filter(|&r| l.row_y[r].is_some()).collect();
    assert!(shown.len() < 40 && shown[0] == 0, "{shown:?}");
    // A row whose label is skipped is found by its cells only.
    let skipped = (0..40).find(|r| l.row_y[*r].is_none()).unwrap();
    let y = l.plot[1] as f32 + (skipped as f32 + 0.5) * l.cell.1;
    assert_eq!(l.at(l.plot[0] as f32 + 0.5, y), Some((skipped, Some(0))));
}

#[test]
fn a_swatch_row_and_a_bold_row_add_to_senna_s_heatmap() {
    let (rows, cols, z) = senna_heat();
    let swatch: Vec<_> = (0..4).map(|i| [0.1 * i as f32, 0.2, 0.3]).collect();
    let heat = Heat {
        title: TITLE,
        rows: &rows,
        cols: &cols,
        col_swatch: Some(&swatch),
        z: &z,
        clip: CLIP,
        bold_row: Some(1),
    };
    let theme = Theme::PAPER;
    let mut f = Frame::blank(420, 300, PAGE);
    f.keep_text();
    let l = draw_heat(
        &mut f,
        &heat,
        Font::for_cell_height(16.0, false),
        Font::for_cell_height(16.0, true),
        &theme,
        &HeatStyle::for_theme(&theme),
    )
    .unwrap();
    let bold: Vec<String> = f
        .take_text()
        .into_iter()
        .filter(|r| r.bold)
        .map(|r| r.text)
        .collect();
    assert_eq!(bold, [TITLE, "GENE2"]);
    // Each column's swatch sits just above it.
    let img = f.to_image();
    let (cw, _) = l.cell;
    for (c, s) in swatch.iter().enumerate() {
        let x = (l.plot[0] as f32 + (c as f32 + 0.5) * cw) as u32;
        let p = img.get_pixel(x, l.plot[1] as u32 - 4).0;
        let want = crate::map::colour::to_srgb_senna(*s);
        assert_eq!([p[0], p[1], p[2]], want, "{c}");
    }
}

#[test]
fn a_thumbnail_is_pinto_s_heat_picture() {
    let theme = Theme::DARK;
    let swatch = [to_linear([10, 20, 30]), to_linear([200, 100, 50])];
    let z = [2.5, -2.5, f32::NAN, 0.0];
    let f = thumbnail(&z, 2, Some(&swatch), 6, &theme, CLIP);
    assert_eq!((f.w, f.h), (12, 18));
    let img = f.to_image();
    let at = |x: u32, y: u32| {
        let p = img.get_pixel(x, y).0;
        [p[0], p[1], p[2]]
    };
    assert_eq!(at(0, 0), [10, 20, 30]);
    assert_eq!(at(11, 5), [200, 100, 50]);
    assert_eq!(at(0, 6), diverging(1.0));
    assert_eq!(at(6, 11), diverging(-1.0));
    assert_eq!(at(0, 12), theme.dimmed);
    assert_eq!(at(11, 17), diverging(0.0));
    // Without a swatch, only the blocks.
    let f = thumbnail(&z, 2, None, 2, &theme, CLIP);
    assert_eq!((f.w, f.h), (4, 4));
}

#[test]
fn a_cell_takes_the_diverging_colour_of_its_clipped_z() {
    let missing: Srgb = [1, 2, 3];
    assert_eq!(cell_colour(1.25, 2.5, missing), diverging(0.5));
    assert_eq!(cell_colour(-2.5, 2.5, missing), diverging(-1.0));
    assert_eq!(cell_colour(0.0, 2.5, missing), diverging(0.0));
    // Beyond the clip the colour saturates.
    assert_eq!(cell_colour(9.0, 2.5, missing), diverging(1.0));
    assert_eq!(cell_colour(-9.0, 2.5, missing), diverging(-1.0));
    // A value that is not a number is missing.
    assert_eq!(cell_colour(f32::NAN, 2.5, missing), missing);
    assert_eq!(cell_colour(f32::INFINITY, 2.5, missing), missing);
    assert_eq!(cell_colour(f32::NEG_INFINITY, 2.5, missing), missing);
}

#[test]
fn a_cell_in_linear_light_follows_the_same_rule() {
    use crate::map::colour::diverging_lin;
    let missing = [0.1, 0.2, 0.3];
    assert_eq!(cell_colour_lin(1.25, 2.5, missing), diverging_lin(0.5));
    assert_eq!(cell_colour_lin(9.0, 2.5, missing), diverging_lin(1.0));
    assert_eq!(cell_colour_lin(-9.0, 2.5, missing), diverging_lin(-1.0));
    assert_eq!(cell_colour_lin(f32::NAN, 2.5, missing), missing);
    assert_eq!(cell_colour_lin(f32::INFINITY, 2.5, missing), missing);
}

#[test]
fn a_thumbnail_s_pixels_are_pinned() {
    let swatch = [to_linear([10, 20, 30]), to_linear([200, 100, 50])];
    let z = [3.0, -0.7, f32::NAN, 0.0, 1.2, f32::INFINITY, -9.0, 0.3, 2.5];
    let f = thumbnail(&z, 3, Some(&swatch), 3, &Theme::DARK, CLIP);
    assert_eq!(fnv(f.to_image().as_raw()), 0xa4fc_8cc5_6fe9_c1e0);
    let f = thumbnail(&z, 3, None, 2, &Theme::LIGHT, 1.0);
    assert_eq!(fnv(f.to_image().as_raw()), 0x99b0_f59d_455e_18a5);
}
