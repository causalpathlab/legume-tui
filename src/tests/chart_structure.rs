//! Structure plots: senna's and pinto's ordering, pinto's widths and bars
//! (stacked from the bottom), senna's decorated figure. The pinned numbers
//! of the figure are what senna's own `draw_structure` (`src/view/chart.rs`,
//! compiled apart from this crate) gives for the same plot with its stack
//! run from the bottom, on panels whose widths both rules agree on.

use super::*;
use crate::map::colour::{to_linear, Srgb};
use crate::map::raster::Frame;
use crate::map::text::Font;
use crate::map::theme::Theme;

const NONE: u32 = u32::MAX;
const NO_CLUSTER: u16 = u16::MAX;

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

fn names(n: &[&str]) -> Vec<String> {
    n.iter().map(ToString::to_string).collect()
}

fn rows(l: &Layout) -> Vec<(&str, Vec<u32>)> {
    l.panels
        .iter()
        .map(|p| (p.name.as_str(), p.rows.clone()))
        .collect()
}

#[test]
fn structure_panels_follow_groups_and_cells_their_dominant_topic() {
    // senna: three topics; topic 2 most prevalent, then 0, then 1.
    let theta = [
        0.1,
        0.1,
        0.8, // p0 group 0, dominant 2
        0.6,
        0.1,
        0.3, // p1 group 0, dominant 0
        0.2,
        0.0,
        0.8, // p2 group 1
        0.0,
        0.0,
        0.9, // p3 no group
        f32::NAN,
        f32::NAN,
        f32::NAN, // p4 no mixture
        0.3,
        0.1,
        0.6, // p5 group 0, dominant 2, weaker than p0
    ];
    let groups = [0, 0, 1, NONE, 0, 0];
    let levels = names(&["A", "B"]);
    let w = Weights {
        k: 3,
        w: &theta,
        valid: None,
    };
    let g = Grouping {
        of: &groups,
        names: &levels,
        order: None,
    };
    let l = Layout::build(&w, &g);
    assert_eq!(l.stack, [2, 0, 1]);
    assert_eq!(l.rank(), [1, 2, 0]);
    // A: p0 (2 at 0.8), p5 (2 at 0.6), then p1 (dominant 0).
    assert_eq!(
        rows(&l),
        [("A", vec![0, 5, 1]), ("B", vec![2]), (UNASSIGNED, vec![3])]
    );
    assert_eq!(l.n_rows(), 5);
}

/// pinto: five cells over K = 2, shares of community 0 given (of 255),
/// the rest is 1's.
fn shares(share0: &[u8]) -> Vec<u8> {
    share0.iter().flat_map(|&q| [q, 255 - q]).collect()
}

#[test]
fn cells_sort_by_dominant_community_then_share() {
    // Community 1 dominates more cells overall, so it stacks first.
    let p = shares(&[200, 10, 250, 60, 100]);
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let all = names(&["all"]);
    let l = Layout::build(
        &w,
        &Grouping {
            of: &[0u16; 5],
            names: &all,
            order: Some(&[0]),
        },
    );
    assert_eq!(l.stack, [1, 0]);
    // Community 1's cells by its share (1: 245, 3: 195, 4: 155), then 0's.
    assert_eq!(l.panels[0].rows, [1, 3, 4, 2, 0]);
}

#[test]
fn panels_follow_the_given_order() {
    let p = shares(&[200, 10, 250, 60, 100]);
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let xy = names(&["x", "y"]);
    let l = Layout::build(
        &w,
        &Grouping {
            of: &[1, 0, 1, NO_CLUSTER, 0],
            names: &xy,
            order: Some(&[1, 0]),
        },
    );
    let got: Vec<(&str, usize)> = l
        .panels
        .iter()
        .map(|p| (p.name.as_str(), p.rows.len()))
        .collect();
    // The cell with no group goes last, as unassigned.
    assert_eq!(got, [("y", 2), ("x", 2), ("unassigned", 1)]);
}

