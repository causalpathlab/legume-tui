//! The detach and switch keys, found in what the user types before it
//! reaches the program.
//!
//! They are Ctrl+] and Ctrl+\ unless [`crate::DETACH_KEY_VAR`] and
//! [`crate::SWITCH_KEY_VAR`] name others. Both are one chord every
//! terminal sends as it is, with no Option-as-Meta setting on macOS and
//! no window manager taking it (as some do Ctrl+Alt+letter). No legume TUI
//! binds them. A terminal sends Ctrl+] as its control byte alone, 0x1d
//! (Ctrl+\ as 0x1c), or, with an extended keyboard mode on, as an
//! escape sequence: `ESC [ 93 ; 5 u` (CSI-u, as kitty reports keys; a
//! press, a repeat or a release) or `ESC [ 27 ; 5 ; 93 ~` (xterm's
//! modifyOtherKeys). A key with Alt is `ESC` then the control byte, or
//! the same sequences with modifier 7. In the plain encoding a terminal
//! sends the same bytes for Ctrl+5 and Ctrl+4 (crossterm reads 0x1c to
//! 0x1f as Ctrl+4 to Ctrl+7), so those keys detach and switch too.
//!
//! The client keeps the terminal raw while it reads it (and its signal
//! keys off on the way to the switch key's picker), so Ctrl+\ is a byte
//! there, not the quit signal a terminal in line mode makes of it, and
//! neither key reaches the program, whatever mode the program's own
//! terminal is in: a program in line mode inside a session (a shell, say)
//! gets no SIGQUIT from Ctrl+\ and no Ctrl+] (a telnet's escape) while
//! they are the session's keys. To send it them, set the keys to others.

const ESC: u8 = 0x1b;
/// The longest escape sequence held back while it might yet be the key.
const HOLD: usize = 32;

/// A key the client keeps: Ctrl, Alt or not, and a character that has a
/// control byte (a letter, or one of `@ \ ] ^ _`).
///
/// A tool's test types the default keys with [`Key::bytes`]; its
/// `legume_tui::testing` harness runs the tool with neither
/// [`crate::DETACH_KEY_VAR`] nor [`crate::SWITCH_KEY_VAR`] set:
///
/// ```
/// use legume_tui::Key;
/// assert_eq!(Key::DETACH.bytes(), b"\x1d");
/// assert_eq!(Key::SWITCH.display(), "ctrl+\\");
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Key {
    /// The character, lower case.
    ch: u8,
    alt: bool,
}

impl Key {
    /// Ctrl+], the detach key unless [`crate::DETACH_KEY_VAR`] names
    /// another.
    pub const DETACH: Self = Self {
        ch: b']',
        alt: false,
    };

    /// Ctrl+\\, the switch key unless [`crate::SWITCH_KEY_VAR`] names
    /// another.
    pub const SWITCH: Self = Self {
        ch: b'\\',
        alt: false,
    };

    /// The detach key: what [`crate::DETACH_KEY_VAR`] names, or
    /// [`Key::DETACH`] when it is unset or not a key we can find.
    #[must_use]
    pub fn detach() -> Self {
        Self::from_env(crate::DETACH_KEY_VAR).unwrap_or(Self::DETACH)
    }

    /// The switch key, as [`Key::detach`] with [`crate::SWITCH_KEY_VAR`].
    #[must_use]
    pub fn switch() -> Self {
        Self::from_env(crate::SWITCH_KEY_VAR).unwrap_or(Self::SWITCH)
    }

    fn from_env(var: &str) -> Option<Self> {
        Self::parse(&std::env::var(var).ok()?)
    }

    /// Read `ctrl+]`, `Ctrl-\`, `control+option+k` and the like. Ctrl
    /// is required: without it the key would be typed text.
    #[must_use]
    pub fn parse(s: &str) -> Option<Self> {
        let s = s.trim();
        // The key itself may be '+' or '-'-free punctuation; take it from
        // the end before splitting the modifiers.
        let ch = s.chars().last()?;
        let mods = s[..s.len() - ch.len_utf8()].to_ascii_lowercase();
        let mods = mods.strip_suffix(['+', '-'])?;
        let (mut ctrl, mut alt) = (false, false);
        for m in mods.split(['+', '-']) {
            match m {
                "ctrl" | "control" | "c" => ctrl = true,
                "alt" | "meta" | "option" | "opt" | "m" => alt = true,
                _ => return None,
            }
        }
        let ch = ch.to_ascii_lowercase();
        let fine = ch.is_ascii_lowercase() || matches!(ch, '@' | '\\' | ']' | '^' | '_');
        (ctrl && fine).then_some(Self { ch: ch as u8, alt })
    }

    /// As a help screen shows it, e.g. `ctrl+]` or `ctrl+alt+k`.
    #[must_use]
    pub fn display(self) -> String {
        let alt = if self.alt { "alt+" } else { "" };
        format!("ctrl+{alt}{}", self.ch as char)
    }

    /// What a terminal sends for the key in the plain encoding, for a test
    /// to type: the control byte (`0x1d` for Ctrl+]), after `ESC` with
    /// Alt.
    #[must_use]
    pub fn bytes(self) -> Vec<u8> {
        if self.alt {
            vec![ESC, self.byte()]
        } else {
            vec![self.byte()]
        }
    }

    /// The byte Ctrl and the character make in the plain encoding.
    fn byte(self) -> u8 {
        self.ch.to_ascii_uppercase() & 0x1f
    }

    /// The modifier bits the extended encodings report for the key.
    fn mods(self) -> u32 {
        CTRL | if self.alt { ALT } else { 0 }
    }
}

