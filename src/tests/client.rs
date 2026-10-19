//! The relay loop over socket pairs and a pipe standing in for the
//! terminal, and the label prompt over a pseudo-terminal pair.

use super::*;
use std::io::Write;
use std::time::Instant;

#[test]
fn resets_end_any_open_sequence_first() {
    for reset in [MODES_OFF.to_string(), end_reset(true), end_reset(false)] {
        assert!(reset.starts_with("\x1b\\\x18"), "{reset:?}");
    }
}

#[test]
fn the_alternate_screen_is_entered_by_nobody_but_the_program() {
    assert!(!MODES_OFF.contains("\x1b[?1049"));
    assert!(end_reset(true).ends_with(LEAVE));
    assert!(!end_reset(false).contains("\x1b[?1049"));
}

/// A relay's ends: the client's socket and the host's, the keys and
/// where they are typed, and the resize and stop pipes with the ends that
/// wake them.
struct Relay {
    sock: UnixStream,
    host: UnixStream,
    keys: std::io::PipeReader,
    typing: std::io::PipeWriter,
    winch: UnixStream,
    _winch_in: UnixStream,
    stop: UnixStream,
    stop_in: UnixStream,
}

impl Relay {
    fn new() -> Self {
        let (sock, host) = UnixStream::pair().unwrap();
        let (keys, typing) = std::io::pipe().unwrap();
        let (winch, _winch_in) = UnixStream::pair().unwrap();
        let (stop, stop_in) = UnixStream::pair().unwrap();
        Self {
            sock,
            host,
            keys,
            typing,
            winch,
            _winch_in,
            stop,
            stop_in,
        }
    }

    /// Run the relay on its own thread, writing the terminal's output to
    /// nowhere; how it ends.
    fn spawn(
        sock: UnixStream,
        keys: std::io::PipeReader,
        winch: UnixStream,
        stop: UnixStream,
    ) -> std::thread::JoinHandle<Outcome> {
        std::thread::spawn(move || {
            let mut screen = Scan::default();
            relay(
                &sock,
                keys.as_fd(),
                &mut Vec::new(),
                &winch,
                &stop,
                &mut screen,
            )
            .unwrap()
        })
    }
}

/// The relay, the host sending `frames` and then letting it go: what
/// reached the terminal, and the screen it left.
fn relayed(frames: &[(u8, &[u8])]) -> (Vec<u8>, bool) {
    let r = Relay::new();
    for (tag, body) in frames {
        wire::write_frame(&r.host, *tag, body).unwrap();
    }
    drop(r.host);
    let mut out = Vec::new();
    let mut screen = Scan::default();
    let outcome = relay(
        &r.sock,
        r.keys.as_fd(),
        &mut out,
        &r.winch,
        &r.stop,
        &mut screen,
    )
    .unwrap();
    assert_eq!(outcome, Outcome::Lost);
    (out, screen.alt())
}

#[test]
fn the_screen_is_followed_through_what_is_relayed() {
    // A program that has not entered the alternate screen (an early
    // error): nothing to leave.
    let (out, alt) = relayed(&[(wire::OUTPUT, b"error: no input\r\n")]);
    assert_eq!(out, b"error: no input\r\n");
    assert!(!alt);
    // One that enters it, and one that enters and leaves it.
    assert!(relayed(&[(wire::OUTPUT, b"\x1b[?1049hx")]).1);
    assert!(!relayed(&[(wire::OUTPUT, b"\x1b[?1049hx\x1b[?1049l")]).1);
    // Joining a program on the alternate screen: entered, cleared.
    let (out, alt) = relayed(&[(wire::SCREEN, &[1]), (wire::OUTPUT, b"x")]);
    assert_eq!(out, format!("{ENTER}x").as_bytes());
    assert!(alt);
    let (out, alt) = relayed(&[(wire::SCREEN, &[0]), (wire::OUTPUT, b"x")]);
    assert_eq!(out, b"x");
    assert!(!alt);
}

