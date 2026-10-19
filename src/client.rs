//! The client: the user's terminal, raw, joined to a host. What is typed
//! goes to the program (but the detach and switch keys), what the program
//! writes comes back untouched, and size changes follow SIGWINCH. After a
//! detach by the key, an unnamed session is offered a label; after the
//! switch key, the picker chooses the session to go on with.
//!
//! The client never waits on the host to take what is typed: it is held,
//! up to a bound past which the oldest goes, while the relay goes on
//! reading the terminal, so the detach key works whatever the program
//! does. It follows the alternate screen through the output it relays, so
//! it leaves that screen at the end only if it is in it.

use crate::chord::{Action, Chord, Key};
use crate::scan::Scan;
use crate::wire::{self, Size};
use crate::{dir, picker, Tool};
use std::collections::VecDeque;
use std::io::{self, Write};
use std::os::fd::{AsFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Modes a TUI may have left on in the terminal, all turned off: written
/// to the user's terminal when a client starts. First ST and CAN, which
/// end any escape sequence or control string the output stopped in the
/// middle of (a kitty image chunk, sixel, a title), so what follows is
/// read as the resets it is. Then kitty images deleted, the kitty keyboard
/// stack emptied, mouse reporting, bracketed paste and focus events off,
/// colours reset, cursor shown. The program's repaint turns back on what
/// it needs. The screen stays the user's own: the program's repaint, or
/// the host's [`wire::SCREEN`], takes it to the alternate one, so what a
/// program says before it gets there (an error it ends with) stays in
/// sight.
pub(crate) const MODES_OFF: &str = concat!(
    "\x1b\\\x18",
    "\x1b_Ga=d,d=A\x1b\\",
    "\x1b[<99u",
    "\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1006l\x1b[?1015l",
    "\x1b[?2004l\x1b[?1004l",
    "\x1b[0m\x1b[?25h",
);
/// Into the alternate screen, cleared, for the program to repaint: where
/// a client joining a program on that screen goes.
const ENTER: &str = "\x1b[?1049h\x1b[H\x1b[2J";
/// Back to the user's own screen.
const LEAVE: &str = "\x1b[?1049l";

/// How long a part of an escape sequence is held for the rest.
const HOLD: Duration = Duration::from_millis(30);
/// The most typed bytes held for a host not taking them; the oldest go.
const PENDING: usize = 1 << 20;
/// How long saying the program's end was taken ([`wire::HEARD`]) may
/// wait for room: as long as the host waits to hear it.
const HEARD_WAIT: Duration = Duration::from_secs(1);

/// How a client's time with the session ended.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Outcome {
    /// The user left with the detach key.
    Detached,
    /// The client was told to stop by a signal, or the terminal went away.
    Stopped,
    /// The user asked for another session with the switch key.
    Switch,
    /// The program ended with this status, leaving this to be said
    /// ([`crate::Session::leave`]).
    Exited { code: i32, left: Option<String> },
    /// Another client attached.
    Elsewhere,
    /// The host went away without a word.
    Lost,
}

/// The fixed reset written when a client ends, the terminal's screen the
/// alternate one (`alt`) or not, as the output relayed left it. Only the
/// alternate one is left: leaving the user's own screen would move the
/// cursor back over what was printed last.
pub(crate) fn end_reset(alt: bool) -> String {
    if alt {
        format!("{MODES_OFF}{LEAVE}")
    } else {
        MODES_OFF.to_string()
    }
}

/// The terminal in raw mode until this is dropped.
struct Raw<'a> {
    fd: BorrowedFd<'a>,
    saved: rustix::termios::Termios,
}

impl<'a> Raw<'a> {
    fn enter(fd: BorrowedFd<'a>) -> io::Result<Self> {
        use rustix::termios::{tcgetattr, tcsetattr, OptionalActions};
        let saved = tcgetattr(fd)?;
        let mut raw = saved.clone();
        raw.make_raw();
        tcsetattr(fd, OptionalActions::Now, &raw)?;
        Ok(Self { fd, saved })
    }
}

impl Drop for Raw<'_> {
    fn drop(&mut self) {
        use rustix::termios::{tcsetattr, OptionalActions};
        let _ = tcsetattr(self.fd, OptionalActions::Now, &self.saved);
    }
}

