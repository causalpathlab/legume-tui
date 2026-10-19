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

#[test]
fn a_child_that_ignores_the_interrupt_runs_on_until_killed() {
    let env = Session::plain(&TOOL).child_env();
    let stopper = Arc::new(Stopper::new());
    let held = Arc::clone(&stopper);
    let (ready, is_ready) = std::sync::mpsc::channel();
    let worker = std::thread::spawn(move || {
        // Says so once the interrupt is ignored, so the stop comes after.
        let mut fit = Command::new("sh");
        let script = "trap '' INT; echo ready >&2; sleep 30";
        env.apply(fit.args(["-c", script]), Group::Own);
        run_one(fit, &held, |s| {
            if s == Said::Line("ready".into()) {
                let _ = ready.send(());
            }
        })
    });
    is_ready
        .recv_timeout(Duration::from_secs(10))
        .expect("the child started");
    assert!(!stopper.stop(), "interrupted first");
    std::thread::sleep(Duration::from_secs(1));
    assert!(!worker.is_finished(), "the interrupt did not end it");
    let asked = Instant::now();
    assert!(stopper.stop(), "then killed");
    assert_eq!(worker.join().unwrap(), Err(Failed::Stopped));
    assert!(asked.elapsed() < Duration::from_secs(10));
}

// TEMPORARY diagnostics for macOS: a fresh script file, as pinto's test
// writes one, stopped 50ms after it is held.
#[test]
fn diag_fresh_script_stopped_early() {
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;
    for (wait_ms, via_runner) in [(50u64, true), (50, false), (1500, true), (1500, false)] {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("slow");
        std::fs::write(
            &path,
            "#!/bin/sh\necho start >&2\ntrap '' INT\necho trapped >&2\nsleep 30\n",
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let env = Session::plain(&TOOL).child_env();
        let stopper = Arc::new(Stopper::new());
        let t0 = Instant::now();
        if via_runner {
            let held = Arc::clone(&stopper);
            let worker = std::thread::spawn(move || {
                let mut c = Command::new(&path);
                env.apply(&mut c, Group::Own);
                let mut said = Vec::new();
                let out = run_one(c, &held, |s| {
                    said.push(format!("{:?}@{}ms", s, t0.elapsed().as_millis()))
                });
                (out, said)
            });
            while stopper.id().is_none() {
                std::thread::sleep(Duration::from_millis(1));
            }
            std::thread::sleep(Duration::from_millis(wait_ms));
            let at = t0.elapsed().as_millis();
            stopper.stop();
            std::thread::sleep(Duration::from_millis(1000));
            let alive = !worker.is_finished();
            stopper.kill();
            let (out, said) = worker.join().unwrap();
            eprintln!(
                "DIAG runner wait={wait_ms} stop@{at}ms alive_after_1s={alive} out={out:?} said={said:?}"
            );
        } else {
            let mut c = Command::new(&path);
            env.apply(&mut c, Group::Own);
            let child = c.stdin(std::process::Stdio::null()).spawn().unwrap();
            stopper.hold(child);
            std::thread::sleep(Duration::from_millis(wait_ms));
            let at = t0.elapsed().as_millis();
            stopper.stop();
            let mut status = None;
            while status.is_none() && t0.elapsed().as_millis() < at + 1000 {
                status = stopper.try_wait().unwrap();
                std::thread::sleep(Duration::from_millis(5));
            }
            let ended = t0.elapsed().as_millis();
            if status.is_none() {
                stopper.kill();
                while stopper.try_wait().unwrap().is_none() {}
            }
            eprintln!(
                "DIAG held wait={wait_ms} stop@{at}ms ended@{ended}ms status={status:?} signal={:?}",
                status.and_then(|s| s.signal())
            );
        }
    }
}
