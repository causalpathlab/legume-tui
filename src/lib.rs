//! Detachable terminal sessions for the legume tools.
//!
//! A TUI started through [`Tool::session`] runs in a *host*: a small
//! process, off the user's login session, that owns a pseudo-terminal and
//! relays bytes between it and the user's terminal. The user can leave
//! with Ctrl+Alt+D ([`detach_key`]; or lose the connection) and come back
//! by starting the tool again or with `<tool> attach`; the program, and
//! whatever it started, keeps running in between. Ctrl+Alt+S
//! ([`switch_key`]) goes to another of the tool's sessions. A session
//! keeps its id (`tool-view-1`) for life and may carry a label the user
//! gives it, on detach or later. The host never changes the bytes, so
//! kitty, sixel and iTerm2 images reach the terminal as the program wrote
//! them.
//!
//! ```no_run
//! # fn main() -> std::io::Result<()> {
//! use legume_tui::{Modes, Mouse, Tool};
//! const TOOL: Tool = Tool::new("tool");
//!
//! legume_tui::serve_if_host(); // first thing in main
//! // ...
//! let session = TOOL.session("view", &legume_tui::Options::default())?;
//! let mut terminal = session.terminal_with(Modes {
//!     mouse: Mouse::Buttons,
//!     ..Modes::default()
//! })?;
//! while !session.stop_requested() {
//!     session.draw(&mut terminal, |f| { /* ... */ })?;
//!     session.status("what the program is doing, in a line");
//!     let event = session.poll_with(&mut terminal, std::time::Duration::from_secs(1), || {
//!         // after an attach: ask the terminal again, e.g. with
//!         // legume_tui::graphics::reprobe
//!         Ok(())
//!     })?;
//!     // handle the event, do the work ...
//! #   break;
//! }
//! session.restore();
//! # Ok(())
//! # }
//! ```
//!
//! Unix only.
#![cfg(unix)]

mod chord;
mod client;
pub mod dir;
mod host;
mod mode;
mod picker;
mod scan;
mod screen;
mod stop;
mod wire;

#[cfg(feature = "clap")]
pub mod cli;
#[cfg(feature = "graphics")]
pub mod graphics;
#[cfg(feature = "testing")]
pub mod testing;

pub use dir::{runtime_dir, Listed};
pub use mode::{hosts_by_default, Bypass};
pub use screen::{Modes, Mouse};
pub use wire::Size;

use ratatui::crossterm::event::{self, Event};
use std::io;
use std::os::fd::{BorrowedFd, FromRawFd, OwnedFd, RawFd};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::Duration;

/// Set to `1` (anything but empty or `0`) to run every TUI plain.
pub const NO_DETACH_VAR: &str = "LEGUME_TUI_NO_DETACH";
/// The detach key in place of Ctrl+Alt+D, e.g. `ctrl+alt+k` or `ctrl+\`:
/// Ctrl, Alt or not, and a letter or one of `@ \ ] ^ _`.
pub const DETACH_KEY_VAR: &str = "LEGUME_TUI_DETACH_KEY";
/// The switch key in place of Ctrl+Alt+S, spelled as for
/// [`DETACH_KEY_VAR`].
pub const SWITCH_KEY_VAR: &str = "LEGUME_TUI_SWITCH_KEY";
/// Set by a host in its program's environment, to the session's id. A
/// TUI that finds it set and is not that program runs plain: no host is
/// started inside another.
pub const SESSION_VAR: &str = "LEGUME_TUI_SESSION";
/// Where the sockets live, in place of the default runtime directory.
pub const DIR_VAR: &str = "LEGUME_TUI_DIR";
/// Set in a host process: the name of the session it serves.
pub(crate) const HOST_VAR: &str = "LEGUME_TUI_HOST";
/// The launching terminal's size, for the host to start the pty with.
pub(crate) const SIZE_VAR: &str = "LEGUME_TUI_SIZE";
/// The file descriptor of the program's control channel.
pub(crate) const CONTROL_VAR: &str = "LEGUME_TUI_CONTROL";
/// The host's process id: only its own child takes the control channel.
pub(crate) const PARENT_VAR: &str = "LEGUME_TUI_HOST_PID";
/// The file descriptor of the listening socket the launching client
/// bound for its host.
pub(crate) const LISTEN_VAR: &str = "LEGUME_TUI_LISTEN";

