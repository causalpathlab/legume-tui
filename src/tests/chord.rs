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
fn ctrl_bracket_detaches_and_keeps_what_came_before() {
    assert_eq!(feed(b"ab\x1dcd"), (b"ab".to_vec(), Some(Detach)));
    assert_eq!(feed(b"\x1d"), (Vec::new(), Some(Detach)));
    assert_eq!(feed(&Key::DETACH.bytes()), (Vec::new(), Some(Detach)));
}

#[test]
fn csi_u_key_detaches() {
    assert_eq!(feed(b"x\x1b[93;5u"), (b"x".to_vec(), Some(Detach)));
    // With the event type: press and repeat.
    assert_eq!(feed(b"\x1b[93;5:1u").1, Some(Detach));
    assert_eq!(feed(b"\x1b[93;5:2u").1, Some(Detach));
    // Num Lock and Caps Lock do not count as modifiers.
    assert_eq!(feed(b"\x1b[93;133u").1, Some(Detach));
    assert_eq!(feed(b"\x1b[93;69u").1, Some(Detach));
    // An alternate key and associated text.
    assert_eq!(feed(b"\x1b[93:125;5;29u").1, Some(Detach));
}

#[test]
fn modify_other_keys_key_detaches() {
    assert_eq!(feed(b"\x1b[27;5;93~").1, Some(Detach));
}

#[test]
fn key_release_is_swallowed() {
    assert_eq!(feed(b"a\x1b[93;5:3ub"), (b"ab".to_vec(), None));
}

#[test]
fn other_keys_pass_unchanged() {
    for keys in [
        &b"\x04"[..],              // Ctrl+D
        b"\x1b\x04",               // Ctrl+Alt+D, the old detach key
        b"\x1b\x13",               // Ctrl+Alt+S, the old switch key
        b"\x1b[100;7u",            // Ctrl+Alt+D
        b"]",                      // ] alone
        b"\x1b[93u",               // ] alone
        b"\x1b[93;3u",             // Alt+]
        b"\x1b[93;7u",             // Ctrl+Alt+]
        b"\x1b[93;6u",             // Ctrl+Shift+]
        b"\x1b[92;7u",             // Ctrl+Alt+\
        b"\x1b[A\x1b[1;5C",        // arrows
        b"\x1b[<0;10;5M",          // SGR mouse
        b"\x1b[M !!",              // legacy mouse
        b"\x1b[200~text\x1b[201~", // a paste
        b"\x1bx",                  // Alt+X
        b"\x1b[27;5;100~",         // modifyOtherKeys Ctrl+D
        b"\x1b[27;7;93~",          // modifyOtherKeys Ctrl+Alt+]
    ] {
        assert_eq!(feed(keys), (keys.to_vec(), None), "{keys:?}");
    }
}

#[test]
fn a_sequence_split_between_reads_is_held_then_found() {
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"a\x1b[9"), (b"a".to_vec(), None));
    assert!(c.holding());
    assert_eq!(c.feed(b"3;5u"), (Vec::new(), Some(Detach)));

    // An Esc held, then the key: the Esc is the program's.
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b"), (Vec::new(), None));
    assert_eq!(c.feed(b"\x1d"), (b"\x1b".to_vec(), Some(Detach)));

    // An Alt key held, then its control byte.
    let mut c = detaching(key("ctrl+alt+d"));
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
    assert_eq!(Key::DETACH.display(), "ctrl+]");
    assert_eq!(Key::SWITCH.display(), "ctrl+\\");
    assert_eq!(key("ctrl+]"), Key::DETACH);
    assert_eq!(key("Control-\\"), Key::SWITCH);
    assert_eq!(key("ctrl+alt+d").display(), "ctrl+alt+d");
    assert_eq!(key("Control-Option-D"), key("ctrl+alt+d"));
    assert_eq!(key("ctrl+alt+]").display(), "ctrl+alt+]");
    assert_eq!(key("ctrl-K").display(), "ctrl+k");
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
fn a_key_types_as_its_plain_encoding() {
    assert_eq!(Key::DETACH.bytes(), b"\x1d");
    assert_eq!(Key::SWITCH.bytes(), b"\x1c");
    assert_eq!(key("ctrl+alt+d").bytes(), b"\x1b\x04");
    assert_eq!(key("ctrl+k").bytes(), b"\x0b");
    for k in [Key::DETACH, Key::SWITCH, key("ctrl+alt+k")] {
        let mut c = Chord::new(k, k);
        assert_eq!(c.feed(&k.bytes()), (Vec::new(), Some(Detach)), "{k:?}");
    }
}

#[test]
fn another_key_is_found_in_every_encoding() {
    let mut alt = detaching(key("ctrl+alt+k"));
    assert_eq!(alt.feed(b"ab\x1b\x0bcd"), (b"ab".to_vec(), Some(Detach)));
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x1b[107;7u").1,
        Some(Detach)
    );
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x1b[27;7;107~").1,
        Some(Detach)
    );
    // Ctrl+K alone is the program's, and so is the default now.
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x0b"),
        (b"\x0b".to_vec(), None)
    );
    assert_eq!(
        detaching(key("ctrl+alt+k")).feed(b"\x1d"),
        (b"\x1d".to_vec(), None)
    );
}

#[test]
fn the_switch_key_is_found_in_every_encoding() {
    assert_eq!(feed(b"ab\x1ccd"), (b"ab".to_vec(), Some(Switch)));
    assert_eq!(feed(b"\x1b[92;5u").1, Some(Switch));
    assert_eq!(feed(b"\x1b[92;5:2u").1, Some(Switch));
    assert_eq!(feed(b"\x1b[27;5;92~").1, Some(Switch));
    // Its release is swallowed too; \ and Alt+\ are the program's.
    assert_eq!(feed(b"a\x1b[92;5:3ub"), (b"ab".to_vec(), None));
    for keys in [&b"\\"[..], b"\x1b\\", b"\x1b[92u", b"\x1b[92;3u"] {
        assert_eq!(feed(keys), (keys.to_vec(), None), "{keys:?}");
    }
    // Split between reads.
    let mut c = Chord::new(Key::DETACH, Key::SWITCH);
    assert_eq!(c.feed(b"\x1b[9"), (Vec::new(), None));
    assert_eq!(c.feed(b"2;5u"), (Vec::new(), Some(Switch)));
}

#[test]
fn another_switch_key_and_the_detach_key_winning_a_tie() {
    let mut c = Chord::new(Key::DETACH, key("ctrl+alt+s"));
    assert_eq!(c.feed(b"x\x1b\x13y"), (b"x".to_vec(), Some(Switch)));
    assert_eq!(c.feed(b"\x1b[115;7u").1, Some(Switch));
    // The default switch key is the program's now.
    assert_eq!(c.feed(b"\x1c"), (b"\x1c".to_vec(), None));
    // The same key for both: it detaches.
    let mut same = Chord::new(Key::SWITCH, Key::SWITCH);
    assert_eq!(same.feed(b"\x1c").1, Some(Detach));
    assert_eq!(same.feed(b"\x1b[92;5u").1, Some(Detach));
}
