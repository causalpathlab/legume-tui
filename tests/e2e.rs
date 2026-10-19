//! The demo example driven end to end through pseudo-terminals standing
//! in for the user's terminal: launch hosted, detach with the key, attach
//! again and see a repaint, lose a client, stop one by a signal, refuse a
//! nested host and repaint a nested screen, resume through the picker,
//! name a session on detach and later, pick on a bare attach, switch
//! between sessions with the key, and kill, leaving no process or socket
//! behind. Programs that misbehave: one that fails at once, one slow to
//! start its screen, one that stops reading its input. The demo sets
//! SIGPIPE to its default, as some tools do. Then what a program leaves
//! and holds for the user, work a stop waits for, children in groups of
//! their own when the host is lost, and a terminal's canned answers read
//! off the modelled screen.

#![cfg(sessions)]

use legume_tui::testing::{alive, Private, Replies, Setup, PROMPT, WAIT};
use std::io::Write;
use std::path::PathBuf;
use std::process::Command;
use std::time::{Duration, Instant};

/// The demo example, built alongside the tests.
fn demo() -> PathBuf {
    let exe = std::env::current_exe().unwrap();
    let debug = exe.parent().unwrap().parent().unwrap();
    let demo = debug.join("examples").join("demo");
    assert!(demo.exists(), "build the example first: {}", demo.display());
    demo
}

/// A private directory for the demo's sessions.
fn private() -> Private {
    Private::new(demo())
}

#[test]
fn detach_attach_lose_nest_and_kill() {
    let dir = private();
    let sock = dir.path().join("demo-view-1.sock");

    // Launch: hosted by default, the first client is this terminal.
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");
    assert!(sock.exists());
    assert!(!first.cooked(), "the client's terminal is raw");

    // A TUI the hosted program starts runs plain: no second host.
    first.type_(b"n");
    first.wait_for("bypass=Some(Nested) hosted=false");

    // Detach with the key: the client exits, the terminal is restored,
    // the program runs on.
    first.wait_for("detach-key=Ctrl+Alt+D");
    first.wait_for("switch-key=Ctrl+Alt+S");
    first.type_(b"\x1b\x04");
    // Asked for a name on the restored screen; Enter keeps it unnamed.
    first.answer(1, b"\r");
    assert_eq!(first.exit_code(), 0);
    first.wait_for("Detached demo-view-1. Name it");
    first.wait_for("[detached from demo-view-1");
    assert!(
        first.text().contains("\x1b[?1049l"),
        "left the alternate screen"
    );
    assert!(first.cooked(), "terminal restored after detach");
    let pid = dir.program_pid();
    assert!(alive(pid));
    let listed = dir.run(&["sessions"]);
    assert!(listed.contains("detached"), "{listed:?}");
    assert!(
        listed.contains("demo-screen after 1 repaints"),
        "{listed:?}"
    );
    assert!(listed.contains("s ago\tpid "), "{listed:?}");

    // Attach from another terminal, with the key in CSI-u this time: a
    // full repaint arrives.
    let mut second = dir.term(&["attach"], &[]);
    second.wait_for("demo-screen-repaint-2");
    assert!(dir.run(&["sessions"]).contains("attached"));
    second.type_(b"\x1b[100;7u");
    second.answer(1, b"\r");
    assert_eq!(second.exit_code(), 0);
    assert!(second.cooked());

    // A client lost without a word while output flows: the host writes
    // to a dead socket and lives, the program runs on, detached.
    let mut third = dir.term(&["attach", "view-1"], &[]);
    third.wait_for("demo-screen-repaint-3");
    third.child.kill().unwrap();
    let _ = third.child.wait();
    dir.wait_listed("detached");
    std::thread::sleep(Duration::from_millis(300));
    assert!(alive(pid));
    dir.wait_listed("detached");

    // A client stopped by a signal leaves its terminal as it found it.
    let mut fourth = dir.term(&["attach", "1"], &[]);
    fourth.wait_for("demo-screen-repaint-4");
    fourth.signal(rustix::process::Signal::TERM);
    assert_eq!(fourth.exit_code(), 0);
    assert!(fourth.cooked(), "terminal restored after SIGTERM");
    assert!(!fourth.text().contains(PROMPT), "no name asked for");
    assert!(fourth.text().contains("\x1b[?1049l"));
    dir.wait_listed("detached");
    assert!(alive(pid));

    // Kill: through the program's own stop path; nothing left behind.
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-view-1 stopped"), "{said:?}");
    assert!(!sock.exists());
    dir.gone(pid);
    assert!(dir.run(&["sessions"]).contains("no live demo sessions"));
    assert!(dir.is_empty(), "left behind: {:?}", dir.sockets());
}

#[test]
fn the_terminal_attached_last_is_the_one_the_program_hears_of() {
    let dir = private();
    let mut first = dir.term(&[], &[("TERM_PROGRAM", "first-terminal")]);
    first.wait_for("terminal=first-terminal");
    first.type_(b"\x1b\x04");
    first.answer(1, b"\r");
    assert_eq!(first.exit_code(), 0);
    let pid = dir.program_pid();
    // From another terminal: the repaint names it.
    let mut second = dir.term(&["attach"], &[("TERM_PROGRAM", "second-terminal")]);
    second.wait_for("terminal=second-terminal");
    assert!(dir.run(&["kill"]).contains("stopped"));
    assert_eq!(second.exit_code(), 0);
    dir.gone(pid);
}