#[test]
fn cells_no_group_claims_go_last_as_unassigned() {
    let p = shares(&[200, 10, 250, 60, 100]);
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let x = names(&["x"]);
    let l = Layout::build(
        &w,
        &Grouping {
            of: &[0, NO_CLUSTER, 0, NO_CLUSTER, 0],
            names: &x,
            order: Some(&[0]),
        },
    );
    let got: Vec<(&str, usize)> = l
        .panels
        .iter()
        .map(|p| (p.name.as_str(), p.rows.len()))
        .collect();
    assert_eq!(got, [("x", 3), ("unassigned", 2)]);
}

#[test]
fn rows_marked_invalid_are_left_out() {
    // pinto passes `cluster != NO_CLUSTER`; by default a row counts when
    // it has a first number and some weight.
    let p = [1.0f32, 0.0, 0.0, 0.0, f32::NAN, 1.0, 0.0, 1.0];
    let all = names(&["all"]);
    let g = Grouping {
        of: &[0u32; 4],
        names: &all,
        order: None,
    };
    let default = Layout::build(
        &Weights {
            k: 2,
            w: &p,
            valid: None,
        },
        &g,
    );
    assert_eq!(default.panels[0].rows, [0, 3]);
    let given = Layout::build(
        &Weights {
            k: 2,
            w: &p,
            valid: Some(&[false, true, false, true]),
        },
        &g,
    );
    assert_eq!(given.panels[0].rows, [3, 1]);
}

#[test]
fn panels_split_into_plots_of_their_own_on_one_stack() {
    let p = shares(&[200, 10, 250, 60, 100]);
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let xy = names(&["x", "y"]);
    let l = Layout::build(
        &w,
        &Grouping {
            of: &[1u16, 0, 1, 0, 0],
            names: &xy,
            order: None,
        },
    );
    let stack = l.stack.clone();
    let tiles = l.into_panels();
    let got: Vec<(&str, usize)> = tiles
        .iter()
        .map(|(n, t)| (n.as_str(), t.panels.len()))
        .collect();
    assert_eq!(got, [("x", 1), ("y", 1)]);
    assert!(tiles.iter().all(|(_, t)| t.stack == stack));
}

#[test]
fn a_dominant_tie_goes_to_the_more_prevalent_component() {
    // Component 0 is the more prevalent; r0 ties 0 and 1, so it sorts
    // with 0's rows (senna would have put it with 1's, the last maximum).
    let p = [0.5f32, 0.5, 0.8, 0.2, 0.4, 0.6];
    let all = names(&["all"]);
    let l = Layout::build(
        &Weights {
            k: 2,
            w: &p,
            valid: None,
        },
        &Grouping {
            of: &[0u32; 3],
            names: &all,
            order: None,
        },
    );
    assert_eq!(l.stack, [0, 1]);
    assert_eq!(l.panels[0].rows, [1, 0, 2]);
}

#[test]
fn widths_sum_to_the_room_and_give_every_panel_a_pixel() {
    assert_eq!(widths(&[3, 3], 10, 2), [4, 4]);
    // Largest remainders take what is left over.
    assert_eq!(widths(&[1, 2], 12, 0), [4, 8]);
    assert_eq!(widths(&[2, 1], 11, 0), [7, 4]);
    // A panel too small for a pixel borrows one from the widest.
    let w = widths(&[1, 1000, 1], 50, 2);
    assert_eq!(w.iter().sum::<usize>(), 46);
    assert!(w.iter().all(|&x| x >= 1), "{w:?}");
    // No room at all: nothing to share.
    assert_eq!(widths(&[1, 1, 1], 3, 2), [0, 0, 0]);
    assert_eq!(widths(&[], 10, 2), Vec::<usize>::new());
}

const RED: Srgb = [255, 0, 0];
const BLUE: Srgb = [0, 0, 255];

/// pinto's frame: bytes in, bytes back.
fn pinto_frame(w: usize, h: usize, theme: &Theme) -> Frame {
    let [r, g, b] = theme.background;
    let rgba: Vec<u8> = std::iter::repeat_n([r, g, b, 255], w * h)
        .flatten()
        .collect();
    Frame::from_srgba(w, h, &rgba, theme.background)
}

