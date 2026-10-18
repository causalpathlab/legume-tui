//! The modes a program's screen turns on in the terminal (mouse reports,
//! keyboard enhancement, bracketed paste, focus events). The session owns
//! them: on with its screen and again after every attach (the client's
//! reset turned them off), off when the screen is handed back, and off on
//! a panic.

use ratatui::crossterm::event::KeyboardEnhancementFlags;
use std::fmt::Write as _;
use std::io::{self, Write};
use std::sync::Mutex;

/// What mouse reports the screen asks for.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Mouse {
    /// None: the terminal selects text as it does.
    #[default]
    Off,
    /// Presses, releases and the wheel (SGR encoded).
    Buttons,
    /// Every motion too, as crossterm's `EnableMouseCapture`.
    Full,
}

/// The terminal modes a screen turns on, beside the raw mode and the
/// alternate screen every one has.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Modes {
    pub mouse: Mouse,
    /// Kitty's progressive keyboard enhancement, with these flags.
    pub keyboard_enhancement: Option<KeyboardEnhancementFlags>,
    pub bracketed_paste: bool,
    /// Focus gained and lost reported as events.
    pub focus: bool,
}

impl Modes {
    /// The sequences that turn the modes on, or off.
    fn codes(self, on: bool) -> String {
        let mut s = String::new();
        let mut set = |modes: &[u16]| {
            for m in modes {
                let _ = write!(s, "\x1b[?{m}{}", if on { 'h' } else { 'l' });
            }
        };
        match self.mouse {
            Mouse::Off => {}
            Mouse::Buttons => set(&[1000, 1006]),
            Mouse::Full => set(&[1000, 1002, 1003, 1015, 1006]),
        }
        if self.bracketed_paste {
            set(&[2004]);
        }
        if self.focus {
            set(&[1004]);
        }
        match self.keyboard_enhancement {
            Some(flags) if on => {
                let _ = write!(s, "\x1b[>{}u", flags.bits());
            }
            Some(_) => s.push_str("\x1b[<1u"),
            None => {}
        }
        s
    }
}

/// The modes on in the terminal now, for a panic to turn off: the
/// terminal is the process's, whichever session set them.
static ON: Mutex<Option<Modes>> = Mutex::new(None);

fn write_codes(codes: &str) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(codes.as_bytes())?;
    out.flush()
}

/// Turn `modes` on, as the screen's.
pub(crate) fn turn_on(modes: Modes) -> io::Result<()> {
    *ON.lock().unwrap_or_else(|e| e.into_inner()) = Some(modes);
    write_codes(&modes.codes(true))
}

/// Turn off what [`turn_on`] turned on.
pub(crate) fn turn_off() -> io::Result<()> {
    let on = ON.lock().unwrap_or_else(|e| e.into_inner()).take();
    match on {
        Some(modes) => write_codes(&modes.codes(false)),
        None => Ok(()),
    }
}

/// From now on, once per process: a panic turns the modes off, then does
/// what it did.
pub(crate) fn off_on_panic() {
    static HOOKED: std::sync::Once = std::sync::Once::new();
    HOOKED.call_once(|| {
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            // Not waited for: the panic may hold it.
            if let Ok(mut on) = ON.try_lock() {
                if let Some(modes) = on.take() {
                    let _ = write_codes(&modes.codes(false));
                }
            }
            before(info);
        }));
    });
}
