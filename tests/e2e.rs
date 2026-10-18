//! The demo example driven end to end through pseudo-terminals standing
//! in for the user's terminal: launch hosted, detach with the key, attach
//! again and see a repaint, lose a client, stop one by a signal, refuse a
//! nested host and repaint a nested screen, resume through the picker,
//! name a session on detach and later, pick on a bare attach, switch
//! between sessions with the key, and kill, leaving no process or socket
//! behind. Programs that misbehave: one that fails at once, one slow to
//! start its screen, one that stops reading its input. The demo sets
//! SIGPIPE to its default, as some tools do.

#![cfg(unix)]

use legume_tui::testing::{alive, Private, PROMPT, WAIT};
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

    // Killed by name: no socket, status or label left.
    for name in ["work", "other"] {
        assert!(dir.run(&["kill", name]).contains("stopped"));
    }
    assert!(dir.is_empty());
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
    // On the user's own screen, which nothing left.
    assert!(!t.text().contains("\x1b[?1049"), "{:?}", t.text());
    assert!(t.cooked());
    assert!(dir.is_empty());
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
