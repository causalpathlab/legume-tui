//! Work on another thread: answers picked up, waited for, or lost to a
//! panic that comes back as an error; the spinner and the waits.

use super::*;

/// An answer comes once: polled, read, taken, and gone after.
#[test]
fn a_pending_answer_is_picked_up_and_taken_once() {
    let (tx, mut p) = Pending::<u8>::pair();
    assert!(!p.poll() && p.out().is_none() && !p.gone());
    tx.send(Ok(7)).unwrap();
    assert!(p.poll());
    assert_eq!(p.out(), Some(&Ok(7)));
    drop(tx);
    assert!(p.poll() && !p.gone(), "the answer came: not gone");
    assert_eq!(p.take(), Some(Ok(7)));
    assert_eq!(p.take(), None);
    assert_eq!(p.try_take("stopped"), None);
}

/// A worker that dies without an answer is gone, said once.
#[test]
fn a_worker_gone_without_an_answer_says_so_once() {
    let (tx, mut p) = Pending::<u8>::pair();
    assert_eq!(p.try_take("stopped"), None);
    drop(tx);
    assert_eq!(p.try_take("stopped"), Some(Err("stopped".into())));
    assert_eq!(p.try_take("stopped"), None);
}

/// Spawned work answers, a panic in it too.
#[test]
fn spawned_work_answers_even_when_it_panics() {
    let mut p = Pending::<u8>::spawn(|| Ok(3));
    while !p.wait_for(Duration::from_millis(10)) {}
    assert_eq!(p.take(), Some(Ok(3)));
    let mut p = Pending::<u8>::spawn(|| panic!("oops"));
    while !p.wait_for(Duration::from_millis(10)) {}
    assert_eq!(p.take(), Some(Err("the work panicked: oops".into())));
}

/// Errors of any type the stop converts into, as faba's anyhow ones.
#[test]
fn a_pending_answer_keeps_its_own_error_type() {
    #[derive(Debug, PartialEq)]
    struct Mine(String);
    impl From<Failed> for Mine {
        fn from(s: Failed) -> Self {
            Self(s.to_string())
        }
    }
    let mut p = Pending::<u8, Mine>::spawn(|| panic!("{}", String::from("owned")));
    while !p.wait_for(Duration::from_millis(10)) {}
    assert_eq!(p.take(), Some(Err(Mine("the work panicked: owned".into()))));
}

/// Work on a thread comes back with its result, whether the wait polled
/// it or not; work that panics is an error saying with what.
#[test]
fn work_on_a_thread_comes_back_or_says_why_it_panicked() {
    let (waited, out) = on_thread("w", || Ok::<_, String>(3), |p| (1, p.wait_for(GRACE)));
    assert_eq!((waited, out.unwrap()), ((1, true), 3));
    let (_, out) = on_thread("w", || Ok::<_, String>(4), |_| ());
    assert_eq!(out.unwrap(), 4);
    let owned = String::from("an owned message");
    let (_, out) = on_thread(
        "the work",
        || -> Result<u8, String> { panic!("{owned}") },
        |_| (),
    );
    assert_eq!(out.unwrap_err(), "the work panicked: an owned message");
    let (gone, out) = on_thread(
        "the work",
        || -> Result<u8, String> { panic!("a fixed message") },
        |p| {
            while !p.wait_for(GRACE) {}
            p.gone()
        },
    );
    assert!(gone);
    assert_eq!(out.unwrap_err(), "the work panicked: a fixed message");
    let (_, out) = on_thread(
        "w",
        || Ok::<_, String>(5),
        |p| {
            while !p.wait_for(GRACE) {}
            p.take()
        },
    );
    assert_eq!(out.unwrap_err(), "w ended without a result");
}

#[test]
fn panic_text_reads_str_and_string_payloads() {
    assert_eq!(panic_text(&"fixed"), "fixed");
    assert_eq!(panic_text(&String::from("owned")), "owned");
    assert_eq!(panic_text(&3), "no message");
}

