//! Text broken to fit: key hints whole, words under their key, capped
//! lines.

use super::*;
use ratatui::style::{Color, Modifier};

#[test]
fn key_hints_break_between_entries_only() {
    assert_eq!(
        wrap_keys("↑↓ move   enter open   q quit", 20),
        ["↑↓ move", "enter open   q quit"]
    );
    assert_eq!(wrap_keys("a b   c d", 80), ["a b   c d"]);
}

#[test]
fn words_wrap_and_a_long_word_has_a_line_of_its_own() {
    assert_eq!(wrap_words("one two three", 7), ["one two", "three"]);
    assert_eq!(wrap_words("abcdefghij x", 4), ["abcdefghij", "x"]);
    assert_eq!(wrap_words("", 4), [""]);
}

#[test]
fn a_keys_words_wrap_under_themselves() {
    assert_eq!(
        key_lines(2, "q", 5, "quit the screen and go back", 20),
        ["  q     quit the", "        screen and", "        go back"]
    );
}

#[test]
fn a_footer_wraps_without_parting_a_key_from_its_words() {
    let key = Style::default()
        .fg(Color::Yellow)
        .add_modifier(Modifier::BOLD);
    let line = Line::from(vec![
        Span::raw("view "),
        Span::styled("q", key),
        Span::raw(" quit "),
        Span::styled("enter", key),
        Span::raw(" open "),
    ]);
    let short = line.clone();
    assert_eq!(wrap_help(short, 80, key).len(), 1);
    let rows = wrap_help(line, 12, key);
    let text: Vec<String> = rows.iter().map(ToString::to_string).collect();
    assert_eq!(text, ["view q quit ", " enter open "]);
}

#[test]
fn capped_lines_end_in_an_ellipsis() {
    assert_eq!(wrap_at_most("a bb ccc dddd", 4, 9), ["a bb", "ccc", "dddd"]);
    assert_eq!(wrap_at_most("a bb ccc dddd", 4, 2), ["a bb", "ccc…"]);
    assert_eq!(wrap_at_most("toolongword", 4, 3), ["tool"]);
    assert_eq!(wrap_at_most("", 4, 3), [""]);
    assert!(wrap_at_most("a", 4, 0).is_empty());
}

#[test]
fn words_are_cut_whole_when_they_do_not_fit() {
    assert_eq!(cut_words("fits here", 9), "fits here");
    assert_eq!(cut_words("one two three", 9), "one two…");
    assert_eq!(cut_words("abcdefghij", 5), "abcd…");
}