/// How long a launching client waits for its host's socket.
const LAUNCH: Duration = Duration::from_secs(5);
/// The shortest wait of [`Session::poll`] while detached.
const DETACHED_POLL: Duration = Duration::from_secs(1);

/// How a TUI wants its session.
#[derive(Clone, Debug, Default)]
pub struct Options {
    /// Run plain, in this terminal, as before (`--no-detach`).
    pub no_detach: bool,
    /// Ask for hosting where it is not the default (see
    /// [`hosts_by_default`]).
    pub detach: bool,
    /// The session's label, unique among the tool's live sessions; its id
    /// is the tool, the screen and the label, in place of the next free
    /// number. A new session always starts, without the resume picker,
    /// and if it cannot be hosted [`Tool::session`] fails rather than run
    /// it plain.
    pub name: Option<String>,
    /// The user named what to open (a file, a run): start a new session
    /// without the resume picker; a one-line hint names the detached
    /// sessions instead.
    pub target: bool,
}

/// The detach key as a help screen shows it: `Ctrl+Alt+D`, or what
/// [`DETACH_KEY_VAR`] sets.
pub fn detach_key() -> String {
    chord::Key::detach().display()
}

/// The switch key as a help screen shows it: `Ctrl+Alt+S`, or what
/// [`SWITCH_KEY_VAR`] sets.
pub fn switch_key() -> String {
    chord::Key::switch().display()
}

/// A session of this process offers the detach and switch keys
/// ([`Session::offers_keys`]).
static KEYS_OFFERED: AtomicBool = AtomicBool::new(false);

/// The detach and switch keys, each with what it does, for a help screen
/// to list: none until a session of this process offers them
/// ([`Session::offers_keys`]).
pub fn help_keys() -> Vec<(String, String)> {
    if !KEYS_OFFERED.load(SeqCst) {
        return Vec::new();
    }
    vec![
        (
            detach_key(),
            format!(
                "detach: leave it running (named on the first detach); \
                 `{} attach` (or starting it again) returns",
                exe_name()
            ),
        ),
        (
            switch_key(),
            "switch: leave it running and pick another session".to_string(),
        ),
    ]
}

/// The running executable's file name, as a user types it.
pub(crate) fn exe_name() -> String {
    std::env::args_os()
        .next()
        .map(PathBuf::from)
        .and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned()))
        .unwrap_or_else(|| "<tool>".into())
}

/// If this process was started as a session's host, serve that session
/// and exit; otherwise return at once. Call it first thing in `main`, so a
/// host does none of the tool's own work before it starts the program.
pub fn serve_if_host() {
    if let Some(name) = std::env::var_os(HOST_VAR) {
        host::serve_session(&name.to_string_lossy());
    }
}

/// A tool whose TUIs run in sessions: its sessions are those whose ids
/// start with its name and `-` (`tool-view-1`), and only those are
/// offered, listed and found for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tool {
    name: &'static str,
}

impl Tool {
    pub const fn new(name: &'static str) -> Self {
        Self { name }
    }

