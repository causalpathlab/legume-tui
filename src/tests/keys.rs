//! List keys, the wheel, stray chords and the clipboard's encoding.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn mouse(kind: MouseEventKind) -> MouseEvent {
    MouseEvent {
        kind,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    }
}

#[test]
fn list_keys_move_by_a_row_a_page_or_to_an_end_and_stop_there() {
    let mv = |code, row| list_move(&key(code), row, 25, false);
    assert_eq!(mv(KeyCode::Down, 3), Some(4));
    assert_eq!(mv(KeyCode::Up, 0), Some(0));
    assert_eq!(mv(KeyCode::PageDown, 20), Some(24));
    assert_eq!(mv(KeyCode::PageUp, 12), Some(2));
    assert_eq!(mv(KeyCode::Home, 9), Some(0));
    assert_eq!(mv(KeyCode::End, 9), Some(24));
    assert_eq!(list_move(&key(KeyCode::End), 0, 0, false), Some(0));
    assert_eq!(mv(KeyCode::Char('j'), 3), None, "j/k only when asked");
    assert_eq!(list_move(&key(KeyCode::Char('j')), 3, 25, true), Some(4));
    assert_eq!(list_move(&key(KeyCode::Char('k')), 3, 25, true), Some(2));
    let ctrl_down = KeyEvent::new(KeyCode::Down, KeyModifiers::CONTROL);
    assert_eq!(
        list_delta(&ctrl_down, true),
        None,
        "a chord is the caller's"
    );
    assert_eq!(moved(3, -5, 9), 0);
    assert_eq!(moved(3, 50, 9), 9);
}

#[test]
fn a_step_moves_the_selection_or_says_the_key_is_not_its() {
    let mut sel = 5;
    assert!(list_step(&mut sel, 8, &key(KeyCode::PageDown), false));
    assert_eq!(sel, 7);
    assert!(!list_step(&mut sel, 8, &key(KeyCode::Enter), false));
    assert_eq!(sel, 7);
}

#[test]
fn the_wheel_is_the_arrows_and_a_left_click_counts() {
    assert_eq!(
        wheel_key(&mouse(MouseEventKind::ScrollDown)),
        Some(key(KeyCode::Down))
    );
    assert_eq!(
        wheel_key(&mouse(MouseEventKind::ScrollUp)),
        Some(key(KeyCode::Up))
    );
    assert_eq!(wheel_key(&mouse(MouseEventKind::Moved)), None);
    assert!(is_click_or_wheel(&mouse(MouseEventKind::Down(
        MouseButton::Left
    ))));
    assert!(!is_click_or_wheel(&mouse(MouseEventKind::Down(
        MouseButton::Right
    ))));
}

#[test]
fn no_chord_or_shift_enter_passes_for_its_plain_key_but_the_run_key_does() {
    let held = |code, m| is_stray(&KeyEvent::new(code, m));
    assert!(held(KeyCode::Char('s'), KeyModifiers::CONTROL));
    assert!(held(KeyCode::Enter, KeyModifiers::SHIFT));
    assert!(held(KeyCode::Enter, KeyModifiers::ALT));
    assert!(!held(KeyCode::Char('S'), KeyModifiers::SHIFT));
    assert!(!held(KeyCode::Enter, KeyModifiers::NONE));
    assert!(!held(KeyCode::Char('r'), KeyModifiers::CONTROL));
}

#[test]
fn the_clipboard_gets_standard_padded_base64() {
    assert_eq!(base64(b""), "");
    assert_eq!(base64(b"f"), "Zg==");
    assert_eq!(base64(b"fo"), "Zm8=");
    assert_eq!(base64(b"foo"), "Zm9v");
    assert_eq!(base64("héllo wörld".as_bytes()), "aMOpbGxvIHfDtnJsZA==");
    assert_eq!(osc52("hi"), "\x1b]52;c;aGk=\x07");
}
