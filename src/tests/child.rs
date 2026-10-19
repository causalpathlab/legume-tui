//! Children run where they can be stopped, their logs and bars followed.

use super::*;
use std::os::unix::process::CommandExt;
use std::sync::Arc;
use std::time::{Duration, Instant};

#[test]
fn log_lines_lose_their_lead_colours_and_redraws() {
    assert_eq!(log_line("[2026-01-01 INFO tool] fitting"), "fitting");
    assert_eq!(log_line("\u{1b}[32mok\u{1b}[0m  "), "ok");
    assert_eq!(log_line("[not a prefix"), "[not a prefix");
    let mut seen = Vec::new();
    let log = "step 1\rstep 2\n[t INFO x] done\n".as_bytes();
    let last = follow_log(Some(log), |l| seen.push(l.to_string()));
    assert_eq!(seen, ["step 1", "step 2", "done"]);
    assert_eq!(last, "done");
}

#[test]
fn bar_frames_are_progress_and_the_rest_is_log() {
    assert_eq!(
        progress_of("#####----- 3/10 (2s) GENE1 blocks"),
        Some(Progress {
            pos: 3,
            len: 10,
            what: "GENE1 blocks".into()
        })
    );
    assert_eq!(
        progress_of("---------- 0/140 (1 minute) sweeps")
            .unwrap()
            .what,
        "sweeps"
    );
    assert_eq!(progress_of("⠁ streamed 12 fragments").unwrap().len, 0);
    assert_eq!(progress_of("Selected 300 / 300 features"), None);
    assert_eq!(progress_of("-- not a bar"), None);
    assert_eq!(progress_of("- a list item"), None, "no spinner tick");

    // As a terminal writes it: frames after `\r`, no new line between.
    let raw = "[t INFO x] start\r\n[00:00:01] \u{1b}[36m##--\u{1b}[0m 1/2 (1s) epochs\r\u{1b}[2K\
               [00:00:02] ####-- 2/2 (0s) epochs\r\u{1b}[2K[t INFO x] done\r\n";
    let mut said = Vec::new();
    let last = follow(Some(raw.as_bytes()), |s| said.push(s));
    assert_eq!(last, "done");
    let epochs = |pos| {
        Said::Progress(Progress {
            pos,
            len: 2,
            what: "epochs".into(),
        })
    };
    assert_eq!(
        said,
        [
            Said::Line("start".into()),
            epochs(1),
            epochs(2),
            Said::Line("done".into()),
        ]
    );
}

#[test]
fn a_bar_message_carries_its_first_number_as_the_score() {
    let said = |what: &str| {
        Progress {
            what: what.into(),
            ..Progress::default()
        }
        .score()
    };
    assert_eq!(said("fit gap=0.412 recon=0.3"), Some(0.412));
    assert_eq!(said("llik=-1843.2 kl=12.3"), Some(-1843.2));
    assert_eq!(said("fit CT1 nll=2.5"), Some(2.5));
    assert_eq!(said("fit 3 nll=2.5"), Some(2.5));
    assert_eq!(said("fit 3"), None);
    assert_eq!(said("nll=inf nll=1.5"), Some(1.5));
    assert_eq!(said("reading cells"), None);
}

/// `sh -c script`, in a process group of its own, as a hosted session
/// starts it.
fn grouped(script: &str) -> Command {
    let mut command = Command::new("sh");
    command.args(["-c", script]).process_group(0);
    command
}

#[test]
fn a_child_runs_on_a_terminal_so_its_bars_draw() {
    // A bar library draws only on a terminal: so does this stand-in.
    let script = "if [ -t 2 ]; then printf '[00:00:00] #--- 1/4 (3s) steps\\r' >&2; fi; \
                  echo '[t INFO x] ok' >&2";
    let mut command = Command::new("sh");
    command.args(["-c", script]);
    let mut said = Vec::new();
    run_one(command, &Stopper::new(), |s| said.push(s)).unwrap();
    assert_eq!(
        said,
        [
            Said::Progress(Progress {
                pos: 1,
                len: 4,
                what: "steps".into()
            }),
            Said::Line("ok".into()),
        ]
    );
}

