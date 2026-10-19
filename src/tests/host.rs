//! The host driven over its socket, with `sh` as the program: no real
//! terminal is needed, the host makes its own pty.

use super::*;
use crate::dir::{self, bind};
use std::io::{PipeReader, Read};
use std::time::Instant;

const WAIT: Duration = Duration::from_secs(5);

/// A host running a shell script in a temporary directory, with the
/// control channel kept here to read.
struct Running {
    _dir: tempfile::TempDir,
    path: PathBuf,
    control: PipeReader,
    done: std::thread::JoinHandle<i32>,
    shared: Arc<Shared>,
}

fn host(script: &str) -> Running {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool-test-1.sock");
    let (control, done, shared) = start(&path, script);
    Running {
        _dir: dir,
        path,
        control,
        done,
        shared,
    }
}

/// A host on `path` running a shell script: the control channel, the
/// thread that waits for it to end, and what its threads share.
fn start(path: &Path, script: &str) -> (PipeReader, std::thread::JoinHandle<i32>, Arc<Shared>) {
    let listener = bind(path).unwrap();
    let (r, w) = std::io::pipe().unwrap();
    rustix::io::ioctl_fionbio(&r, true).unwrap();
    let mut program = Command::new("sh");
    program.arg("-c").arg(script);
    let h = Host::start(
        listener,
        path.parent().unwrap().to_path_buf(),
        path.file_stem().unwrap().to_string_lossy().into_owned(),
        program,
        size(24, 80),
        Control::Given(w.into()),
    )
    .unwrap();
    let shared = Arc::clone(&h.shared);
    (r, std::thread::spawn(move || h.wait()), shared)
}