fn px(f: &Frame, x: u32, y: u32) -> Srgb {
    let p = f.to_image().get_pixel(x, y).0;
    [p[0], p[1], p[2]]
}

fn five(share0: &[u8]) -> (Vec<u8>, Layout) {
    let p = shares(share0);
    let all = names(&["all"]);
    let l = Layout::build(
        &Weights {
            k: 2,
            w: &p,
            valid: None,
        },
        &Grouping {
            of: &[0u16; 5],
            names: &all,
            order: Some(&[0]),
        },
    );
    (p, l)
}

#[test]
fn bars_stack_from_the_bottom_in_palette_colours() {
    let (p, l) = five(&[255, 255, 0, 0, 0]);
    let palette = [to_linear(RED), to_linear(BLUE)];
    let mut f = pinto_frame(5, 10, &Theme::DARK);
    let style = BarStyle {
        palette: &palette,
        focus: None,
        gap: GAP,
        hits: true,
    };
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let d = bars(&mut f, [0, 0, 5, 10], &l, &w, &style);
    assert_eq!((d.spans[0].x0, d.spans[0].x1), (0, 4));
    assert_eq!(d.spans[0].rows, 5);
    assert_eq!(d.rect, [0, 0, 5, 10]);
    // Community 1 (three cells) first, all blue; then 0's cells, all red.
    assert_eq!(px(&f, 0, 9), BLUE);
    assert_eq!(px(&f, 0, 0), BLUE);
    assert_eq!(px(&f, 4, 0), RED);
    // Each pixel knows its community, for clicks; none outside.
    assert_eq!(
        (d.at(0, 9), d.at(4, 0), d.at(5, 0), d.at(0, 10)),
        (Some((0, 1)), Some((0, 0)), None, None)
    );
}

#[test]
fn a_mixed_column_stacks_the_most_prevalent_at_the_bottom() {
    let (p, l) = five(&[64, 64, 64, 64, 64]);
    assert_eq!(l.stack, [1, 0]);
    let palette = [to_linear(RED), to_linear(BLUE)];
    let mut f = pinto_frame(1, 8, &Theme::LIGHT);
    let style = BarStyle {
        palette: &palette,
        focus: None,
        gap: GAP,
        hits: true,
    };
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let d = bars(&mut f, [0, 0, 1, 8], &l, &w, &style);
    // 191 of 255 is blue: the bottom six rows of eight.
    assert_eq!(px(&f, 0, 7), BLUE);
    assert_eq!(px(&f, 0, 2), BLUE);
    assert_eq!(px(&f, 0, 1), RED);
    assert_eq!(px(&f, 0, 0), RED);
    assert_eq!((d.at(0, 2), d.at(0, 1)), (Some((0, 1)), Some((0, 0))));
}

#[test]
fn a_focus_dims_the_other_communities() {
    let (p, l) = five(&[255, 255, 0, 0, 0]);
    let palette = [to_linear(RED), to_linear(BLUE)];
    let theme = Theme::DARK;
    let mut f = pinto_frame(5, 10, &theme);
    let style = BarStyle {
        palette: &palette,
        focus: Some((&[true, false], to_linear(theme.dimmed))),
        gap: GAP,
        hits: true,
    };
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let d = bars(&mut f, [0, 0, 5, 10], &l, &w, &style);
    assert_eq!(px(&f, 0, 9), theme.dimmed);
    assert_eq!(px(&f, 4, 0), RED);
    // A dimmed community can still be clicked.
    assert_eq!(d.at(0, 9), Some((0, 1)));
}

