use super::*;

use Action::{Detach, Switch};

fn feed(input: &[u8]) -> (Vec<u8>, Option<Action>) {
    Chord::new(Key::DETACH, Key::SWITCH).feed(input)
}

/// A chord with `detach` and the default switch key.
fn detaching(detach: Key) -> Chord {
    Chord::new(detach, Key::SWITCH)
}

fn key(s: &str) -> Key {
    Key::parse(s).unwrap_or_else(|| panic!("{s:?} is a key"))
}

#[test]
fn legacy_ctrl_alt_d_detaches_and_keeps_what_came_before() {
    assert_eq!(feed(b"ab\x1b\x04cd"), (b"ab".to_vec(), Some(Detach)));
    assert_eq!(feed(b"\x1b\x04"), (Vec::new(), Some(Detach)));
}

#[test]
fn csi_u_chord_detaches() {
    assert_eq!(feed(b"x\x1b[100;7u"), (b"x".to_vec(), Some(Detach)));
    // With the event type: press and repeat.
    assert_eq!(feed(b"\x1b[100;7:1u").1, Some(Detach));
    assert_eq!(feed(b"\x1b[100;7:2u").1, Some(Detach));
    // Num Lock and Caps Lock do not count as modifiers.
    assert_eq!(feed(b"\x1b[100;135u").1, Some(Detach));
    assert_eq!(feed(b"\x1b[100;71u").1, Some(Detach));
    // An alternate key and associated text.
    assert_eq!(feed(b"\x1b[100:68;7;100u").1, Some(Detach));
}

#[test]
fn modify_other_keys_chord_detaches() {
    assert_eq!(feed(b"\x1b[27;7;100~").1, Some(Detach));
}

#[test]
fn chord_release_is_swallowed() {
    assert_eq!(feed(b"a\x1b[100;7:3ub"), (b"ab".to_vec(), None));
}

#[test]
fn other_keys_pass_unchanged() {
    for keys in [
        &b"\x04"[..],              // Ctrl+D
        b"\x1bd",                  // Alt+D
        b"\x1b[100;5u",            // Ctrl+D
        b"\x1b[100;3u",            // Alt+D
        b"\x1b[100;8u",            // Ctrl+Alt+Shift+D
        b"\x1b[100u",              // d alone
        b"\x1b[97;7u",             // Ctrl+Alt+A
        b"\x1b[A\x1b[1;5C",        // arrows
        b"\x1b[<0;10;5M",          // SGR mouse
        b"\x1b[M !!",              // legacy mouse
        b"\x1b[200~text\x1b[201~", // a paste
        b"\x1bx",                  // Alt+X
        b"\x1b\x01",               // Ctrl+Alt+A
        b"\x1c",                   // Ctrl+\
        b"\x1b[27;5;100~",         // modifyOtherKeys Ctrl+D
    ] {
        assert_eq!(feed(keys), (keys.to_vec(), None), "{keys:?}");
    }
}

#[test]
fn a_sequence_split_between_reads_is_held_then_found() {
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"a\x1b[10"), (b"a".to_vec(), None));
    assert!(c.holding());
    assert_eq!(c.feed(b"0;7u"), (Vec::new(), Some(Detach)));

    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b"), (Vec::new(), None));
    assert_eq!(c.feed(b"\x04"), (Vec::new(), Some(Detach)));

    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b"), (Vec::new(), None));
    assert_eq!(c.feed(b"[1;5"), (Vec::new(), None));
    assert_eq!(c.feed(b"C"), (b"\x1b[1;5C".to_vec(), None));
    assert!(!c.holding());
}

#[test]
fn a_lone_escape_is_given_up_on() {
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b"), (Vec::new(), None));
    assert!(c.holding());
    assert_eq!(c.flush(), b"\x1b".to_vec());
    assert!(!c.holding());
}