#[test]
fn program_exit_ends_the_client_with_its_output() {
    let dir = private();
    let mut t = dir.term(&[], &[]);
    t.wait_for("demo-screen-repaint-1");
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    t.wait_for("demo-screen done after 1 repaints");
    assert!(!t.text().contains(PROMPT), "no name asked for");
    // It left the alternate screen itself: the client does not again.
    let text = t.text();
    let done = text.find("demo-screen done").unwrap();
    assert!(!text[done..].contains("\x1b[?1049l"), "{:?}", &text[done..]);
    assert!(t.cooked());
    assert!(dir.sockets().is_empty());
}

#[test]
fn bypasses_run_plain_without_a_host() {
    let dir = private();
    let mut t = dir.term(&["which"], &[(legume_tui::NO_DETACH_VAR, "1")]);
    assert_eq!(t.exit_code(), 0);
    t.wait_for("bypass=Some(Env) hosted=false");
    let mut t = dir.term(&["which"], &[("TMUX", "/placeholder,1,0")]);
    assert_eq!(t.exit_code(), 0);
    t.wait_for("bypass=Some(Multiplexer) hosted=false");
    let mut t = dir.term(&["which"], &[(legume_tui::SESSION_VAR, "demo-x-1")]);
    assert_eq!(t.exit_code(), 0);
    t.wait_for("bypass=Some(Nested) hosted=false");
    // Output not a terminal.
    assert!(dir
        .run(&["which"])
        .contains("bypass=Some(NotTerminal) hosted=false"));
    // None of them left a socket.
    assert!(dir.is_empty());
}

#[test]
fn a_nested_screen_repaints_after_a_reattach() {
    let dir = private();
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");
    // A screen the hosted program starts runs in the same session.
    first.type_(b"v");
    first.wait_for("nested-repaint-1");
    first.type_(b"\x1b\x04");
    first.answer(1, b"\r");
    assert_eq!(first.exit_code(), 0);
    let listed = dir.wait_listed("detached");
    assert!(listed.contains("nested after 1 repaints"), "{listed:?}");

    // From the same size of terminal: no resize to hide behind.
    let mut second = dir.term(&["attach"], &[]);
    second.wait_for("nested-repaint-2");
    // Back to the outer screen, which repaints too.
    second.type_(b"q");
    second.wait_for("nested done after 2 repaints");
    second.wait_for("demo-screen-repaint-2");
    second.type_(b"q");
    assert_eq!(second.exit_code(), 0);
    assert!(second.cooked());
    assert!(dir.is_empty());
}

#[test]
fn kill_reaches_a_nested_screen_the_program_started() {
    let dir = private();
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");
    let pid = dir.program_pid();
    // The program hands the terminal to a screen it started, and waits.
    first.type_(b"v");
    first.wait_for("nested-repaint-1");

    // One plain kill: the program passes it on, the nested screen quits
    // through its own stop path, then the program does through its own.
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-view-1 stopped"), "{said:?}");
    first.wait_for("nested done after 1 repaints");
    first.wait_for("demo-screen done");
    assert_eq!(first.exit_code(), 0);
    assert!(first.cooked());
    assert!(!alive(pid));
    assert!(dir.is_empty());
}

#[test]
fn starting_again_offers_the_detached_sessions() {
    let dir = private();
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");
    first.type_(b"\x1b\x04");
    first.answer(1, b"\r");
    assert_eq!(first.exit_code(), 0);

    // Started again: the picker, on the detached session.
    let mut again = dir.term(&[], &[]);
    again.wait_for("> demo-view-1");
    again.wait_for("detached");
    again.type_(b"\r");
    again.wait_for("demo-screen-repaint-2");
    again.type_(b"\x1b\x04");
    again.answer(1, b"\r");
    assert_eq!(again.exit_code(), 0);

    // Or a new one beside it.
    let mut new = dir.term(&[], &[]);
    new.wait_for("> demo-view-1");
    new.type_(b"n");
    new.wait_for("demo-screen-repaint-1");
    new.type_(b"\x1b\x04");
    new.answer(1, b"\r");
    assert_eq!(new.exit_code(), 0);
    new.wait_for("[detached from demo-view-2");

    // Or neither.
    let mut neither = dir.term(&[], &[]);
    neither.wait_for("> demo-view-1");
    neither.type_(b"q");
    assert_eq!(neither.exit_code(), 0);
    assert!(neither.cooked());

    // With a target, no picker: a new session, and a hint.
    let mut target = dir.term(&["open"], &[]);
    target.wait_for("[detached: demo-view-1, demo-view-2; `demo attach NAME` to return]");
    target.wait_for("demo-screen-repaint-1");
    target.type_(b"q");
    assert_eq!(target.exit_code(), 0);

    for name in ["view-1", "view-2"] {
        assert!(dir.run(&["kill", name]).contains("stopped"));
    }
    assert!(dir.is_empty());
}

