//! Work on another thread while a screen stays up: an answer picked up
//! between frames ([`Pending`]), work started once and read by anyone
//! holding it ([`Loading`]), work run while the caller waits on it
//! ([`on_thread`]), a spinner, and a wait that shows something only when
//! the work takes more than a moment ([`waiting`], [`counting`],
//! [`during_inline`]).
//!
//! senna's `Pending`, `Loading`, `LOADING` and `SPINNER`
//! (`src/tui/mod.rs`) and its stderr spinner (`src/tui/busy.rs`), and
//! faba's `Pending`, `on_thread`, `panic_text`, `SPINNER`,
//! `spinner_frame` (`src/tui.rs`) and the waiting under its busy popup
//! (`src/tui/busy.rs`). A panic in the work is an error saying so, with its
//! message, everywhere: never a waiter left waiting, never a panic raised
//! again.

use std::any::Any;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

/// Frames of the spinners the tools draw.
pub const SPINNER: [char; 8] = ['⠁', '⠂', '⠄', '⡀', '⢀', '⠠', '⠐', '⠈'];

/// How long a [`spinner_frame`] shows: senna's 100 ms, the period of a
/// [`waiting`] tick. faba turned one every 200 ms
/// ([`spinner_frame_every`]).
pub const SPIN: Duration = Duration::from_millis(100);

/// The [`SPINNER`] frame `elapsed` into a spin: one every [`SPIN`].
#[must_use]
pub fn spinner_frame(elapsed: Duration) -> char {
    spinner_frame_every(elapsed, SPIN)
}

/// The [`SPINNER`] frame `elapsed` into a spin turning one every `period`
/// (a zero period as a nanosecond).
#[must_use]
pub fn spinner_frame_every(elapsed: Duration, period: Duration) -> char {
    let turns = elapsed.as_nanos() / period.as_nanos().max(1);
    SPINNER[(turns % SPINNER.len() as u128) as usize]
}

/// Why work came back without its own answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Stopped {
    /// It panicked, with the panic's message.
    Panicked { what: String, message: String },
    /// It ended and left nothing to read: the answer was taken already.
    NoResult { what: String },
}

impl Stopped {
    /// `what` panicked with `panic`.
    #[must_use]
    pub fn panicked(what: &str, panic: &(dyn Any + Send)) -> Self {
        Self::Panicked {
            what: what.to_string(),
            message: panic_text(panic).to_string(),
        }
    }
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Panicked { what, message } => write!(f, "{what} panicked: {message}"),
            Self::NoResult { what } => write!(f, "{what} ended without a result"),
        }
    }
}

impl std::error::Error for Stopped {}

/// The text of the error, for work whose errors are text.
impl From<Stopped> for String {
    fn from(stopped: Stopped) -> Self {
        stopped.to_string()
    }
}

/// A panic's message, when it is text.
#[must_use]
pub fn panic_text(panic: &(dyn Any + Send)) -> &str {
    panic
        .downcast_ref::<&str>()
        .copied()
        .or_else(|| panic.downcast_ref::<String>().map(String::as_str))
        .unwrap_or("no message")
}

/// Where a [`Pending`] answer is.
#[derive(Debug)]
enum State<A> {
    Waiting,
    Came(A),
    Taken,
    /// The sender dropped without an answer.
    Gone,
}

/// The answer of work on another thread, as a screen picks it up between
/// frames from the channel it comes on.
#[derive(Debug)]
pub struct Pending<T, E = String> {
    rx: Receiver<Result<T, E>>,
    state: State<Result<T, E>>,
}

impl<T, E> Pending<T, E> {
    /// Waiting on `rx`.
    #[must_use]
    pub fn new(rx: Receiver<Result<T, E>>) -> Self {
        Self {
            rx,
            state: State::Waiting,
        }
    }

    /// A pending answer and where to send it, for a worker that answers
    /// more than one waiter.
    #[must_use]
    pub fn pair() -> (Sender<Result<T, E>>, Self) {
        let (tx, rx) = channel();
        (tx, Self::new(rx))
    }

    /// Pick up the answer, if it has come; whether the work is done.
    pub fn poll(&mut self) -> bool {
        if matches!(self.state, State::Waiting) {
            match self.rx.try_recv() {
                Ok(out) => self.state = State::Came(out),
                Err(TryRecvError::Disconnected) => self.state = State::Gone,
                Err(TryRecvError::Empty) => {}
            }
        }
        self.done()
    }

    /// [`Self::poll`], waiting up to `wait` for the answer.
    pub fn wait_for(&mut self, wait: Duration) -> bool {
        if matches!(self.state, State::Waiting) {
            match self.rx.recv_timeout(wait) {
                Ok(out) => self.state = State::Came(out),
                Err(RecvTimeoutError::Disconnected) => self.state = State::Gone,
                Err(RecvTimeoutError::Timeout) => {}
            }
        }
        self.done()
    }