#[test]
fn bars_keep_no_hits_unless_asked() {
    let (p, l) = five(&[255, 255, 0, 0, 0]);
    let palette = [to_linear(RED), to_linear(BLUE)];
    let mut f = pinto_frame(5, 10, &Theme::DARK);
    let style = BarStyle {
        palette: &palette,
        focus: None,
        gap: GAP,
        hits: false,
    };
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let d = bars(&mut f, [0, 0, 5, 10], &l, &w, &style);
    assert_eq!(px(&f, 0, 9), BLUE);
    assert_eq!(d.at(0, 9), None);
}

#[test]
fn a_column_averages_its_rows_and_renormalises() {
    // Four rows on two columns: each column the mean of two rows. u8 and
    // f32 weights give the same pixels.
    let palette = [to_linear(RED), to_linear(BLUE)];
    let style = BarStyle {
        palette: &palette,
        focus: None,
        gap: GAP,
        hits: true,
    };
    let all = names(&["all"]);
    let g = Grouping {
        of: &[0u32; 4],
        names: &all,
        order: None,
    };
    let draw = |f: &mut Frame, w: &dyn Fn(&mut Frame, &Layout) -> Drawn, l: &Layout| w(f, l);
    let bytes = [255u8, 0, 255, 0, 0, 255, 0, 255];
    let floats: Vec<f32> = bytes.iter().map(|&b| f32::from(b) / 255.0).collect();
    let wb = Weights {
        k: 2,
        w: &bytes,
        valid: None,
    };
    let wf = Weights {
        k: 2,
        w: &floats,
        valid: None,
    };
    let lb = Layout::build(&wb, &g);
    let lf = Layout::build(&wf, &g);
    assert_eq!(lb, lf);
    let mut fb = pinto_frame(2, 4, &Theme::LIGHT);
    let mut ff = pinto_frame(2, 4, &Theme::LIGHT);
    draw(&mut fb, &|f, l| bars(f, [0, 0, 2, 4], l, &wb, &style), &lb);
    draw(&mut ff, &|f, l| bars(f, [0, 0, 2, 4], l, &wf, &style), &lf);
    assert_eq!(fb.to_image(), ff.to_image());
    // Squeezed into one column, each is half of each.
    let mut f1 = pinto_frame(1, 4, &Theme::LIGHT);
    bars(&mut f1, [0, 0, 1, 4], &lb, &wb, &style);
    assert_eq!(px(&f1, 0, 3), RED);
    assert_eq!(px(&f1, 0, 1), BLUE);

    // A column whose rows weigh nothing stays the page.
    let zero = [0u8, 0, 255, 0];
    let wz = Weights {
        k: 2,
        w: &zero,
        valid: Some(&[true, true]),
    };
    let one = Grouping {
        of: &[0u32; 2],
        names: &all,
        order: None,
    };
    let lz = Layout::build(&wz, &one);
    assert_eq!(lz.n_rows(), 2);
    let mut fz = pinto_frame(2, 4, &Theme::LIGHT);
    let d = bars(&mut fz, [0, 0, 2, 4], &lz, &wz, &style);
    let page = Theme::LIGHT.background;
    let empty = if px(&fz, 0, 3) == page { 0 } else { 1 };
    assert_eq!(px(&fz, empty, 0), page);
    assert_eq!(px(&fz, empty, 3), page);
    assert_eq!(px(&fz, 1 - empty, 3), RED);
    assert_eq!(d.at(empty as usize, 3), None);
}

#[test]
fn panels_sit_side_by_side_with_a_gap() {
    let p = shares(&[255, 255, 0, 0]);
    let xy = names(&["x", "y"]);
    let l = Layout::build(
        &Weights {
            k: 2,
            w: &p,
            valid: None,
        },
        &Grouping {
            of: &[0u16, 0, 1, 1],
            names: &xy,
            order: None,
        },
    );
    let palette = [to_linear(RED), to_linear(BLUE)];
    let style = BarStyle {
        palette: &palette,
        focus: None,
        gap: 2,
        hits: true,
    };
    let mut f = pinto_frame(14, 4, &Theme::DARK);
    let w = Weights {
        k: 2,
        w: &p,
        valid: None,
    };
    let d = bars(&mut f, [2, 0, 12, 4], &l, &w, &style);
    let spans: Vec<(usize, usize, &str, usize)> = d
        .spans
        .iter()
        .map(|s| (s.x0, s.x1, s.name.as_str(), s.rows))
        .collect();
    assert_eq!(spans, [(2, 5, "x", 2), (8, 11, "y", 2)]);
    assert_eq!(px(&f, 6, 3), Theme::DARK.background);
    assert_eq!(px(&f, 1, 3), Theme::DARK.background);
    assert_eq!(px(&f, 2, 3), RED);
    assert_eq!(px(&f, 11, 3), BLUE);
    assert_eq!(
        (d.at(3, 0), d.at(6, 0), d.at(9, 0)),
        (Some((0, 0)), None, Some((1, 1)))
    );
}