#[test]
fn the_detach_key_can_be_changed() {
    let dir = private();
    let keys = [
        (legume_tui::DETACH_KEY_VAR, "ctrl+\\"),
        (legume_tui::SWITCH_KEY_VAR, "ctrl+]"),
    ];
    let mut t = dir.term(&[], &keys);
    t.wait_for("detach-key=Ctrl+\\");
    t.wait_for("switch-key=Ctrl+]");
    // The switch key, to the picker and straight back.
    t.type_(b"\x1d");
    t.wait_for("(current)");
    t.type_(b"q");
    t.wait_for("demo-screen-repaint-2");
    t.type_(b"\x1c");
    t.answer(1, b"\r");
    assert_eq!(t.exit_code(), 0);
    assert!(t.cooked());
    assert!(dir.run(&["kill"]).contains("stopped"));
}

/// Start a session in a new terminal and detach from it, keeping it
/// unnamed or naming it `label`.
fn detached(dir: &Private, label: &str) {
    // A target: a new session, whatever is live.
    let mut t = dir.term(&["open"], &[]);
    t.wait_for("demo-screen-repaint-1");
    t.type_(b"\x1b\x04");
    t.answer(1, format!("{label}\r").as_bytes());
    assert_eq!(t.exit_code(), 0);
}

#[test]
fn a_session_is_named_on_its_first_detach_and_found_by_its_name() {
    let dir = private();
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");
    first.type_(b"\x1b\x04");
    first.wait_for("Detached demo-view-1. Name it (Enter to keep): ");
    // An invalid name is said, and asked again.
    first.answer(1, b"a b\r");
    first.wait_for("use letters, digits");
    first.answer(2, b"work\r");
    assert_eq!(first.exit_code(), 0);
    first.wait_for("detached: work");
    assert!(first.cooked());
    assert!(dir
        .wait_listed("work (demo-view-1)\tdetached")
        .contains("pid "));

    // Back by its name, and not asked again.
    let mut again = dir.term(&["attach", "work"], &[]);
    again.wait_for("demo-screen-repaint-2");
    again.type_(b"\x1b\x04");
    assert_eq!(again.exit_code(), 0);
    again.wait_for("detached: work");
    assert!(!again.text().contains(PROMPT));

    // Another session: the name is taken; Ctrl+C keeps it unnamed.
    let mut second = dir.term(&["open"], &[]);
    second.wait_for("[detached: work; `demo attach NAME` to return]");
    second.wait_for("demo-screen-repaint-1");
    second.type_(b"\x1b[100;7u");
    second.answer(1, b"work\r");
    second.wait_for("work is taken by demo-view-1");
    second.answer(2, b"\x03");
    assert_eq!(second.exit_code(), 0);
    assert!(second.cooked());
    second.wait_for("[detached from demo-view-2");
    dir.wait_listed("demo-view-2\tdetached");

    // So does the end of the input (Ctrl+D).
    let mut third = dir.term(&["attach", "view-2"], &[]);
    third.wait_for("demo-screen-repaint-2");
    third.type_(b"\x1b\x04");
    third.answer(1, b"\x04");
    assert_eq!(third.exit_code(), 0);
    third.wait_for("[detached from demo-view-2");

    // Renamed later, to a free name only.
    let (ok, said) = dir.run_all(&["rename", "view-2", "other"]);
    assert!(ok && said.contains("demo-view-2 is now other"), "{said:?}");
    let (ok, said) = dir.run_all(&["rename", "other", "work"]);
    assert!(
        !ok && said.contains("work is taken by demo-view-1"),
        "{said:?}"
    );
    let (ok, said) = dir.run_all(&["rename", "other", "a/b"]);
    assert!(!ok, "{said:?}");
    let listed = dir.wait_listed("other (demo-view-2)\tdetached");
    assert!(
        listed.contains("work (demo-view-1)\tdetached"),
        "{listed:?}"
    );

    // Killed by name, said by its id then its label (tools' tests read
    // `<id> stopped`): no socket, status or label left.
    for (name, id) in [("work", "view-1"), ("other", "view-2")] {
        let said = dir.run(&["kill", name]);
        let line = format!("demo-{id} stopped (labelled {name})");
        assert!(said.lines().any(|l| l == line), "{said:?}");
    }
    assert!(dir.is_empty());
}

