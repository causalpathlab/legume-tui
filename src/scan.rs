//! Where the program's output stands between escape sequences, read by
//! the host without changing a byte: so it switches clients only between
//! sequences, and knows whether the program left the alternate screen.
//!
//! A small subset of the VT500 parser: escape sequences, control
//! sequences (CSI) and control strings (APC, DCS, OSC, PM, SOS, ended by
//! ST, and by BEL for OSC). Kitty image chunks and sixel are control
//! strings.

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;
/// CAN and SUB end any sequence.
const CAN: u8 = 0x18;
const SUB: u8 = 0x1a;

/// The longest CSI parameter list kept to look for the alternate screen.
const PARAMS: usize = 32;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum State {
    #[default]
    Ground,
    Esc,
    /// `ESC` and intermediate bytes.
    EscInter,
    Csi,
    /// In a control string.
    Str,
    /// `ESC` in a control string: ST if a `\` follows.
    StrEsc,
}

/// The output read so far.
#[derive(Debug, Default)]
pub(crate) struct Scan {
    state: State,
    /// The current CSI's parameter and intermediate bytes.
    params: Vec<u8>,
    alt: bool,
}

impl Scan {
    /// Read `bytes`.
    pub fn feed(&mut self, bytes: &[u8]) {
        self.cut(bytes);
    }

    /// Read `bytes`: the first offset in them that is between sequences (0
    /// if the stream already was, `bytes.len()` if it only gets there at
    /// the end), with whether the screen was the alternate one there, or
    /// None if it never is. Where a client joining there starts.
    pub fn cut(&mut self, bytes: &[u8]) -> Option<(usize, bool)> {
        let mut first = (self.state == State::Ground).then_some((0, self.alt));
        let mut i = 0;
        while i < bytes.len() {
            // Text, and the body of a control string, run on to the next
            // byte that can end them: skipped whole.
            let stops: fn(&u8) -> bool = match self.state {
                State::Ground => |b| matches!(*b, ESC | CAN | SUB),
                State::Str => |b| matches!(*b, ESC | BEL | CAN | SUB),
                _ => |_| true,
            };
            match bytes[i..].iter().position(stops) {
                Some(n) => i += n,
                None => break,
            }
            self.step(bytes[i]);
            i += 1;
            if first.is_none() && self.state == State::Ground {
                first = Some((i, self.alt));
            }
        }
        first
    }

    /// Whether the program's last word on the screen was to enter the
    /// alternate one.
    pub fn alt(&self) -> bool {
        self.alt
    }

    fn step(&mut self, b: u8) {
        use State::*;
        if matches!(b, CAN | SUB) {
            self.state = Ground;
            return;
        }
        self.state = match (self.state, b) {
            (Ground, ESC) => Esc,
            (Ground, _) => Ground,
            (Esc | EscInter | StrEsc, ESC) => Esc,
            (Esc | StrEsc, b'[') => {
                self.params.clear();
                Csi
            }
            (Esc | StrEsc, b']' | b'P' | b'_' | b'^' | b'X') => Str,
            (StrEsc, b'\\') => Ground,
            (Esc | EscInter | StrEsc, 0x20..=0x2f) => EscInter,
            (Esc | EscInter | StrEsc, 0x30..=0x7e) => Ground,
            // Controls inside an escape are acted on and do not end it.
            (s @ (Esc | EscInter | StrEsc), _) => s,
            (Csi, ESC) => Esc,
            (Csi, 0x40..=0x7e) => {
                self.csi(b);
                Ground
            }
            (Csi, _) => {
                if self.params.len() < PARAMS {
                    self.params.push(b);
                }
                Csi
            }
            (Str, BEL) => Ground,
            (Str, ESC) => StrEsc,
            (Str, _) => Str,
        };
    }

    /// A complete CSI with final byte `last`: the private modes that
    /// switch screens (`?1049`, `?1047`, `?47`).
    fn csi(&mut self, last: u8) {
        let on = match last {
            b'h' => true,
            b'l' => false,
            _ => return,
        };
        let Some(modes) = self.params.strip_prefix(b"?") else {
            return;
        };
        let switches = modes
            .split(|&c| c == b';')
            .any(|m| matches!(m, b"1049" | b"1047" | b"47"));
        if switches {
            self.alt = on;
        }
    }
}

#[cfg(test)]
#[path = "tests/scan.rs"]
mod tests;
