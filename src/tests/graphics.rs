//! Which image protocol is picked, and the terminal query: what it reads,
//! and that it leaves nothing reading after it.

use super::*;
use crate::Tool;

#[test]
fn a_named_protocol_is_taken_and_blocks_never_ask() {
    assert_eq!(Graphics::Auto.protocol(), None);
    assert_eq!(Graphics::Kitty.protocol(), Some(ProtocolType::Kitty));
    assert_eq!(Graphics::Sixel.protocol(), Some(ProtocolType::Sixel));
    assert_eq!(Graphics::Iterm2.protocol(), Some(ProtocolType::Iterm2));
    assert_eq!(Graphics::Blocks.protocol(), Some(ProtocolType::Halfblocks));
    let session = Session::plain(&Tool::new("tool"));
    for fallback in [Fallback::Probe, Fallback::Env, Fallback::ProbeThenEnv] {
        let p = pick(&session, Graphics::Blocks, fallback);
        assert_eq!(p.protocol_type(), ProtocolType::Halfblocks);
    }
    // Named outright: taken without the terminal's word, as is.
    let p = pick(&session, Graphics::Sixel, Fallback::Env);
    assert_eq!(p.protocol_type(), ProtocolType::Sixel);
}

#[test]
fn the_protocol_comes_from_the_environment_without_asking_the_terminal() {
    let proto = |vars: &[(&str, &str)]| protocol_from_env(env(vars));
    assert_eq!(proto(&[("KITTY_WINDOW_ID", "1")]), ProtocolType::Kitty);
    assert_eq!(proto(&[("TERM", "xterm-kitty")]), ProtocolType::Kitty);
    assert_eq!(proto(&[("TERM_PROGRAM", "ghostty")]), ProtocolType::Kitty);
    assert_eq!(
        proto(&[("TERM_PROGRAM", "iTerm.app")]),
        ProtocolType::Iterm2
    );
    assert_eq!(proto(&[("LC_TERMINAL", "iTerm2")]), ProtocolType::Iterm2);
    assert_eq!(proto(&[("TERM_PROGRAM", "WezTerm")]), ProtocolType::Iterm2);
    assert_eq!(proto(&[("TERM", "foot")]), ProtocolType::Sixel);
    assert_eq!(proto(&[("TERM", "mlterm")]), ProtocolType::Sixel);
    assert_eq!(
        proto(&[("TERM_PROGRAM", "Apple_Terminal")]),
        ProtocolType::Halfblocks
    );
    assert_eq!(proto(&[]), ProtocolType::Halfblocks);
    // Through tmux a picture needs passthrough: half-blocks unless asked.
    assert_eq!(
        proto(&[("TMUX", "/tmp/tmux"), ("KITTY_WINDOW_ID", "1")]),
        ProtocolType::Halfblocks
    );
}

/// The environment `vars` as a lookup.
fn env<'a>(vars: &'a [(&'a str, &'a str)]) -> impl Fn(&str) -> Option<String> + 'a {
    move |k| {
        vars.iter()
            .find(|(n, _)| *n == k)
            .map(|(_, v)| (*v).to_string())
    }
}

/// Answered, inside tmux, the environment, and what is taken.
type Row<'a> = (
    Option<ProtocolType>,
    bool,
    &'a [(&'a str, &'a str)],
    ProtocolType,
);

