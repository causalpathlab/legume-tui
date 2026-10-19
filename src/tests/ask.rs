//! Asking the terminal over a pty pair: the answer read to its end and no
//! further, nothing left reading when none comes, and the keyboard's
//! reply told from the others.

use super::*;
use std::fs::File;
use std::os::fd::OwnedFd;

/// A pseudo-terminal, its program's end raw as a TUI has it: the
/// terminal's end, and the program's.
fn raw_pty() -> (File, OwnedFd) {
    let (terminal, program) = crate::pty::open().unwrap();
    let mut t = rustix::termios::tcgetattr(&program).unwrap();
    t.make_raw();
    rustix::termios::tcsetattr(&program, rustix::termios::OptionalActions::Now, &t).unwrap();
    (terminal, program)
}

/// The kitty keyboard query and the status report, asked of `program`.
fn ask_keyboard(program: &OwnedFd, wait: Duration) -> (bool, bool, Vec<u8>) {
    let mut reply = KeyboardReply::default();
    let mut sent = Vec::new();
    let query = format!("{KEYBOARD}{STATUS}");
    let ended = ask(program.as_fd(), &mut sent, query.as_bytes(), wait, |b| {
        reply.push(b)
    })
    .unwrap();
    (ended, reply.heard, sent)
}

/// What the next ordinary read of `fd` gets, waiting up to two seconds.
fn next_read(fd: &OwnedFd) -> Vec<u8> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let wait = Timespec::try_from(Duration::from_secs(2)).unwrap();
    let mut fds = [PollFd::new(fd, PollFlags::IN)];
    assert_eq!(poll(&mut fds, Some(&wait)).unwrap(), 1, "nothing to read");
    let mut buf = [0u8; 64];
    let n = rustix::io::read(fd, &mut buf).unwrap();
    buf[..n].to_vec()
}

#[test]
fn a_keyboard_reply_is_heard_and_the_answer_read_no_further() {
    let (mut terminal, program) = raw_pty();
    terminal.write_all(b"\x1b[?1u\x1b[0nq").unwrap();
    let (ended, heard, sent) = ask_keyboard(&program, Duration::from_secs(2));
    assert!(ended && heard);
    assert_eq!(sent, b"\x1b[?u\x1b[5n", "the query went out");
    assert_eq!(next_read(&program), b"q", "the key is left for the program");
}

#[test]
fn other_replies_are_not_taken_for_the_keyboard() {
    let (mut terminal, program) = raw_pty();
    // The device attributes, a cell size and a status: no keyboard.
    terminal
        .write_all(b"\x1b[?62;4;22c\x1b[6;20;10t\x1b[0n")
        .unwrap();
    let (ended, heard, _) = ask_keyboard(&program, Duration::from_secs(2));
    assert!(ended && !heard);
}

#[test]
fn a_terminal_that_never_answers_leaves_no_read_behind() {
    let (mut terminal, program) = raw_pty();
    let before = rustix::termios::tcgetattr(&program).unwrap();
    let start = Instant::now();
    let (ended, heard, _) = ask_keyboard(&program, Duration::from_millis(200));
    assert!(!ended && !heard);
    assert!(start.elapsed() >= Duration::from_millis(200));
    assert_eq!(
        rustix::termios::tcgetattr(&program).unwrap().local_modes,
        before.local_modes,
        "the terminal's modes are back"
    );
    terminal.write_all(b"keys").unwrap();
    assert_eq!(next_read(&program), b"keys");
}

#[test]
fn the_keyboard_reply_is_found_byte_by_byte() {
    let heard = |bytes: &[u8]| {
        let mut r = KeyboardReply::default();
        bytes.iter().for_each(|&b| r.push(b));
        r.heard
    };
    assert!(heard(b"\x1b[?0u"));
    assert!(heard(b"\x1b[?31u"));
    assert!(heard(b"x\x1b[?62c\x1b[?1u"), "after another reply");
    assert!(!heard(b"\x1b[?62;4c"));
    assert!(!heard(b"\x1b[1u"), "no question mark");
    assert!(!heard(b"\x1b[?;u"));
}
