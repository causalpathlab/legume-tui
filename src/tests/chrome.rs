//! The screen's chrome: styles from a theme or the terminal's own colours,
//! windows that keep a row in sight, cut names, and popups.

use super::*;
use crate::map::theme::Theme;
use ratatui::backend::TestBackend;
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

fn text(lines: &[Line<'_>]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect()
}

/// What row `y` of a test terminal shows.
fn row(t: &Terminal<TestBackend>, y: u16) -> String {
    let buf = t.backend().buffer();
    (0..buf.area.width)
        .map(|x| buf[(x, y)].symbol().to_string())
        .collect()
}

#[test]
fn cuts_keep_the_end_or_the_start_and_say_so() {
    assert_eq!(short("abcdef", 4), "abc…");
    assert_eq!(short("abc", 4), "abc");
    assert_eq!(tail("abcdef", 4), "…def");
    assert_eq!(tail("abc", 4), "abc");
    assert_eq!(tail("abcdef", 4).chars().count(), 4);
    assert_eq!(middle("abcdefgh", 5), "ab…gh");
    assert_eq!(middle("abcdefgh", 6), "abc…gh");
    assert_eq!(middle("abc", 4), "abc");
}

#[test]
fn the_cursor_row_sits_in_the_middle_of_its_window_where_it_can() {
    assert_eq!(first_row(0, 10, 100), 0);
    assert_eq!(first_row(50, 10, 100), 45);
    assert_eq!(first_row(99, 10, 100), 90);
    assert_eq!(first_row(3, 10, 5), 0);
}

#[test]
fn wrapping_cuts_the_first_piece_and_the_rest_to_their_widths() {
    assert_eq!(wrap("abcdefg", 3, 2), ["abc", "de", "fg"]);
    assert_eq!(wrap("ab", 3, 2), ["ab"]);
    assert!(wrap("", 3, 2).is_empty());
}

#[test]
fn wrapped_rows_count_a_leading_space_and_wide_characters_as_ratatui_wraps() {
    // A space at the start of a row takes its column.
    assert_eq!(wrapped_rows(&format!(" {}", "x".repeat(18)), 18), 2);
    // Display width, not characters: each of these takes two columns.
    assert_eq!(wrapped_rows("\u{6587}\u{4ef6}\u{540d}", 4), 2);
}

#[test]
fn a_popup_keeps_its_keys_line_after_a_leading_space_line() {
    let mut term = Terminal::new(TestBackend::new(20, 10)).unwrap();
    term.draw(|f| {
        let lines = vec![
            Line::from(" title"),
            Line::from(format!(" {}", "x".repeat(18))),
            Line::from(" keys"),
        ];
        Chrome::TERMINAL.popup(f, f.area(), lines, 20, At::Middle);
    })
    .unwrap();
    let shown: String = term
        .backend()
        .buffer()
        .content()
        .iter()
        .map(|c| c.symbol())
        .collect();
    assert!(shown.contains("keys"), "{shown}");
}

#[test]
fn wrapped_rows_count_words_and_break_long_ones() {
    assert_eq!(wrapped_rows("", 10), 1);
    assert_eq!(wrapped_rows("one two three", 9), 2);
    assert_eq!(wrapped_rows("one two three", 13), 1);
    assert_eq!(wrapped_rows("abcdefghij", 4), 3);
    assert_eq!(wrapped_rows("ab abcdefghij", 4), 4);
}

#[test]
fn a_scrolled_list_says_how_many_rows_are_cut_off() {
    let rows: Vec<Line<'static>> = (0..10).map(|i| Line::from(format!("row {i}"))).collect();
    let c = Chrome::themed(Theme::PAPER);
    assert_eq!(
        text(&c.scrolled(rows.clone(), 0, 4)),
        ["row 0", "row 1", "row 2", " ↓ 7 more"]
    );
    assert_eq!(
        text(&c.scrolled(rows.clone(), 9, 4)),
        [" ↑ 7 more", "row 7", "row 8", "row 9"]
    );
    assert_eq!(text(&c.scrolled(rows, 5, 20)).len(), 10);
}

#[test]
fn a_page_window_stops_at_the_end_and_says_where_it_stands() {
    let body: Vec<Line<'static>> = (0..10).map(|i| Line::from(format!("{i}"))).collect();
    let max = std::cell::Cell::new(0);
    let (shown, stands) = page_window(body.clone(), 4, 0, &max);
    assert_eq!(
        (text(&shown), stands),
        (
            vec!["0", "1", "2", "3"]
                .into_iter()
                .map(String::from)
                .collect(),
            "more below · "
        )
    );
    assert_eq!(max.get(), 6);
    let (shown, stands) = page_window(body.clone(), 4, 100, &max);
    assert_eq!(text(&shown)[0], "6");
    assert_eq!(stands, "end · ");
    let (_, stands) = page_window(body, 20, 0, &max);
    assert_eq!((stands, max.get()), ("", 0));
}

#[test]
fn a_theme_paints_the_page_and_the_terminal_keeps_its_own() {
    let paper = Chrome::themed(Theme::PAPER);
    assert_eq!(paper.page().bg, Some(Color::Rgb(246, 245, 242)));
    assert_eq!(paper.page().fg, Some(Color::Rgb(30, 30, 32)));
    assert_eq!(paper.hint().fg, Some(Color::Rgb(104, 104, 104)));
    let sel = paper.selected();
    assert_eq!((sel.bg, sel.fg), (paper.page().fg, paper.page().bg));
    assert!(sel.add_modifier.contains(Modifier::BOLD));

    let term = Chrome::TERMINAL;
    assert_eq!(term, Chrome::default());
    assert_eq!(term.page(), Style::default());
    assert_eq!(term.hint().fg, Some(Color::DarkGray));
    assert!(term.selected().add_modifier.contains(Modifier::REVERSED));
    for c in [paper, term] {
        assert!(c.bold().add_modifier.contains(Modifier::BOLD));
    }
}

#[test]
fn a_popup_too_short_for_its_lines_keeps_its_keys_on_the_bottom_row() {
    let mut t = Terminal::new(TestBackend::new(30, 6)).unwrap();
    let lines: Vec<Line<'_>> = (0..6)
        .map(|i| Line::from(format!("line {i}")))
        .chain(std::iter::once(Line::from("q quit")))
        .collect();
    t.draw(|f| Chrome::themed(Theme::PAPER).popup(f, f.area(), lines, 20, At::Middle))
        .unwrap();
    assert!(row(&t, 1).contains("line 0"));
    assert!(row(&t, 4).contains("q quit"), "{}", row(&t, 4));
    assert!(row(&t, 0).contains('┌') && row(&t, 5).contains('└'));
}

#[test]
fn a_toast_sits_at_the_top() {
    let mut t = Terminal::new(TestBackend::new(30, 10)).unwrap();
    t.draw(|f| Chrome::TERMINAL.toast(f, f.area(), "saved"))
        .unwrap();
    assert!(row(&t, 2).contains("saved"), "{}", row(&t, 2));
}