#[test]
fn a_host_not_reading_holds_up_neither_the_keys_nor_the_detach_key() {
    // Its socket is never read: far more is typed than it buffers.
    let Relay {
        sock,
        host: _host,
        keys,
        mut typing,
        winch,
        _winch_in,
        stop,
        stop_in: _stop_in,
    } = Relay::new();
    let (typed, all_typed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let chunk = [b'x'; 1 << 16];
        for _ in 0..(8 << 20) / chunk.len() {
            typing.write_all(&chunk).unwrap();
        }
        typing.write_all(b"\x1b\x04").unwrap();
        let _ = typed.send(());
    });
    let relay = Relay::spawn(sock, keys, winch, stop);
    all_typed
        .recv_timeout(Duration::from_secs(10))
        .expect("the relay stopped reading the keys");
    assert_eq!(relay.join().unwrap(), Outcome::Detached);
}

#[test]
fn frames_that_come_together_are_all_relayed() {
    let r = Relay::new();
    let mut burst = Vec::new();
    for part in [&b"one "[..], b"two ", b"three"] {
        burst.extend(wire::frame(wire::OUTPUT, part));
    }
    burst.extend(wire::frame(wire::EXIT, &3i32.to_be_bytes()));
    wire::send_all(&r.host, &burst).unwrap();
    let mut out = Vec::new();
    let mut screen = Scan::default();
    let outcome = relay(
        &r.sock,
        r.keys.as_fd(),
        &mut out,
        &r.winch,
        &r.stop,
        &mut screen,
    )
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Exited {
            code: 3,
            left: None
        }
    );
    assert_eq!(out, b"one two three");
}

#[test]
fn what_the_program_left_comes_with_its_status() {
    let r = Relay::new();
    let mut body = 2i32.to_be_bytes().to_vec();
    body.extend_from_slice(b"fits finished: 2 of 2 done");
    wire::send_all(&r.host, &wire::frame(wire::EXIT, &body)).unwrap();
    let mut out = Vec::new();
    let mut screen = Scan::default();
    let outcome = relay(
        &r.sock,
        r.keys.as_fd(),
        &mut out,
        &r.winch,
        &r.stop,
        &mut screen,
    )
    .unwrap();
    assert_eq!(
        outcome,
        Outcome::Exited {
            code: 2,
            left: Some("fits finished: 2 of 2 done".into())
        }
    );
    // Said back, so the host lets the text go.
    let mut host = &r.host;
    host.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
    assert_eq!(
        wire::read_frame(&mut host).unwrap(),
        Some((wire::HEARD, Vec::new()))
    );
}

#[test]
fn the_last_frame_finishes_one_begun_and_drops_what_waits() {
    let (sock, mut host) = UnixStream::pair().unwrap();
    let mut outbox = Outbox::default();
    outbox.push(wire::INPUT, b"begun");
    outbox.push(wire::INPUT, b"waiting");
    // As a flush that sent the head alone leaves it.
    wire::send_all(&sock, &wire::frame(wire::INPUT, b"begun")[..wire::HEAD]).unwrap();
    outbox.sent = wire::HEAD;
    outbox.last(&sock, wire::HEARD).unwrap();
    drop(sock);
    assert_eq!(
        wire::read_frame(&mut host).unwrap(),
        Some((wire::INPUT, b"begun".to_vec()))
    );
    assert_eq!(
        wire::read_frame(&mut host).unwrap(),
        Some((wire::HEARD, Vec::new()))
    );
    assert_eq!(wire::read_frame(&mut host).unwrap(), None);
    assert!(outbox.is_empty());
}

#[test]
fn held_input_past_the_bound_loses_its_oldest() {
    let mut outbox = Outbox::default();
    let chunk = vec![b'x'; 1 << 14];
    outbox.push(wire::INPUT, b"first");
    outbox.push(wire::RESIZE, &Size::default().encode());
    for _ in 0..2 * PENDING / chunk.len() {
        outbox.push(wire::INPUT, &chunk);
    }
    outbox.push(wire::INPUT, b"last");
    assert!(outbox.bytes <= PENDING, "{}", outbox.bytes);
    assert_eq!(
        outbox.bytes,
        outbox.frames.iter().map(|(_, f)| f.len()).sum::<usize>()
    );
    // The oldest typed went; the size stayed, and the newest.
    assert_eq!(outbox.frames[0].0, wire::RESIZE);
    assert_eq!(
        outbox.frames.back().unwrap().1,
        wire::frame(wire::INPUT, b"last")
    );
    // A frame half sent stays, to be sent whole.
    let mut outbox = Outbox::default();
    outbox.push(wire::INPUT, b"begun");
    outbox.sent = 3;
    for _ in 0..2 * PENDING / chunk.len() {
        outbox.push(wire::INPUT, &chunk);
    }
    assert_eq!(outbox.frames[0].1, wire::frame(wire::INPUT, b"begun"));
    assert!(outbox.bytes <= PENDING + 10, "{}", outbox.bytes);
}