#[test]
fn the_probe_takes_the_answer_unless_the_terminal_falls_short() {
    use ProtocolType::{Halfblocks, Iterm2, Kitty, Sixel};
    // (answered, inside tmux, environment): taken.
    let table: &[Row] = &[
        (Some(Kitty), false, &[], Kitty),
        (Some(Sixel), false, &[], Sixel),
        (None, false, &[], Halfblocks),
        // WezTerm: iTerm2 in place of its kitty or sixel.
        (
            Some(Kitty),
            false,
            &[("WEZTERM_EXECUTABLE", "/x"), ("TERM_PROGRAM", "WezTerm")],
            Iterm2,
        ),
        (
            Some(Sixel),
            false,
            &[("WEZTERM_EXECUTABLE", "/x")],
            Halfblocks,
        ),
        // Konsole: neither its kitty nor its sixel.
        (Some(Kitty), false, &[("KONSOLE_VERSION", "1")], Halfblocks),
        (Some(Sixel), false, &[("KONSOLE_VERSION", "1")], Halfblocks),
        (Some(Kitty), false, &[("KONSOLE_VERSION", "")], Kitty),
        // iTerm2 from the environment, when nothing better was answered.
        (None, false, &[("TERM_PROGRAM", "vscode")], Iterm2),
        (None, false, &[("TERM_PROGRAM", "mintty")], Iterm2),
        (None, false, &[("TERM_PROGRAM", "WarpTerminal.app")], Iterm2),
        (None, false, &[("LC_TERMINAL", "iTerm2")], Iterm2),
        (Some(Kitty), false, &[("TERM_PROGRAM", "vscode")], Kitty),
        // Inside tmux: the outer terminal's hint, and the list as well.
        (None, true, &[("ITERM_SESSION_ID", "w0")], Iterm2),
        (None, false, &[("ITERM_SESSION_ID", "w0")], Halfblocks),
        (
            None,
            true,
            &[("TERM_PROGRAM", "tmux"), ("LC_TERMINAL", "iTerm2")],
            Iterm2,
        ),
        (None, true, &[("TERM_PROGRAM", "tmux")], Halfblocks),
    ];
    for (answered, tmux, vars, taken) in table {
        assert_eq!(
            probe_protocol(*answered, *tmux, env(vars)),
            *taken,
            "{answered:?} tmux={tmux} {vars:?}"
        );
    }
}

#[test]
fn tmux_is_found_from_the_attached_terminals_environment() {
    assert!(tmux_from_env(env(&[("TMUX", "/tmp/tmux-1/default,1,0")])));
    assert!(tmux_from_env(env(&[("TERM", "tmux-256color")])));
    assert!(tmux_from_env(env(&[("TERM_PROGRAM", "tmux")])));
    assert!(!tmux_from_env(env(&[("TMUX", "")])));
    assert!(!tmux_from_env(env(&[("TERM", "xterm-kitty")])));
    assert!(!tmux_from_env(env(&[])));
}

#[test]
fn kitty_pictures_go_inline_over_ssh_and_through_a_file_elsewhere() {
    assert_eq!(transport_from_env(env(&[])), Transport::File);
    assert_eq!(
        transport_from_env(env(&[("SSH_CONNECTION", "10.0.0.1 1 10.0.0.2 22")])),
        Transport::Inline
    );
    assert_eq!(
        transport_from_env(env(&[("SSH_TTY", "/dev/pts/1")])),
        Transport::Inline
    );
    assert_eq!(
        transport_from_env(env(&[("SSH_CLIENT", "")])),
        Transport::File
    );
}

#[test]
fn a_tools_own_choice_is_taken_and_reprobe_says_what_changed() {
    let session = Session::plain(&Tool::new("tool"));
    // Without a terminal to ask (stdin is not one in a test): no answer.
    for wanted in [
        ProtocolType::Kitty,
        ProtocolType::Iterm2,
        ProtocolType::Halfblocks,
    ] {
        let (p, answer) = pick_protocol(&session, Some(wanted), Fallback::Env);
        assert_eq!(p.protocol_type(), wanted);
        assert_eq!(answer, None);
    }
    let blocks = Picker::halfblocks();
    let (repick, _) = reprobe_with(&session, &blocks, Graphics::Blocks, Fallback::Probe);
    assert!(matches!(repick, Repick::Same));
    let (repick, _) = reprobe_with(&session, &blocks, Graphics::Sixel, Fallback::Env);
    assert!(matches!(repick, Repick::Changed(p) if p.protocol_type() == ProtocolType::Sixel));
    #[allow(deprecated)]
    let mut kitty = Picker::from_fontsize((10, 20).into());
    kitty.set_protocol_type(ProtocolType::Kitty);
    assert!(matches!(repick_kitty(&kitty), Repick::Resend));
}

/// What pictures made with `kitty` need when kitty is picked again, at
/// the same size.
fn repick_kitty(kitty: &Picker) -> Repick {
    #[allow(deprecated)]
    let mut again = Picker::from_fontsize((10, 20).into());
    again.set_protocol_type(ProtocolType::Kitty);
    repick(kitty, again)
}