const PAGE: Srgb = [246, 245, 242];
const PALETTE: [crate::map::colour::Lin; 3] = [[0.8, 0.2, 0.1], [0.1, 0.5, 0.8], [0.2, 0.7, 0.2]];

/// senna's even plot: two panels of four cells, dyadic mixtures, so both
/// width rules and both column averages give the same pixels.
fn even() -> (Vec<f32>, [u32; 8]) {
    let theta = vec![
        0.25, 0.25, 0.5, //
        0.5, 0.25, 0.25, //
        0.125, 0.125, 0.75, //
        0.25, 0.5, 0.25, //
        0.0, 0.25, 0.75, //
        0.5, 0.5, 0.0, //
        0.75, 0.0, 0.25, //
        0.25, 0.0, 0.75,
    ];
    (theta, [0, 0, 0, 0, 1, 1, 1, 1])
}

fn senna_figure(panels: &[&str], keep_text: bool) -> (Frame, Layout, Drawn) {
    let (theta, groups) = even();
    let levels = names(panels);
    let w = Weights {
        k: 3,
        w: &theta,
        valid: None,
    };
    let l = Layout::build(
        &w,
        &Grouping {
            of: &groups,
            names: &levels,
            order: None,
        },
    );
    let mut f = Frame::blank(301, 200, PAGE);
    if keep_text {
        f.keep_text();
    }
    let key = names(&["T0", "T1", "T2"]);
    let fig = Figure {
        title: Some("topic mixture per cell · one panel per cluster · 3 topics"),
        key: Some(&key),
        names: Names::Auto,
        font: Font::for_cell_height(16.0, false),
        bold: Font::for_cell_height(16.0, true),
        theme: &Theme::PAPER,
    };
    let style = BarStyle {
        palette: &PALETTE,
        focus: None,
        gap: GAP,
        hits: true,
    };
    let d = figure(&mut f, &l, &w, &style, &fig);
    (f, l, d)
}