/// Signals that end the client as a detach, with the terminal restored:
/// `kill`, `kill -INT`, `kill -QUIT`, and a hangup while the terminal is
/// still there to restore (a multiplexer around us, say).
const STOPS: [i32; 4] = [
    signal_hook::consts::SIGTERM,
    signal_hook::consts::SIGINT,
    signal_hook::consts::SIGQUIT,
    signal_hook::consts::SIGHUP,
];

/// Set whenever the client is not handling [`STOPS`] itself: they then
/// end the process, as by default (a handler left registered with nothing
/// to do would ignore them). None if that could not be arranged.
fn stops_by_default() -> Option<Arc<AtomicBool>> {
    static GATE: OnceLock<Option<Arc<AtomicBool>>> = OnceLock::new();
    GATE.get_or_init(|| {
        let gate = Arc::new(AtomicBool::new(true));
        for &s in &STOPS {
            signal_hook::flag::register_conditional_default(s, Arc::clone(&gate)).ok()?;
        }
        Some(gate)
    })
    .clone()
}

/// A socket pair signals write to, to wake a poll, while this lives; then
/// the signals do what they did before ([`STOPS`] end the process).
pub(crate) struct Woken {
    pub(crate) pipe: UnixStream,
    ids: Vec<signal_hook::SigId>,
    /// Given for [`STOPS`]: set again on drop.
    by_default: Option<Arc<AtomicBool>>,
}

impl Woken {
    /// Woken by [`STOPS`].
    pub(crate) fn stops() -> io::Result<Self> {
        let by_default = stops_by_default();
        let mut woken = Self::by(&STOPS)?;
        if let Some(gate) = &by_default {
            gate.store(false, SeqCst);
        }
        woken.by_default = by_default;
        Ok(woken)
    }

    /// Woken by `signals`.
    fn by(signals: &[i32]) -> io::Result<Self> {
        let (pipe, wake_in) = UnixStream::pair()?;
        pipe.set_nonblocking(true)?;
        let mut woken = Self {
            pipe,
            ids: Vec::new(),
            by_default: None,
        };
        for &s in signals {
            woken.ids.push(signal_hook::low_level::pipe::register(
                s,
                wake_in.try_clone()?,
            )?);
        }
        Ok(woken)
    }
}

impl Drop for Woken {
    fn drop(&mut self) {
        // First, so a signal in between is not lost.
        if let Some(gate) = &self.by_default {
            gate.store(true, SeqCst);
        }
        for id in self.ids.drain(..) {
            signal_hook::low_level::unregister(id);
        }
    }
}

/// Frames for the host, sent as fast as it takes them and never waited
/// for. Past [`PENDING`] bytes the oldest typed input goes; the frame
/// being sent, and sizes, stay.
#[derive(Default)]
struct Outbox {
    /// Each frame's tag, and the frame as it goes on the wire.
    frames: VecDeque<(u8, Vec<u8>)>,
    /// How much of the first frame is sent.
    sent: usize,
    bytes: usize,
}

impl Outbox {
    fn push(&mut self, tag: u8, body: &[u8]) {
        let frame = wire::frame(tag, body);
        self.bytes += frame.len();
        self.frames.push_back((tag, frame));
        let mut i = usize::from(self.sent > 0);
        while self.bytes > PENDING && i + 1 < self.frames.len() {
            if self.frames[i].0 == wire::INPUT {
                let (_, lost) = self.frames.remove(i).unwrap_or_default();
                self.bytes -= lost.len();
            } else {
                i += 1;
            }
        }
    }

    /// Send what the socket takes now.
    fn flush(&mut self, sock: &UnixStream) -> io::Result<()> {
        while let Some((_, frame)) = self.frames.front() {
            match wire::send_now(sock, &frame[self.sent..]) {
                Ok(n) => self.sent += n,
                Err(e) if e.kind() == io::ErrorKind::WouldBlock => return Ok(()),
                Err(e) => return Err(e),
            }
            if self.sent == frame.len() {
                self.bytes -= frame.len();
                self.sent = 0;
                self.frames.pop_front();
            }
        }
        Ok(())
    }

    fn is_empty(&self) -> bool {
        self.frames.is_empty()
    }