#[test]
fn a_session_that_cannot_start_says_why_again_when_its_screen_goes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = private();
    let said = "legume-tui: running without a session (";
    let key = format!(
        "), so {} does not detach; LEGUME_TUI_NO_DETACH=1 runs without trying",
        legume_tui::detach_key()
    );

    // A socket path past the length one may have: said, then covered by
    // the screen, which is not held up for it.
    let long = dir.path().join("d".repeat(110));
    let mut t = dir.term(&[], &[(legume_tui::DIR_VAR, long.to_str().unwrap())]);
    t.wait_for(said);
    t.wait_for(&key);
    t.wait_for("demo-screen-repaint-1");
    let from = t.seen_len();
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    // Said again as the screen goes, on the user's own screen.
    t.wait_for_after(from, said);
    t.wait_for_after(from, &key);
    t.wait_screen("running without a session");

    // A directory open to other users, and a program that leaves words of
    // its own: both are said, the program's first.
    let open = dir.path().join("open");
    std::fs::create_dir(&open).unwrap();
    std::fs::set_permissions(&open, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut t = dir.term(&["bye"], &[(legume_tui::DIR_VAR, open.to_str().unwrap())]);
    t.wait_screen("demo-bye waiting");
    let from = t.seen_len();
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    t.wait_for_after(from, "open to other users (expected mode 0700)), so");
    let end = &t.text()[from..];
    let (left, why) = (end.find("demo-bye: left by the quit path"), end.find(said));
    assert!(
        matches!((left, why), (Some(l), Some(w)) if l < w),
        "{end:?}"
    );
    // Run plain, with no host: nothing left where it was refused.
    assert_eq!(std::fs::read_dir(&open).unwrap().count(), 0);
}

#[test]
fn no_name_is_asked_for_when_the_client_did_not_detach_by_the_key() {
    let dir = private();
    let mut first = dir.term(&[], &[]);
    first.wait_for("demo-screen-repaint-1");

    // Taken over from another terminal.
    let mut second = dir.term(&["attach"], &[]);
    second.wait_for("demo-screen-repaint-2");
    assert_eq!(first.exit_code(), 0);
    first.wait_for("[demo-view-1 was attached from another terminal]");
    assert!(!first.text().contains(PROMPT));

    // The host lost: its program goes with its terminal.
    let pid = dir.program_pid();
    let ps = Command::new("ps")
        .args(["-o", "ppid=", "-p", &pid.to_string()])
        .output()
        .unwrap();
    let host: i32 = String::from_utf8_lossy(&ps.stdout).trim().parse().unwrap();
    let host = rustix::process::Pid::from_raw(host).unwrap();
    rustix::process::kill_process(host, rustix::process::Signal::KILL).unwrap();
    assert_eq!(second.exit_code(), 1);
    second.wait_for("[lost the connection to demo-view-1]");
    assert!(second.cooked());
    assert!(!second.text().contains(PROMPT));
}

#[test]
fn a_bare_attach_picks_among_several_and_renames_in_place() {
    let dir = private();
    detached(&dir, "");
    detached(&dir, "");

    let mut pick = dir.term(&["attach"], &[]);
    pick.wait_for("> demo-view-1");
    pick.wait_for("demo-view-2");
    pick.wait_for("rename");
    // `r` labels the highlighted session in place.
    pick.type_(b"r");
    pick.wait_for("set label");
    pick.type_(b"work\r");
    dir.wait_listed("work (demo-view-1)\tdetached");
    // Then down to the other one.
    pick.type_(b"j\r");
    dir.wait_listed("demo-view-2\tattached");
    assert!(!pick.text().contains("start new"));
    pick.type_(b"\x1b\x04");
    pick.answer(1, b"\r");
    assert_eq!(pick.exit_code(), 0);

    // Labels show in the picker; quitting it attaches nothing.
    let mut quit = dir.term(&["attach"], &[]);
    quit.wait_for("> work (demo-view-1)");
    quit.type_(b"q");
    assert_eq!(quit.exit_code(), 0);
    assert!(quit.cooked());
    assert!(!dir.run(&["sessions"]).contains("\tattached"));

    for name in ["work", "view-2"] {
        assert!(dir.run(&["kill", name]).contains("stopped"));
    }
    assert!(dir.is_empty());
}

#[test]
fn the_switch_key_moves_a_client_between_sessions() {
    let dir = private();
    detached(&dir, "one");
    detached(&dir, "");

    let mut t = dir.term(&["attach", "one"], &[]);
    t.wait_for("demo-screen-repaint-2");
    dir.wait_listed("one (demo-view-1)\tattached");

    // The plain encoding: the picker, the current session marked and the
    // other offered first.
    let mark = t.seen_len();
    t.type_(b"\x1b\x13");
    t.wait_for_after(mark, "(current)");
    t.wait_for_after(mark, "> demo-view-2");
    t.type_(b"\r");
    dir.wait_listed("demo-view-2\tattached");
    dir.wait_listed("one (demo-view-1)\tdetached");

    // CSI-u, and back.
    let mark = t.seen_len();
    t.type_(b"\x1b[115;7u");
    t.wait_for_after(mark, "(current)");
    t.wait_for_after(mark, "> one (demo-view-1)");
    t.type_(b"\r");
    dir.wait_listed("one (demo-view-1)\tattached");
    dir.wait_listed("demo-view-2\tdetached");

    // Quitting the picker returns to the same session, repainted.
    let mark = t.seen_len();
    t.type_(b"\x1b\x13");
    t.wait_for_after(mark, "(current)");
    dir.wait_listed("one (demo-view-1)\tdetached");
    t.type_(b"q");
    dir.wait_listed("one (demo-view-1)\tattached");
    dir.wait_listed("after 4 repaints");
    assert!(!t.text().contains(PROMPT), "a switch asks for no name");

    // Detached at last: named already, so not asked.
    t.type_(b"\x1b\x04");
    assert_eq!(t.exit_code(), 0);
    t.wait_for("detached: one");
    assert!(t.cooked());
    assert!(!t.text().contains(PROMPT));
    for name in ["one", "view-2"] {
        assert!(dir.run(&["kill", name]).contains("stopped"));
    }
}

