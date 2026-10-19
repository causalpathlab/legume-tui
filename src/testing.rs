//! A harness for a tool's tests: its TUI driven end to end through
//! pseudo-terminals standing in for the user's, with the sessions in a
//! private directory of the test's own. The terminal answers what the
//! program asks of it as a test says ([`Replies`]), and keeps a model of
//! what it shows ([`Term::screen`]).
//!
//! ```no_run
//! use legume_tui::testing::{Private, Replies, Setup};
//! // In a tool's integration test: env!("CARGO_BIN_EXE_tool").
//! let dir = Private::new("target/debug/tool");
//! let mut t = dir.term(&["view"], &[]);
//! t.wait_screen("a line the screen draws");
//! t.type_(b"\x1b\x04");
//! t.answer(1, b"\r");
//! assert_eq!(t.exit_code(), 0);
//! dir.wait_listed("detached");
//! // A terminal with kitty's pictures and keyboard, in a folder of its own.
//! let setup = Setup::new()
//!     .cwd("target")
//!     .replies(Replies::plain().kitty().keyboard(0).cell(10, 20));
//! let t = dir.term_with(&["view"], &[], setup);
//! ```

use crate::Size;
use std::fs::File;
use std::io::{Read, Write};
use std::os::fd::{BorrowedFd, OwnedFd};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

/// How long a test waits for what it expects, unless told otherwise
/// ([`Private::patience`]).
pub const WAIT: Duration = Duration::from_secs(15);

/// What a detach by the key asks of an unnamed session.
pub const PROMPT: &str = "Name it (Enter to keep): ";

/// The size of a terminal [`Private::term`] makes.
pub const SIZE: Size = Size {
    rows: 24,
    cols: 80,
    width: 800,
    height: 480,
};

/// A private directory for the sessions of the tool at `exe`, and the
/// tool run with it; a session a failed test left running is killed with
/// it (`<tool> kill --force`).
pub struct Private {
    dir: tempfile::TempDir,
    exe: PathBuf,
    wait: Duration,
}

impl Private {
    pub fn new(exe: impl Into<PathBuf>) -> Self {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().expect("a temporary directory");
        std::fs::set_permissions(dir.path(), std::fs::Permissions::from_mode(0o700))
            .expect("the directory made private");
        Self {
            dir,
            exe: exe.into(),
            wait: WAIT,
        }
    }

    /// Wait `wait` for what is expected, in place of [`WAIT`].
    pub fn patience(mut self, wait: Duration) -> Self {
        self.wait = wait;
        self
    }

    pub fn path(&self) -> &Path {
        self.dir.path()
    }

