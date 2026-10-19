//! The processes a TUI starts and follows on a worker thread (a fit, a
//! save): run one to its end where it can be stopped ([`run_one`],
//! [`Stopper`]), and follow its log a line at a time ([`follow`]).
//!
//! A child's stderr is a pseudo-terminal, so the progress bars it draws
//! (indicatif hides them from a pipe) reach us: each frame becomes a
//! [`Progress`], the rest are lines of its log. [`Bar::track`] follows
//! one bar through them, its score trend included, for [`gauge`] to draw.
//!
//! A child set up in a process group of its own ([`crate::Group::Own`])
//! is stopped with what it started, and while a [`Stopper`] holds it a
//! hangup of the program (its host gone, or the user's terminal) is
//! passed on to it, which the kernel would not do. A save
//! ([`Stopper::saving`]) is spared both: the program's quit path lets it
//! finish ([`Stopper::quit`]), and it outlives a hangup to write what it
//! has whole.
//!
//! ```no_run
//! use legume_tui::child::{run_one, Said, Stopper};
//! use legume_tui::Group;
//! # let session = legume_tui::Session::plain(&legume_tui::Tool::new("tool"));
//! let stopper = std::sync::Arc::new(Stopper::new());
//! let env = session.child_env(); // owned: for the worker thread
//! let held = std::sync::Arc::clone(&stopper);
//! std::thread::spawn(move || {
//!     let mut fit = std::process::Command::new("tool");
//!     env.apply(fit.arg("fit"), Group::Own);
//!     run_one(fit, &held, |said| match said {
//!         Said::Progress(p) => { /* a bar: p.pos of p.len, p.score() */ }
//!         Said::Line(line) => { /* a line of its log */ }
//!         _ => {}
//!     })
//! });
//! // On the quit key, or a `kill`: a save is let finish.
//! stopper.quit();
//! ```

mod bar;
pub use bar::{gauge, spark, Bar, TREND};

use rustix::process::{kill_process, kill_process_group, Pid, Signal};
use std::io::{self, Read};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU8, Ordering::SeqCst};
use std::sync::Mutex;

/// Stops a run of child commands: the one running is interrupted, as
/// Ctrl+C would, so a fit can wrap up; asked again it is killed. A child
/// started after a stop gets it at once.
#[derive(Debug, Default)]
pub struct Stopper {
    /// How far stopping has gone: [`RUNNING`], interrupted (1), or [`KILL`].
    level: AtomicU8,
    child: Mutex<Option<Held>>,
    /// Its children write what must be whole: quitting lets them finish.
    saves: bool,
}

const RUNNING: u8 = 0;
const KILL: u8 = 2;

/// A child where [`Stopper::stop`] reaches it.
#[derive(Debug)]
struct Held {
    child: Child,
    /// The process group it leads, if it leads one.
    group: Option<Pid>,
    /// A hangup of the program is passed on to that group.
    hangs_up: bool,
}

impl Stopper {
    pub fn new() -> Self {
        Self::default()
    }

    /// A stopper for children that save (write what must be whole):
    /// [`Stopper::quit`] lets them finish, and a hangup of the program is
    /// not passed on to them. [`Stopper::stop`] still stops them.
    ///
    /// What a save is spared does not keep its stderr and stdout: those
    /// are the program's (its terminal, its pipes, [`run_one`]'s
    /// terminal) and close with it, so a write there after a hangup fails
    /// (and Rust's `eprintln!` panics on it). A save that must outlive its
    /// program writes what it has before anything it says, logs to a file,
    /// or takes a failed write in its stride.
    pub fn saving() -> Self {
        let mut stopper = Self::default();
        stopper.saves = true;
        stopper
    }

    /// Whether its children save ([`Stopper::saving`]).
    pub fn saves(&self) -> bool {
        self.saves
    }