#[test]
fn a_cells_pixel_size_comes_from_the_window_size() {
    assert_eq!(cell_pixels(200, 50, 2000, 1000), Some((10, 20)));
    assert_eq!(cell_pixels(200, 50, 0, 0), None, "not reported");
    assert_eq!(cell_pixels(0, 0, 2000, 1000), None);
}

#[cfg(sessions)]
mod query {
    use super::*;
    use std::fs::File;
    use std::io::Write;
    use std::os::fd::{AsFd, OwnedFd};

    /// A pseudo-terminal, its program's end raw as a TUI has it: the
    /// terminal's end, and the program's.
    fn raw_pty() -> (File, OwnedFd) {
        let (terminal, program) = crate::host::pty(crate::Size::default()).unwrap();
        let mut t = rustix::termios::tcgetattr(&program).unwrap();
        t.make_raw();
        rustix::termios::tcsetattr(&program, rustix::termios::OptionalActions::Now, &t).unwrap();
        (terminal, program)
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
    fn a_terminal_that_never_answers_leaves_no_read_behind() {
        let (mut terminal, program) = raw_pty();
        let before = rustix::termios::tcgetattr(&program).unwrap();
        let mut sent = Vec::new();
        let start = std::time::Instant::now();
        let answer = ask(
            program.as_fd(),
            &mut sent,
            false,
            Duration::from_millis(200),
        )
        .unwrap();
        assert_eq!(answer, None, "no answer came");
        assert!(start.elapsed() >= Duration::from_millis(200));
        assert!(
            String::from_utf8_lossy(&sent).ends_with("\x1b[5n"),
            "the query went out"
        );
        assert_eq!(
            rustix::termios::tcgetattr(&program).unwrap().local_modes,
            before.local_modes,
            "the terminal's modes are back"
        );
        // Keys typed after it reach the program's next read: nothing the
        // query started is still reading.
        terminal.write_all(b"keys").unwrap();
        assert_eq!(next_read(&program), b"keys");
        terminal.write_all(b"more").unwrap();
        assert_eq!(next_read(&program), b"more");
    }

    #[test]
    fn the_answer_is_read_to_its_end_and_no_further() {
        let (mut terminal, program) = raw_pty();
        // The terminal's answer, then a key typed right after it.
        terminal
            .write_all(
                b"\x1b_Gi=31;OK\x1b\\\x1b[?62;4;22c\x1b[6;20;10t\
                  \x1b]11;rgb:ffff/0000/8080\x07\x1b[0nq",
            )
            .unwrap();
        let answer = ask(
            program.as_fd(),
            &mut Vec::new(),
            false,
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(
            answer,
            Some(Answer {
                protocol: Some(ProtocolType::Kitty),
                cell: Some((10, 20)),
                background: Some((255, 0, 128)),
                keyboard_enhancement: false,
            })
        );
        assert_eq!(next_read(&program), b"q", "the key is left for the program");
    }

    #[test]
    fn the_keyboard_is_asked_in_the_same_round_trip_but_not_through_tmux() {
        let (mut terminal, program) = raw_pty();
        terminal.write_all(b"\x1b[?62;22c\x1b[?1u\x1b[0n").unwrap();
        let mut sent = Vec::new();
        let answer = ask(program.as_fd(), &mut sent, false, Duration::from_secs(2)).unwrap();
        assert!(answer.unwrap().keyboard_enhancement);
        let sent = String::from_utf8_lossy(&sent).into_owned();
        assert!(sent.ends_with("\x1b[?u\x1b[5n"), "{sent:?}");

        terminal.write_all(b"\x1b[0n").unwrap();
        let mut sent = Vec::new();
        let answer = ask(program.as_fd(), &mut sent, true, Duration::from_secs(2)).unwrap();
        assert!(!answer.unwrap().keyboard_enhancement);
        assert!(
            !String::from_utf8_lossy(&sent).contains("[?u"),
            "not wrapped for tmux"
        );
    }

    #[test]
    fn sixel_is_taken_when_kitty_is_not_there() {
        let (mut terminal, program) = raw_pty();
        terminal.write_all(b"\x1b[?62;4c\x1b[0n").unwrap();
        let answer = ask(
            program.as_fd(),
            &mut Vec::new(),
            false,
            Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(answer.and_then(|a| a.protocol), Some(ProtocolType::Sixel));
    }
}
