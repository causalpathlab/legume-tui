//! No log lines over the screen: a line written to the terminal while a
//! TUI has it lands in the middle of the picture.
//!
//! [`quiet`] turns logging off while the screen is up, whatever the
//! logger. A tool that wants its records kept, or that shows them in a
//! pane of its own, installs its stderr logger behind a [`gate`]: then
//! [`hold`] keeps what it logs while the screen is up and writes it once
//! the normal screen is back, and [`quiet_stderr`] drops it there while
//! a pane beside the gate still gets every record.
//!
//! ```no_run
//! let _quiet = legume_tui::logs::quiet(); // as the screen comes up
//! // ... the TUI runs; what matters reaches its status line instead
//! // dropped: the log level is back
//! ```
//!
//! ```no_run
//! # struct Stderr;
//! # impl log::Log for Stderr {
//! #     fn enabled(&self, _: &log::Metadata) -> bool { true }
//! #     fn log(&self, _: &log::Record) {}
//! #     fn flush(&self) {}
//! # }
//! // At the start, the tool's own logger behind the gate:
//! log::set_logger(legume_tui::logs::gate(Stderr)).unwrap();
//! log::set_max_level(log::LevelFilter::Info);
//! // With the screen: what is logged meanwhile is written after it.
//! let _held = legume_tui::logs::hold();
//! ```

use log::{Level, LevelFilter, Log, Metadata, Record};
use std::collections::VecDeque;
use std::sync::atomic::{AtomicUsize, Ordering::SeqCst};
use std::sync::Mutex;

/// Logging off while it lives ([`quiet`]).
#[must_use = "logging is back on when this is dropped"]
pub struct Quiet {
    level: LevelFilter,
}

/// Turn logging off (`log::set_max_level(Off)`) until the guard is
/// dropped; the level then is what it was. A panic that unwinds past the
/// guard drops it, so what the panic leads to is logged; one caught
/// before it, on any thread, leaves logging off.
pub fn quiet() -> Quiet {
    let level = log::max_level();
    log::set_max_level(LevelFilter::Off);
    Quiet { level }
}

impl Drop for Quiet {
    fn drop(&mut self) {
        log::set_max_level(self.level);
    }
}

/// A logger behind the gate ([`gate`]): records pass on to it unless a
/// [`hold`] keeps them or a [`quiet_stderr`] drops them.
pub struct Gate {
    inner: Box<dyn Log>,
    kept: Mutex<Kept>,
}

/// The most records a gate keeps while held (a screen may be up for days,
/// detached): the oldest give way, and how many did is said in their
/// place.
const KEPT: usize = 10_000;

/// What a gate kept while held.
#[derive(Default)]
struct Kept {
    /// Level, target and message, oldest first.
    records: VecDeque<(Level, String, String)>,
    /// How many gave way to newer ones.
    dropped: usize,
}

/// The gates made, to write what they kept when the hold ends.
static GATES: Mutex<Vec<&'static Gate>> = Mutex::new(Vec::new());
/// How many [`Hold`]s live.
static HOLDS: AtomicUsize = AtomicUsize::new(0);
/// How many [`QuietStderr`]s live.
static MUTED: AtomicUsize = AtomicUsize::new(0);

/// `inner` (the tool's logger, writing to stderr) behind a gate, for
/// `log::set_logger`, or for a logger of the tool's own to pass records
/// on to (beside a pane that shows them). It lives as long as the process.
pub fn gate(inner: impl Log + 'static) -> &'static Gate {
    let gate: &'static Gate = Box::leak(Box::new(Gate {
        inner: Box::new(inner),
        kept: Mutex::new(Kept::default()),
    }));
    GATES.lock().unwrap_or_else(|e| e.into_inner()).push(gate);
    gate
}

impl Gate {
    fn kept(&self) -> std::sync::MutexGuard<'_, Kept> {
        self.kept.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Write what was kept, in order, after a line saying how many gave
    /// way, if any did.
    fn write_kept(&self) {
        let Kept { records, dropped } = std::mem::take(&mut *self.kept());
        if dropped > 0 {
            self.inner.log(
                &Record::builder()
                    .level(Level::Warn)
                    .target(module_path!())
                    .args(format_args!(
                        "{dropped} log records held while the screen was up were dropped; \
                         the latest follow"
                    ))
                    .build(),
            );
        }
        for (level, target, message) in records {
            self.inner.log(
                &Record::builder()
                    .level(level)
                    .target(&target)
                    .args(format_args!("{message}"))
                    .build(),
            );
        }
    }
}

impl Log for Gate {
    fn enabled(&self, metadata: &Metadata) -> bool {
        self.inner.enabled(metadata)
    }

    fn log(&self, record: &Record) {
        if !self.inner.enabled(record.metadata()) {
            return;
        }
        if HOLDS.load(SeqCst) > 0 && crate::screen_shown() {
            let entry = (
                record.level(),
                record.target().to_string(),
                record.args().to_string(),
            );
            let mut kept = self.kept();
            if kept.records.len() == KEPT {
                kept.records.pop_front();
                kept.dropped += 1;
            }
            kept.records.push_back(entry);
            return;
        }
        if MUTED.load(SeqCst) > 0 {
            return;
        }
        // What was held comes first.
        self.write_kept();
        self.inner.log(record);
    }

    fn flush(&self) {
        self.inner.flush();
    }
}

/// Write what every gate kept: the screen is down, or the hold is over.
pub(crate) fn replay() {
    let gates = GATES.lock().unwrap_or_else(|e| e.into_inner()).clone();
    for gate in gates {
        gate.write_kept();
    }
}

/// Records held while it lives ([`hold`]).
#[must_use = "the records are written when this is dropped"]
pub struct Hold(());

/// Keep what goes through a [`gate`] while a screen of this process is
/// up, until the guard is dropped: it is written, in order, as the screen
/// goes (handed over to a child or to work off the screen, or restored)
/// and when the last guard is dropped. Meanwhile records off the screen
/// are written as they come, after what was kept. Only the latest 10,000
/// are kept; a line says how many older ones were dropped.
pub fn hold() -> Hold {
    HOLDS.fetch_add(1, SeqCst);
    Hold(())
}

impl Drop for Hold {
    fn drop(&mut self) {
        if HOLDS.fetch_sub(1, SeqCst) == 1 {
            replay();
        }
    }
}

/// Records through a gate dropped while it lives ([`quiet_stderr`]).
#[must_use = "the gate opens again when this is dropped"]
pub struct QuietStderr(());

/// Drop what goes through a [`gate`] (the tool's stderr logger) until the
/// guard is dropped, the log level untouched: a pane of the tool's own
/// that gets the records beside the gate keeps getting them. A [`hold`]
/// keeps them instead while the screen is up.
pub fn quiet_stderr() -> QuietStderr {
    MUTED.fetch_add(1, SeqCst);
    QuietStderr(())
}

impl Drop for QuietStderr {
    fn drop(&mut self) {
        MUTED.fetch_sub(1, SeqCst);
    }
}

#[cfg(test)]
#[path = "tests/logs.rs"]
mod tests;