impl Running {
    /// What the host has told the program so far.
    fn told(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut buf = [0u8; 64];
        while let Ok(n) = self.control.read(&mut buf) {
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n]);
        }
        out
    }

    /// Wait until the host has told the program `what`.
    fn wait_told(&mut self, what: &[u8]) -> Vec<u8> {
        let start = Instant::now();
        let mut all = Vec::new();
        while start.elapsed() < WAIT {
            all.extend(self.told());
            if all.ends_with(what) {
                return all;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        panic!("not told {what:?}, only {all:?}");
    }
}

/// Connect and say hello with `size`.
fn hello(path: &Path, size: Size) -> UnixStream {
    hello_from(path, size, &[])
}

/// Connect and say hello with `size`, from a terminal whose environment
/// has `env`.
fn hello_from(path: &Path, size: Size, env: &[(&str, &str)]) -> UnixStream {
    let env: Vec<_> = env
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    let s = wire::connect(path).unwrap();
    wire::write_frame(&s, wire::HELLO, &wire::hello_body(size, &env)).unwrap();
    s
}

/// Attach: hello, then ready.
fn attach_now(path: &Path) -> UnixStream {
    let s = hello(path, Size::default());
    wire::write_frame(&s, wire::READY, &[]).unwrap();
    s
}

/// Read output until it contains `needle`; what came, and the frame that
/// ended the stream if one did.
fn output_until(s: &mut UnixStream, needle: &[u8]) -> Vec<u8> {
    wire::set_timeout(s, true, Some(WAIT)).unwrap();
    let mut out = Vec::new();
    while !out.windows(needle.len()).any(|w| w == needle) {
        match wire::read_frame(s).unwrap() {
            Some((wire::OUTPUT, b)) => out.extend(b),
            Some((wire::SCREEN, _)) => {}
            other => panic!("expected output, got {other:?} after {out:?}"),
        }
    }
    out
}

/// The next frame that is not output.
fn next_event(s: &mut UnixStream) -> Option<(u8, Vec<u8>)> {
    wire::set_timeout(s, true, Some(WAIT)).unwrap();
    loop {
        match wire::read_frame(s).unwrap() {
            Some((wire::OUTPUT | wire::SCREEN, _)) => continue,
            other => return other,
        }
    }
}

fn input(s: &mut UnixStream, bytes: &[u8]) {
    wire::write_frame(s, wire::INPUT, bytes).unwrap();
}

fn size(rows: u16, cols: u16) -> Size {
    Size {
        rows,
        cols,
        width: 0,
        height: 0,
    }
}

#[test]
fn relay_round_trip_is_byte_exact() {
    let mut h = host(r#"read x; echo "got:$x"; printf '\033_Ga=T;AAAA\033\\'; read y"#);
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    input(&mut c, b"ping\n");
    let out = output_until(&mut c, b"\x1b_Ga=T;AAAA\x1b\\");
    let text = String::from_utf8_lossy(&out);
    assert!(text.contains("got:ping"), "{text:?}");
    input(&mut c, b"\n");
    assert_eq!(next_event(&mut c), Some((wire::EXIT, exit_body(0))));
    assert_eq!(h.done.join().unwrap(), 0);
}

/// What [`wire::EXIT`] carries.
fn exit_body(code: i32) -> Vec<u8> {
    code.to_be_bytes().to_vec()
}

#[test]
fn repaint_is_asked_only_once_the_client_is_ready() {
    let mut h = host("while :; do printf x; sleep 0.05; done");
    let mut c = hello(&h.path, Size::default());
    // Hello alone: no output, no word to the program.
    std::thread::sleep(Duration::from_millis(300));
    wire::set_timeout(&c, true, Some(Duration::from_millis(200))).unwrap();
    assert!(wire::read_frame(&mut c).is_err(), "output before ready");
    assert!(h.told().is_empty());
    // Ready: the program is told, and output flows.
    wire::write_frame(&c, wire::READY, &[]).unwrap();
    assert_eq!(h.wait_told(&[ATTACHED]), vec![ATTACHED]);
    output_until(&mut c, b"x");
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn the_attached_terminals_environment_is_there_when_the_program_is_told() {
    let mut h = host("while read x; do :; done");
    let (dir, id) = h.shared.at.clone();
    assert_eq!(dir::read_terminal_env(&dir, &id), None);
    let first = hello_from(
        &h.path,
        Size::default(),
        &[("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "1")],
    );
    wire::write_frame(&first, wire::READY, &[]).unwrap();
    h.wait_told(&[ATTACHED]);
    let env = |pairs: &[(&str, &str)]| -> Vec<(String, String)> {
        pairs
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect()
    };
    assert_eq!(
        dir::read_terminal_env(&dir, &id),
        Some(env(&[("TERM", "xterm-kitty"), ("KITTY_WINDOW_ID", "1")]))
    );
    // Another terminal takes over: its own, and nothing of the last one.
    let second = hello_from(&h.path, Size::default(), &[("TERM", "foot")]);
    wire::write_frame(&second, wire::READY, &[]).unwrap();
    h.wait_told(&[ATTACHED]);
    assert_eq!(
        dir::read_terminal_env(&dir, &id),
        Some(env(&[("TERM", "foot")]))
    );
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
    assert_eq!(dir::read_terminal_env(&dir, &id), None, "taken away");
}

#[test]
fn hello_sizes_the_pty_and_resize_follows() {
    let h = host("read x; stty size; read y; stty size; read z");
    let mut c = hello(&h.path, size(33, 77));
    wire::write_frame(&c, wire::READY, &[]).unwrap();
    input(&mut c, b"\n");
    output_until(&mut c, b"33 77");
    wire::write_frame(&c, wire::RESIZE, &size(40, 100).encode()).unwrap();
    input(&mut c, b"\n");
    output_until(&mut c, b"40 100");
    input(&mut c, b"\n");
    h.done.join().unwrap();
}

#[test]
fn pixel_size_reaches_the_pty() {
    let (ours, theirs) = pty(Size::default()).unwrap();
    let size = Size {
        rows: 30,
        cols: 90,
        width: 900,
        height: 600,
    };
    // As the host applies what a client sent.
    let sent = Size::decode(&size.encode()).unwrap();
    sent.apply(&ours).unwrap();
    assert_eq!(Size::of(&theirs).unwrap(), size);
    assert_eq!(Size::from_env(&size.to_env()), Some(size));
}

#[test]
fn a_lost_client_leaves_the_program_running_and_it_can_attach_again() {
    let mut h = host("while read x; do echo \"got:$x\"; done");
    let c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    assert_eq!(dir::ask(&h.path).map(|l| l.attached), Some(true));
    drop(c);
    h.wait_told(&[DETACHED]);
    let listed = dir::ask(&h.path).unwrap();
    assert!(listed.pid > 0 && !listed.attached);
    assert_eq!(listed.name, "tool-test-1");
    assert!(listed.command.starts_with("sh -c while read x"));
    assert!(listed.age().ends_with("s ago"), "{}", listed.age());
    // Still there: attach again and talk to it.
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    input(&mut c, b"again\n");
    output_until(&mut c, b"got:again");
    input(&mut c, b"\x04");
    assert_eq!(next_event(&mut c).map(|(t, _)| t), Some(wire::EXIT));
    h.done.join().unwrap();
}

#[test]
fn a_second_client_takes_over() {
    let mut h = host("while read x; do echo \"got:$x\"; done");
    let mut first = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    let mut second = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    assert_eq!(
        next_event(&mut first).map(|(t, _)| t),
        Some(wire::ELSEWHERE)
    );
    // Only the new client's keys reach the program.
    input(&mut second, b"two\n");
    output_until(&mut second, b"got:two");
    // The old one leaving does not detach the new one.
    drop(first);
    std::thread::sleep(Duration::from_millis(100));
    assert!(!h.told().contains(&DETACHED));
    input(&mut second, b"\x04");
    h.done.join().unwrap();
}

#[test]
fn program_exit_reaches_the_client_and_removes_the_session_files() {
    let h = host("read x; echo bye; exit 3");
    let dir = h.path.parent().unwrap();
    let files = [
        dir::status_file(dir, "tool-test-1"),
        dir::label_file(dir, "tool-test-1"),
        dir::log_file(dir, "tool-test-1"),
    ];
    for f in &files {
        std::fs::write(f, "x").unwrap();
    }
    let mut c = attach_now(&h.path);
    input(&mut c, b"\n");
    output_until(&mut c, b"bye");
    // All gone before the client hears the program ended.
    assert_eq!(next_event(&mut c), Some((wire::EXIT, exit_body(3))));
    assert!(!h.path.exists());
    for f in &files {
        assert!(!f.exists(), "{}", f.display());
    }
    assert_eq!(h.done.join().unwrap(), 3);
}

/// A script that leaves `text` ([`crate::Session::leave`]) for session
/// `tool-test-1` beside `path`, then runs `then`.
fn leaving(path: &Path, text: &str, then: &str) -> String {
    let leave = dir::leave_file(path.parent().unwrap(), "tool-test-1");
    format!("printf '{text}' > '{}'; {then}", leave.display())
}

#[test]
fn what_a_program_leaves_goes_to_the_client_attached_at_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool-test-1.sock");
    let (_control, done, _) = start(&path, &leaving(&path, "a summary", "read x; exit 4"));
    let mut c = attach_now(&path);
    input(&mut c, b"\n");
    let mut said = exit_body(4);
    said.extend_from_slice(b"a summary");
    assert_eq!(next_event(&mut c), Some((wire::EXIT, said)));
    wire::write_frame(&c, wire::HEARD, &[]).unwrap();
    assert_eq!(done.join().unwrap(), 4);
    // Said by that client alone: nothing stays for a later look.
    assert!(!dir::leave_file(dir.path(), "tool-test-1").exists());
}

#[test]
fn what_a_client_closing_unheard_was_sent_is_kept() {
    // A client leaving by a key as the end comes: the frame is sent, not
    // taken, so what the program left stays for the next look.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool-test-1.sock");
    let (_control, done, _) = start(&path, &leaving(&path, "a summary", "read x; exit 4"));
    let mut c = attach_now(&path);
    input(&mut c, b"\n");
    let mut said = exit_body(4);
    said.extend_from_slice(b"a summary");
    assert_eq!(next_event(&mut c), Some((wire::EXIT, said)));
    drop(c);
    assert_eq!(done.join().unwrap(), 4);
    assert_eq!(
        dir::take_leave(dir.path(), "tool-test-1").as_deref(),
        Some("a summary")
    );
}

#[test]
fn what_a_program_ending_unseen_leaves_is_kept() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("tool-test-1.sock");
    let (_control, done, _) = start(&path, &leaving(&path, "a summary", "exit 0"));
    assert_eq!(done.join().unwrap(), 0);
    assert!(!path.exists());
    assert_eq!(
        dir::take_leave(dir.path(), "tool-test-1").as_deref(),
        Some("a summary")
    );
}

#[test]
fn a_kill_from_elsewhere_finds_nothing_the_attached_client_is_told() {
    // The program leaves its words as it stops, while a client is there:
    // they are that client's, however the kill and the end interleave.
    for _ in 0..5 {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("tool-test-1.sock");
        let leave = dir::leave_file(dir.path(), "tool-test-1");
        let script = format!(
            "trap \"printf 'stopped by kill' > '{}'; exit 0\" TERM; \
             echo ready; while :; do sleep 0.01; done",
            leave.display()
        );
        let (_control, done, _) = start(&path, &script);
        let mut c = attach_now(&path);
        output_until(&mut c, b"ready");
        assert!(dir::stop(&path, false, WAIT).unwrap());
        // As `kill` does once the session is gone.
        assert_eq!(dir::take_leave(dir.path(), "tool-test-1"), None);
        let mut said = exit_body(0);
        said.extend_from_slice(b"stopped by kill");
        assert_eq!(next_event(&mut c), Some((wire::EXIT, said)));
        wire::write_frame(&c, wire::HEARD, &[]).unwrap();
        assert_eq!(done.join().unwrap(), 0);
        assert!(!leave.exists());
    }
}

#[test]
fn what_a_program_says_before_its_first_client_waits_for_it() {
    // An error at the start: the program is gone before anyone is there.
    let h = host("echo 'error: no input'; exit 1");
    std::thread::sleep(Duration::from_millis(300));
    let mut c = attach_now(&h.path);
    output_until(&mut c, b"error: no input");
    assert_eq!(next_event(&mut c), Some((wire::EXIT, exit_body(1))));
    assert_eq!(h.done.join().unwrap(), 1);
    assert!(!h.path.exists());
}

#[test]
fn a_client_joining_later_is_told_the_screen_is_the_alternate_one() {
    let mut h = host(r#"printf '\033[?1049h'; while read x; do echo "got:$x"; done"#);
    let first = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    drop(first);
    h.wait_told(&[DETACHED]);
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    input(&mut c, b"hi\n");
    wire::set_timeout(&c, true, Some(WAIT)).unwrap();
    assert_eq!(
        wire::read_frame(&mut c).unwrap(),
        Some((wire::SCREEN, vec![1]))
    );
    output_until(&mut c, b"got:hi");
    input(&mut c, b"\x04");
    h.done.join().unwrap();
}

#[test]
fn a_new_client_starts_at_the_start_of_a_character() {
    // The first client takes what came before it and goes; the next one
    // arrives between the two bytes of a character.
    let mut h = host(r#"read x; printf 'x\303'; sleep 1; printf '\251after'; read y"#);
    let mut first = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    input(&mut first, b"\n");
    output_until(&mut first, b"x\xc3");
    drop(first);
    h.wait_told(&[DETACHED]);
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    let out = output_until(&mut c, b"after");
    assert_eq!(out, b"after", "{:?}", String::from_utf8_lossy(&out));
    input(&mut c, b"\n");
    h.done.join().unwrap();
}

#[test]
fn input_a_program_is_not_reading_is_dropped_and_the_socket_read_on() {
    let mut h = host("sleep 30");
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    // Far more than its queue and the pty take; a host that stopped
    // reading the socket would fail these writes.
    wire::set_timeout(&c, false, Some(WAIT)).unwrap();
    for _ in 0..512 {
        input(&mut c, &[b'x'; 1 << 14]);
    }
    let start = Instant::now();
    while h.shared.dropped.load(SeqCst) == 0 {
        assert!(start.elapsed() < WAIT, "nothing dropped");
        std::thread::sleep(Duration::from_millis(10));
    }
    drop(c);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn a_kill_is_waited_out_and_the_wait_said() {
    let p = |limit: u64, first: u64, every: u64| dir::Patience {
        limit: Duration::from_millis(limit),
        say: Some((Duration::from_millis(first), Duration::from_millis(every))),
    };
    // Wrapping up takes a while: waited for, said once.
    let h = host(r#"trap 'sleep 0.6; exit 0' TERM; echo ready; while :; do sleep 0.05; done"#);
    let mut c = attach_now(&h.path);
    output_until(&mut c, b"ready");
    let mut said = 0;
    assert!(dir::stop_waiting(&h.path, false, p(5000, 200, 10_000), || said += 1).unwrap());
    assert_eq!(said, 1);
    assert_eq!(h.done.join().unwrap(), 0);
    // Not stopping at all: said again and again, then given up on.
    let h = host(r#"trap '' TERM; echo ready; while :; do sleep 0.05; done"#);
    let mut c = attach_now(&h.path);
    output_until(&mut c, b"ready");
    let mut said = 0;
    assert!(!dir::stop_waiting(&h.path, false, p(700, 100, 200), || said += 1).unwrap());
    assert!((2..=4).contains(&said), "{said}");
    assert!(dir::stop(&h.path, true, WAIT).unwrap());
    h.done.join().unwrap();
}

#[test]
fn kill_asks_the_program_to_stop_through_its_own_path() {
    let h = host(
        r#"trap 'echo stopping; exit 7' TERM; read x; echo ready; while :; do sleep 0.05; done"#,
    );
    // It says ready once a client is there and asks.
    let mut c = attach_now(&h.path);
    input(&mut c, b"\n");
    output_until(&mut c, b"ready");
    assert!(dir::stop(&h.path, false, WAIT).unwrap());
    output_until(&mut c, b"stopping");
    assert_eq!(next_event(&mut c), Some((wire::EXIT, exit_body(7))));
    assert_eq!(h.done.join().unwrap(), 7);
    assert!(!h.path.exists());
}

#[test]
fn force_kills_a_program_that_ignores_the_stop() {
    let h = host(r#"trap '' TERM; read x; echo ready; while :; do sleep 0.05; done"#);
    // It says ready once a client is there and asks.
    let mut c = attach_now(&h.path);
    input(&mut c, b"\n");
    output_until(&mut c, b"ready");
    assert!(!dir::stop(&h.path, false, Duration::from_millis(500)).unwrap());
    assert!(dir::stop(&h.path, true, WAIT).unwrap());
    assert_eq!(h.done.join().unwrap(), 128 + 9);
    assert!(!h.path.exists());
}

#[test]
fn another_version_is_turned_away() {
    let h = host("while read x; do :; done");
    let mut s = wire::connect(&h.path).unwrap();
    let mut body = wire::hello_body(Size::default(), &[]);
    body[0] = wire::VERSION + 1;
    wire::write_frame(&s, wire::HELLO, &body).unwrap();
    wire::set_timeout(&s, true, Some(WAIT)).unwrap();
    assert_eq!(wire::read_frame(&mut s).unwrap(), None);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn a_live_socket_is_not_taken() {
    let h = host("while read x; do :; done");
    assert_eq!(bind(&h.path).unwrap_err().kind(), io::ErrorKind::AddrInUse);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn a_client_that_stops_reading_holds_up_nobody() {
    let mut h = host("while :; do echo 0123456789abcdef0123456789abcdef; done");
    // Attached, then never read: its socket and queue fill up.
    let stuck = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    std::thread::sleep(Duration::from_millis(500));
    // Questions are still answered, at once.
    let asked = Instant::now();
    assert_eq!(dir::ask(&h.path).map(|l| l.attached), Some(true));
    assert!(asked.elapsed() < Duration::from_secs(1));
    // Another terminal takes over and gets the output.
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    output_until(&mut c, b"0123456789abcdef");
    drop((stuck, c));
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn a_program_not_reading_holds_up_no_attach() {
    let mut h = host("sleep 30");
    let mut first = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    // Far more than the terminal's input queue takes.
    for _ in 0..16 {
        input(&mut first, &[b'x'; 1 << 14]);
    }
    let second = hello(&h.path, size(30, 90));
    wire::write_frame(&second, wire::READY, &[]).unwrap();
    h.wait_told(&[ATTACHED]);
    assert_eq!(
        next_event(&mut first).map(|(t, _)| t),
        Some(wire::ELSEWHERE)
    );
    wire::write_frame(&second, wire::RESIZE, &size(31, 91).encode()).unwrap();
    assert_eq!(dir::ask(&h.path).map(|l| l.attached), Some(true));
    drop(second);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn only_the_attached_client_sizes_the_pty() {
    let h = host("while read x; do stty size; done");
    let mut c = hello(&h.path, size(33, 77));
    wire::write_frame(&c, wire::READY, &[]).unwrap();
    input(&mut c, b"\n");
    output_until(&mut c, b"33 77");
    // Another says hello and resizes, but never becomes ready.
    let other = hello(&h.path, size(40, 100));
    wire::write_frame(&other, wire::RESIZE, &size(50, 120).encode()).unwrap();
    std::thread::sleep(Duration::from_millis(200));
    drop(other);
    input(&mut c, b"\n");
    let out = output_until(&mut c, b"33 77");
    assert!(!String::from_utf8_lossy(&out).contains("40 100"), "{out:?}");
    // A size sent before ready is the one it starts with.
    let mut c = hello(&h.path, size(33, 77));
    wire::write_frame(&c, wire::RESIZE, &size(44, 88).encode()).unwrap();
    wire::write_frame(&c, wire::READY, &[]).unwrap();
    input(&mut c, b"\n");
    output_until(&mut c, b"44 88");
    input(&mut c, b"\x04");
    h.done.join().unwrap();
}

#[test]
fn a_watcher_hears_every_attach_and_detach() {
    let mut h = host("while read x; do :; done");
    let mut w = wire::connect(&h.path).unwrap();
    wire::write_frame(&w, wire::WATCH, &[]).unwrap();
    wire::set_timeout(&w, true, Some(WAIT)).unwrap();
    let mut heard = |n: usize| {
        let mut b = vec![0u8; n];
        w.read_exact(&mut b).unwrap();
        b
    };
    assert_eq!(heard(1), [DETACHED]);
    let c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    assert_eq!(heard(1), [ATTACHED]);
    drop(c);
    assert_eq!(heard(1), [DETACHED]);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn a_new_client_starts_between_escape_sequences() {
    // An image chunk split by a pause, a new client arriving in the pause
    // (the first takes what came before it, and goes).
    let mut h =
        host(r#"read x; printf 'x\033_Ga=T;HEAD'; sleep 1; printf 'TAIL\033\\after'; read y"#);
    let mut first = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    input(&mut first, b"\n");
    output_until(&mut first, b"HEAD");
    drop(first);
    h.wait_told(&[DETACHED]);
    let mut c = attach_now(&h.path);
    h.wait_told(&[ATTACHED]);
    let out = output_until(&mut c, b"after");
    assert_eq!(out, b"after", "{:?}", String::from_utf8_lossy(&out));
    input(&mut c, b"\n");
    h.done.join().unwrap();
}

#[test]
fn sessions_are_picked_by_name_its_tail_or_alone() {
    let d = tempfile::tempdir().unwrap();
    let path = |name: &str| dir::socket(d.path(), name);
    let (_r1, one, _) = start(&path("tool-view-foo"), "while read x; do :; done");
    let picked = |w: Option<&str>| dir::pick(d.path(), "tool-", w).map(|l| l.name);
    assert_eq!(picked(None).unwrap(), "tool-view-foo");
    assert_eq!(picked(Some("foo")).unwrap(), "tool-view-foo");
    assert_eq!(picked(Some("view-foo")).unwrap(), "tool-view-foo");
    assert_eq!(picked(Some("tool-view-foo")).unwrap(), "tool-view-foo");
    let (_r2, two, _) = start(&path("tool-run-foo"), "while read x; do :; done");
    // Two end in -foo: name more of it.
    let both = picked(Some("foo")).unwrap_err();
    assert_eq!(both.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(picked(Some("run-foo")).unwrap(), "tool-run-foo");
    assert!(picked(None).is_err());
    assert_eq!(
        picked(Some("bar")).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    for name in ["tool-view-foo", "tool-run-foo"] {
        dir::stop(&path(name), true, WAIT).unwrap();
    }
    one.join().unwrap();
    two.join().unwrap();
}

#[test]
fn sessions_are_picked_by_label_before_id() {
    let d = tempfile::tempdir().unwrap();
    let path = |name: &str| dir::socket(d.path(), name);
    let (_r1, one, _) = start(&path("tool-view-1"), "while read x; do :; done");
    let (_r2, two, _) = start(&path("tool-view-2"), "while read x; do :; done");
    let picked = |w: &str| dir::pick(d.path(), "tool-", Some(w)).map(|l| l.name);
    dir::set_label(d.path(), "tool-", "tool-view-2", "work").unwrap();
    assert_eq!(picked("work").unwrap(), "tool-view-2");
    // The id still finds it, in full, after the prefix, or by its end.
    assert_eq!(picked("tool-view-2").unwrap(), "tool-view-2");
    assert_eq!(picked("view-2").unwrap(), "tool-view-2");
    assert_eq!(picked("2").unwrap(), "tool-view-2");
    // A label that is the end of another's id: the label wins.
    dir::set_label(d.path(), "tool-", "tool-view-2", "1").unwrap();
    assert_eq!(picked("1").unwrap(), "tool-view-2");
    assert_eq!(picked("view-1").unwrap(), "tool-view-1");
    assert_eq!(picked("work").unwrap_err().kind(), io::ErrorKind::NotFound);
    let listed = dir::live(d.path(), "tool-").unwrap();
    let shown: Vec<_> = listed.iter().map(dir::Listed::shown).collect();
    assert_eq!(shown, ["tool-view-1", "1 (tool-view-2)"]);
    // Both are named by what a user types.
    assert_eq!(dir::names(&listed), "tool-view-1, 1");
    for name in ["tool-view-1", "tool-view-2"] {
        dir::stop(&path(name), true, WAIT).unwrap();
    }
    one.join().unwrap();
    two.join().unwrap();
}

/// Run in a process of its own by the test below, with SIGPIPE at its
/// default, as a tool that resets it runs: a host and a client whose
/// peers vanish mid-write must not die of it.
#[test]
#[ignore = "run by sigpipe_at_its_default_kills_neither_host_nor_client"]
fn sigpipe_child() {
    if std::env::var_os("LEGUME_TUI_SIGPIPE_CHILD").is_none() {
        return;
    }
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let mut h = host("while :; do echo 0123456789abcdef; sleep 0.01; done");
    // The host writes output to a client that is gone.
    for _ in 0..3 {
        let c = attach_now(&h.path);
        h.wait_told(&[ATTACHED]);
        drop(c);
        h.wait_told(&[DETACHED]);
    }
    std::thread::sleep(Duration::from_millis(200));
    assert!(dir::ask(&h.path).is_some(), "the host is still there");
    // A client writes to a host that is gone.
    let (a, b) = wire::pair().unwrap();
    drop(b);
    let err = wire::write_frame(&a, wire::INPUT, b"x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    dir::stop(&h.path, true, WAIT).unwrap();
    h.done.join().unwrap();
}

#[test]
fn sigpipe_at_its_default_kills_neither_host_nor_client() {
    let out = Command::new(std::env::current_exe().unwrap())
        .args(["--ignored", "--exact", "host::tests::sigpipe_child"])
        .env("LEGUME_TUI_SIGPIPE_CHILD", "1")
        .output()
        .unwrap();
    let said = String::from_utf8_lossy(&out.stdout);
    assert!(out.status.success(), "{:?}: {said}", out.status);
    assert!(said.contains("1 passed"), "{said}");
}

#[test]
fn a_socket_whose_peer_has_closed_is_read_and_written_as_one_gone() {
    // macOS refuses socket options there: the timeouts are let be.
    let (mut a, b) = wire::pair().unwrap();
    drop(b);
    wire::set_timeout(&a, true, Some(WAIT)).unwrap();
    wire::set_timeout(&a, false, Some(WAIT)).unwrap();
    let err = wire::write_frame(&a, wire::INPUT, b"x").unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    assert_eq!(wire::read_frame(&mut a).unwrap(), None);
}
