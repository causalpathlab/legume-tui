//! The test terminal's answers to what a program asks it.

use super::*;

/// What `replies` answers to `out`, written in the pieces `cut` makes,
/// and what is left unread at the end.
fn answered(replies: &Replies, out: &[u8], cut: &[usize]) -> (String, usize) {
    let mut unread = Vec::new();
    let mut said = Vec::new();
    let mut from = 0;
    for &to in cut.iter().chain([&out.len()]) {
        unread.extend_from_slice(&out[from..to]);
        from = to;
        let (reply, read) = replies.to(&unread);
        said.extend(reply);
        unread.drain(..read);
    }
    (String::from_utf8(said).unwrap(), unread.len())
}

/// The graphics query and the keyboard's, as the program sends them.
const QUERY: &[u8] =
    b"\x1b_Gi=31,s=1,v=1,a=q,t=d,f=24;AAAA\x1b\\\x1b[c\x1b[16t\x1b]11;?\x07\x1b[?u\x1b[5n";

#[test]
fn every_terminal_answers_the_attributes_and_the_status() {
    let (said, left) = answered(&Replies::plain(), QUERY, &[]);
    assert_eq!(said, "\x1b[?62;22c\x1b[0n");
    assert_eq!(left, 0);
    let (said, _) = answered(&Replies::silent(), QUERY, &[]);
    assert_eq!(said, "", "never a word");
}

#[test]
fn a_richer_terminal_answers_each_query_in_turn_however_it_comes() {
    let replies = Replies::plain()
        .kitty()
        .sixel()
        .keyboard(1)
        .cell(10, 20)
        .background(255, 0, 128);
    let whole = "\x1b_Gi=31;OK\x1b\\\x1b[?62;4;22c\x1b[6;20;10t\
                 \x1b]11;rgb:ffff/0000/8080\x07\x1b[?1u\x1b[0n";
    assert_eq!(answered(&replies, QUERY, &[]), (whole.to_string(), 0));
    // Cut anywhere, mid-query: the same answers.
    for cut in 1..QUERY.len() {
        assert_eq!(answered(&replies, QUERY, &[cut]).0, whole, "cut at {cut}");
    }
}

#[test]
fn the_rest_of_the_output_is_not_a_query() {
    let drawn = b"\x1b[?1049h\x1b[2J\x1b[1;1Hhello\x1b_Ga=T,i=7;AAAA\x1b\\\x1b[0m";
    let (said, left) = answered(&Replies::plain().kitty(), drawn, &[]);
    assert_eq!(said, "", "a picture is not the kitty query");
    assert_eq!(left, 0);
    // Queries wrapped for tmux are its to answer, not the terminal's.
    let wrapped = b"\x1bPtmux;\x1b\x1b[c\x1b\x1b[5n\x1b\x1b_Gi=31,a=q;AAAA\x1b\x1b\\\x1b\\";
    let (said, left) = answered(&Replies::plain().kitty(), wrapped, &[]);
    assert_eq!(said, "", "nothing inside a passthrough answered");
    assert_eq!(left, 0);
    for cut in 1..wrapped.len() {
        let (said, _) = answered(&Replies::plain(), wrapped, &[cut]);
        assert_eq!(said, "", "cut at {cut}");
    }
    // A sequence cut short is kept for the rest.
    let (_, left) = answered(&Replies::plain(), b"text\x1b[", &[]);
    assert_eq!(left, 2);
}

#[test]
fn the_tool_runs_without_the_test_runners_backtrace_setting() {
    // A runner with RUST_BACKTRACE set (as CI does) would have a tool's
    // error end in a trace that scrolls the error off the screen.
    let cmd = Private::new("tool").command();
    let envs: Vec<_> = cmd.get_envs().collect();
    for name in ["RUST_BACKTRACE", "RUST_LIB_BACKTRACE"] {
        assert!(
            envs.contains(&(std::ffi::OsStr::new(name), None)),
            "{name} not removed: {envs:?}"
        );
    }
}