#[test]
fn a_lone_escape_goes_on_while_output_keeps_coming() {
    let Relay {
        sock,
        host,
        keys,
        mut typing,
        winch,
        _winch_in,
        stop,
        stop_in,
    } = Relay::new();
    // Output far more often than the hold.
    let busy = {
        let host = host.try_clone().unwrap();
        std::thread::spawn(move || {
            for _ in 0..400 {
                if wire::write_frame(&host, wire::OUTPUT, b"x").is_err() {
                    break;
                }
                std::thread::sleep(Duration::from_millis(2));
            }
        })
    };
    let relay = Relay::spawn(sock, keys, winch, stop);
    std::thread::sleep(Duration::from_millis(50));
    let typed = Instant::now();
    typing.write_all(b"\x1b").unwrap();
    let mut host = host;
    host.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
    let frame = wire::read_frame(&mut host).unwrap();
    assert_eq!(frame, Some((wire::INPUT, b"\x1b".to_vec())));
    assert!(
        typed.elapsed() < Duration::from_millis(300),
        "{:?}",
        typed.elapsed()
    );
    // A signal to stop ends it, not as a detach by the key.
    wire::send_all(&stop_in, &[1]).unwrap();
    assert_eq!(relay.join().unwrap(), Outcome::Stopped);
    drop(host);
    busy.join().unwrap();
}

/// The relay over a pipe for the keys: what it ends with after `typed`.
fn relay_ends(typed: &[u8]) -> Outcome {
    let mut r = Relay::new();
    r.typing.write_all(typed).unwrap();
    let mut screen = Scan::default();
    relay(
        &r.sock,
        r.keys.as_fd(),
        &mut Vec::new(),
        &r.winch,
        &r.stop,
        &mut screen,
    )
    .unwrap()
}

#[test]
fn the_keys_end_the_relay_as_a_detach_or_a_switch() {
    assert_eq!(relay_ends(b"\x1b\x04"), Outcome::Detached);
    assert_eq!(relay_ends(b"\x1b[100;7u"), Outcome::Detached);
    assert_eq!(relay_ends(b"\x1b\x13"), Outcome::Switch);
    assert_eq!(relay_ends(b"\x1b[115;7u"), Outcome::Switch);
}

/// A pseudo-terminal pair in line mode: the user's side to type on, and
/// the side the prompt reads.
fn pty_pair() -> (std::fs::File, std::os::fd::OwnedFd) {
    let (ours, theirs) = crate::host::pty(Size::default()).unwrap();
    let mode = rustix::termios::tcgetattr(&theirs).unwrap();
    assert!(mode
        .local_modes
        .contains(rustix::termios::LocalModes::ICANON));
    (ours, theirs)
}