#[test]
fn an_overlong_sequence_is_not_held() {
    let mut long = b"\x1b[".to_vec();
    long.extend(std::iter::repeat_n(b'1', 40));
    let (out, chord) = Chord::new(Key::DETACH, Key::SWITCH).feed(&long);
    assert_eq!(chord, None);
    assert_eq!(out, long);
}

#[test]
fn keys_are_read_and_shown() {
    assert_eq!(Key::DETACH.display(), "Ctrl+Alt+D");
    assert_eq!(key("ctrl+alt+d"), Key::DETACH);
    assert_eq!(key("Control-Option-D"), Key::DETACH);
    assert_eq!(key("ctrl+\\").display(), "Ctrl+\\");
    assert_eq!(key("ctrl+alt+]").display(), "Ctrl+Alt+]");
    assert_eq!(key("ctrl-k").display(), "Ctrl+K");
    // Ctrl is required, and the key must have a control byte.
    for bad in [
        "d",
        "alt+d",
        "ctrl+",
        "ctrl+1",
        "ctrl+shift+d",
        "ctrl+[",
        "",
    ] {
        assert_eq!(Key::parse(bad), None, "{bad:?}");
    }
}

#[test]
fn another_key_is_found_in_every_encoding() {
    let mut plain = detaching(key("ctrl+\\"));
    assert_eq!(plain.feed(b"ab\x1ccd"), (b"ab".to_vec(), Some(Detach)));
    assert_eq!(
        detaching(key("ctrl+\\")).feed(b"\x1b[92;5u").1,
        Some(Detach)
    );
    assert_eq!(
        detaching(key("ctrl+\\")).feed(b"\x1b[27;5;92~").1,
        Some(Detach)
    );
    // The default no longer detaches.
    assert_eq!(detaching(key("ctrl+\\")).feed(b"\x1b\x04").1, None);

    let mut alt = detaching(key("ctrl+alt+k"));
    assert_eq!(alt.feed(b"\x1b\x0b").1, Some(Detach));
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x1b[107;7u").1,
        Some(Detach)
    );
    // Ctrl+K alone is the program's.
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x0b"),
        (b"\x0b".to_vec(), None)
    );
}

#[test]
fn the_switch_key_is_found_in_every_encoding() {
    assert_eq!(feed(b"ab\x1b\x13cd"), (b"ab".to_vec(), Some(Switch)));
    assert_eq!(feed(b"\x1b[115;7u").1, Some(Switch));
    assert_eq!(feed(b"\x1b[27;7;115~").1, Some(Switch));
    // Its release is swallowed too; Ctrl+S and Alt+S are the program's.
    assert_eq!(feed(b"a\x1b[115;7:3ub"), (b"ab".to_vec(), None));
    for keys in [&b"\x13"[..], b"\x1bs", b"\x1b[115;5u", b"\x1b[115;3u"] {
        assert_eq!(feed(keys), (keys.to_vec(), None), "{keys:?}");
    }
    // Split between reads.
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b[11"), (Vec::new(), None));
    assert_eq!(c.feed(b"5;7u"), (Vec::new(), Some(Switch)));
}

#[test]
fn another_switch_key_and_the_detach_key_winning_a_tie() {
    let mut c = Chord::new(Key::DETACH, key("ctrl+]"));
    assert_eq!(c.feed(b"x\x1dy"), (b"x".to_vec(), Some(Switch)));
    assert_eq!(c.feed(b"\x1b[93;5u").1, Some(Switch));
    // The default switch key is the program's now.
    assert_eq!(c.feed(b"\x1b\x13").1, None);
    assert_eq!(Key::SWITCH.display(), "Ctrl+Alt+S");
    // The same key for both: it detaches.
    let mut same = Chord::new(Key::SWITCH, Key::SWITCH);
    assert_eq!(same.feed(b"\x1b\x13").1, Some(Detach));
    assert_eq!(same.feed(b"\x1b[115;7u").1, Some(Detach));
}