type Run = (&'static str, f32, f32, f32, bool, bool);

fn runs(f: &mut Frame) -> Vec<(String, f32, f32, f32, bool, bool)> {
    f.take_text()
        .into_iter()
        .map(|r| (r.text, r.x, r.y, r.w, r.bold, r.vertical))
        .collect()
}

fn owned(r: &[Run]) -> Vec<(String, f32, f32, f32, bool, bool)> {
    r.iter()
        .map(|&(t, x, y, w, b, v)| (t.to_string(), x, y, w, b, v))
        .collect()
}

#[test]
fn the_figure_is_senna_s_stacked_from_the_bottom() {
    let (f, l, d) = senna_figure(&["CT1", "CT2"], false);
    assert_eq!(l.stack, [2, 0, 1]);
    assert_eq!(
        rows(&l),
        [("CT1", vec![2, 0, 1, 3]), ("CT2", vec![4, 7, 6, 5])]
    );
    assert_eq!(fnv(f.to_image().as_raw()), 0x3418_683d_cd17_a3ff);
    let lh = 16;
    assert_eq!(
        d.rect,
        [lh / 2, 2 * lh, 301 - 5 * 7 - lh, 200 - (lh + lh / 2)]
    );
    // The bottom of the first column is the most prevalent topic.
    let x = d.rect[0];
    assert_eq!(d.at(x, d.rect[3] - 1), Some((0, 2)));
    assert_eq!(d.at(x, d.rect[1]), Some((0, 1)));
}

#[test]
fn figure_turns_names_and_keeps_text() {
    let (mut f, ..) = senna_figure(&["CT1", "CT2"], true);
    // With text kept, the pixels are the bars and the key's swatches.
    assert_eq!(fnv(f.to_image().as_raw()), 0xb375_c2f0_578c_0585);
    let title = "topic mixture per cell · one panel per cluster · 3 topics";
    let key: [Run; 3] = [
        ("T2", 282.0, 32.0, 14.0, false, false),
        ("T0", 282.0, 48.0, 14.0, false, false),
        ("T1", 282.0, 64.0, 14.0, false, false),
    ];
    let mut flat = vec![
        (title, 8.0, 8.0, 399.0, true, false),
        ("CT1", 58.0, 180.0, 21.0, false, false),
        ("CT2", 180.0, 180.0, 21.0, false, false),
    ];
    flat.extend(key);
    assert_eq!(runs(&mut f), owned(&flat));

    // A name wider than its panel turns them all, cut at 18 characters.
    let (mut f, ..) = senna_figure(&["A-LONG-PANEL-NAME-ONE-TWO", "CT2"], true);
    let mut turned = vec![
        (title, 8.0, 8.0, 399.0, true, false),
        ("A-LONG-PANEL-NAME-", 60.0, 196.0, 126.0, false, true),
        ("CT2", 182.0, 196.0, 21.0, false, true),
    ];
    turned.extend(key);
    assert_eq!(runs(&mut f), owned(&turned));
}

#[test]
fn a_figure_too_small_says_so() {
    let (theta, groups) = even();
    let levels = names(&["CT1", "CT2"]);
    let w = Weights {
        k: 3,
        w: &theta,
        valid: None,
    };
    let l = Layout::build(
        &w,
        &Grouping {
            of: &groups,
            names: &levels,
            order: None,
        },
    );
    let mut f = Frame::blank(16, 16, PAGE);
    f.keep_text();
    let fig = Figure {
        title: None,
        key: None,
        names: Names::None,
        font: Font::for_cell_height(16.0, false),
        bold: Font::for_cell_height(16.0, true),
        theme: &Theme::PAPER,
    };
    let style = BarStyle {
        palette: &PALETTE,
        focus: None,
        gap: GAP,
        hits: true,
    };
    let d = figure(&mut f, &l, &w, &style, &fig);
    assert!(d.spans.is_empty());
    let said: Vec<String> = f.take_text().into_iter().map(|r| r.text).collect();
    assert_eq!(said, ["too small to draw"]);
}

#[test]
fn name_rows_alternate_and_never_overlap() {
    let span = |x0, x1, name: &str| Span {
        x0,
        x1,
        name: name.into(),
        rows: 1,
    };
    // 10 px a cell: panels of 3, 3, 6 and 1 cells.
    let spans = [
        span(0, 29, "first-panel"),
        span(32, 61, "second"),
        span(64, 123, "third-name-is-long"),
        span(126, 135, "x"),
    ];
    let rows = name_rows(&spans, 14, 10.0, 2);
    assert_eq!(rows.len(), 2);
    assert!(rows.iter().all(|r| r.chars().count() == 14));
    // Names alternate between the rows, each starting at its panel or
    // just past the name before it on its row.
    assert_eq!(rows[0], "first-p third ");
    assert_eq!(rows[1], "   second   x ");
    // Never two names touching on a row.
    for r in &rows {
        let words: Vec<&str> = r.split(' ').filter(|w| !w.is_empty()).collect();
        assert!(words.len() <= 2, "{r}");
    }
    // One row: names past the last that fits are left out.
    let one = name_rows(&spans, 14, 10.0, 1);
    assert_eq!(one, ["first-p secon "]);
    assert_eq!(name_rows(&spans, 14, 10.0, 0), Vec::<String>::new());
}