    /// Whether the answer came (taken or not), or the work went without
    /// one.
    #[must_use]
    pub fn done(&self) -> bool {
        !matches!(self.state, State::Waiting)
    }

    /// Whether the work ended without an answer: its sender dropped (it
    /// died).
    #[must_use]
    pub fn gone(&self) -> bool {
        matches!(self.state, State::Gone)
    }

    /// The answer, once it came and until it is taken.
    #[must_use]
    pub fn out(&self) -> Option<&Result<T, E>> {
        match &self.state {
            State::Came(out) => Some(out),
            _ => None,
        }
    }

    /// The answer itself, once it came; none after.
    pub fn take(&mut self) -> Option<Result<T, E>> {
        match std::mem::replace(&mut self.state, State::Taken) {
            State::Came(out) => Some(out),
            other => {
                self.state = other;
                None
            }
        }
    }

    /// Pick up and take the answer once it comes, or `stopped` once when
    /// the work went without one; none while it runs and after.
    pub fn try_take(&mut self, stopped: impl Into<E>) -> Option<Result<T, E>> {
        self.poll();
        if self.gone() {
            self.state = State::Taken;
            return Some(Err(stopped.into()));
        }
        self.take()
    }
}

impl<T: Send + 'static, E: From<Stopped> + Send + 'static> Pending<T, E> {
    /// `work` on a thread of its own; a panic in it is its answer
    /// ([`Stopped::Panicked`], as "the work").
    pub fn spawn(work: impl FnOnce() -> Result<T, E> + Send + 'static) -> Self {
        let (tx, pending) = Self::pair();
        std::thread::spawn(move || {
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|panic| Err(Stopped::panicked("the work", &*panic).into()));
            let _ = tx.send(out);
        });
        pending
    }
}

/// Run `work` on a thread while `meanwhile` waits here on its [`Pending`]
/// answer; what `meanwhile` returns, and the work's answer: the one left
/// in the pending (`meanwhile` may poll it, not take it), else (the work
/// panicked) [`Stopped::Panicked`] saying `what` panicked, with the
/// panic's message.
pub fn on_thread<T: Send, E: From<Stopped> + Send, R>(
    what: &str,
    work: impl FnOnce() -> Result<T, E> + Send,
    meanwhile: impl FnOnce(&mut Pending<T, E>) -> R,
) -> (R, Result<T, E>) {
    let (tx, mut pending) = Pending::pair();
    std::thread::scope(|s| {
        let worker = s.spawn(move || {
            // The receiver outlives the scope, so the send cannot fail.
            let _ = tx.send(work());
        });
        let waited = meanwhile(&mut pending);
        // Joined here, a panic is the work's error: the scope raises it
        // again only for threads it joins itself.
        let joined = worker.join();
        pending.poll();
        let out = match (pending.take(), joined) {
            (Some(out), _) => out,
            (None, Err(panic)) => Err(Stopped::panicked(what, &*panic).into()),
            (None, Ok(())) => Err(Stopped::NoResult {
                what: what.to_string(),
            }
            .into()),
        };
        (waited, out)
    })
}

/// What an answer not come yet says ([`Loading::answer`]); an action that
/// got it is done again when the answer comes.
pub const LOADING: &str = "loading…";

/// What a [`Loading`] does when started.
type Work<T> = Box<dyn FnOnce() -> Result<T, String> + Send>;

/// Work started once on a worker thread and read by anyone holding it:
/// waited for where the answer is needed now ([`Self::get`]), looked at
/// where it is not ([`Self::try_get`]).
pub struct Loading<T> {
    work: Mutex<Option<Work<T>>>,
    value: Arc<OnceLock<Result<T, String>>>,
    since: OnceLock<Instant>,
}

impl<T: Send + Sync + 'static> Loading<T> {
    /// `work`, not started yet.
    pub fn new(work: impl FnOnce() -> Result<T, String> + Send + 'static) -> Self {
        Self {
            work: Mutex::new(Some(Box::new(work))),
            value: Arc::default(),
            since: OnceLock::new(),
        }
    }

    /// An answer known already, with no work to start.
    #[must_use]
    pub fn ready(value: Result<T, String>) -> Self {
        Self {
            work: Mutex::new(None),
            value: Arc::new(OnceLock::from(value)),
            since: OnceLock::new(),
        }
    }

    /// Start the work on a worker thread, unless it was started already.
    pub fn start(&self) {
        let Some(work) = self.work.lock().ok().and_then(|mut w| w.take()) else {
            return;
        };
        let _ = self.since.set(Instant::now());
        let value = self.value.clone();
        std::thread::spawn(move || {
            // A panic is an answer too, so a waiter never waits for ever.
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(work))
                .unwrap_or_else(|panic| Err(Stopped::panicked("the work", &*panic).into()));
            let _ = value.set(out);
        });
    }

    /// The answer, if it has come.
    pub fn try_get(&self) -> Option<&Result<T, String>> {
        self.value.get()
    }

    /// The answer: waited for when `wait`, else [`LOADING`] until it
    /// comes (the work started meanwhile).
    pub fn answer(&self, wait: bool) -> Result<&T, String> {
        let read = if wait {
            self.get()
        } else {
            self.start();
            self.try_get().ok_or_else(|| LOADING.to_string())?
        };
        read.as_ref().map_err(Clone::clone)
    }

    /// How long the work has been running, while it runs.
    pub fn busy(&self) -> Option<Duration> {
        self.value
            .get()
            .is_none()
            .then(|| self.since.get().map(Instant::elapsed))
            .flatten()
    }

    /// The answer, starting the work if need be and waiting for it.
    pub fn get(&self) -> &Result<T, String> {
        self.start();
        self.value.wait()
    }
}