#[test]
fn an_error_at_the_start_stays_in_sight() {
    let dir = private();
    let mut t = dir.term(&["fail"], &[]);
    assert_eq!(t.exit_code(), 1);
    t.wait_for("demo-fail: no such input");
    t.wait_for("[demo-fail-1 exited with status 1]");
    // On the user's own screen, which nothing left, and in sight there,
    // whatever backtrace setting the test runner has.
    assert!(!t.text().contains("\x1b[?1049"), "{:?}", t.text());
    t.wait_screen("demo-fail: no such input");
    assert!(t.cooked());
    assert!(dir.is_empty());
}

#[test]
fn a_long_error_at_the_start_reaches_the_terminal_whole_before_the_end_is_said() {
    // Several KB said at once as the program ends, more than the screen
    // holds: all of it relayed, in order, and then the client's word on
    // the end, on the screen's last lines.
    let dir = private();
    let mut t = dir.term(&["fail"], &[("RUST_BACKTRACE", "1")]);
    assert_eq!(t.exit_code(), 1);
    t.wait_for("[demo-fail-1 exited with status 1]");
    let text = t.text();
    let first = text.find("demo-fail: no such input").expect("the error");
    let frame = text.find("199: demo::frame_199").expect("the last frame");
    let last = text
        .find("demo-fail: end of trace")
        .expect("the trace's end");
    let end = text.find("[demo-fail-1 exited").unwrap();
    assert!(first < frame && frame < last && last < end, "{text:?}");
    assert!(text.len() > 8 * 1024, "{} bytes", text.len());
    let screen = t.screen();
    assert!(screen.contains("demo-fail: end of trace"), "{screen}");
    assert!(
        screen.contains("[demo-fail-1 exited with status 1]"),
        "{screen}"
    );
    assert!(!t.alternate() && t.cooked());
    assert!(dir.is_empty());
}

#[test]
fn the_terminal_mode_is_read_after_the_program_is_gone() {
    // The tool, a session leader on the terminal, has ended: its mode is
    // still read (macOS revokes the test's own handle on that terminal
    // when its controlling process ends).
    let dir = private();
    let mut t = dir.term(&["which"], &[]);
    t.status();
    assert!(t.child.try_wait().unwrap().is_some());
    assert!(t.cooked());
}

#[test]
fn kill_ends_a_program_still_working_before_its_screen() {
    let dir = private();
    let mut t = dir.term(&["slow"], &[]);
    t.wait_for("demo-slow loading");
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-slow-1 stopped"), "{said:?}");
    assert!(!said.contains("waiting for"), "{said:?}");
    // Ended by the SIGTERM, as by default.
    assert_eq!(t.exit_code(), 128 + 15);
    t.wait_for("[demo-slow-1 exited with status 143]");
    assert!(t.cooked());
    assert!(dir.is_empty());
}

#[test]
fn the_detach_key_works_while_the_program_reads_nothing() {
    let dir = private();
    let mut t = dir.term(&["stall"], &[]);
    t.wait_for("demo-stall-stalled");
    // Typed far past what the program, its host and the socket take.
    let mut flood = t.master.try_clone().unwrap();
    let (typed, all_typed) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let chunk = [b'x'; 1 << 16];
        for _ in 0..128 {
            if flood.write_all(&chunk).is_err() {
                break;
            }
        }
        let _ = typed.send(());
    });
    all_typed
        .recv_timeout(WAIT)
        .expect("the client stopped reading the terminal");
    t.type_(b"\x1b\x04");
    t.answer(1, b"\r");
    assert_eq!(t.exit_code(), 0);
    t.wait_for("[detached from demo-stall-1");
    assert!(t.cooked());
    assert!(dir.run(&["kill"]).contains("demo-stall-1 stopped"));
}

#[test]
fn a_stop_signal_in_the_switch_picker_ends_the_client() {
    use std::os::unix::process::ExitStatusExt;
    let dir = private();
    detached(&dir, "");
    let mut t = dir.term(&["attach"], &[]);
    t.wait_for("demo-screen-repaint-2");
    t.type_(b"\x1b\x13");
    t.wait_for("(current)");
    t.signal(rustix::process::Signal::TERM);
    assert_eq!(t.status().signal(), Some(15));
}