const SHIFT: u32 = 1;
const ALT: u32 = 2;
const CTRL: u32 = 4;
const SUPER_HYPER_META: u32 = 8 | 16 | 32;

/// What a key the client keeps asks for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Action {
    /// Leave the session running.
    Detach,
    /// Leave it for another, chosen in the picker.
    Switch,
}

/// What a complete escape sequence is to us.
#[derive(Debug, PartialEq, Eq)]
enum Hit {
    /// One of the keys, pressed (or repeated).
    Key(Action),
    /// The key let go of: swallowed, it means nothing to the program.
    Release,
    /// Anything else: passed on.
    Other,
}

/// Finds the detach and switch keys in the input stream. A sequence split
/// between two reads is held until the rest arrives, or until
/// [`Chord::flush`] gives up on it (a lone Esc key press looks like the
/// start of one).
#[derive(Debug)]
pub(crate) struct Chord {
    /// The detach key first: it wins if both are the same.
    keys: [(Key, Action); 2],
    held: Vec<u8>,
}

impl Chord {
    pub fn new(detach: Key, switch: Key) -> Self {
        Self {
            keys: [(detach, Action::Detach), (switch, Action::Switch)],
            held: Vec::new(),
        }
    }

    /// Take `input`: the bytes to pass on, and what a key that came asks
    /// for. On a key, what follows it is dropped.
    pub fn feed(&mut self, input: &[u8]) -> (Vec<u8>, Option<Action>) {
        let mut buf = std::mem::take(&mut self.held);
        buf.extend_from_slice(input);
        let mut out = Vec::with_capacity(buf.len());
        let mut i = 0;
        while i < buf.len() {
            let plain = self.keys.iter().find(|(k, _)| !k.alt && k.byte() == buf[i]);
            if let Some(&(_, action)) = plain {
                return (out, Some(action));
            }
            match buf[i] {
                ESC => match sequence(&buf[i..], &self.keys) {
                    Seq::Partial => {
                        self.held = buf[i..].to_vec();
                        break;
                    }
                    Seq::Done(_, Hit::Key(action)) => return (out, Some(action)),
                    Seq::Done(len, Hit::Release) => i += len,
                    Seq::Done(len, Hit::Other) => {
                        out.extend_from_slice(&buf[i..i + len]);
                        i += len;
                    }
                    Seq::Not => {
                        out.push(ESC);
                        i += 1;
                    }
                },
                b => {
                    out.push(b);
                    i += 1;
                }
            }
        }
        (out, None)
    }

    /// Whether bytes are held back waiting for the rest of a sequence.
    pub fn holding(&self) -> bool {
        !self.held.is_empty()
    }

    /// Give up waiting: the held bytes, to pass on as they are.
    pub fn flush(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.held)
    }
}

/// Where an escape sequence at the start of `s` stands.
#[derive(Debug, PartialEq, Eq)]
enum Seq {
    /// It may yet be complete: more input is needed.
    Partial,
    /// Complete, this long.
    Done(usize, Hit),
    /// Not a control sequence (an Alt+key, say): the ESC passes alone.
    Not,
}

/// Read `ESC` and a key's control byte (Alt and the key, plain), or a
/// control sequence (`ESC [`, parameters, intermediates, a final byte), at
/// the start of `s`.
fn sequence(s: &[u8], keys: &[(Key, Action)]) -> Seq {
    match s.get(1) {
        None => return Seq::Partial,
        Some(&b) => {
            if let Some(&(_, action)) = keys.iter().find(|(k, _)| k.alt && k.byte() == b) {
                return Seq::Done(2, Hit::Key(action));
            }
            if b != b'[' {
                return Seq::Not;
            }
        }
    }
    for (j, &c) in s.iter().enumerate().skip(2) {
        match c {
            0x20..=0x3f => {}
            0x40..=0x7e => {
                let params = std::str::from_utf8(&s[2..j]).unwrap_or("");
                return Seq::Done(j + 1, hit(params, c, keys));
            }
            _ => return Seq::Not,
        }
    }
    if s.len() >= HOLD {
        Seq::Not
    } else {
        Seq::Partial
    }
}

/// Whether the sequence `ESC [ params final` is one of `keys` with its
/// modifiers and no others (Caps Lock and Num Lock do not count).
fn hit(params: &str, last: u8, keys: &[(Key, Action)]) -> Hit {
    let fields: Vec<&str> = params.split(';').collect();
    let number = |s: &str| s.split(':').next().and_then(|v| v.parse::<u32>().ok());
    let (code, mods) = match last {
        // CSI code[:alternates] ; mods[:event] [; text] u
        b'u' => (number(fields[0]), fields.get(1).copied().unwrap_or("1")),
        // CSI 27 ; mods ; code ~
        b'~' if fields.len() == 3 && fields[0] == "27" => (number(fields[2]), fields[1]),
        _ => return Hit::Other,
    };
    let mut parts = mods.split(':');
    let Some(bits) = parts
        .next()
        .and_then(|m| m.parse::<u32>().ok())
        .and_then(|m| m.checked_sub(1))
    else {
        return Hit::Other;
    };
    let bits = bits & (SHIFT | ALT | CTRL | SUPER_HYPER_META);
    let Some(&(_, action)) = keys
        .iter()
        .find(|(k, _)| code == Some(u32::from(k.ch)) && bits == k.mods())
    else {
        return Hit::Other;
    };
    match parts.next() {
        Some("3") => Hit::Release,
        _ => Hit::Key(action),
    }
}

#[cfg(test)]
#[path = "tests/chord.rs"]
mod tests;