/// How long work may take before a wait shows anything, so quick work
/// never flickers the screen.
pub const GRACE: Duration = Duration::from_millis(150);

/// How often a wait ticks once it shows: a [`SPIN`].
pub const TICK: Duration = SPIN;

/// How far work a wait shows has got.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tick {
    /// Since the work started.
    pub elapsed: Duration,
    /// What the work has counted off, at most `total`.
    pub done: usize,
    /// What it counts to; 0 when it counts nothing.
    pub total: usize,
}

impl Tick {
    /// The spinner's frame now.
    #[must_use]
    pub fn spin(&self) -> char {
        spinner_frame(self.elapsed)
    }
}

/// Run `work` on a thread; after [`GRACE`], call `tick` with how far it
/// has got, again every [`TICK`], until it is done (a popup draws a frame
/// there). Whether `tick` was called, and the work's answer: a panic in it
/// is [`Stopped::Panicked`] saying `what` panicked.
pub fn waiting<T: Send, E: From<Stopped> + Send>(
    what: &str,
    work: impl FnOnce() -> Result<T, E> + Send,
    tick: impl FnMut(Tick),
) -> (bool, Result<T, E>) {
    counting(what, 0, |_| work(), tick)
}

/// [`waiting`], for work that counts off `total` steps on the counter it is
/// given; each [`Tick`] says how many.
pub fn counting<T: Send, E: From<Stopped> + Send>(
    what: &str,
    total: usize,
    work: impl FnOnce(&AtomicUsize) -> Result<T, E> + Send,
    mut tick: impl FnMut(Tick),
) -> (bool, Result<T, E>) {
    let done = AtomicUsize::new(0);
    let counter = &done;
    let start = Instant::now();
    on_thread(
        what,
        || work(counter),
        |pending| {
            let mut shown = false;
            let mut wait = GRACE;
            // Ends when the work answers, or dies (its sender dropped).
            while !pending.wait_for(wait) {
                tick(Tick {
                    elapsed: start.elapsed(),
                    done: counter.load(Ordering::Relaxed).min(total),
                    total,
                });
                shown = true;
                wait = TICK;
            }
            shown
        },
    )
}

/// Run `work`; when it takes more than a moment, a spinner, `what` it does
/// and the seconds on stderr's own line, rewritten in place and wiped when
/// done; nothing when stderr is not a terminal. For work before a screen
/// is up, so the terminal never sits blank without a word.
pub fn during_inline<T: Send, E: From<Stopped> + Send>(
    what: &str,
    work: impl FnOnce() -> Result<T, E> + Send,
) -> Result<T, E> {
    during_inline_counting(what, 0, |_| work())
}

/// [`during_inline`], with how many of `total` steps `work` has counted
/// off on the counter it is given.
pub fn during_inline_counting<T: Send, E: From<Stopped> + Send>(
    what: &str,
    total: usize,
    work: impl FnOnce(&AtomicUsize) -> Result<T, E> + Send,
) -> Result<T, E> {
    use std::io::{IsTerminal, Write};
    let terminal = std::io::stderr().is_terminal();
    let (shown, out) = counting(what, total, work, |t| {
        if !terminal {
            return;
        }
        let width = ratatui::crossterm::terminal::size().map_or(80, |(w, _)| usize::from(w));
        let mut err = std::io::stderr();
        let _ = write!(err, "\r{}", line(what, t, width.saturating_sub(1)));
        let _ = err.flush();
    });
    if shown && terminal {
        eprint!("\r\x1b[K");
    }
    out
}

/// The spinner's line, `width` wide: ` ⠂ reading BMM_vae… 3s`, or
/// ` ⠂ reading… 2/5 3s` when it counts.
fn line(what: &str, t: Tick, width: usize) -> String {
    let secs = t.elapsed.as_secs();
    let text = match t.total {
        0 => format!(" {} {what}… {secs}s", t.spin()),
        total => format!(" {} {what}… {}/{total} {secs}s", t.spin(), t.done),
    };
    let pad = width.saturating_sub(text.chars().count());
    let mut out: String = text.chars().take(width).collect();
    out.extend(std::iter::repeat_n(' ', pad));
    out
}

#[cfg(test)]
#[path = "tests/work.rs"]
mod tests;
