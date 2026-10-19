//! Children run where they can be stopped, their logs and bars followed.

use super::*;
use crate::child::Bar;
use std::cell::RefCell;
use std::os::unix::process::CommandExt;
use std::rc::Rc;
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

/// indicatif's frame `n` of a bar, padded as it pads to the line's width.
fn frame(n: u32) -> String {
    format!("[00:00:00] ####------ {n}/10 (0s) sweeps {n}: moves=4   ")
}

/// indicatif's clear of a bar drawn with a line under it: a redraw's start,
/// or, with nothing after, a finish.
const CLEAR: &str = "\u{1b}[1A\r\u{1b}[2K\u{1b}[1B\r\u{1b}[2K\u{1b}[1A";

/// A child's output as it streams: one chunk a read (split where it is
/// longer than the reader's buffer), and whether its bar shows, and is
/// visible, when each next read starts, after the last chunk's words.
struct Streamed {
    chunks: std::collections::VecDeque<Vec<u8>>,
    bar: Rc<RefCell<Option<Bar>>>,
    shown: Vec<bool>,
    visible: Vec<bool>,
    started: bool,
}

impl Read for Streamed {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if std::mem::replace(&mut self.started, true) {
            let bar = self.bar.borrow();
            self.shown.push(bar.as_ref().is_some_and(|b| b.shown));
            self.visible.push(bar.as_ref().is_some_and(Bar::visible));
        }
        let Some(mut chunk) = self.chunks.pop_front() else {
            return Ok(0);
        };
        if chunk.len() > buf.len() {
            self.chunks.push_front(chunk.split_off(buf.len()));
        }
        buf[..chunk.len()].copy_from_slice(&chunk);
        Ok(chunk.len())
    }
}

/// `chunks` streamed through [`follow`] into [`Bar::track`]: what was
/// said, and after each chunk whether the bar shows and is visible.
fn streamed(chunks: &[String]) -> (Vec<Said>, Vec<bool>, Vec<bool>) {
    let bar = Rc::new(RefCell::new(None));
    let mut log = Streamed {
        chunks: chunks.iter().map(|c| c.as_bytes().to_vec()).collect(),
        bar: Rc::clone(&bar),
        shown: Vec::new(),
        visible: Vec::new(),
        started: false,
    };
    let mut said = Vec::new();
    follow(Some(&mut log), |s| {
        Bar::track(&mut bar.borrow_mut(), &s);
        said.push(s);
    });
    (said, log.shown, log.visible)
}

/// The frames said.
fn frames(said: &[Said]) -> Vec<&Progress> {
    said.iter()
        .filter_map(|s| match s {
            Said::Progress(p) => Some(p),
            _ => None,
        })
        .collect()
}

#[test]
fn a_redraw_or_a_line_split_between_reads_keeps_the_bar_visible() {
    // A read can end anywhere: between a redraw's erase and its frame, or
    // in the middle of a line printed over the bar. Whatever is said for a
    // moment, the bar stays visible.
    let line = format!("{CLEAR}epoch 1 done\r\n{}", frame(3));
    let cut = line.find("done").unwrap();
    let chunks = [
        frame(1),
        CLEAR.to_string(),
        frame(2),
        line[..cut].to_string(),
        line[cut..].to_string(),
    ];
    let (_, _, visible) = streamed(&chunks);
    assert_eq!(visible, [false, true, true, true, true]);
}

#[test]
fn a_bar_comes_back_only_as_itself() {
    // A finished bar, cleared; another bar of the same length is drawn:
    // the finished one does not flash back.
    let other = "[00:00:00] #--------- 1/10 (0s) greedy 1: moves=7   ".to_string();
    let chunks = [
        frame(1),
        format!("{CLEAR}{}", frame(2)),
        CLEAR.to_string(),
        other,
    ];
    let (said, ..) = streamed(&chunks);
    let frames: Vec<u64> = frames(&said).iter().map(|p| p.pos).collect();
    assert_eq!(
        frames,
        [1, 2, 1],
        "the greedy bar's own frame, said at the end"
    );
}