/// Run the prompt for `tool-view-1` on a pty, typing each of `lines` once
/// it has asked; the label given, what it wrote, and what it was offered.
fn prompted(lines: &[&[u8]], stop_after: bool) -> (Option<String>, String, Vec<String>) {
    let (mut user, tty) = pty_pair();
    let (stop, stop_in) = UnixStream::pair().unwrap();
    let (said_tx, said) = std::sync::mpsc::channel::<Vec<u8>>();
    let asked = std::thread::spawn(move || {
        let mut out = Vec::new();
        let mut offered = Vec::new();
        let label = {
            let mut out = Tee(&mut out, said_tx);
            prompt(tty.as_fd(), &mut out, &stop, "tool-view-1", |label| {
                offered.push(label.to_string());
                match label {
                    "taken" => Err(io::Error::other("taken is taken by tool-view-2")),
                    _ => dir::check_name(label),
                }
            })
        };
        (label, String::from_utf8(out).unwrap(), offered)
    });
    let question = "Name it (Enter to keep): ";
    let mut out = Vec::new();
    let mut asks = |n: usize| {
        while String::from_utf8_lossy(&out).matches(question).count() < n {
            out.extend(said.recv_timeout(Duration::from_secs(5)).unwrap());
        }
    };
    for (i, line) in lines.iter().enumerate() {
        asks(i + 1);
        user.write_all(line).unwrap();
    }
    if stop_after {
        asks(lines.len() + 1);
        wire::send_all(&stop_in, &[1]).unwrap();
    }
    asked.join().unwrap()
}

/// Writes to a buffer and passes each write on.
struct Tee<'a>(&'a mut Vec<u8>, std::sync::mpsc::Sender<Vec<u8>>);

impl Write for Tee<'_> {
    fn write(&mut self, b: &[u8]) -> io::Result<usize> {
        self.0.extend_from_slice(b);
        let _ = self.1.send(b.to_vec());
        Ok(b.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn a_name_given_is_the_label() {
    let (label, out, offered) = prompted(&[b"work\r"], false);
    assert_eq!(label.as_deref(), Some("work"));
    assert_eq!(out, "Detached tool-view-1. Name it (Enter to keep): ");
    assert_eq!(offered, ["work"]);
    // Spaces around it do not count.
    assert_eq!(prompted(&[b"  work \n"], false).0.as_deref(), Some("work"));
}

#[test]
fn enter_keeps_it_unnamed() {
    let (label, _, offered) = prompted(&[b"\r"], false);
    assert_eq!(label, None);
    assert!(offered.is_empty());
}

#[test]
fn an_invalid_or_taken_name_is_said_and_asked_again() {
    let (label, out, offered) = prompted(&[b"a b\r", b"taken\r", b"fine\r"], false);
    assert_eq!(label.as_deref(), Some("fine"));
    assert_eq!(offered, ["a b", "taken", "fine"]);
    assert_eq!(out.matches("Name it").count(), 3, "{out:?}");
    assert!(out.contains("use letters, digits"), "{out:?}");
    assert!(out.contains("taken is taken by tool-view-2"), "{out:?}");
}

#[test]
fn a_stop_or_the_end_of_input_skips_it() {
    // Ctrl+C reaches the client as SIGINT, which wakes the stop pipe.
    let (label, out, offered) = prompted(&[], true);
    assert_eq!(label, None);
    assert!(offered.is_empty());
    assert!(out.ends_with(": \n"), "{out:?}");
    // Ctrl+D on an empty line: the end of the input.
    assert_eq!(prompted(&[b"\x04"], false).0, None);
    // A stop after a refused name.
    let (label, _, offered) = prompted(&[b"a b\r"], true);
    assert_eq!(label, None);
    assert_eq!(offered, ["a b"]);
}

/// Run in a process of its own by the test below: once the client is done
/// with them, a stop signal ends the process, as by default.
#[test]
#[ignore = "run by stop_signals_end_the_process_once_the_client_is_done"]
fn stop_signal_child() {
    let Ok(sig) = std::env::var("LEGUME_TUI_STOP_CHILD") else {
        return;
    };
    {
        let _stop = Woken::stops().unwrap();
    }
    let sig: i32 = sig.parse().unwrap();
    signal_hook::low_level::raise(sig).unwrap();
    std::thread::sleep(Duration::from_millis(500));
}

#[test]
fn stop_signals_end_the_process_once_the_client_is_done() {
    use std::os::unix::process::ExitStatusExt;
    for sig in STOPS {
        let out = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--ignored", "--exact", "client::tests::stop_signal_child"])
            .env("LEGUME_TUI_STOP_CHILD", sig.to_string())
            .output()
            .unwrap();
        assert_eq!(
            out.status.signal(),
            Some(sig),
            "{:?}: {}",
            out.status,
            String::from_utf8_lossy(&out.stdout)
        );
    }
}