    /// Go one step further, interrupt then kill, and tell the child.
    /// Returns whether this stop kills.
    pub fn stop(&self) -> bool {
        let mut was = self.level.load(SeqCst);
        let now = loop {
            let now = (was + 1).min(KILL);
            match self.level.compare_exchange_weak(was, now, SeqCst, SeqCst) {
                Ok(_) => break now,
                Err(l) => was = l,
            }
        };
        self.deliver();
        now == KILL
    }

    /// What the program's quit path (its quit key, a `kill`) does: a save
    /// is let finish, anything else is stopped ([`Stopper::stop`]).
    /// Returns whether it was stopped.
    pub fn quit(&self) -> bool {
        if self.saves {
            return false;
        }
        self.stop();
        true
    }

    /// Kill the child now, interrupted or not.
    pub fn kill(&self) {
        self.level.store(KILL, SeqCst);
        self.deliver();
    }

    pub fn is_stopped(&self) -> bool {
        self.level.load(SeqCst) > RUNNING
    }

    /// Keep `child`, started by the caller (its pipes taken as it needs),
    /// where [`Stopper::stop`] reaches it, until [`Stopper::release`];
    /// stopped already, it gets the stop asked for at once. One that leads
    /// a process group of its own is stopped with all of it, and, unless
    /// it saves, hears a hangup of the program.
    pub fn hold(&self, child: Child) {
        let pid = Pid::from_child(&child);
        let group = rustix::process::getpgid(Some(pid))
            .ok()
            .filter(|g| *g == pid);
        let hangs_up =
            !self.saves && group.is_some_and(|g| crate::stop::hold_group(g.as_raw_nonzero().get()));
        if let Some(old) = self.lock().replace(Held {
            child,
            group,
            hangs_up,
        }) {
            release_group(&old);
        }
        self.deliver();
    }

    /// The child [`Stopper::hold`] kept, to wait for.
    pub fn release(&self) -> Option<Child> {
        let held = self.lock().take()?;
        release_group(&held);
        Some(held.child)
    }

    /// The process id of the child held, if one is.
    pub fn id(&self) -> Option<u32> {
        self.lock().as_ref().map(|h| h.child.id())
    }

    /// Whether the child held has ended, as [`Child::try_wait`] says, for
    /// a loop that polls it: once it has, it is let go, as by
    /// [`Stopper::release`] and waited for. None while it runs, or with
    /// none held.
    pub fn try_wait(&self) -> io::Result<Option<ExitStatus>> {
        let mut held = self.lock();
        let Some(h) = held.as_mut() else {
            return Ok(None);
        };
        let ended = h.child.try_wait()?;
        if ended.is_some() {
            if let Some(h) = held.take() {
                release_group(&h);
            }
        }
        Ok(ended)
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, Option<Held>> {
        self.child.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Send the child the stop asked for so far, if any.
    fn deliver(&self) {
        let level = self.level.load(SeqCst);
        if level == RUNNING {
            return;
        }
        if let Some(held) = self.lock().as_mut() {
            signal(held, level == KILL);
        }
    }
}

impl Drop for Stopper {
    /// A child still held is let go, not stopped: a hangup no longer goes
    /// to it.
    fn drop(&mut self) {
        if let Some(held) = self.lock().take() {
            release_group(&held);
        }
    }
}

/// A hangup no longer goes to what `held` leads.
fn release_group(held: &Held) {
    if let (true, Some(group)) = (held.hangs_up, held.group) {
        crate::stop::release_group(group.as_raw_nonzero().get());
    }
}

/// Interrupt (SIGINT) or kill (SIGKILL) `held`'s child, and every process
/// in the group it leads: anything left holding its stderr would keep
/// the log open.
fn signal(held: &mut Held, kill: bool) {
    // Not waited for yet: the pid and group are still its.
    let sig = if kill { Signal::KILL } else { Signal::INT };
    let _ = match held.group {
        Some(group) => kill_process_group(group, sig),
        None => kill_process(Pid::from_child(&held.child), sig),
    };
    if kill {
        let _ = held.child.kill();
    }
}

/// Why [`run_one`] did not finish well.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Failed {
    /// The [`Stopper`] was stopped.
    Stopped,
    /// The command did not start.
    Start(String),
    /// It ended badly: its error line, or how it ended and what it was
    /// doing then.
    Exit(String),
}

/// What a child said on its stderr.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Said {
    /// A line of its log.
    Line(String),
    /// A frame of a progress bar.
    Progress(Progress),
}

