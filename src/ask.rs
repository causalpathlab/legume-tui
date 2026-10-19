//! Asking the terminal: a query written to it and the answer read back on
//! the calling thread, a byte at a time and only while one is waiting, up
//! to the end of the answer (the reply to the device status report every
//! query ends with). Once it returns nothing is left reading stdin, even
//! when the terminal never answered: a reply that comes after the timeout
//! reaches the program as input, as any late reply would.

use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::time::{Duration, Instant};

/// Kitty's keyboard query: a terminal with the enhancement answers with
/// the flags on now (`CSI ? flags u`), others say nothing.
pub(crate) const KEYBOARD: &str = "\x1b[?u";
/// The device status report every query ends with: all terminals answer.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) const STATUS: &str = "\x1b[5n";
/// The terminal's answer to [`STATUS`].
const STATUS_REPLY: &[u8] = b"\x1b[0n";

/// Write `query` to `output` and read the terminal's answer from `input`
/// for at most `timeout`, each byte to `each`, up to the reply to the
/// status report `query` ends with; whether that came in time. Echo and
/// line mode are off while it reads, and put back after.
pub(crate) fn ask(
    input: BorrowedFd<'_>,
    output: &mut impl Write,
    query: &[u8],
    timeout: Duration,
    each: impl FnMut(u8),
) -> io::Result<bool> {
    use rustix::termios::{tcgetattr, tcsetattr, LocalModes, OptionalActions, SpecialCodeIndex};
    let before = tcgetattr(input)?;
    let mut quiet = before.clone();
    quiet.local_modes &= !(LocalModes::ICANON | LocalModes::ECHO);
    // Each read returns what has come, however little.
    quiet.special_codes[SpecialCodeIndex::VMIN] = 1;
    quiet.special_codes[SpecialCodeIndex::VTIME] = 0;
    tcsetattr(input, OptionalActions::Now, &quiet)?;
    let answered = (|| {
        output.write_all(query)?;
        output.flush()?;
        read_answer(input, timeout, each)
    })();
    tcsetattr(input, OptionalActions::Now, &before)?;
    answered
}

/// The answer read from `input` as [`ask`] says.
fn read_answer(
    input: BorrowedFd<'_>,
    timeout: Duration,
    mut each: impl FnMut(u8),
) -> io::Result<bool> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let start = Instant::now();
    // The last bytes read, to find the end of the answer in.
    let mut tail = [0u8; STATUS_REPLY.len()];
    loop {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            return Ok(false);
        }
        let left = Timespec::try_from(left).map_err(io::Error::other)?;
        let mut fds = [PollFd::new(&input, PollFlags::IN)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        let mut byte = [0u8];
        match rustix::io::read(input, &mut byte) {
            Ok(0) => return Ok(false),
            Ok(_) => {}
            Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
            Err(e) => return Err(e.into()),
        }
        each(byte[0]);
        tail.rotate_left(1);
        tail[STATUS_REPLY.len() - 1] = byte[0];
        if tail == STATUS_REPLY {
            return Ok(true);
        }
    }
}

/// Kitty's keyboard reply (`CSI ? flags u`) found among an answer's bytes.
#[derive(Debug, Default)]
pub(crate) struct KeyboardReply {
    /// How much of the reply has been read: ESC, `[`, `?`, then digits.
    at: u8,
    /// The reply came.
    pub heard: bool,
}

impl KeyboardReply {
    pub fn push(&mut self, b: u8) {
        self.at = match (self.at, b) {
            (_, 0x1b) => 1,
            (1, b'[') => 2,
            (2, b'?') => 3,
            (3, b'0'..=b'9') => 3,
            (3, b'u') => {
                self.heard = true;
                0
            }
            _ => 0,
        };
    }
}

/// Whether the terminal on stdin and stdout has kitty's keyboard
/// enhancement; None when stdin is not a terminal or no answer came in
/// time. Asked as it is, never wrapped for tmux: the keys come from the
/// terminal the program talks to.
pub(crate) fn keyboard() -> Option<bool> {
    let stdin = io::stdin();
    if !rustix::termios::isatty(&stdin) {
        return None;
    }
    let mut reply = KeyboardReply::default();
    let query = format!("{KEYBOARD}{STATUS}");
    let ended = ask(
        stdin.as_fd(),
        &mut io::stdout(),
        query.as_bytes(),
        crate::ASK_TIMEOUT,
        |b| reply.push(b),
    );
    ended.ok()?.then_some(reply.heard)
}

#[cfg(test)]
#[path = "tests/ask.rs"]
mod tests;