    /// The tool's name, as its sessions' ids start.
    pub fn name(&self) -> &'static str {
        self.name
    }

    /// What every id of the tool's sessions starts with.
    pub fn prefix(&self) -> String {
        format!("{}-", self.name)
    }

    /// Start the session of the tool's TUI `screen`, named
    /// `tool-screen-N` (`tool-screen-<name>` with [`Options::name`]).
    ///
    /// Where the TUI is hosted (by default, in an interactive terminal
    /// outside tmux and screen), the call in the launching process does
    /// not return. If the tool has live sessions and nothing was named
    /// ([`Options::target`], [`Options::name`]), a picker first offers to
    /// return to one of them. Otherwise it starts the host, which runs
    /// this executable again with the same arguments, relays the terminal
    /// until the user detaches or the program ends, then exits the process
    /// with the program's status. In that program the call returns a
    /// hosted [`Session`]. Elsewhere it returns a plain one and the TUI
    /// runs as before. Call it before any work the program would otherwise
    /// repeat.
    pub fn session(&self, screen: &str, opts: &Options) -> io::Result<Session> {
        serve_if_host();
        let session = match mode::mode(opts, &mode::Env::read()) {
            mode::Mode::Hosted => match control() {
                Some(control) => Session::hosted(control),
                // The channel was not ours after all (closed, or reused).
                None => Session::nested(),
            },
            mode::Mode::Plain(Bypass::Nested) => Session::nested(),
            mode::Mode::Plain(why) => Session::plain(why),
            mode::Mode::Launch => {
                let launched = dir::runtime_dir().and_then(|dir| {
                    let live = dir::live(&dir, &self.prefix()).unwrap_or_default();
                    if let Some(code) = self.resume(&dir, opts, &live) {
                        std::process::exit(code);
                    }
                    self.launch(&dir, screen, opts, &live)
                });
                match launched {
                    Ok(code) => std::process::exit(code),
                    // Asked for by name: not one to run plain in its place.
                    Err(e) if opts.name.is_some() => return Err(e),
                    Err(e) => {
                        eprintln!("legume-tui: running without a session ({e})");
                        Session::plain(Bypass::NoHost)
                    }
                }
            }
        };
        if session.offers_keys() {
            KEYS_OFFERED.store(true, SeqCst);
        }
        Ok(session)
    }

    /// Before a launch: with nothing named and `live` sessions of the
    /// tool, the picker; a status to exit with if the user returned to
    /// one, or chose neither. With something named, a hint naming the
    /// detached sessions.
    fn resume(&self, dir: &Path, opts: &Options, live: &[Listed]) -> Option<i32> {
        if live.is_empty() {
            return None;
        }
        if opts.target || opts.name.is_some() {
            let detached: Vec<_> = live.iter().filter(|l| !l.attached).collect();
            if !detached.is_empty() {
                eprintln!(
                    "[detached: {}; `{} attach NAME` to return]",
                    dir::names(detached),
                    exe_name()
                );
            }
            return None;
        }
        match picker::choose_or_say(dir, self, true, None, live.to_vec())? {
            picker::Choice::Attach(name) => {
                let attached = client::attach_and_report(dir, self, &name);
                Some(attached.unwrap_or_else(|e| {
                    eprintln!("legume-tui: cannot attach to {name} ({e})");
                    1
                }))
            }
            picker::Choice::Quit => Some(0),
            picker::Choice::New => None,
        }
    }

    /// Start a host for a new session of `screen` in `dir` and be its
    /// first client; returns the status to exit with. `live` are the
    /// tool's sessions as listed just now. Fails, with no host and nothing
    /// of the session left behind, if the host cannot start.
    fn launch(&self, dir: &Path, screen: &str, opts: &Options, live: &[Listed]) -> io::Result<i32> {
        use std::os::fd::AsRawFd;
        // Bound here, so the name is ours and so is the host we reach on it.
        let base = format!("{}-{screen}", self.name);
        let (name, listener) = dir::claim(dir, &self.prefix(), &base, opts.name.as_deref(), live)?;
        let mut claim = Claim {
            dir,
            name,
            listener: Some(listener),
            up: false,
        };
        let name = claim.name.clone();
        let path = dir::socket(dir, &name);
        let size = Size::of(io::stdout()).unwrap_or_default();
        let log = dir::log_file(dir, &name);
        let listen = claim.listener.as_ref().map_or(-1, |l| l.as_raw_fd());
        let mut host = std::process::Command::new(std::env::current_exe()?);
        host.args(std::env::args_os().skip(1))
            .env(HOST_VAR, &name)
            .env(SIZE_VAR, size.to_env())
            .env(LISTEN_VAR, listen.to_string())
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::fs::File::create(&log)?);
        // In the child, between fork and exec: only system calls.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut host, move || {
                // Off the user's session before it runs a line of its own: a
                // ctrl+c or a hangup there while it starts cannot reach it.
                rustix::process::setsid()?;
                rustix::io::fcntl_setfd(
                    BorrowedFd::borrow_raw(listen),
                    rustix::io::FdFlags::empty(),
                )?;
                Ok(())
            });
        }
        let spawned = host.spawn();
        // The host has its own copy now.
        claim.listener = None;
        let mut host = spawned?;
        // Up once it answers: its program is started and it serves clients.
        let start = std::time::Instant::now();
        loop {
            if dir::ask(&path).is_some() {
                break;
            }
            let ended = host.try_wait()?.is_some();
            if ended || start.elapsed() > LAUNCH {
                if !ended {
                    // Its program goes with it: the pty closes, it hangs up.
                    let _ = host.kill();
                    let _ = host.wait();
                }
                // Read before the claim takes it away.
                let said = std::fs::read_to_string(&log).unwrap_or_default();
                let said = said.trim();
                return Err(io::Error::other(if said.is_empty() {
                    "the session host did not start".to_string()
                } else {
                    said.to_string()
                }));
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        // The host outlives us; it is not ours to wait for, and what it
        // left is its own to take away.
        drop(host);
        claim.up = true;
        // From here the program runs: a failure must not start a second
        // copy plain, so it is reported and left to attach to.
        match client::attach_and_report(dir, self, &name) {
            Ok(code) => Ok(code),
            Err(e) => {
                eprintln!("legume-tui: cannot relay {name} ({e}); it runs on, detached");
                Ok(1)
            }
        }
    }
}