    /// The tool, its sessions here, in a clean environment: no session,
    /// multiplexer or opt-out around it, and no backtrace setting from the
    /// test runner (CI's `RUST_BACKTRACE=1` would have an error the tool
    /// ends with followed by a trace longer than the screen, so what a
    /// test looks for on the screen depended on where it ran). A test
    /// that wants one passes it in `env`.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.env(crate::DIR_VAR, self.path())
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .env_remove("STY")
            .env_remove("RUST_BACKTRACE")
            .env_remove("RUST_LIB_BACKTRACE")
            .env_remove(crate::SESSION_VAR)
            .env_remove(crate::NO_DETACH_VAR);
        cmd
    }

    /// The tool with `args` and `env` on a new terminal of [`SIZE`].
    pub fn term(&self, args: &[&str], env: &[(&str, &str)]) -> Term {
        self.term_sized(args, env, SIZE)
    }

    /// The tool with `args` and `env` on a new terminal of `size`.
    pub fn term_sized(&self, args: &[&str], env: &[(&str, &str)], size: Size) -> Term {
        self.term_with(args, env, Setup::new().size(size))
    }

    /// The tool with `args` and `env` on a new terminal set up as `setup`
    /// says: its size, the tool's working directory, what it answers.
    pub fn term_with(&self, args: &[&str], env: &[(&str, &str)], setup: Setup) -> Term {
        let (master, slave) = crate::host::pty(setup.size).expect("a pseudo-terminal");
        let mut cmd = self.command();
        if let Some(cwd) = &setup.cwd {
            cmd.current_dir(cwd);
        }
        cmd.args(args)
            .stdin(Stdio::from(slave.try_clone().unwrap()))
            .stdout(Stdio::from(slave.try_clone().unwrap()))
            .stderr(Stdio::from(slave.try_clone().unwrap()));
        for (k, v) in env {
            cmd.env(k, v);
        }
        // In the child, between fork and exec: the pty becomes its terminal.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut cmd, || {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                Ok(())
            });
        }
        let child = cmd.spawn().expect("the tool started");
        let seen = Arc::new(Mutex::new(Vec::new()));
        let size = setup.size.or_default();
        let screen = Arc::new(Mutex::new(vt100::Parser::new(size.rows, size.cols, 0)));
        {
            let mut master = master.try_clone().unwrap();
            let seen = Arc::clone(&seen);
            let screen = Arc::clone(&screen);
            let replies = setup.replies;
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                // What is not yet known to be a query or not.
                let mut unread = Vec::new();
                while let Ok(n) = master.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    seen.lock().unwrap().extend_from_slice(&buf[..n]);
                    screen.lock().unwrap().process(&buf[..n]);
                    unread.extend_from_slice(&buf[..n]);
                    let (reply, read) = replies.to(&unread);
                    unread.drain(..read);
                    if !reply.is_empty() {
                        let _ = master.write_all(&reply);
                    }
                }
            });
        }
        Term {
            master,
            _slave: slave,
            seen,
            screen,
            child,
            wait: self.wait,
        }
    }

    /// Run the tool with `args`, no terminal: whether it succeeded, and
    /// all it printed (stdout, then stderr).
    pub fn run_all(&self, args: &[&str]) -> (bool, String) {
        let out = self.command().args(args).output().expect("the tool ran");
        let mut said = String::from_utf8_lossy(&out.stdout).into_owned();
        said.push_str(&String::from_utf8_lossy(&out.stderr));
        (out.status.success(), said)
    }

    /// All the tool printed when run with `args`, no terminal.
    pub fn run(&self, args: &[&str]) -> String {
        self.run_all(args).1
    }

    /// Wait until `<tool> sessions` says `what`; what it said.
    pub fn wait_listed(&self, what: &str) -> String {
        let start = Instant::now();
        loop {
            let listed = self.run(&["sessions"]);
            if listed.contains(what) {
                return listed;
            }
            assert!(
                start.elapsed() < self.wait,
                "never listed {what:?}: {listed:?}"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// The program's pid, from `<tool> sessions` (the first one listed).
    pub fn program_pid(&self) -> u32 {
        let listed = self.run(&["sessions"]);
        listed
            .split_whitespace()
            .skip_while(|w| *w != "pid")
            .nth(1)
            .and_then(|p| p.parse().ok())
            .unwrap_or_else(|| panic!("no pid in {listed:?}"))
    }

    /// The ids of the sessions whose sockets are here.
    pub fn sockets(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.path()) else {
            return Vec::new();
        };
        entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().to_string_lossy().into_owned();
                name.strip_suffix(".sock").map(str::to_string)
            })
            .collect()
    }

    /// Whether nothing is left here: no socket, status, label or log.
    pub fn is_empty(&self) -> bool {
        std::fs::read_dir(self.path()).map_or(true, |mut d| d.next().is_none())
    }

    /// Wait until process `pid` is gone.
    pub fn gone(&self, pid: u32) {
        let start = Instant::now();
        while alive(pid) {
            assert!(start.elapsed() < self.wait, "{pid} is still running");
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}

impl Drop for Private {
    fn drop(&mut self) {
        for id in self.sockets() {
            self.run(&["kill", &id, "--force"]);
        }
    }
}

/// Whether process `pid` is alive.
pub fn alive(pid: u32) -> bool {
    rustix::process::Pid::from_raw(pid as i32)
        .is_some_and(|p| rustix::process::test_kill_process(p).is_ok())
}

/// How [`Private::term_with`] sets a terminal up.
#[derive(Clone, Debug)]
pub struct Setup {
    size: Size,
    cwd: Option<PathBuf>,
    replies: Replies,
}

impl Default for Setup {
    fn default() -> Self {
        Self {
            size: SIZE,
            cwd: None,
            replies: Replies::plain(),
        }
    }
}

impl Setup {
    /// A terminal of [`SIZE`] that answers as [`Replies::plain`], the tool
    /// started in the test's working directory.
    pub fn new() -> Self {
        Self::default()
    }

    pub fn size(self, size: Size) -> Self {
        Self { size, ..self }
    }

    /// Start the tool in `cwd`.
    pub fn cwd(self, cwd: impl Into<PathBuf>) -> Self {
        Self {
            cwd: Some(cwd.into()),
            ..self
        }
    }

    pub fn replies(self, replies: Replies) -> Self {
        Self { replies, ..self }
    }
}

/// What the terminal answers when the program asks it (the graphics
/// query, the keyboard check): the device attributes (`CSI c`), the
/// status report (`CSI 5 n`), and, as set, kitty's graphics query, its
/// keyboard query (`CSI ? u`), the cell size (`CSI 16 t`) and the
/// background colour (OSC 11). Queries wrapped for tmux are not answered.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Replies {
    /// Whether it answers at all.
    answers: bool,
    kitty: bool,
    sixel: bool,
    keyboard: Option<u8>,
    cell: Option<(u16, u16)>,
    background: Option<(u8, u8, u8)>,
}

