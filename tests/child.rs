//! The child runner through the public API, as a tool's worker thread
//! uses it: a child set up as the session starts its children, its bars
//! read off the pseudo-terminal its stderr is, a stop reaching it and
//! what it started, a save let finish when the program quits, and a child
//! the tool started itself polled while held.

#![cfg(all(unix, feature = "child"))]

use legume_tui::child::{run_one, Failed, Progress, Said, Stopper};
use legume_tui::{Group, Session, Tool};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

const TOOL: Tool = Tool::new("tool");

#[test]
fn a_worker_follows_a_childs_bars_and_stops_it() {
    let env = Session::plain(&TOOL).child_env();
    let stopper = Arc::new(Stopper::new());
    let held = Arc::clone(&stopper);
    let worker = std::thread::spawn(move || {
        // Bars, a line, then a wait the stop cuts short.
        let script = "[ -t 2 ] && printf '##-- 1/2 (1s) fit nll=2.5\\r' >&2; \
                      echo '[t INFO tool] fitting' >&2; exec sleep 30";
        let mut fit = Command::new("sh");
        env.apply(fit.args(["-c", script]), Group::Own);
        let mut said = Vec::new();
        let out = run_one(fit, &held, |s| said.push(s));
        (out, said)
    });
    std::thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    assert!(stopper.quit(), "not a save: stopped");
    let (out, said) = worker.join().unwrap();
    assert_eq!(out, Err(Failed::Stopped));
    assert!(asked.elapsed() < Duration::from_secs(10));
    let bar = Progress::new(1, 2, "fit nll=2.5");
    assert_eq!(bar.score(), Some(2.5));
    assert_eq!(said, [Said::Progress(bar), Said::Line("fitting".into())]);
}

#[test]
fn quitting_lets_a_save_run_by_the_runner_finish() {
    let save = Arc::new(Stopper::saving());
    let held = Arc::clone(&save);
    let worker = std::thread::spawn(move || {
        let mut write = Command::new("sh");
        write.args(["-c", "sleep 0.5; echo '[t INFO tool] saved' >&2"]);
        let mut said = Vec::new();
        (run_one(write, &held, |s| said.push(s)), said)
    });
    std::thread::sleep(Duration::from_millis(100));
    assert!(!save.quit());
    let (out, said) = worker.join().unwrap();
    assert_eq!(out, Ok(()));
    assert_eq!(said, [Said::Line("saved".into())]);
}

#[test]
fn a_stop_reaches_what_a_plain_programs_child_started() {
    let env = Session::plain(&TOOL).child_env();
    let stopper = Arc::new(Stopper::new());
    let held = Arc::clone(&stopper);
    let worker = std::thread::spawn(move || {
        // A grandchild that keeps the log open, and ignores an interrupt
        // (as a shell's background job does).
        let mut fit = Command::new("sh");
        env.apply(fit.args(["-c", "sleep 300 & exec sleep 300"]), Group::Own);
        run_one(fit, &held, |_| {})
    });
    std::thread::sleep(Duration::from_millis(300));
    let asked = Instant::now();
    assert!(!stopper.stop(), "interrupted first");
    std::thread::sleep(Duration::from_millis(200));
    assert!(stopper.stop(), "then killed, with the grandchild");
    assert_eq!(worker.join().unwrap(), Err(Failed::Stopped));
    assert!(asked.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_child_held_is_polled_as_a_loop_does_and_let_go_when_it_ends() {
    let env = Session::plain(&TOOL).child_env();
    let stopper = Stopper::new();
    assert_eq!(stopper.try_wait().unwrap(), None, "none held");
    let mut job = Command::new("sh");
    let child = env
        .apply(job.args(["-c", "sleep 0.3; exit 3"]), Group::Own)
        .spawn()
        .unwrap();
    let pid = child.id();
    stopper.hold(child);
    assert_eq!(stopper.id(), Some(pid));
    assert_eq!(stopper.try_wait().unwrap(), None, "still running");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = stopper.try_wait().unwrap() {
            break status;
        }
        assert!(start.elapsed() < Duration::from_secs(10));
        std::thread::sleep(Duration::from_millis(20));
    };
    assert_eq!(status.code(), Some(3));
    assert_eq!(stopper.id(), None, "let go once it ended");
    assert!(stopper.release().is_none());
}