#[test]
fn kill_wakes_a_screen_waiting_long_for_a_key() {
    let dir = private();
    // It waits a minute for a key: the attach, and then the kill, wake it.
    let mut t = dir.term(&["idle"], &[]);
    t.wait_for("idle-repaint-1");
    let asked = Instant::now();
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-idle-1 stopped"), "{said:?}");
    assert!(!said.contains("waiting for"), "{said:?}");
    assert!(
        asked.elapsed() < Duration::from_secs(3),
        "{:?}",
        asked.elapsed()
    );
    t.wait_for("idle done after 1 repaints");
    assert_eq!(t.exit_code(), 0);
    assert!(t.cooked());
    assert!(dir.is_empty());
}

/// A stand-in host for session `name` in `dir` that takes one watch and
/// answers it `delay` after it came, attached; when it answered, then the
/// connection, held open by the caller as a host holds its watchers.
fn late_host(
    dir: &Private,
    name: &str,
    delay: Duration,
) -> std::thread::JoinHandle<(Instant, std::os::unix::net::UnixStream)> {
    use std::io::Read;
    let listener =
        std::os::unix::net::UnixListener::bind(dir.path().join(format!("{name}.sock"))).unwrap();
    std::thread::spawn(move || {
        let (mut conn, _) = listener.accept().unwrap();
        // A frame: its tag (a watch), then an empty body's length.
        let mut head = [0u8; 5];
        conn.read_exact(&mut head).unwrap();
        assert_eq!(head, [7, 0, 0, 0, 0], "a watch");
        std::thread::sleep(delay);
        conn.write_all(b"A").unwrap();
        (Instant::now(), conn)
    })
}

/// A screen started inside a session whose host says it is attached only
/// `delay` after the screen asked, while the screen's first round waits a
/// minute for a key: it must still repaint and draw soon after the word.
fn a_late_first_word_is_heard(delay: Duration) {
    let dir = private();
    let host = late_host(&dir, "demo-host-1", delay);
    let mut t = dir.term(&["idle"], &[(legume_tui::SESSION_VAR, "demo-host-1")]);
    let (said, _conn) = host.join().unwrap();
    t.wait_for("idle-repaint-1");
    assert!(
        said.elapsed() < Duration::from_secs(2),
        "drawn {:?} after the host said attached",
        said.elapsed()
    );
    t.type_(b"q");
    t.wait_for("idle done after 1 repaints");
    assert_eq!(t.exit_code(), 0);
    assert!(t.cooked());
}

#[test]
fn a_nested_screen_hears_a_host_that_answers_late() {
    a_late_first_word_is_heard(Duration::from_millis(300));
}

#[test]
fn a_nested_screen_hears_a_host_that_answers_later_than_it_waits() {
    a_late_first_word_is_heard(Duration::from_millis(1500));
}

#[test]
fn kill_reaches_a_child_run_before_the_screen_then_ends_the_program() {
    let dir = private();
    let mut t = dir.term(&["first"], &[]);
    t.wait_for("nested-repaint-1");
    // Passed on to the child, which quits through its own stop path; the
    // program, with no screen up, then ends as `kill` ends it there.
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-first-1 stopped"), "{said:?}");
    t.wait_for("nested done after 1 repaints");
    assert_eq!(t.exit_code(), 128 + 15);
    t.wait_for("[demo-first-1 exited with status 143]");
    assert!(!t.text().contains("demo-first after"));
    assert!(t.cooked());
    assert!(dir.is_empty());
}

/// Detach `t` by the key, keeping its session unnamed.
fn detach(t: &mut legume_tui::testing::Term) {
    t.type_(b"\x1b\x04");
    t.answer(1, b"\r");
    assert_eq!(t.exit_code(), 0);
}

#[test]
fn what_a_program_leaves_is_said_once_where_it_will_be_read() {
    let dir = private();
    // Attached as it ends: said on the terminal, after the screen.
    let mut t = dir.term(&["bye"], &[]);
    t.wait_screen("demo-bye waiting");
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    t.wait_for("demo-bye: left by the quit path");
    assert!(!t.alternate(), "said on the user's own screen");
    assert!(dir.is_empty(), "taken once said: {:?}", dir.path());

    // Ended by a `kill` from another terminal while one is attached: the
    // attached one says it, and the kill does not.
    for _ in 0..3 {
        let mut t = dir.term(&["bye"], &[]);
        t.wait_screen("demo-bye waiting");
        let said = dir.run(&["kill"]);
        assert!(said.contains("demo-bye-1 stopped"), "{said:?}");
        assert!(!said.contains("left by the quit path"), "{said:?}");
        assert_eq!(t.exit_code(), 0);
        t.wait_for("demo-bye: left by the quit path");
        assert!(dir.is_empty(), "taken once said: {:?}", dir.path());
        assert!(!dir.run(&["sessions"]).contains("ended"), "said once");
    }

    // Ended by a `kill` with nobody attached: the kill says it.
    let mut t = dir.term(&["bye"], &[]);
    t.wait_screen("demo-bye waiting");
    detach(&mut t);
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-bye-1 stopped"), "{said:?}");
    assert!(said.contains("demo-bye: left by the quit path"), "{said:?}");
    assert!(dir.is_empty());

    // Ended on its own while detached: the next `sessions` says it, once.
    let folder = tempfile::tempdir().unwrap();
    let mut t = dir.term_with(&["bye"], &[], Setup::new().cwd(folder.path()));
    t.wait_screen("demo-bye waiting");
    detach(&mut t);
    let pid = dir.program_pid();
    std::fs::write(folder.path().join("end"), "").unwrap();
    dir.gone(pid);
    let listed = dir.run(&["sessions"]);
    assert!(
        listed.contains("demo-bye-1 ended: demo-bye: left on its own"),
        "{listed:?}"
    );
    assert!(!dir.run(&["sessions"]).contains("ended"), "said once");

    // Or the next start of the tool does.
    std::fs::remove_file(folder.path().join("end")).unwrap();
    let mut t = dir.term_with(&["bye"], &[], Setup::new().cwd(folder.path()));
    t.wait_screen("demo-bye waiting");
    detach(&mut t);
    let pid = dir.program_pid();
    std::fs::write(folder.path().join("end"), "").unwrap();
    dir.gone(pid);
    let mut again = dir.term(&["bye"], &[]);
    again.wait_for("demo-bye-1 ended: demo-bye: left on its own");
    again.wait_screen("demo-bye waiting");
    again.type_(b"q");
    assert_eq!(again.exit_code(), 0);
    assert!(dir.is_empty());
}

