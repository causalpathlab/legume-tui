//! A harness for a tool's tests: its TUI driven end to end through
//! pseudo-terminals standing in for the user's, with the sessions in a
//! private directory of the test's own.
//!
//! ```no_run
//! use legume_tui::testing::Private;
//! // In a tool's integration test: env!("CARGO_BIN_EXE_tool").
//! let dir = Private::new("target/debug/tool");
//! let mut t = dir.term(&["view"], &[]);
//! t.wait_for("a line the screen draws");
//! t.type_(b"\x1b\x04");
//! t.answer(1, b"\r");
//! assert_eq!(t.exit_code(), 0);
//! dir.wait_listed("detached");
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
    /// multiplexer or opt-out around it.
    pub fn command(&self) -> Command {
        let mut cmd = Command::new(&self.exe);
        cmd.env(crate::DIR_VAR, self.path())
            .env("TERM", "xterm-256color")
            .env_remove("TMUX")
            .env_remove("STY")
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
        let (master, slave) = crate::host::pty(size).expect("a pseudo-terminal");
        let mut cmd = self.command();
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
        {
            let mut master = master.try_clone().unwrap();
            let seen = Arc::clone(&seen);
            std::thread::spawn(move || {
                let mut buf = [0u8; 4096];
                while let Ok(n) = master.read(&mut buf) {
                    if n == 0 {
                        break;
                    }
                    seen.lock().unwrap().extend_from_slice(&buf[..n]);
                }
            });
        }
        Term {
            master,
            slave,
            seen,
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

/// A user's terminal: a pty whose other end the tool runs on, and all it
/// has written so far.
pub struct Term {
    /// Our end: what is written to it is typed.
    pub master: File,
    slave: OwnedFd,
    seen: Arc<Mutex<Vec<u8>>>,
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

    /// Whether the terminal is back in its line mode.
    pub fn cooked(&self) -> bool {
        let t = rustix::termios::tcgetattr(&self.slave).unwrap();
        t.local_modes.contains(rustix::termios::LocalModes::ICANON)
    }
}