/// The kind of each word said.
fn kinds(said: &[Said]) -> Vec<&'static str> {
    said.iter()
        .map(|s| match s {
            Said::Progress(_) => "frame",
            Said::Cleared => "cleared",
            Said::Line(_) => "line",
        })
        .collect()
}

/// indicatif's one-line clear: back to the line's start, erase it.
const CLEAR1: &str = "\r\u{1b}[2K";

#[test]
fn a_bar_on_the_child_s_screen_shows_while_it_streams() {
    // A frame is said whole, when the next redraw begins; between redraws
    // the bar is on screen, and each frame is said once.
    let chunks = [
        frame(1),
        format!("{CLEAR}{}", frame(2)),
        format!("{CLEAR}{}", frame(3)),
    ];
    let (said, shown, _) = streamed(&chunks);
    assert_eq!(shown, [false, true, true]);
    assert_eq!(kinds(&said), ["frame", "frame", "frame"]);
}

#[test]
fn a_bar_finished_and_cleared_is_hidden_at_once() {
    // Cleared, and the fit works on in silence: no line comes to hide it.
    for clear in [CLEAR, CLEAR1] {
        let chunks = [frame(1), format!("{clear}{}", frame(2)), clear.to_string()];
        let (said, shown, _) = streamed(&chunks);
        assert_eq!(shown, [false, true, false], "{clear:?}");
        assert_eq!(kinds(&said), ["frame", "frame", "cleared"], "{clear:?}");
    }
}

#[test]
fn a_line_printed_over_a_bar_leaves_it_shown_once_redrawn() {
    // indicatif clears the bar, prints the line, and draws the bar again.
    let chunks = [
        frame(1),
        format!("{CLEAR}{}", frame(2)),
        format!("{CLEAR}epoch 1 done\r\n{}", frame(3)),
    ];
    let (said, shown, _) = streamed(&chunks);
    assert_eq!(shown, [false, true, true]);
    assert!(said.contains(&Said::Line("epoch 1 done".into())));
}

#[test]
fn terminal_control_that_erases_nothing_is_not_a_clear() {
    // The cursor hidden, or a colour reset on its own: the bar stays.
    let chunks = [
        frame(1),
        format!("{CLEAR}{}", frame(2)),
        "\u{1b}[?25l\r\u{1b}[0m\r".to_string(),
    ];
    let (said, shown, _) = streamed(&chunks);
    assert_eq!(shown, [false, true, true]);
    assert!(!said.contains(&Said::Cleared));
}

#[test]
fn a_frame_cut_between_reads_waits_for_the_rest() {
    // Cut inside its count: not a frame of length 1, nothing said early.
    let whole = frame(3);
    let (head, tail) = whole.split_at(whole.find("/10").unwrap() + 2);
    let chunks = [
        frame(1),
        format!("{CLEAR}{}", frame(2)),
        format!("{CLEAR}{head}"),
        format!("{tail}{CLEAR1}"),
    ];
    let (said, ..) = streamed(&chunks);
    let lens: Vec<u64> = frames(&said).iter().map(|p| p.len).collect();
    assert_eq!(lens, [10, 10, 10]);
}

#[test]
fn a_bar_cleared_is_said_once_a_redraw_not_at_all_nor_a_blank_line() {
    let log = format!("{}{CLEAR}{}{CLEAR}", frame(1), frame(2));
    let mut seen = Vec::new();
    follow(Some(log.as_bytes()), |s| seen.push(s));
    assert_eq!(kinds(&seen), ["frame", "frame", "cleared"]);
    // Log lines with no bar say no clear, and a blank line is nothing.
    let mut blank = Vec::new();
    follow(Some("a\r\n\nb\n".as_bytes()), |s| blank.push(s));
    assert_eq!(blank, [Said::Line("a".into()), Said::Line("b".into())]);
}