/// Where a progress bar stands: `pos` of `len` (0 for a spinner), and
/// what it counts.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Progress {
    pub pos: u64,
    pub len: u64,
    pub what: String,
}

impl Progress {
    pub fn new(pos: u64, len: u64, what: impl Into<String>) -> Self {
        Self {
            pos,
            len,
            what: what.into(),
        }
    }

    /// The score the bar's message carries: the first finite `key=number`
    /// word, as in `fit gap=0.412 recon=0.3` (a bare number, such as one in
    /// a label, is not a score). None while the message has none.
    pub fn score(&self) -> Option<f64> {
        scored(&self.what).map(|(_, v)| v)
    }
}

/// The value of a `key=number` word.
fn number(word: &str) -> Option<f64> {
    word.split_once('=')?.1.parse().ok()
}

/// `what` up to and including its score word, and the score: the first
/// finite `key=number` word ([`Progress::score`]).
fn scored(what: &str) -> Option<(&str, f64)> {
    let mut rest = what;
    loop {
        let word = rest.trim_start();
        let end = word.find(char::is_whitespace).unwrap_or(word.len());
        if end == 0 {
            return None;
        }
        rest = &word[end..];
        if let Some(v) = number(&word[..end]).filter(|v| v.is_finite()) {
            return Some((&what[..what.len() - rest.len()], v));
        }
    }
}

/// The columns a child's terminal has: wide enough that a bar and its
/// label are not cut.
const COLUMNS: u16 = 160;

/// A pseudo-terminal for a child's stderr: our end to read, and the
/// child's to write to.
fn terminal() -> std::io::Result<(std::fs::File, std::os::fd::OwnedFd)> {
    let (ours, theirs) = crate::pty::open()?;
    let size = rustix::termios::Winsize {
        ws_row: 50,
        ws_col: COLUMNS,
        ws_xpixel: 0,
        ws_ypixel: 0,
    };
    rustix::termios::tcsetwinsize(&theirs, size)?;
    Ok((ours, theirs))
}

/// Run `command` to its end where `stopper` can stop it, what it says on
/// its stderr (a terminal, where one can be made) to `each`; its stdin
/// and stdout are let go. Set it up as the session starts its children
/// first ([`crate::Session::command`], [`crate::ChildEnv::apply`]), with
/// [`crate::Group::Own`] for a stop to reach what it starts too: in the
/// program's group, a stop reaches the child alone, and what it started
/// may keep the log open, and this call waiting, after it.
pub fn run_one(
    mut command: Command,
    stopper: &Stopper,
    each: impl FnMut(Said),
) -> Result<(), Failed> {
    let program = std::path::Path::new(command.get_program())
        .file_name()
        .map_or_else(String::new, |n| n.to_string_lossy().into_owned());
    let terminal = match terminal() {
        Ok((ours, theirs)) => {
            command.stderr(Stdio::from(theirs));
            Some(ours)
        }
        Err(_) => {
            command.stderr(Stdio::piped());
            None
        }
    };
    let mut child = command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .spawn()
        .map_err(|e| Failed::Start(format!("cannot run {program}: {e}")))?;
    // Our copy of the child's end closes, so its end is the end of the log.
    drop(command);
    let log: Option<Box<dyn Read + Send>> = match terminal {
        Some(t) => Some(Box::new(t)),
        None => child
            .stderr
            .take()
            .map(|e| Box::new(e) as Box<dyn Read + Send>),
    };
    stopper.hold(child);
    let last = follow(log, each);
    let status = stopper.release().map(|mut c| c.wait());
    if stopper.is_stopped() {
        return Err(Failed::Stopped);
    }
    match status {
        Some(Ok(s)) if s.success() => Ok(()),
        Some(Ok(s)) => Err(Failed::Exit(reason(&last, &s.to_string()))),
        Some(Err(e)) => Err(Failed::Exit(e.to_string())),
        None => Err(Failed::Exit(reason(&last, "ended"))),
    }
}