#[test]
fn an_error_held_while_detached_is_seen_on_the_next_attach() {
    let dir = private();
    let folder = tempfile::tempdir().unwrap();
    let mut t = dir.term_with(&["error"], &[], Setup::new().cwd(folder.path()));
    t.wait_screen("demo-error waiting");
    detach(&mut t);
    let pid = dir.program_pid();
    std::fs::write(folder.path().join("end"), "").unwrap();
    dir.wait_listed("stopped: demo-error: the input went away");
    assert!(alive(pid), "held for the user");

    let mut back = dir.term(&["attach"], &[]);
    back.wait_screen("any key quits");
    assert!(back.screen().contains("demo-error: the input went away"));
    let shown = back.seen_len();
    back.type_(b"x");
    assert_eq!(back.exit_code(), 2);
    back.wait_for_after(shown, "Error: demo-error: the input went away");
    back.wait_for("[demo-error-1 exited with status 2]");
    // Said by the program, which a terminal heard: not left as well.
    said_once_after_the_screen(&back.text(), "demo-error: the input went away");
    assert!(back.cooked());
    assert!(dir.is_empty());

    // Ended by a `kill` while held: the kill says it, once.
    std::fs::remove_file(folder.path().join("end")).unwrap();
    let mut t = dir.term_with(&["error"], &[], Setup::new().cwd(folder.path()));
    t.wait_screen("demo-error waiting");
    detach(&mut t);
    std::fs::write(folder.path().join("end"), "").unwrap();
    dir.wait_listed("stopped: demo-error");
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-error-1 stopped"), "{said:?}");
    assert_eq!(said.matches("the input went away").count(), 1, "{said:?}");
    assert!(dir.is_empty());
}

/// That `needle` is said once in `text` after the last time it left the
/// alternate screen.
fn said_once_after_the_screen(text: &str, needle: &str) {
    let after = text.rfind("\x1b[?1049l").map_or(text, |at| &text[at..]);
    assert_eq!(after.matches(needle).count(), 1, "{after:?}");
}

#[test]
fn an_error_seen_as_it_happens_is_said_once() {
    let dir = private();
    let folder = tempfile::tempdir().unwrap();
    let end = folder.path().join("end");
    // Attached: the program says it, through the session.
    let mut t = dir.term_with(&["error"], &[], Setup::new().cwd(folder.path()));
    t.wait_screen("demo-error waiting");
    std::fs::write(&end, "").unwrap();
    assert_eq!(t.exit_code(), 2);
    t.wait_for("[demo-error-1 exited with status 2]");
    said_once_after_the_screen(&t.text(), "demo-error: the input went away");
    assert!(dir.is_empty(), "nothing left: {:?}", dir.sockets());
    // Plain: the program alone says it.
    std::fs::remove_file(&end).unwrap();
    let plain = [(legume_tui::NO_DETACH_VAR, "1")];
    let mut t = dir.term_with(&["error"], &plain, Setup::new().cwd(folder.path()));
    t.wait_screen("demo-error waiting");
    std::fs::write(&end, "").unwrap();
    assert_eq!(t.exit_code(), 2);
    t.wait_for("Error: demo-error: the input went away");
    said_once_after_the_screen(&t.text(), "demo-error: the input went away");
}

#[test]
fn kill_waits_for_a_child_run_as_work_a_stop_waits_for() {
    let dir = private();
    let mut t = dir.term(&["task"], &[]);
    t.wait_for("nested-repaint-1");
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-task-1 stopped"), "{said:?}");
    // Passed on to the child, then the work went on to its end.
    t.wait_for("nested done after 1 repaints");
    assert_eq!(t.exit_code(), 0);
    t.wait_for("demo-task done");
    t.wait_for("demo-task stopped after its work");
    assert!(dir.is_empty());
}