impl Default for Replies {
    fn default() -> Self {
        Self::plain()
    }
}

impl Replies {
    /// What every terminal answers, and no more: a query ends at once and
    /// finds nothing.
    pub fn plain() -> Self {
        Self {
            answers: true,
            kitty: false,
            sixel: false,
            keyboard: None,
            cell: None,
            background: None,
        }
    }

    /// Nothing: a query waits out its time.
    pub fn silent() -> Self {
        Self {
            answers: false,
            ..Self::plain()
        }
    }

    /// Kitty's graphics protocol.
    pub fn kitty(self) -> Self {
        Self {
            kitty: true,
            ..self
        }
    }

    /// Sixel, in the device attributes.
    pub fn sixel(self) -> Self {
        Self {
            sixel: true,
            ..self
        }
    }

    /// Kitty's keyboard enhancement, with these flags on now.
    pub fn keyboard(self, flags: u8) -> Self {
        Self {
            keyboard: Some(flags),
            ..self
        }
    }

    /// A cell of `width` by `height` pixels.
    pub fn cell(self, width: u16, height: u16) -> Self {
        Self {
            cell: Some((width, height)),
            ..self
        }
    }

    /// The background colour.
    pub fn background(self, red: u8, green: u8, blue: u8) -> Self {
        Self {
            background: Some((red, green, blue)),
            ..self
        }
    }

    /// The answers to the queries in `out`, what the program wrote, and
    /// how much of it is read: a query not yet whole is left for more.
    fn to(&self, out: &[u8]) -> (Vec<u8>, usize) {
        let mut reply = Vec::new();
        if !self.answers {
            return (reply, out.len());
        }
        let mut at = 0;
        while at < out.len() {
            let rest = &out[at..];
            if rest[0] != 0x1b {
                at += 1;
                continue;
            }
            match self.answer(rest) {
                Asked::Query(len, answer) => {
                    reply.extend_from_slice(answer.as_bytes());
                    at += len;
                }
                Asked::Partial => break,
                Asked::Not => at += 1,
            }
        }
        (reply, at)
    }

    /// The answer to the query `rest` starts with, if it starts with one.
    fn answer(&self, rest: &[u8]) -> Asked {
        let fixed: [(&[u8], Option<String>); 5] = [
            (b"\x1b[c", Some(self.attributes())),
            (b"\x1b[5n", Some("\x1b[0n".into())),
            (b"\x1b[?u", self.keyboard.map(|f| format!("\x1b[?{f}u"))),
            (
                b"\x1b[16t",
                self.cell.map(|(w, h)| format!("\x1b[6;{h};{w}t")),
            ),
            (
                b"\x1b]11;?\x07",
                self.background.map(|(r, g, b)| {
                    format!("\x1b]11;rgb:{r:02x}{r:02x}/{g:02x}{g:02x}/{b:02x}{b:02x}\x07")
                }),
            ),
        ];
        for (query, answer) in fixed {
            if rest.starts_with(query) {
                return Asked::Query(query.len(), answer.unwrap_or_default());
            }
            if query.starts_with(rest) {
                return Asked::Partial;
            }
        }
        // A passthrough (tmux's `\ePtmux;...\e\\`), the escapes in it
        // doubled: skipped whole, nothing in it answered.
        if let Some(body) = rest.strip_prefix(b"\x1bP") {
            let mut i = 0;
            while i < body.len() {
                match (body[i], body.get(i + 1)) {
                    (0x1b, Some(0x1b)) => i += 2,
                    (0x1b, Some(b'\\')) => return Asked::Query(2 + i + 2, String::new()),
                    (0x1b, None) => break,
                    _ => i += 1,
                }
            }
            return Asked::Partial;
        }
        // Kitty's graphics query: an image command that asks (`a=q`).
        if rest.starts_with(b"\x1b_G") || b"\x1b_G".starts_with(rest) {
            let Some(end) = rest.windows(2).position(|w| w == b"\x1b\\") else {
                return Asked::Partial;
            };
            let command = String::from_utf8_lossy(&rest[..end]);
            let asks = command.contains("a=q") && command.contains("i=31");
            let answer = if asks && self.kitty {
                "\x1b_Gi=31;OK\x1b\\".to_string()
            } else {
                String::new()
            };
            return Asked::Query(end + 2, answer);
        }
        Asked::Not
    }

