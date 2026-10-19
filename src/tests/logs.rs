//! Logging kept off the screen: turned off, held behind a gate, or muted
//! there while a pane beside it hears all.

use super::*;
use std::sync::Arc;

/// One test: the level is the process's.
#[test]
fn off_while_a_guard_lives_and_back_after_it_or_an_unwind() {
    log::set_max_level(LevelFilter::Info);
    {
        let _outer = quiet();
        assert_eq!(log::max_level(), LevelFilter::Off);
        {
            let _inner = quiet();
            assert_eq!(log::max_level(), LevelFilter::Off);
        }
        assert_eq!(log::max_level(), LevelFilter::Off, "the outer holds");
    }
    assert_eq!(log::max_level(), LevelFilter::Info);
    log::set_max_level(LevelFilter::Warn);
    // A panic caught on another thread, the guard still up: off.
    let _guard = quiet();
    let worker = std::thread::spawn(|| panic!("on purpose")).join();
    assert!(worker.is_err());
    assert_eq!(log::max_level(), LevelFilter::Off, "the guard holds");
    drop(_guard);
    assert_eq!(log::max_level(), LevelFilter::Warn);
    let caught = std::panic::catch_unwind(|| {
        let _quiet = quiet();
        panic!("on purpose");
    });
    assert!(caught.is_err());
    assert_eq!(log::max_level(), LevelFilter::Warn, "the unwind dropped it");
}

/// A logger that keeps the messages it is given, as a stderr logger or a
/// pane would show them.
#[derive(Clone, Default)]
struct Written(Arc<Mutex<Vec<String>>>);

impl Written {
    fn all(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }
}

impl Log for Written {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= Level::Info
    }
    fn log(&self, record: &Record) {
        self.0.lock().unwrap().push(record.args().to_string());
    }
    fn flush(&self) {}
}

/// A tool's logger: every record to its pane, and on through the gate.
struct Tee {
    pane: Written,
    stderr: &'static Gate,
}

impl Log for Tee {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }
    fn log(&self, record: &Record) {
        self.pane.log(record);
        self.stderr.log(record);
    }
    fn flush(&self) {}
}

fn say(logger: &dyn Log, level: Level, message: &str) {
    logger.log(
        &Record::builder()
            .level(level)
            .target("tool")
            .args(format_args!("{message}"))
            .build(),
    );
}

/// One test: the screen and the guards are the process's.
#[test]
fn a_gate_holds_records_while_the_screen_is_up_and_mutes_only_itself() {
    let stderr = Written::default();
    let pane = Written::default();
    let tee = Tee {
        pane: pane.clone(),
        stderr: gate(stderr.clone()),
    };
    let info = |m: &str| say(&tee, Level::Info, m);
    info("before");
    {
        let _held = hold();
        info("off the screen");
        crate::set_shown(true);
        info("on the screen");
        say(&tee, Level::Debug, "not enabled");
        assert_eq!(stderr.all(), ["before", "off the screen"], "kept");
        // Handed over, or restored: written then.
        crate::set_shown(false);
        assert_eq!(stderr.all().last().unwrap(), "on the screen");
        crate::set_shown(true);
        info("up again");
    }
    // The hold over, the screen still up: written in order.
    assert_eq!(
        stderr.all(),
        ["before", "off the screen", "on the screen", "up again"]
    );
    crate::set_shown(false);

    let muted = quiet_stderr();
    info("in the pane only");
    drop(muted);
    info("after");
    assert_eq!(stderr.all().last().unwrap(), "after");
    assert!(!stderr.all().contains(&"in the pane only".to_string()));
    assert!(pane.all().contains(&"in the pane only".to_string()));

    // Held long: the oldest give way, and how many is said in their place.
    let before = stderr.all().len();
    {
        let _held = hold();
        crate::set_shown(true);
        for i in 0..KEPT + 3 {
            info(&format!("n{i}"));
        }
        crate::set_shown(false);
    }
    let all = stderr.all();
    let written = &all[before..];
    assert_eq!(written.len(), KEPT + 1);
    assert!(written[0].starts_with("3 log records"), "{}", written[0]);
    assert_eq!(written[1], "n3");
    assert_eq!(written[KEPT], format!("n{}", KEPT + 2));
}
