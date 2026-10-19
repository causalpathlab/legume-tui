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
#[non_exhaustive]
pub enum Mouse {
    /// None: the terminal selects text as it does.
    #[default]
    Off,
    /// Presses, releases and the wheel (SGR encoded).
    Buttons,
    /// Every motion too, as crossterm's `EnableMouseCapture`.
    Full,
}

/// Whether the screen asks for kitty's progressive keyboard enhancement
/// (keys held with Shift, Alt or Ctrl told apart: Shift+Enter from Enter).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum Keyboard {
    /// Not asked for.
    #[default]
    Off,
    /// Asked for with these flags, whatever the terminal; one that does not
    /// know the request ignores it.
    On(KeyboardEnhancementFlags),
    /// Asked for with these flags where the attached terminal says it has
    /// it: asked again after every attach, before the modes are on
    /// ([`crate::Session::keyboard_enhanced`]).
    IfSupported(KeyboardEnhancementFlags),
}

impl Keyboard {
    /// The flags to ask for, given whether the terminal has the
    /// enhancement (`supported`).
    pub(crate) fn flags(self, supported: bool) -> Option<KeyboardEnhancementFlags> {
        match self {
            Self::Off => None,
            Self::On(flags) => Some(flags),
            Self::IfSupported(flags) => supported.then_some(flags),
        }
    }
}

/// The terminal modes a screen turns on, beside the raw mode and the
/// alternate screen every one has: none by default, each set in turn
/// (`Modes::new().mouse(Mouse::Buttons)`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Modes {
    pub mouse: Mouse,
    pub keyboard_enhancement: Keyboard,
    pub bracketed_paste: bool,
    /// Focus gained and lost reported as events.
    pub focus: bool,
}

impl Modes {
    /// None on.
    pub const fn new() -> Self {
        Self {
            mouse: Mouse::Off,
            keyboard_enhancement: Keyboard::Off,
            bracketed_paste: false,
            focus: false,
        }
    }

    pub const fn mouse(self, mouse: Mouse) -> Self {
        Self { mouse, ..self }
    }

    pub const fn keyboard_enhancement(self, keyboard_enhancement: Keyboard) -> Self {
        Self {
            keyboard_enhancement,
            ..self
        }
    }

    pub const fn bracketed_paste(self, bracketed_paste: bool) -> Self {
        Self {
            bracketed_paste,
            ..self
        }
    }

    pub const fn focus(self, focus: bool) -> Self {
        Self { focus, ..self }
    }
}

/// [`Modes`] as turned on: the keyboard's flags settled.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct On {
    modes: Modes,
    keyboard: Option<KeyboardEnhancementFlags>,
}

impl On {
    /// The sequences that turn the modes on, or off.
    fn codes(self, on: bool) -> String {
        let Self { modes, keyboard } = self;
        let mut s = String::new();
        let mut set = |modes: &[u16]| {
            for m in modes {
                let _ = write!(s, "\x1b[?{m}{}", if on { 'h' } else { 'l' });
            }
        };
        match modes.mouse {
            Mouse::Off => {}
            Mouse::Buttons => set(&[1000, 1006]),
            Mouse::Full => set(&[1000, 1002, 1003, 1015, 1006]),
        }
        if modes.bracketed_paste {
            set(&[2004]);
        }
        if modes.focus {
            set(&[1004]);
        }
        match keyboard {
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
static ON: Mutex<Option<On>> = Mutex::new(None);

fn write_codes(codes: &str) -> io::Result<()> {
    let mut out = io::stdout();
    out.write_all(codes.as_bytes())?;
    out.flush()
}

/// Turn `modes` on, as the screen's, the keyboard's flags as `supported`
/// says ([`Keyboard::flags`]).
pub(crate) fn turn_on(modes: Modes, supported: bool) -> io::Result<()> {
    let on = On {
        modes,
        keyboard: modes.keyboard_enhancement.flags(supported),
    };
    *ON.lock().unwrap_or_else(|e| e.into_inner()) = Some(on);
    write_codes(&on.codes(true))
}

/// Turn off what [`turn_on`] turned on.
pub(crate) fn turn_off() -> io::Result<()> {
    let on = ON.lock().unwrap_or_else(|e| e.into_inner()).take();
    match on {
        Some(on) => write_codes(&on.codes(false)),
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