/// Why a child that ended badly did: its error line when its last line
/// is one, else how it ended (`exit status: 1`, `signal: 9 (SIGKILL)`)
/// and what it was doing then.
fn reason(last: &str, ended: &str) -> String {
    if let Some(why) = last.strip_prefix("Error: ") {
        why.to_string()
    } else if last.starts_with("error") || last.contains("panicked") {
        last.to_string()
    } else if last.is_empty() {
        ended.to_string()
    } else {
        format!("{ended}, after: {last}")
    }
}

/// Read a child's stderr to the end, in pieces split at `\r` and `\n`
/// (a progress bar redraws after a `\r`, with no new line), each as
/// [`log_line`] trims it: a bar frame as a [`Said::Progress`], anything
/// else as a [`Said::Line`]. Returns the last line.
pub fn follow(log: Option<impl Read>, mut each: impl FnMut(Said)) -> String {
    let mut last = String::new();
    let Some(mut log) = log else {
        return last;
    };
    let mut said = |piece: &[u8]| {
        let line = log_line(&String::from_utf8_lossy(piece));
        if line.is_empty() {
            return;
        }
        match progress_of(&line) {
            Some(p) => each(Said::Progress(p)),
            None => {
                last.clone_from(&line);
                each(Said::Line(line));
            }
        }
    };
    let mut pending: Vec<u8> = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        let n = match log.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            // A terminal whose child is gone reads as an error.
            Err(_) => break,
        };
        for &b in &buf[..n] {
            if b == b'\r' || b == b'\n' {
                said(&pending);
                pending.clear();
            } else {
                pending.push(b);
            }
        }
    }
    said(&pending);
    last
}

/// Read a child's log to the end a line at a time, every line to `each`;
/// progress bars left out. Returns the last line.
pub fn follow_log(log: Option<impl Read>, mut each: impl FnMut(&str)) -> String {
    follow(log, |s| {
        if let Said::Line(l) = s {
            each(&l);
        }
    })
}

/// A progress bar's frame, its elapsed time already gone: the bar of `#`
/// and `-`, `pos/len`, the time left in brackets, then what it counts. A
/// spinner's frame starts with one of its ticks (braille dots, as
/// indicatif draws them).
pub fn progress_of(line: &str) -> Option<Progress> {
    let (head, rest) = line.split_once(char::is_whitespace)?;
    if head.chars().all(|c| ('\u{2800}'..='\u{28ff}').contains(&c)) {
        return Some(Progress {
            what: rest.trim().to_string(),
            ..Progress::default()
        });
    }
    if !head.chars().all(|c| c == '#' || c == '-') {
        return None;
    }
    let rest = rest.trim_start();
    let (count, rest) = rest.split_once(char::is_whitespace).unwrap_or((rest, ""));
    let (pos, len) = count.split_once('/')?;
    let what = rest.trim_start();
    let what = match what.strip_prefix('(') {
        Some(w) => w.split_once(')').map_or("", |(_, after)| after),
        None => what,
    };
    Some(Progress {
        pos: pos.parse().ok()?,
        len: len.parse().ok()?,
        what: what.trim().to_string(),
    })
}

/// A log line without terminal colours or its "[time LEVEL module] "
/// prefix: the message is what matters on a status line.
pub fn log_line(line: &str) -> String {
    let mut plain = String::with_capacity(line.len());
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        if c != '\u{1b}' {
            plain.push(c);
        } else if chars.next() == Some('[') {
            // A CSI sequence runs to its final byte.
            for c in chars.by_ref() {
                if ('@'..='~').contains(&c) {
                    break;
                }
            }
        }
    }
    let t = plain.trim();
    match t.split_once("] ") {
        Some((head, msg)) if head.starts_with('[') => msg.trim().to_string(),
        _ => t.to_string(),
    }
}

#[cfg(test)]
#[path = "tests/child.rs"]
mod tests;