#[test]
fn kill_waits_for_work_a_stop_waits_for() {
    let dir = private();
    let mut t = dir.term(&["work"], &[]);
    t.wait_for("demo-work working");
    let said = dir.run(&["kill"]);
    assert!(said.contains("demo-work-1 stopped"), "{said:?}");
    // Not ended by the signal: the work finished, then the program saw
    // the stop.
    assert_eq!(t.exit_code(), 0);
    t.wait_for("demo-work done");
    t.wait_for("demo-work stopped after its work");
    assert!(dir.is_empty());
}

/// The fit's and the save's pids, from the program's status.
fn jobs_of(dir: &Private) -> (u32, u32) {
    let listed = dir.wait_listed("jobs fit=");
    let pid = |key: &str| -> u32 {
        let at = listed.find(key).unwrap() + key.len();
        let digits: String = listed[at..]
            .chars()
            .take_while(char::is_ascii_digit)
            .collect();
        digits.parse().unwrap()
    };
    (pid("fit="), pid("save="))
}

/// End process `pid`, if it is still there.
fn end(pid: u32) {
    if let Some(p) = rustix::process::Pid::from_raw(pid as i32) {
        let _ = rustix::process::kill_process(p, rustix::process::Signal::KILL);
    }
}

#[test]
fn a_lost_host_ends_the_fits_its_program_held_but_not_a_save() {
    let dir = private();
    let folder = tempfile::tempdir().unwrap();
    let setup = || Setup::new().cwd(folder.path());
    let mut t = dir.term_with(&["jobs"], &[], setup());
    t.wait_screen("demo-jobs fit=");
    let (fit, save) = jobs_of(&dir);
    let program = dir.program_pid();
    // The host lost: its program hangs up, and passes it on.
    let ps = Command::new("ps")
        .args(["-o", "ppid=", "-p", &program.to_string()])
        .output()
        .unwrap();
    let host: i32 = String::from_utf8_lossy(&ps.stdout).trim().parse().unwrap();
    let host = rustix::process::Pid::from_raw(host).unwrap();
    rustix::process::kill_process(host, rustix::process::Signal::KILL).unwrap();
    assert_eq!(t.exit_code(), 1);
    dir.gone(program);
    dir.gone(fit);
    let spared = alive(save);
    // Its stderr went with the host; a save that bears that finishes.
    std::fs::write(folder.path().join("go"), "").unwrap();
    let saved = folder.path().join("saved");
    let start = Instant::now();
    while !saved.exists() && start.elapsed() < WAIT {
        std::thread::sleep(Duration::from_millis(20));
    }
    end(save);
    assert!(spared, "a save outlives the hangup to write what it has");
    assert!(saved.exists(), "the save wrote what it had");
    std::fs::remove_file(folder.path().join("go")).unwrap();

    // Quit by its key: the fit is stopped, the save let finish.
    let mut t = dir.term_with(&["jobs"], &[], setup());
    t.wait_screen("demo-jobs fit=");
    let (fit, save) = jobs_of(&dir);
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    dir.gone(fit);
    let spared = alive(save);
    end(save);
    assert!(spared);
}

#[test]
fn the_probe_reads_the_terminals_answers_off_the_screen() {
    let dir = private();
    let kitty = Replies::plain()
        .kitty()
        .keyboard(1)
        .cell(10, 20)
        .background(1, 2, 3);
    let ssh = [("SSH_CONNECTION", "10.0.0.1 1 10.0.0.2 22")];
    let mut t = dir.term_with(&["probe"], &ssh, Setup::new().replies(kitty.clone()));
    t.wait_screen("protocol=Kitty");
    let screen = t.screen();
    for line in [
        "background=Some((1, 2, 3))",
        "keyboard=true",
        "transport=Inline",
    ] {
        assert!(screen.contains(line), "{line:?} in {screen}");
    }
    assert!(t.alternate());
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    assert!(!t.alternate());

    // Plain too, the terminal its own, reached here.
    let plain = [
        (legume_tui::NO_DETACH_VAR, "1"),
        ("SSH_CONNECTION", ""),
        ("SSH_CLIENT", ""),
        ("SSH_TTY", ""),
    ];
    let mut t = dir.term_with(&["probe"], &plain, Setup::new().replies(kitty));
    t.wait_screen("keyboard=true");
    assert!(t.screen().contains("protocol=Kitty"));
    assert!(t.screen().contains("transport=File"));
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);

    // A terminal with nothing more than every one has.
    let mut t = dir.term(&["probe"], &[]);
    t.wait_screen("protocol=Halfblocks");
    assert!(t.screen().contains("keyboard=false"));
    assert!(t.screen().contains("background=None"));
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);

    // One that never answers: its environment's guess.
    let kitty_env = [("KITTY_WINDOW_ID", "1")];
    let silent = Setup::new().replies(Replies::silent());
    let mut t = dir.term_with(&["probe"], &kitty_env, silent);
    t.wait_screen("protocol=Kitty");
    assert!(t.screen().contains("keyboard=false"));
    t.type_(b"q");
    assert_eq!(t.exit_code(), 0);
    assert!(dir.is_empty());
}