#[test]
fn a_failed_command_gives_its_reason_and_a_stopped_one_says_so() {
    let sh = |script: &str| {
        let mut c = Command::new("sh");
        c.args(["-c", script]);
        run_one(c, &Stopper::new(), |_| {})
    };
    assert_eq!(
        sh("echo one >&2; echo 'Error: it broke' >&2; exit 3"),
        Err(Failed::Exit("it broke".into()))
    );
    // Killed while it worked: how it ended, and what it was doing.
    assert_eq!(
        sh("echo '[t INFO x] writing outputs' >&2; kill -9 $$"),
        Err(Failed::Exit(
            "signal: 9 (SIGKILL), after: writing outputs".into()
        ))
    );
    assert_eq!(sh("exit 2"), Err(Failed::Exit("exit status: 2".into())));

    // Stopped before it started: it gets the stop at once.
    let stopper = Stopper::new();
    stopper.stop();
    let start = Instant::now();
    assert_eq!(
        run_one(grouped("sleep 5"), &stopper, |_| {}),
        Err(Failed::Stopped)
    );
    assert!(start.elapsed() < Duration::from_secs(4));

    let c = Command::new("/no/such/program");
    assert!(matches!(
        run_one(c, &Stopper::new(), |_| {}),
        Err(Failed::Start(why)) if why.starts_with("cannot run program:")
    ));
}

#[test]
fn a_stop_interrupts_the_child_and_what_it_started() {
    // The child traps the interrupt and wraps up, as a fit does, but only
    // once the grandchild it waits for is interrupted too: they share the
    // group the child leads.
    let script = "trap 'echo wrapped >&2; exit 0' INT; sleep 30";
    let stopper = Arc::new(Stopper::new());
    let s = Arc::clone(&stopper);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        assert!(!s.stop());
    });
    let start = Instant::now();
    let mut said = Vec::new();
    let out = run_one(grouped(script), &stopper, |s| said.push(s));
    assert_eq!(out, Err(Failed::Stopped));
    assert!(said.contains(&Said::Line("wrapped".into())));
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_second_stop_kills_a_child_that_ignores_the_interrupt() {
    let stopper = Arc::new(Stopper::new());
    let s = Arc::clone(&stopper);
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        assert!(!s.stop());
        std::thread::sleep(Duration::from_millis(300));
        assert!(s.stop());
    });
    let start = Instant::now();
    assert_eq!(
        run_one(grouped("trap '' INT; sleep 30"), &stopper, |_| {}),
        Err(Failed::Stopped)
    );
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn quitting_lets_a_save_finish_and_stops_anything_else() {
    let save = Stopper::saving();
    assert!(save.saves());
    let mut child = grouped("sleep 0.3; echo saved")
        .stdout(Stdio::piped())
        .spawn()
        .unwrap();
    let mut out = child.stdout.take().unwrap();
    save.hold(child);
    assert!(!save.quit(), "a save is let finish");
    assert!(!save.is_stopped());
    let mut said = String::new();
    out.read_to_string(&mut said).unwrap();
    assert_eq!(said, "saved\n");
    assert!(save.release().unwrap().wait().unwrap().success());
    assert!(save.release().is_none(), "released once");

    let fit = Stopper::new();
    fit.hold(grouped("sleep 30").spawn().unwrap());
    assert!(fit.quit());
    assert!(fit.is_stopped());
    let start = Instant::now();
    assert!(!fit.release().unwrap().wait().unwrap().success());
    assert!(start.elapsed() < Duration::from_secs(10));
}

#[test]
fn a_child_in_the_programs_group_is_stopped_alone() {
    // Not leading a group (plain): the stop goes to it, not to ours.
    let stopper = Stopper::new();
    stopper.hold(Command::new("sleep").arg("30").spawn().unwrap());
    stopper.kill();
    let status = stopper.release().unwrap().wait().unwrap();
    assert!(!status.success());
}