/// A session claimed for a launch: until its host is up, a failure takes
/// away all it left (socket, label, log).
struct Claim<'a> {
    dir: &'a Path,
    name: String,
    /// Ours until the host has its own copy.
    listener: Option<std::os::unix::net::UnixListener>,
    up: bool,
}

impl Drop for Claim<'_> {
    fn drop(&mut self) {
        if !self.up {
            // Closed first, so nothing listens on the socket.
            self.listener = None;
            let _ = dir::clear(self.dir, &self.name);
        }
    }
}

/// What the host says about attaching, as this process reads it: the
/// control channel of the program the host started, or a watch on the
/// host by a TUI that program started on the same terminal. Shared by
/// every session the process opens.
struct Control {
    fd: OwnedFd,
    attached: AtomicBool,
    repaint: AtomicBool,
}

impl Control {
    fn new(fd: OwnedFd) -> Arc<Self> {
        Arc::new(Self {
            fd,
            attached: AtomicBool::new(false),
            repaint: AtomicBool::new(false),
        })
    }

    /// Read what the host said since last time.
    fn drain(&self) {
        let mut buf = [0u8; 64];
        loop {
            match rustix::io::read(&self.fd, &mut buf) {
                Ok(0) => {
                    // The host is gone: nobody will attach again.
                    self.attached.store(false, SeqCst);
                    return;
                }
                Ok(n) => self.heard(&buf[..n]),
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return,
            }
        }
    }

    fn heard(&self, said: &[u8]) {
        for &b in said {
            match b {
                host::ATTACHED => {
                    self.attached.store(true, SeqCst);
                    self.repaint.store(true, SeqCst);
                }
                host::DETACHED => self.attached.store(false, SeqCst),
                _ => {}
            }
        }
    }
}

/// A file descriptor handed down to this process, named in environment
/// variable `var`, if it is open here and of type `kind`: ours from here,
/// closed with the process and not passed on to what it starts.
pub(crate) fn inherited_fd(var: &str, kind: rustix::fs::FileType) -> Option<OwnedFd> {
    let fd: RawFd = std::env::var(var).ok()?.parse().ok()?;
    if fd < 0 {
        return None;
    }
    let borrowed = unsafe { BorrowedFd::borrow_raw(fd) };
    let st = rustix::fs::fstat(borrowed).ok()?;
    if rustix::fs::FileType::from_raw_mode(st.st_mode) != kind {
        return None;
    }
    let fd = unsafe { OwnedFd::from_raw_fd(fd) };
    rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC).ok()?;
    Some(fd)
}

/// The control channel named in the environment, taken once per process;
/// None if it is not a pipe open here.
fn control() -> Option<Arc<Control>> {
    static CONTROL: OnceLock<Option<Arc<Control>>> = OnceLock::new();
    CONTROL
        .get_or_init(|| {
            let fd = inherited_fd(CONTROL_VAR, rustix::fs::FileType::Fifo)?;
            rustix::io::ioctl_fionbio(&fd, true).ok()?;
            Some(Control::new(fd))
        })
        .clone()
}

