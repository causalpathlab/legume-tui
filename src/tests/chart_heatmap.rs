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

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as usize
    }
    fn below(&mut self, n: usize) -> usize {
        self.next() % n
    }
    fn name(&mut self, most: usize) -> String {
        const ABC: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789-";
        (0..1 + self.below(most))
            .map(|_| ABC[self.below(ABC.len())] as char)
            .collect()
    }
}

/// A run's box: `x0, y0, x1, y1`.
fn run_box(r: &crate::map::text::TextRun) -> [f32; 4] {
    if r.vertical {
        [r.x, r.y - r.w, r.x + r.line, r.y]
    } else {
        [r.x, r.y, r.x + r.w, r.y + r.line]
    }
}

/// Why `runs` do not sit in a `w × h` frame each apart from the others,
/// if they do not.
fn runs_apart(runs: &[crate::map::text::TextRun], w: usize, h: usize) -> Result<(), String> {
    let boxes: Vec<[f32; 4]> = runs.iter().map(run_box).collect();
    for (r, b) in runs.iter().zip(&boxes) {
        if b[0] < 0.0 || b[1] < 0.0 || b[2] > w as f32 || b[3] > h as f32 {
            return Err(format!("{:?} at {b:?} leaves {w} × {h}", r.text));
        }
    }
    for (i, a) in boxes.iter().enumerate() {
        for (j, b) in boxes.iter().enumerate().skip(i + 1) {
            if a[0] < b[2] && b[0] < a[2] && a[1] < b[3] && b[1] < a[3] {
                return Err(format!("{:?} over {:?}", runs[i].text, runs[j].text));
            }
        }
    }
    Ok(())
}

struct RandomHeat {
    title: String,
    rows: Vec<String>,
    cols: Vec<String>,
    z: Vec<f32>,
    clip: f32,
    bold_row: Option<usize>,
    style: HeatStyle,
    cell_px: f32,
}

impl RandomHeat {
    fn new(g: &mut Lcg) -> Self {
        let (n, m) = (1 + g.below(30), 1 + g.below(12));
        let theme = Theme::PAPER;
        let mut style = HeatStyle::for_theme(&theme);
        style.row_chars = 4 + g.below(20);
        style.col_chars = 4 + g.below(20);
        style.key = g.below(3) > 0;
        Self {
            title: g.name(60),
            rows: (0..n).map(|_| g.name(28)).collect(),
            cols: (0..m).map(|_| g.name(28)).collect(),
            z: (0..n * m).map(|i| (i % 7) as f32 - 3.0).collect(),
            clip: [2.5, 3.0, 1.0, 10.125, 100.0][g.below(5)],
            bold_row: (g.below(2) == 0).then(|| g.below(n)),
            style,
            cell_px: [16.0, 20.0, 24.0, 33.0, 40.0][g.below(5)],
        }
    }

    fn heat(&self) -> Heat<'_> {
        Heat {
            title: &self.title,
            rows: &self.rows,
            cols: &self.cols,
            col_swatch: None,
            z: &self.z,
            clip: self.clip,
            bold_row: self.bold_row,
        }
    }

    fn font(&self) -> Font {
        Font::for_cell_height(self.cell_px, false)
    }

    /// Why drawing at `w × h` does not show every name and the key, a line
    /// per row and column, all inside the frame, if it does not.
    fn shows_all(&self, w: usize, h: usize) -> Result<(), String> {
        let theme = Theme::PAPER;
        let mut f = Frame::blank(w, h, PAGE);
        f.keep_text();
        let l = draw_heat(
            &mut f,
            &self.heat(),
            self.font(),
            Font::for_cell_height(self.cell_px, true),
            &theme,
            &self.style,
        )?;
        if l.cell.0 < l.line || l.cell.1 < l.line {
            return Err(format!("cells of {:?} under a line of {}", l.cell, l.line));
        }
        let runs = f.take_text();
        let said: Vec<&str> = runs.iter().map(|r| r.text.as_str()).collect();
        let cut = |s: &String, n: usize| s.chars().take(n).collect::<String>();
        let mut want: Vec<String> = self
            .rows
            .iter()
            .map(|r| cut(r, self.style.row_chars))
            .chain(self.cols.iter().map(|c| cut(c, self.style.col_chars)))
            .collect();
        if self.style.key {
            want.extend([
                format!("+{}", self.clip),
                "0".into(),
                format!("-{}", self.clip),
                "z".into(),
            ]);
        }
        let mut have: Vec<&str> = said[1..].to_vec();
        for name in &want {
            match have.iter().position(|s| s == name) {
                Some(i) => {
                    have.swap_remove(i);
                }
                None => return Err(format!("{name:?} not drawn")),
            }
        }
        runs_apart(&runs, w, h)
    }
}

