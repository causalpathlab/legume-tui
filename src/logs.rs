//! No log lines over the screen: a line written to the terminal while a
//! TUI has it lands in the middle of the picture.
//!
//! ```no_run
//! let _quiet = legume_tui::logs::quiet(); // as the screen comes up
//! // ... the TUI runs; what matters reaches its status line instead
//! // dropped: the log level is back
//! ```

use log::LevelFilter;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
