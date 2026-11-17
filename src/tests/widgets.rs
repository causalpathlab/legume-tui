//! The shared pieces: where things were drawn for the mouse, pop-ups,
//! buttons and tabs, bars, marks, the session keys and the menu.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// What `draw` puts on a `w` x `h` screen, as text, a row a line.
fn screen(w: u16, h: u16, draw: impl FnOnce(&mut Frame)) -> String {
    let mut t = Terminal::new(TestBackend::new(w, h)).unwrap();
    t.draw(draw).unwrap();
    let buf = t.backend().buffer();
    (0..h)
        .map(|y| (0..w).map(|x| buf[(x, y)].symbol()).collect::<String>())
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn the_last_drawn_at_a_cell_is_the_one_clicked() {
    let hits = Hits::default();
    hits.add(Rect::new(0, 0, 10, 10), "under");
    hits.add(Rect::new(2, 2, 2, 1), "over");
    assert_eq!(hits.at(2, 2), Some("over"));
    assert_eq!(hits.at(5, 5), Some("under"));
    assert_eq!(hits.at(10, 0), None);
    assert_eq!(hits.spot(&"over"), Some((2, 2)));
    hits.clear();
    assert!(hits.is_empty());
    assert_eq!(hits.at(2, 2), None);
}

#[test]
fn a_list_s_rows_name_their_items_as_far_as_the_area_and_list_go() {
    let hits = Hits::default();
    hits.rows(Rect::new(1, 4, 8, 3), 5, 7, |i| i);
    assert_eq!(hits.at(1, 4), Some(5));
    assert_eq!(hits.at(8, 5), Some(6));
    assert_eq!(hits.at(1, 6), None, "the list ended");
    hits.rows(Rect::new(0, 0, 8, 2), 0, 9, |i| i);
    assert_eq!(hits.at(0, 2), None, "the area ended");
    // A draw's rows set at once, as a view that builds them first does.
    hits.set(vec![(Rect::new(0, 0, 4, 1), 7)]);
    assert_eq!((hits.at(0, 0), hits.at(1, 4)), (Some(7), None));
}

#[test]
fn centred_and_shrunk_to_fit() {
    let area = Rect::new(10, 5, 21, 11);
    assert_eq!(centered(area, 5, 3), Rect::new(18, 9, 5, 3));
    assert_eq!(centered(area, 50, 50), area);
}

#[test]
fn a_button_pop_up_puts_its_buttons_first_and_where_they_are_clicked() {
    let hits = Hits::default();
    let look = Look::terminal();
    let text = screen(60, 12, |f| {
        let body = vec![Line::raw("are you sure?")];
        let inner = button_popup(
            f,
            f.area(),
            "Quit",
            &[("yes", 1), ("no", 2)],
            body,
            &hits,
            &look,
        );
        assert_eq!(inner.width, 38, "at least 40 wide");
    });
    let rows: Vec<&str> = text.lines().collect();
    let top = rows.iter().position(|r| r.contains("Quit")).unwrap();
    assert!(rows[top].contains('╭'), "{text}");
    assert!(rows[top + 1].contains("[ yes ]  [ no ]"), "{text}");
    assert!(rows[top + 3].contains("are you sure?"), "{text}");
    let (x, y) = hits.spot(&2).unwrap();
    assert_eq!(usize::from(y), top + 1);
    let row: String = rows[top + 1].chars().skip(usize::from(x)).take(6).collect();
    assert_eq!(row, "[ no ]");
    assert_eq!(hits.at(x + 5, y), Some(2));
    assert_eq!(hits.at(x + 6, y), None);
}

#[test]
fn a_tab_line_names_each_tab_and_its_button() {
    let hits = Hits::default();
    let look = Look::terminal();
    let line = tab_bar(
        Rect::new(3, 1, 40, 1),
        &["one", "two"],
        1,
        Some(("go", 9)),
        &hits,
        |i| i,
        &look,
    );
    let text: String = line.spans.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, " one  two   [ go ]");
    assert_eq!(line.spans[2].style, look.bold, "the active tab");
    assert_eq!(line.spans[1].style, look.hint);
    assert_eq!(hits.at(4, 1), Some(0));
    assert_eq!(hits.at(9, 1), Some(1));
    assert_eq!(hits.spot(&9), Some((3 + 12, 1)));
}

#[test]
fn a_bar_is_full_only_when_done() {
    assert_eq!(filled(2, 3, 10), 6);
    assert_eq!(filled(999, 1000, 10), 9);
    assert_eq!(filled(5, 0, 10), 0, "nothing to count");
    assert_eq!(filled(7, 5, 10), 10, "past the end is full");
    assert_eq!(bar(1, 2, 4), "██░░");
    let look = Look::terminal();
    let [on, off] = bar_spans(1, 4, 8, look.bold, &look);
    assert_eq!(
        (on.content.as_ref(), off.content.as_ref()),
        ("██", "░░░░░░")
    );
    assert_eq!(off.style, look.hint);
}

