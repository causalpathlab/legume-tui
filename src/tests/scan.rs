use super::*;

/// The first offset between sequences in `bytes`, read on from `s`.
fn at(s: &mut Scan, bytes: &[u8]) -> Option<usize> {
    s.cut(bytes).map(|(at, _)| at)
}

#[test]
fn plain_text_is_always_between_sequences() {
    let mut s = Scan::default();
    assert_eq!(at(&mut s, b"hello"), Some(0));
    assert_eq!(at(&mut s, b""), Some(0));
}

#[test]
fn a_sequence_split_across_reads_ends_where_its_tail_does() {
    let mut s = Scan::default();
    assert_eq!(at(&mut s, b"ab\x1b[1;3"), Some(0));
    // The rest of the CSI, then text.
    assert_eq!(at(&mut s, b"1mxy"), Some(2));
    // A kitty image chunk split three ways.
    assert_eq!(at(&mut s, b"\x1b_Ga=T,m=1;AAAA"), Some(0));
    assert_eq!(at(&mut s, b"BBBB"), None);
    assert_eq!(at(&mut s, b"CC\x1b"), None);
    assert_eq!(at(&mut s, b"\\z"), Some(1));
}

#[test]
fn every_kind_of_string_is_followed_to_its_end() {
    for (open, end) in [
        (&b"\x1b]0;title"[..], &b"\x07"[..]),
        (b"\x1b]8;;link", b"\x1b\\"),
        (b"\x1bPq#0;2;0;0;0#0~~", b"\x1b\\"),
        (b"\x1b_Gq=2", b"\x1b\\"),
    ] {
        let mut s = Scan::default();
        s.feed(b"x");
        assert_eq!(at(&mut s, open), Some(0), "{open:?}");
        assert_eq!(at(&mut s, b"more"), None, "{open:?}");
        let mut tail = end.to_vec();
        tail.push(b'!');
        assert_eq!(at(&mut s, &tail), Some(end.len()), "{open:?}");
    }
}

#[test]
fn cancel_ends_any_sequence() {
    let mut s = Scan::default();
    s.feed(b"\x1b_Ga=T;AAAA");
    assert_eq!(at(&mut s, b"\x18x"), Some(1));
    s.feed(b"\x1b[12");
    assert_eq!(at(&mut s, b"\x1ax"), Some(1));
}

#[test]
fn the_alternate_screen_is_followed() {
    let mut s = Scan::default();
    assert!(!s.alt());
    s.feed(b"\x1b[?1049h\x1b[2J");
    assert!(s.alt());
    // Split, and among other modes.
    s.feed(b"\x1b[?25;10");
    s.feed(b"49l");
    assert!(!s.alt());
    s.feed(b"\x1b[?47h");
    assert!(s.alt());
    // Other modes, and a non-private one, change nothing.
    s.feed(b"\x1b[?1000l\x1b[1049l\x1b[?10490l");
    assert!(s.alt());
    s.feed(b"\x1b[?1047l");
    assert!(!s.alt());
}

#[test]
fn the_cut_says_where_the_screen_is_there() {
    let mut s = Scan::default();
    // Between sequences at the start: the screen as it was.
    assert_eq!(s.cut(b"x\x1b[?1049hy"), Some((0, false)));
    assert!(s.alt());
    // A switch back split across reads: the cut is after it.
    s.feed(b"\x1b[?10");
    assert_eq!(s.cut(b"49lz"), Some((3, false)));
    s.feed(b"\x1b[?104");
    assert_eq!(s.cut(b"9hz"), Some((2, true)));
}