#[test]
fn heat_size_is_the_least_frame_that_shows_every_name_and_the_key() {
    let mut g = Lcg(7);
    for case in 0..120 {
        let r = RandomHeat::new(&mut g);
        let min_w = [0, 100, 900][g.below(3)];
        let (w, h) = heat_size(&r.heat(), r.font(), &r.style, min_w);
        let at = |w, h| {
            r.shows_all(w, h)
                .map_err(|e| format!("case {case} at {w} × {h}: {e}"))
        };
        at(w, h).unwrap();
        assert!(w >= min_w);
        if w > min_w {
            assert!(
                at(w - 1, h).is_err(),
                "case {case}: {w} - 1 still shows all"
            );
        }
        assert!(
            at(w, h - 1).is_err(),
            "case {case}: {h} - 1 still shows all"
        );
    }
}

#[test]
fn heat_size_of_senna_s_heatmap() {
    // GENE1..6 under 16 px lines: six rows and four columns a line each,
    // the title (83 characters) setting the width.
    let (rows, cols, z) = senna_heat();
    let heat = Heat {
        title: TITLE,
        rows: &rows,
        cols: &cols,
        col_swatch: None,
        z: &z,
        clip: CLIP,
        bold_row: None,
    };
    let theme = Theme::PAPER;
    let s = HeatStyle::for_theme(&theme);
    let font = Font::for_cell_height(16.0, false);
    let (w, h) = heat_size(&heat, font, &s, 0);
    assert_eq!(w, 8 + 83 * 7);
    // Two lines over the cells, six lines of them, then the band of the
    // column names: three characters and half a line.
    assert_eq!(h, 2 * 16 + 6 * 16 + 3 * 7 + 8);
    assert_eq!(heat_size(&heat, font, &s, 1000), (1000, h));
}

/// [`heat_size`] as searches one pixel at a time.
fn heat_size_linear(h: &Heat, font: Font, s: &HeatStyle, min_w: usize) -> (usize, usize) {
    let (rows, cols) = (h.rows.len(), h.cols.len());
    let lh = font.line_height();
    let g = HeatGeom::new(0, 0, h, font, s);
    let wide = (g.x0 + cols * lh + g.key_w + lh)
        .max(lh / 2 + g.title_w)
        .max(min_w);
    let w = (wide..=wide + lh)
        .find(|&w| HeatGeom::new(w, 0, h, font, s).fits_across(w, cols))
        .unwrap_or(wide + lh);
    let cells = if s.key { rows.max(4) } else { rows } * lh;
    let low = g.y0 + cells + lh / 4 + g.col_label;
    let high = 3 * (g.y0 + cells + g.col_label + lh);
    let h_px = (low..=high)
        .find(|&hh| HeatGeom::new(0, hh, h, font, s).fits_down(hh, rows, s.key))
        .unwrap_or(high);
    (w, h_px)
}

#[test]
fn heat_size_is_the_linear_search_s() {
    let mut g = Lcg(31);
    for case in 0..300 {
        let r = RandomHeat::new(&mut g);
        let min_w = [0, 100, 900][g.below(3)];
        assert_eq!(
            heat_size(&r.heat(), r.font(), &r.style, min_w),
            heat_size_linear(&r.heat(), r.font(), &r.style, min_w),
            "case {case}"
        );
    }
}

#[test]
fn heat_size_of_many_rows_is_quick() {
    let rows: Vec<String> = (0..1500).map(|i| format!("GENE{i}")).collect();
    let cols: Vec<String> = (0..40).map(|j| format!("CT{j}")).collect();
    let z = vec![0.5; rows.len() * cols.len()];
    let heat = Heat {
        title: "T",
        rows: &rows,
        cols: &cols,
        col_swatch: None,
        z: &z,
        clip: CLIP,
        bold_row: None,
    };
    let s = HeatStyle::for_theme(&Theme::PAPER);
    let font = Font::for_cell_height(16.0, false);
    let t = std::time::Instant::now();
    let (_, h) = heat_size(&heat, font, &s, 0);
    let took = t.elapsed();
    assert!(took.as_millis() < 1000, "{took:?}");
    assert!(h >= rows.len() * font.line_height(), "{h}");
}