/// A watch on the host of the session named in the environment, for a
/// TUI a hosted program started on the same terminal; taken once per
/// process. None if there is no such host, or our terminal is not its.
fn watch() -> Option<Arc<Control>> {
    static WATCH: OnceLock<Option<Arc<Control>>> = OnceLock::new();
    WATCH
        .get_or_init(|| {
            if !rustix::termios::isatty(io::stdout()) {
                return None;
            }
            let name = std::env::var(SESSION_VAR).ok()?;
            dir::check_name(&name).ok()?;
            let path = dir::socket(&dir::runtime_dir().ok()?, &name);
            let sock = std::os::unix::net::UnixStream::connect(path).ok()?;
            wire::write_frame(&sock, wire::WATCH, &[]).ok()?;
            sock.set_nonblocking(true).ok()?;
            Some(Control::new(sock.into()))
        })
        .clone()
}

fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// A TUI's session: hosted (detachable) or plain. It stands where
/// `ratatui::init` and `ratatui::restore` stood, and runs each round of
/// the program's loop ([`Session::poll`], [`Session::draw`]).
pub struct Session {
    /// The host's word on attaching: hosted, or nested in a host.
    control: Option<Arc<Control>>,
    bypass: Option<Bypass>,
    /// A repaint is owed: the first in a plain session, and one after a
    /// child had the terminal.
    owed: AtomicBool,
    /// [`Session::terminal`] set the terminal up and it is not restored.
    on: AtomicBool,
    /// `kill` asks the program to stop while the screen is up.
    armed: Mutex<Option<stop::Armed>>,
    /// The modes the screen turns on.
    modes: Mutex<Modes>,
    /// The session's directory and id, in a session's host or nested in
    /// one: where [`Session::status`] writes.
    at: Option<(PathBuf, String)>,
    /// What [`Session::status`] said last.
    said: Mutex<String>,
}

impl Session {
    fn new(control: Option<Arc<Control>>, bypass: Option<Bypass>) -> Self {
        let at = control.as_ref().and_then(|_| session_at());
        Self {
            // Plain, the screen starts with one.
            owed: AtomicBool::new(control.is_none()),
            control,
            bypass,
            on: AtomicBool::new(false),
            armed: Mutex::new(None),
            modes: Mutex::new(Modes::default()),
            at,
            said: Mutex::new(String::new()),
        }
    }

    fn hosted(control: Arc<Control>) -> Self {
        Self::new(Some(control), None)
    }

    fn plain(why: Bypass) -> Self {
        Self::new(None, Some(why))
    }

    /// Plain, inside another TUI's session: repaints after every attach
    /// as the hosted program does, when the host can be reached.
    fn nested() -> Self {
        Self::new(watch(), Some(Bypass::Nested))
    }

    /// The session's id; None when plain.
    pub fn name(&self) -> Option<String> {
        self.is_hosted()
            .then(|| self.at.as_ref().map(|(_, id)| id.clone()))
            .flatten()
    }

    /// Whether the program runs in a host.
    pub fn is_hosted(&self) -> bool {
        self.bypass.is_none()
    }

    /// Say what the program is doing, in one short line, for the picker
    /// and `<tool> sessions` to show. Call it every round: the same line
    /// again is not written. It does nothing outside a session.
    pub fn status(&self, text: &str) {
        let Some((dir, id)) = &self.at else { return };
        let mut said = lock(&self.said);
        if *said != text {
            let _ = dir::write_status(dir, id, text);
            *said = text.to_string();
        }
    }

    /// Why the session is plain; None when hosted.
    pub fn bypass(&self) -> Option<Bypass> {
        self.bypass
    }

    /// Whether the detach and switch keys work here ([`detach_key`],
    /// [`switch_key`], [`help_keys`]): hosted, or started inside a session
    /// whose host it can follow.
    pub fn offers_keys(&self) -> bool {
        self.control.is_some()
    }