/// The spinner turns a frame every period, round and round.
#[test]
fn the_spinner_turns_with_the_time() {
    let at = |ms| spinner_frame(Duration::from_millis(ms));
    assert_eq!(at(0), SPINNER[0]);
    assert_eq!(at(99), SPINNER[0]);
    assert_eq!(at(100), SPINNER[1]);
    assert_eq!(at(100 * SPINNER.len() as u64), SPINNER[0]);
    let slow = |ms| spinner_frame_every(Duration::from_millis(ms), Duration::from_millis(200));
    assert_eq!(slow(199), SPINNER[0]);
    assert_eq!(slow(200), SPINNER[1]);
    assert_eq!(
        spinner_frame_every(Duration::from_secs(1), Duration::ZERO),
        SPINNER[0]
    );
}

/// Loading starts once, says [`LOADING`] until it answers, and is busy
/// only while it runs.
#[test]
fn loading_answers_once_started() {
    let (tx, rx) = channel::<()>();
    let l = Loading::new(move || {
        rx.recv().ok();
        Ok(9)
    });
    assert!(l.busy().is_none(), "not started");
    assert_eq!(l.answer(false), Err(LOADING.to_string()));
    assert!(l.busy().is_some());
    tx.send(()).unwrap();
    assert_eq!(l.answer(true), Ok(&9));
    assert!(l.busy().is_none() && l.try_get().is_some());
    assert_eq!(Loading::ready(Ok(1)).answer(false), Ok(&1));
}

#[test]
fn loading_that_panics_answers_with_the_panic() {
    let l: Loading<u8> = Loading::new(|| panic!("broke"));
    assert_eq!(l.get(), &Err("the work panicked: broke".to_string()));
}

#[test]
fn quick_work_shows_nothing() {
    let (shown, out) = waiting("w", || Ok::<_, String>(7), |_| {});
    assert_eq!(out, Ok(7));
    assert!(!shown);
}

#[test]
fn slow_work_ticks_until_it_answers() {
    let mut ticks = 0;
    let (shown, out) = waiting(
        "w",
        || {
            std::thread::sleep(Duration::from_millis(400));
            Ok::<_, String>("done")
        },
        |_| ticks += 1,
    );
    assert_eq!(out, Ok("done"));
    assert!(shown && ticks >= 1);
}

/// A panic ends the wait with an error, not a panic here.
#[test]
fn work_that_panics_ends_the_wait_with_an_error() {
    let (_, out) = waiting(
        "reading",
        || -> Result<u8, String> {
            std::thread::sleep(Duration::from_millis(200));
            panic!("bad file")
        },
        |_| {},
    );
    assert_eq!(out, Err("reading panicked: bad file".into()));
    let out = during_inline("reading", || -> Result<u8, String> { panic!("bad") });
    assert_eq!(out, Err("reading panicked: bad".into()));
}

/// Each tick says how many of the total the work has counted off.
#[test]
fn counting_work_ticks_with_its_count() {
    let mut seen = Vec::new();
    let (shown, out) = counting(
        "w",
        3,
        |done| {
            for _ in 0..3 {
                std::thread::sleep(Duration::from_millis(120));
                done.fetch_add(1, Ordering::Relaxed);
            }
            // Over the total: never more than it, said.
            done.fetch_add(5, Ordering::Relaxed);
            std::thread::sleep(Duration::from_millis(150));
            Ok::<_, String>(())
        },
        |t| seen.push(t),
    );
    assert!(shown && out.is_ok());
    assert!(seen.iter().all(|t| t.total == 3 && t.done <= 3));
    assert!(seen.windows(2).all(|w| w[0].done <= w[1].done));
    assert_eq!(seen.last().map(|t| t.done), Some(3));
}

#[test]
fn the_line_fills_the_width_and_counts_seconds() {
    let t = |ms, done, total| Tick {
        elapsed: Duration::from_millis(ms),
        done,
        total,
    };
    let l = line("loading", t(2300, 0, 0), 30);
    assert_eq!(l.chars().count(), 30);
    assert!(l.contains("loading… 2s"), "{l}");
    let l = line("reading", t(1000, 2, 5), 30);
    assert!(l.contains("reading… 2/5 1s"), "{l}");
    // Narrower than the text: cut, never wider.
    assert_eq!(line("loading", t(0, 0, 0), 5).chars().count(), 5);
}