    /// Send a last frame, `tag` with no body, after the rest of any frame
    /// begun (what waits behind it, typed for a program that has ended,
    /// is dropped), waiting for room as long as `sock`'s write timeout
    /// lets it.
    fn last(&mut self, sock: &UnixStream, tag: u8) -> io::Result<()> {
        let mut out = match self.frames.pop_front() {
            Some((_, begun)) if self.sent > 0 => begun[self.sent..].to_vec(),
            _ => Vec::new(),
        };
        self.frames.clear();
        self.sent = 0;
        self.bytes = 0;
        out.extend_from_slice(&wire::frame(tag, &[]));
        wire::send_all(sock, &out)
    }
}

/// Be the client of the host on `sock` until the user detaches, the
/// program ends, another client takes over or the host is lost. The
/// terminal is reset and restored on the way out, also when the client
/// is told to stop by a signal.
pub(crate) fn run(sock: UnixStream) -> io::Result<Outcome> {
    let stdin = io::stdin();
    let stdin = stdin.as_fd();
    let mut stdout = io::stdout().lock();
    let size = Size::of(stdout.as_fd()).unwrap_or_default();
    wire::write_frame(
        &sock,
        wire::HELLO,
        &wire::hello_body(size, &wire::terminal_env()),
    )?;

    // Before the terminal goes raw, so no signal finds it raw unhandled.
    let stop = Woken::stops()?;
    let winch = Woken::by(&[signal_hook::consts::SIGWINCH])?;
    let raw = Raw::enter(stdin)?;
    // The screen as the relayed output leaves it: the user's own at first.
    let mut screen = Scan::default();
    let outcome = (|| {
        stdout.write_all(MODES_OFF.as_bytes())?;
        stdout.flush()?;
        wire::write_frame(&sock, wire::READY, &[])?;
        relay(
            &sock,
            stdin,
            &mut stdout,
            &winch.pipe,
            &stop.pipe,
            &mut screen,
        )
    })();
    let _ = stdout.write_all(end_reset(screen.alt()).as_bytes());
    let _ = stdout.flush();
    drop(raw);
    // Only now: a stop signal must not find the terminal raw unhandled.
    drop((stop, winch));
    outcome
}