    /// Set `cmd` up to run as this session does, for a program the tool
    /// starts and waits for itself ([`Session::run_child`] does this): the
    /// user's opt-out ([`NO_DETACH_VAR`]) when this session runs plain by
    /// `--no-detach` or by that variable. Inside a session,
    /// [`SESSION_VAR`] is passed on already.
    pub fn command<'a>(&self, cmd: &'a mut Command) -> &'a mut Command {
        if matches!(self.bypass, Some(Bypass::Flag | Bypass::Env)) {
            cmd.env(NO_DETACH_VAR, "1");
        }
        cmd
    }

    /// The terminal, raw, in the alternate screen, with a panic hook that
    /// restores it (as `ratatui::init`). From here, in a session, `kill`
    /// asks the program to stop ([`Session::stop_requested`]).
    pub fn terminal(&self) -> io::Result<ratatui::DefaultTerminal> {
        self.terminal_with(Modes::default())
    }

    /// [`Session::terminal`], with `modes` on: the session turns them on
    /// again after every attach and off when the terminal is handed back
    /// or on a panic.
    pub fn terminal_with(&self, modes: Modes) -> io::Result<ratatui::DefaultTerminal> {
        *lock(&self.modes) = modes;
        let t = ratatui::try_init()?;
        screen::off_on_panic();
        if !self.on.swap(true, SeqCst) && self.control.is_some() {
            *lock(&self.armed) = stop::Armed::new();
        }
        screen::turn_on(modes)?;
        // Crossterm's event source made now, with its resize handler, so
        // a stop before the first poll wakes that poll.
        let _ = event::poll(Duration::ZERO);
        Ok(t)
    }

    /// Change the modes the screen has on.
    pub fn set_modes(&self, modes: Modes) -> io::Result<()> {
        *lock(&self.modes) = modes;
        if self.on.load(SeqCst) {
            screen::turn_off()?;
            screen::turn_on(modes)?;
        }
        Ok(())
    }

    /// Hand the terminal back (as `ratatui::restore`), its modes off; also
    /// done on drop. From here `kill` ends the program as by default, but
    /// a stop that came before stays asked
    /// ([`Session::stop_requested`]).
    pub fn restore(&self) {
        if self.on.swap(false, SeqCst) {
            let _ = screen::turn_off();
            ratatui::restore();
            *lock(&self.armed) = None;
        }
    }

    /// Whether to repaint everything now and probe the terminal again:
    /// true once after each attach in a hosted session (or one started
    /// inside it), on the first call in a plain one, and after a child had
    /// the terminal ([`Session::run_child`]). Cached graphics (an image
    /// picker and the protocols made with it) must be rebuilt, since the
    /// terminal may not be the one they were made for. [`Session::poll`]
    /// asks it.
    pub fn take_repaint(&self) -> bool {
        match &self.control {
            Some(c) => {
                c.drain();
                let asked = c.repaint.swap(false, SeqCst);
                // Owed, it waits for a terminal to be there.
                asked | (c.attached.load(SeqCst) && self.owed.swap(false, SeqCst))
            }
            None => self.owed.swap(false, SeqCst),
        }
    }

    /// Whether a terminal is there to draw on. While detached, skip
    /// drawing (output is dropped) and do not read the terminal's size;
    /// keep the program's own work going at a slow poll. Always true when
    /// plain, but for a TUI started inside a session.
    pub fn attached(&self) -> bool {
        match &self.control {
            Some(c) => {
                c.drain();
                c.attached.load(SeqCst)
            }
            None => true,
        }
    }

    /// Whether `kill` asked the program to stop (it arrives as SIGTERM):
    /// stop as the quit key does, wrapping up running work. A second
    /// `kill` ends the program at once. Asked while the screen is up
    /// ([`Session::terminal`] to [`Session::restore`]) and while a child
    /// runs ([`Session::run_child`]); elsewhere `kill` ends the program at
    /// once. Always false when plain, but for a TUI started inside a
    /// session.
    pub fn stop_requested(&self) -> bool {
        self.control.is_some() && stop::heard()
    }

    /// Run `cmd` on this terminal ([`Session::command`] sets it up) and
    /// wait for it, as [`std::process::Child::wait`] does, at any point of
    /// the program. While it runs, `kill` reaches only this program, so a
    /// stop is passed on to the child as SIGTERM, which a TUI takes
    /// through its own stop path ([`Session::stop_requested`]). With the
    /// screen up, the child (a TUI of its own) is handed the terminal and
    /// it is taken back after, to be repainted whole; a stop that came
    /// meanwhile stays asked. Before the screen is up, or after it, a stop
    /// that came ends the program once the child is gone, as `kill` does
    /// there.
    pub fn run_child(&self, cmd: &mut Command) -> io::Result<ExitStatus> {
        use ratatui::crossterm::{execute, terminal};
        self.command(cmd);
        let screen = self.on.load(SeqCst);
        if screen {
            let _ = screen::turn_off();
            ratatui::restore();
        }
        let armed = self.control.as_ref().and_then(|_| stop::Armed::new());
        let heard = stop::heard();
        let waited = cmd
            .spawn()
            .and_then(|child| stop::wait(child, armed.is_some()));
        if screen {
            terminal::enable_raw_mode()?;
            execute!(io::stdout(), terminal::EnterAlternateScreen)?;
            self.owed.store(true, SeqCst);
        }
        let came = armed.is_some() && !heard && stop::heard();
        drop(armed);
        if came && !screen {
            stop::end_as_by_default();
        }
        waited
    }

    /// Bring the screen back after an attach: into the alternate screen,
    /// cleared, so the next draw paints all of it, and the session's modes
    /// on again ([`Session::terminal_with`]).
    pub fn repaint(&self, terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
        self.repaint_with(terminal, || Ok(()))
    }

    /// [`Session::repaint`], with `probe` (asking the terminal anything)
    /// done before the modes are on: a mouse report would land amid the
    /// terminal's answer.
    fn repaint_with(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        probe: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<()> {
        use ratatui::crossterm::{execute, terminal::EnterAlternateScreen};
        execute!(io::stdout(), EnterAlternateScreen)?;
        // A resize to the whole screen clears it and forgets what was
        // drawn, without the cursor query `clear` makes (a round trip to a
        // terminal that may have just left again).
        let size = terminal.size()?;
        terminal.resize(ratatui::layout::Rect::new(0, 0, size.width, size.height))?;
        probe()?;
        screen::turn_on(*lock(&self.modes))
    }

    /// One round's wait: [`Session::poll_with`], nothing asked of the
    /// terminal after an attach.
    pub fn poll(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        wanted: Duration,
    ) -> io::Result<Option<Event>> {
        self.poll_with(terminal, wanted, || Ok(()))
    }

    /// One round's wait: the next event, if one comes within `wanted`
    /// (at least a second while detached). A repaint owed
    /// ([`Session::take_repaint`]) is done first, `on_repaint` called in
    /// it to ask the terminal again (before the modes are on: see
    /// `graphics::reprobe`), and then it returns at once, for the
    /// screen to be drawn whole. A `kill` ends the wait.
    pub fn poll_with(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        wanted: Duration,
        on_repaint: impl FnOnce() -> io::Result<()>,
    ) -> io::Result<Option<Event>> {
        if self.take_repaint() {
            self.repaint_with(terminal, on_repaint)?;
            return Ok(None);
        }
        if self.stop_requested() {
            return Ok(None);
        }
        let wait = if self.attached() {
            wanted
        } else {
            wanted.max(DETACHED_POLL)
        };
        if !event::poll(wait)? {
            return Ok(None);
        }
        let event = event::read()?;
        // An attach woke the wait: the screen repainted before the event
        // is taken and drawn.
        if self.take_repaint() {
            self.repaint_with(terminal, on_repaint)?;
        }
        Ok(Some(event))
    }

    /// Draw a frame, as `terminal.draw` does, while a terminal is there to
    /// draw on ([`Session::attached`]); nothing while detached.
    pub fn draw(
        &self,
        terminal: &mut ratatui::DefaultTerminal,
        render: impl FnOnce(&mut ratatui::Frame),
    ) -> io::Result<()> {
        if self.attached() {
            terminal.draw(render)?;
        }
        Ok(())
    }
}

/// The directory and id of the session named in the environment.
fn session_at() -> Option<(PathBuf, String)> {
    let name = std::env::var(SESSION_VAR).ok()?;
    dir::check_name(&name).ok()?;
    Some((dir::runtime_dir().ok()?, name))
}

impl Drop for Session {
    fn drop(&mut self) {
        self.restore();
    }
}

#[cfg(test)]
#[path = "tests/session.rs"]
mod tests;