    /// The device attributes: a VT220, with sixel (4) if it has it.
    fn attributes(&self) -> String {
        if self.sixel {
            "\x1b[?62;4;22c".into()
        } else {
            "\x1b[?62;22c".into()
        }
    }
}

/// Whether what the program wrote starts with a query.
enum Asked {
    /// One this long, and its answer (empty: none).
    Query(usize, String),
    /// Maybe, once more of it comes.
    Partial,
    Not,
}

/// A user's terminal: a pty whose other end the tool runs on, and all it
/// has written so far.
pub struct Term {
    /// Our end: what is written to it is typed.
    pub master: File,
    /// The tool's end, held open so that on Linux ours reads on to the
    /// last of what the tool wrote after it ends, with no error between.
    _slave: OwnedFd,
    seen: Arc<Mutex<Vec<u8>>>,
    /// What it shows, as a terminal would.
    screen: Arc<Mutex<vt100::Parser>>,
    /// The tool, as started on it.
    pub child: Child,
    wait: Duration,
}

impl Term {
    /// All the terminal has been sent, as text.
    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.seen.lock().unwrap()).into_owned()
    }

    /// How many bytes the terminal has been sent: a mark for
    /// [`Term::wait_for_after`].
    pub fn seen_len(&self) -> usize {
        self.seen.lock().unwrap().len()
    }

    /// Wait until `needle` appears.
    pub fn wait_for(&self, needle: &str) {
        self.wait_for_after(0, needle);
    }

    /// Wait until `needle` appears after the first `from` bytes.
    pub fn wait_for_after(&self, from: usize, needle: &str) {
        let start = Instant::now();
        while start.elapsed() < self.wait {
            let seen = self.seen.lock().unwrap();
            let text = String::from_utf8_lossy(&seen[from.min(seen.len())..]).into_owned();
            drop(seen);
            if text.contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = self.text();
        let skip = text.chars().count().saturating_sub(600);
        let tail: String = text.chars().skip(skip).collect();
        panic!("never saw {needle:?} after {from}; the last of it: {tail:?}");
    }

    /// The screen as the terminal shows it now: its rows, the spaces at
    /// their ends left out, a line each.
    pub fn screen(&self) -> String {
        let parser = self.screen.lock().unwrap();
        let screen = parser.screen();
        let (_, cols) = screen.size();
        screen
            .rows(0, cols)
            .map(|row| row.trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    /// Whether the terminal shows its alternate screen now.
    pub fn alternate(&self) -> bool {
        self.screen.lock().unwrap().screen().alternate_screen()
    }

    /// Wait until the screen shows `needle` ([`Term::screen`]).
    pub fn wait_screen(&self, needle: &str) {
        let start = Instant::now();
        while start.elapsed() < self.wait {
            if self.screen().contains(needle) {
                return;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("the screen never showed {needle:?}:\n{}", self.screen());
    }

    pub fn type_(&mut self, bytes: &[u8]) {
        self.master.write_all(bytes).unwrap();
    }

    /// Wait for the label prompt to have been asked `n` times, then type
    /// `answer`.
    pub fn answer(&mut self, n: usize, answer: &[u8]) {
        let start = Instant::now();
        while self.text().matches(PROMPT).count() < n {
            assert!(
                start.elapsed() < self.wait,
                "not asked {n} times: {:?}",
                self.text()
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        self.type_(answer);
    }

    /// Send the tool a signal, as `kill` from another shell does.
    pub fn signal(&self, sig: rustix::process::Signal) {
        let pid = rustix::process::Pid::from_child(&self.child);
        rustix::process::kill_process(pid, sig).unwrap();
    }

    /// How the tool ended, waiting for it.
    pub fn status(&mut self) -> ExitStatus {
        let start = Instant::now();
        while start.elapsed() < self.wait {
            if let Some(s) = self.child.try_wait().unwrap() {
                return s;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        panic!("still running; saw {:?}", self.text());
    }

    /// The tool's exit code, waiting for it; -1 for a death by a signal.
    pub fn exit_code(&mut self) -> i32 {
        self.status().code().unwrap_or(-1)
    }

    /// Whether the terminal is back in its line mode. Read through our
    /// end, which shares the mode with the tool's: macOS revokes the
    /// tool's end, our handle on it too, once the tool (its session
    /// leader) has ended.
    pub fn cooked(&self) -> bool {
        let t = rustix::termios::tcgetattr(&self.master).unwrap();
        t.local_modes.contains(rustix::termios::LocalModes::ICANON)
    }
}

#[cfg(test)]
#[path = "tests/testing.rs"]
mod tests;