/// The relay loop proper; `screen` follows what it writes to `stdout`.
/// What the host sent is taken whole frames at a time, as many as came,
/// and written out at once.
fn relay(
    sock: &UnixStream,
    stdin: BorrowedFd<'_>,
    stdout: &mut impl Write,
    winch: &UnixStream,
    stop: &UnixStream,
    screen: &mut Scan,
) -> io::Result<Outcome> {
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let mut chord = Chord::new(Key::detach(), Key::switch());
    let mut buf = vec![0u8; 1 << 14];
    let mut outbox = Outbox::default();
    let mut inbox = wire::Inbox::default();
    let mut out = Vec::new();
    // When the bytes held now began to be held: they go on as they are
    // [`HOLD`] later, however busy the output keeps the loop.
    let mut held_since: Option<Instant> = None;
    loop {
        let wait = match (chord.holding(), held_since) {
            (false, _) => None,
            (true, None) => {
                held_since = Some(Instant::now());
                Some(HOLD)
            }
            (true, Some(at)) => Some(HOLD.saturating_sub(at.elapsed())),
        };
        if wait == Some(Duration::ZERO) {
            let held = chord.flush();
            held_since = None;
            outbox.push(wire::INPUT, &held);
            outbox.flush(sock)?;
            continue;
        }
        let timeout = wait.and_then(|w| Timespec::try_from(w).ok());
        let to_host = if outbox.is_empty() {
            PollFlags::IN
        } else {
            PollFlags::IN | PollFlags::OUT
        };
        let mut fds = [
            PollFd::from_borrowed_fd(stdin, PollFlags::IN),
            PollFd::new(sock, to_host),
            PollFd::new(winch, PollFlags::IN),
            PollFd::new(stop, PollFlags::IN),
        ];
        match poll(&mut fds, timeout.as_ref()) {
            // The deadline, checked at the top.
            Ok(0) => continue,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        let ready = |i: usize| !fds[i].revents().is_empty();
        let (keys, resized, stopped) = (ready(0), ready(2), ready(3));
        let socket = fds[1].revents();
        if stopped {
            return Ok(Outcome::Stopped);
        }

        if socket.contains(PollFlags::OUT) {
            outbox.flush(sock)?;
        }
        if !(socket - PollFlags::OUT).is_empty() {
            let open = inbox.fill(sock).unwrap_or(false);
            let mut end = (!open).then_some(Outcome::Lost);
            loop {
                match inbox.next() {
                    Ok(Some((wire::OUTPUT, bytes))) => {
                        screen.feed(bytes);
                        out.extend_from_slice(bytes);
                    }
                    // Joining a program on the alternate screen: there too.
                    Ok(Some((wire::SCREEN, b))) => {
                        if b.first() == Some(&1) && !screen.alt() {
                            screen.feed(ENTER.as_bytes());
                            out.extend_from_slice(ENTER.as_bytes());
                        }
                    }
                    Ok(Some((wire::EXIT, b))) if b.len() >= 4 => {
                        let left = String::from_utf8_lossy(&b[4..]).into_owned();
                        end = Some(Outcome::Exited {
                            code: i32::from_be_bytes([b[0], b[1], b[2], b[3]]),
                            left: (!left.is_empty()).then_some(left),
                        });
                        // Taken: the host keeps it for a later look if
                        // this is not heard.
                        let _ = wire::set_timeout(sock, false, Some(HEARD_WAIT));
                        let _ = outbox.last(sock, wire::HEARD);
                        break;
                    }
                    Ok(Some((wire::ELSEWHERE, _))) => {
                        end = Some(Outcome::Elsewhere);
                        break;
                    }
                    Ok(Some(_)) => {}
                    Ok(None) => break,
                    Err(_) => {
                        end = Some(Outcome::Lost);
                        break;
                    }
                }
            }
            if !out.is_empty() {
                stdout.write_all(&out)?;
                stdout.flush()?;
                out.clear();
            }
            if let Some(end) = end {
                return Ok(end);
            }
        }
        if resized {
            while rustix::io::read(winch, &mut [0u8; 64]).is_ok_and(|n| n > 0) {}
            if let Ok(size) = Size::of(io::stdout()) {
                outbox.push(wire::RESIZE, &size.encode());
                outbox.flush(sock)?;
            }
        }
        if keys {
            let n = match rustix::io::read(stdin, &mut buf) {
                // The terminal is gone (a dropped connection): the session
                // carries on without us.
                Ok(0) => return Ok(Outcome::Stopped),
                Ok(n) => n,
                Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
                Err(_) => return Ok(Outcome::Stopped),
            };
            let (pass, action) = chord.feed(&buf[..n]);
            // Whatever is held now is new.
            held_since = None;
            if !pass.is_empty() {
                outbox.push(wire::INPUT, &pass);
                outbox.flush(sock)?;
            }
            match action {
                Some(Action::Detach) => return Ok(Outcome::Detached),
                Some(Action::Switch) => return Ok(Outcome::Switch),
                None => {}
            }
        }
    }
}

/// Attach to session `id` of `tool` in `dir` and report how it ended, as
/// a user reads it; the program's status is returned for the caller to
/// exit with. The switch key moves the client to the session the picker
/// gives, or back to this one. One that ended while the picker was open
/// is said, and the picker offered again.
pub(crate) fn attach_and_report(dir: &Path, tool: &Tool, id: &str) -> io::Result<i32> {
    let mut id = id.to_string();
    let mut switched = false;
    loop {
        let sock = match wire::connect(dir::socket(dir, &id)) {
            Ok(s) => s,
            Err(_) if switched => {
                eprintln!("[{id} has ended]");
                match switch(dir, tool, None) {
                    Some(other) => id = other,
                    None => return Ok(0),
                }
                continue;
            }
            Err(e) => return Err(e),
        };
        let outcome = run(sock)?;
        if outcome == Outcome::Switch {
            id = switch(dir, tool, Some(&id)).unwrap_or(id);
            switched = true;
            continue;
        }
        return Ok(report(dir, tool, &id, &outcome));
    }
}

/// The session to go on with after the switch key: the one picked, else
/// `current` again (None with no current one, or no session left).
fn switch(dir: &Path, tool: &Tool, current: Option<&str>) -> Option<String> {
    let live = dir::live(dir, &tool.prefix()).unwrap_or_default();
    if live.is_empty() {
        return None;
    }
    match picker::choose_or_say(dir, tool, false, current, live) {
        Some(picker::Choice::Attach(other)) => Some(other),
        _ => current.map(str::to_string),
    }
}

/// Say how the client's time with session `id` ended; the status to exit
/// with.
fn report(dir: &Path, tool: &Tool, id: &str, outcome: &Outcome) -> i32 {
    match outcome {
        Outcome::Detached | Outcome::Stopped | Outcome::Switch => {
            // Only a detach by the key asks for a label.
            let label = dir::read_label(dir, id).or_else(|| {
                (*outcome == Outcome::Detached)
                    .then(|| ask_label(dir, &tool.prefix(), id))
                    .flatten()
            });
            match label {
                Some(label) => eprintln!("detached: {label}"),
                None => eprintln!(
                    "[detached from {id}; `{} attach {id}` to return]",
                    crate::exe_name()
                ),
            }
            0
        }
        Outcome::Elsewhere => {
            eprintln!("[{id} was attached from another terminal]");
            0
        }
        Outcome::Lost => {
            eprintln!("[lost the connection to {id}]");
            1
        }
        Outcome::Exited { code, left } => {
            // What the program left, as it would have said it: handed
            // over by the host, so no other terminal says it.
            if let Some(text) = left {
                eprintln!("{text}");
            }
            if *code != 0 {
                eprintln!("[{id} exited with status {code}]");
            }
            *code
        }
    }
}

/// Offer session `id` of the tool whose sessions start with `prefix`,
/// which has no label, one, on the user's restored terminal; the label it
/// was given. Not asked when stdin or stderr is not a terminal.
fn ask_label(dir: &Path, prefix: &str, id: &str) -> Option<String> {
    if !rustix::termios::isatty(io::stdin()) || !rustix::termios::isatty(io::stderr()) {
        return None;
    }
    // Keys typed after the detach key, or its auto-repeat, are not the
    // answer (an ESC and Ctrl+D would end or spoil it).
    let _ = rustix::termios::tcflush(io::stdin(), rustix::termios::QueueSelector::IFlush);
    // Ctrl+C, and a stop from elsewhere, skip the question.
    let stop = Woken::stops().ok()?;
    prompt(
        io::stdin().as_fd(),
        &mut io::stderr(),
        &stop.pipe,
        id,
        |label| dir::set_label(dir, prefix, id, label),
    )
}

/// Ask for a label for `id` on `out`, reading lines from `input` (a
/// terminal in line mode), until `take` takes one; a refused one is
/// reported and asked again. None on an empty line, at the end of the
/// input, or when `stop` is woken.
pub(crate) fn prompt(
    input: BorrowedFd<'_>,
    out: &mut impl Write,
    stop: &UnixStream,
    id: &str,
    mut take: impl FnMut(&str) -> io::Result<()>,
) -> Option<String> {
    loop {
        let _ = write!(out, "Detached {id}. Name it (Enter to keep): ");
        let _ = out.flush();
        let Some(line) = read_line(input, stop) else {
            let _ = writeln!(out);
            return None;
        };
        let label = line.trim();
        if label.is_empty() {
            return None;
        }
        match take(label) {
            Ok(()) => return Some(label.to_string()),
            Err(e) => {
                let _ = writeln!(out, "{e}");
            }
        }
    }
}

/// One line from `input`, without its end; None at the end of the input,
/// or when `stop` is woken.
fn read_line(input: BorrowedFd<'_>, stop: &UnixStream) -> Option<String> {
    use rustix::event::{poll, PollFd, PollFlags};
    let mut line = Vec::new();
    let mut buf = [0u8; 256];
    loop {
        let mut fds = [
            PollFd::from_borrowed_fd(input, PollFlags::IN),
            PollFd::new(stop, PollFlags::IN),
        ];
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(_) => return None,
        }
        if !fds[1].revents().is_empty() {
            return None;
        }
        if fds[0].revents().is_empty() {
            continue;
        }
        match rustix::io::read(input, &mut buf) {
            Ok(0) => return None,
            Ok(n) => line.extend_from_slice(&buf[..n]),
            Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
            Err(_) => return None,
        }
        if let Some(end) = line.iter().position(|&b| b == b'\n') {
            line.truncate(end);
            return Some(String::from_utf8_lossy(&line).into_owned());
        }
    }
}

#[cfg(test)]
#[path = "tests/client.rs"]
mod tests;