#[test]
fn clock_digits_and_hidden_marks() {
    assert_eq!(
        clock(std::time::Duration::from_secs(3 * 3600 + 61)),
        "03:01:01"
    );
    assert_eq!(
        [digits(0), digits(9), digits(10), digits(-100)],
        [1, 1, 2, 4]
    );
    assert_eq!(digits(i64::MIN), 20);
    assert_eq!(hidden_marks(2, 5), "‹2 5›");
    assert_eq!(hidden_marks(0, 5), "5›");
    assert_eq!(hidden_marks(2, 0), "‹2");
    assert_eq!(hidden_marks(0, 0), "");
}

#[test]
fn session_keys_cover_the_end_of_the_title_row() {
    let look = Look::terminal();
    let text = screen(30, 2, |f| {
        f.render_widget(Paragraph::new("x".repeat(30)), f.area());
        draw_session_keys(f, f.area(), &[("^b d", "detach")], &look);
    });
    let top = text.lines().next().unwrap();
    assert!(top.ends_with(" ^b d detach  "), "{top}");
    assert!(top.starts_with("xxxx"), "{top}");
    let none: [(&str, &str); 0] = [];
    let text = screen(10, 1, |f| draw_session_keys(f, f.area(), &none, &look));
    assert_eq!(text.trim(), "");
}

fn menu() -> Menu<u8> {
    Menu::new(
        "which?",
        numbered(vec![
            ("one".into(), "the first".into(), 1),
            ("two".into(), "the second".into(), 2),
        ]),
    )
}

#[test]
fn arrows_move_enter_or_a_key_takes_esc_cancels() {
    let mut m = menu();
    assert_eq!(m.key(KeyCode::Down), Outcome::Open);
    assert_eq!(m.sel, 1);
    assert_eq!(m.key(KeyCode::Down), Outcome::Open);
    assert_eq!(m.sel, 1, "the last stays");
    assert_eq!(m.key(KeyCode::Enter), Outcome::Chosen(2));
    assert_eq!(m.key(KeyCode::Char('1')), Outcome::Chosen(1));
    assert_eq!(m.sel, 0, "a key selects what it takes");
    assert_eq!(m.key(KeyCode::Char('9')), Outcome::Open, "no such option");
    assert_eq!(m.key(KeyCode::Esc), Outcome::Cancel);
    assert_eq!(m.key(KeyCode::Char('q')), Outcome::Cancel);
}

#[test]
fn options_are_keyed_by_digits_then_letters_never_by_the_closing_key() {
    let items = |n| {
        (0..n)
            .map(|i| (format!("CT{i}"), String::new(), i))
            .collect()
    };
    let keys: Vec<Option<char>> = numbered(items(70)).iter().map(|o| o.key).collect();
    assert!(!keys.contains(&Some(CLOSES)));
    let keyed: std::collections::BTreeSet<char> = keys.iter().flatten().copied().collect();
    assert_eq!(keyed.len(), keys.iter().flatten().count(), "no key twice");
    assert_eq!(keys[8..11], [Some('9'), Some('a'), Some('b')]);
    assert_eq!(keys[25], Some('r'), "after p comes r");
    assert_eq!(keys[34], Some('A'), "past z, capitals");
    assert_eq!(keys[69], None, "past the keys, only the arrows reach it");
    let keys: String = numbered_but(items(30), "tn")
        .iter()
        .filter_map(|o| o.key)
        .collect();
    assert!(!keys.contains(['t', 'n', 'q']), "{keys}");
}

#[test]
fn a_menu_shows_its_question_options_the_selected_detail_and_keys() {
    let look = Look::terminal();
    let mut m = menu();
    m.sel = 1;
    let mut t = Terminal::new(TestBackend::new(70, 14)).unwrap();
    t.draw(|f| m.draw(f, f.area(), &look)).unwrap();
    let buf = t.backend().buffer();
    let rows: Vec<String> = (0..14)
        .map(|y| (0..70).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let text = rows.join("\n");
    for want in [
        "which?",
        " 1  one",
        " 2  two",
        "the second",
        "esc or q cancels",
    ] {
        assert!(text.contains(want), "{want} in\n{text}");
    }
    assert!(!text.contains("the first"), "only the selected detail");
    let y = rows.iter().position(|r| r.contains(" 2  two")).unwrap();
    let x = rows[y].find(" 2  two").unwrap();
    let x = rows[y][..x].chars().count() as u16;
    assert_eq!(buf[(x + 1, y as u16)].modifier, Modifier::REVERSED);
}

#[test]
fn a_key_line_lights_the_keys_in_the_accent_in_bold() {
    let look = Look::terminal();
    let line = key_line(&[("q", "quit"), ("s", "save")], &look);
    let keys: Vec<_> = line
        .spans
        .iter()
        .filter(|s| s.style == crate::ink::highlight(&look))
        .map(|s| s.content.to_string())
        .collect();
    assert_eq!(keys, ["q", "s"]);
}
