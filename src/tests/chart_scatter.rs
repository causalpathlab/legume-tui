//! Scatter plots: points land where their data says, onto the edge past
//! the range and not at all when NaN, odd ranges still draw, and the text
//! is kept for a page.

use super::*;
use crate::map::raster::Frame;
use crate::map::text::Font;
use crate::map::theme::Theme;

fn plot(s: Scatter) -> (Frame, Result<ScatterLayout, &'static str>) {
    let theme = Theme::PAPER;
    let mut frame = Frame::blank(320, 240, theme.background);
    frame.keep_text();
    let font = Font::for_cell_height(12.0, false);
    let bold = Font::for_cell_height(12.0, true);
    let out = draw_scatter(&mut frame, &s, font, bold, &theme, &ScatterStyle::default());
    (frame, out)
}

fn series() -> Vec<String> {
    vec!["A".into(), "B".into()]
}

#[test]
fn points_land_by_their_data_past_the_range_on_its_edge_and_nan_nowhere() {
    let points = [Point::new(0.0, 5.0, 1, 1.0)];
    let series = series();
    let s = Scatter::new(&points, &series).with_ranges(Some((-1.0, 1.0)), Some((0.0, 10.0)));
    let l = plot(s).1.unwrap();
    let [x0, y0, x1, y1] = l.plot;
    let (cx, cy) = l.px(0.0, 5.0).unwrap();
    assert!((cx - (x0 + x1) / 2.0).abs() < 1e-3 && (cy - (y0 + y1) / 2.0).abs() < 1e-3);
    assert_eq!(l.px(9.0, 99.0), Some((x1, y0)));
    assert_eq!(l.px(f32::INFINITY, f32::NEG_INFINITY), Some((x1, y1)));
    assert_eq!(l.px(f32::NAN, 1.0), None);
}

#[test]
fn a_range_of_one_value_or_given_backwards_still_draws() {
    let points = [Point::new(3.0, 3.0, 0, 1.0)];
    let series = series();
    let s = Scatter::new(&points, &series).with_ranges(Some((3.0, 3.0)), Some((10.0, 0.0)));
    let l = plot(s).1.unwrap();
    assert_eq!((l.x_range, l.y_range), ((2.5, 3.5), (0.0, 10.0)));
    assert!(l
        .px(3.0, 3.0)
        .is_some_and(|(x, y)| x.is_finite() && y.is_finite()));
}

#[test]
fn a_page_keeps_the_title_the_key_the_names_and_the_lines_values_as_text() {
    let points = [Point::new(0.5, 8.0, 1, 0.2)];
    let series = series();
    let named = [(0, "GENE1 · B".to_string())];
    let s = Scatter::new(&points, &series)
        .with_title("effect × |z|")
        .with_labels("effect", "|z|")
        .with_ranges(Some((-1.0, 1.0)), Some((0.0, 10.0)))
        .with_lines(&[2.0], &[0.0])
        .with_named(&named)
        .with_ringed(&[0]);
    let (mut frame, out) = plot(s);
    out.unwrap();
    let text: Vec<String> = frame.take_text().into_iter().map(|t| t.text).collect();
    for want in [
        "effect × |z|",
        "A",
        "B",
        "GENE1 · B",
        "|z|",
        "0",
        "2",
        "-1",
        "1",
        "10",
    ] {
        assert!(text.iter().any(|t| t == want), "{want} not in {text:?}");
    }
}

#[test]
fn nothing_to_plot_says_so() {
    let theme = Theme::PAPER;
    let mut frame = Frame::blank(320, 240, theme.background);
    let font = Font::for_cell_height(12.0, false);
    let s = Scatter::new(&[], &[]);
    let out = draw_scatter(&mut frame, &s, font, font, &theme, &ScatterStyle::default());
    assert_eq!(out, Err(NOTHING));
}
