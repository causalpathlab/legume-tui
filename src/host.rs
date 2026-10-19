//! The host: owns a pseudo-terminal, runs the program on it in a session
//! of its own, and relays bytes between it and at most one attached
//! client. It never changes the bytes, so graphics escapes pass untouched,
//! and keeps no picture of the screen: on attach the program repaints. It
//! does follow where the output stands between escape sequences
//! ([`crate::scan`]), so a new client starts at the start of one (and of
//! a character), told whether the screen there is the alternate one.
//! What the program writes before its first client is ready is kept for
//! that client, so an error it ends with at the start is seen.
//!
//! The program hears from its host through a pipe (see [`crate::Session`]):
//! [`ATTACHED`] once a client is ready, which is the program's cue to
//! repaint and probe the terminal, and [`DETACHED`] when it leaves. What
//! the client said of its terminal's environment is written beside the
//! socket first ([`crate::Session::terminal_env`]). A TUI
//! the program starts on the same terminal hears the same over a
//! [`wire::WATCH`] connection.
//!
//! No lock is held across a write to a client or to the pty: each client
//! has a writer thread with a bounded queue, the pty's input has its own,
//! and the attach state is read from atomics. Typed input a program is
//! not reading is dropped once its queue is full, so the socket is always
//! read and the client is never held up.
//!
//! A [`wire::KILL`] connection is held, as a watcher's, until the host
//! ends: `kill` waits for the session by reading it to its end.

use crate::scan::Scan;
use crate::wire::{self, Size};
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::mpsc::{sync_channel, Receiver, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

/// A client is attached and ready: repaint and probe.
pub(crate) const ATTACHED: u8 = b'A';
/// The client left.
pub(crate) const DETACHED: u8 = b'D';

/// A client that stops taking output this long is let go, so a stalled
/// connection cannot stall the program.
const STALL: Duration = Duration::from_secs(10);

/// How long the program's last words have to reach the client after it
/// ends.
const LAST_WORDS: Duration = Duration::from_secs(1);

/// Frames queued for a client before the program waits for it.
const OUT_QUEUE: usize = 64;
/// Typed chunks queued for a program that is not reading; more are
/// dropped.
const IN_QUEUE: usize = 256;
/// How often a socket a child may still hold is looked at again.
const RETRY: Duration = Duration::from_millis(5);
/// The most output kept for the first client: the latest of it.
const EARLY: usize = 64 << 10;
/// The most read from the pty at once.
const CHUNK: usize = 1 << 16;
/// How long a program that ended before its first client was ready keeps
/// what it said for that client: as long as a launch waits for its host.
const FIRST: Duration = Duration::from_secs(5);

/// The channel to the program.
pub(crate) enum Control {
    /// Make a pipe and hand the program its reading end, named in the
    /// environment ([`crate::CONTROL_VAR`]).
    Pipe,
    /// Write to this instead; the program is told nothing.
    #[cfg(test)]
    Given(OwnedFd),
}

/// A host whose program is running.
pub(crate) struct Host {
    child: Child,
    shared: Arc<Shared>,
    /// Signalled when the pty has nothing more to read.
    drained: std::sync::mpsc::Receiver<()>,
    /// The thread taking connections; it closes the socket as it ends.
    listening: std::thread::JoinHandle<()>,
    /// Where the session's files are, and its id.
    dir: PathBuf,
    id: String,
}

/// The attached client.
struct Client {
    id: u64,
    /// Frames for its writer thread.
    out: Arc<Queue>,
    /// To cut it off at once.
    sock: UnixStream,
    writer: std::thread::JoinHandle<()>,
}

/// Frames for a client's writer thread, whole, at most [`OUT_QUEUE`]
/// waiting. Whoever waits on it is woken when a frame comes or goes, and
/// when it closes, which it does as its client stops being the attached
/// one.
#[derive(Default)]
struct Queue {
    frames: Mutex<Frames>,
    changed: Condvar,
}

#[derive(Default)]
struct Frames {
    waiting: std::collections::VecDeque<Vec<u8>>,
    /// No more frames are taken: the client is let go.
    closed: bool,
}

impl Queue {
    fn lock(&self) -> MutexGuard<'_, Frames> {
        self.frames.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Queue `frame` while `still()` holds, waiting for room; false if the
    /// queue closed, or `still()` stopped holding, first.
    fn push(&self, frame: Vec<u8>, still: impl Fn() -> bool) -> bool {
        let mut f = self.lock();
        loop {
            if f.closed || !still() {
                return false;
            }
            if f.waiting.len() < OUT_QUEUE {
                f.waiting.push_back(frame);
                self.changed.notify_all();
                return true;
            }
            f = self.changed.wait(f).unwrap_or_else(|e| e.into_inner());
        }
    }

    /// Queue `frame` last, however many wait before it, and close.
    fn finish(&self, frame: Vec<u8>) {
        let mut f = self.lock();
        if !f.closed {
            f.waiting.push_back(frame);
            f.closed = true;
        }
        self.changed.notify_all();
    }

    /// Close, dropping what waits.
    fn close(&self) {
        let mut f = self.lock();
        f.closed = true;
        f.waiting.clear();
        self.changed.notify_all();
    }

    /// The next frame, waiting for one; None once closed and empty.
    fn pop(&self) -> Option<Vec<u8>> {
        let mut f = self.lock();
        loop {
            if let Some(frame) = f.waiting.pop_front() {
                self.changed.notify_all();
                return Some(frame);
            }
            if f.closed {
                return None;
            }
            f = self.changed.wait(f).unwrap_or_else(|e| e.into_inner());
        }
    }
}

/// What the relay threads share.
struct Shared {
    client: Mutex<Option<Client>>,
    /// The attached client's number, 0 for none, read without the lock.
    current: Arc<AtomicU64>,
    next: AtomicU64,
    /// Our end of the pty, for its size and its foreground process group.
    pty: File,
    /// Typed bytes for the pty, with the number of the client that typed
    /// them.
    input: SyncSender<(u64, Vec<u8>)>,
    /// The writing end of the control channel, non-blocking.
    control: OwnedFd,
    /// TUIs started by the program that follow the attach state, and
    /// `kill`s waiting for the session to end.
    watchers: Mutex<Vec<UnixStream>>,
    /// What the program wrote before any client was ready, for the first
    /// one; None from then on.
    early: Mutex<Option<Vec<u8>>>,
    /// Signalled when [`Shared::early`] is taken, or a stop is asked for.
    early_gone: Condvar,
    /// The client given [`Shared::early`]: its output goes on from there.
    heir: AtomicU64,
    /// Typed bytes dropped because the program was not reading them.
    dropped: AtomicU64,
    /// Someone asked the program to stop ([`wire::KILL`]).
    stopped: AtomicBool,
    pid: rustix::process::Pid,
    /// When the program started, in seconds since the epoch.
    started: u64,
    /// The program's command line, for listings.
    command: String,
    closing: AtomicBool,
    /// The session's directory and id, where the attached terminal's
    /// environment is written for the program.
    at: (PathBuf, String),
}

impl Shared {
    /// Tell the program, and whoever watches; a full pipe (a program not
    /// listening) loses it.
    fn tell(&self, what: u8) {
        let _ = rustix::io::write(&self.control, &[what]);
        let mut w = self.watchers.lock().unwrap_or_else(|e| e.into_inner());
        w.retain(|s| match wire::send_all(s, &[what]) {
            Ok(()) => true,
            Err(e) => e.kind() == io::ErrorKind::WouldBlock,
        });
    }

    /// Let the current client go, if it is still `id`, and say so.
    fn drop_client(&self, id: u64) {
        let mut c = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if c.as_ref().is_some_and(|c| c.id == id) {
            if let Some(gone) = c.take() {
                self.current.store(0, SeqCst);
                gone.out.close();
                let _ = gone.sock.shutdown(std::net::Shutdown::Both);
            }
            drop(c);
            self.tell(DETACHED);
        }
    }

    /// The pty's size, as the program will see it.
    fn resize(&self, size: Size) {
        let _ = size.or_default().apply(&self.pty);
    }

    /// Wake the event loop of whatever runs in the terminal's foreground
    /// (crossterm reports a resize), and of the program itself, so a
    /// message about attaching is read without waiting for a key. The
    /// foreground may be a TUI the program started.
    fn wake(&self) {
        use rustix::process::{kill_process, kill_process_group, Signal};
        if let Ok(group) = rustix::termios::tcgetpgrp(&self.pty) {
            let _ = kill_process_group(group, Signal::WINCH);
        }
        let _ = kill_process(self.pid, Signal::WINCH);
    }

    /// Queue a frame for client `id` while it stays the client; false if
    /// it is gone.
    fn send_to(&self, id: u64, out: &Queue, frame: Vec<u8>) -> bool {
        out.push(frame, || self.current.load(SeqCst) == id)
    }

    /// Keep output nobody is attached for, while no client has been ready
    /// yet: the latest [`EARLY`] bytes of it, cut after a line's end where
    /// there is one. Called with the client lock held.
    fn keep_early(&self, bytes: &[u8]) {
        let mut early = self.early.lock().unwrap_or_else(|e| e.into_inner());
        let Some(kept) = early.as_mut() else { return };
        kept.extend_from_slice(bytes);
        if kept.len() > EARLY {
            let over = kept.len() - EARLY;
            let cut = kept[over..]
                .iter()
                .position(|&b| b == b'\n')
                .map_or(over, |at| over + at + 1);
            kept.drain(..cut);
        }
    }

    /// Wake [`Host::wait`] waiting on [`Shared::early`]: a stop was asked
    /// for. Under its lock, so a wait that looked just before is waiting.
    fn stop_early_wait(&self) {
        let _early = self.early.lock().unwrap_or_else(|e| e.into_inner());
        self.early_gone.notify_all();
    }
}

/// From `at` in `bytes`, past any bytes that continue a UTF-8 character
/// begun before it.
fn char_start(bytes: &[u8], at: usize) -> usize {
    at + bytes[at..]
        .iter()
        .take_while(|&&b| (0x80..=0xbf).contains(&b))
        .count()
}

/// A program's status as a host exits with it: its code, or `128 + n` for
/// signal `n`.
fn exit_code(status: std::process::ExitStatus) -> i32 {
    use std::os::unix::process::ExitStatusExt;
    status
        .code()
        .unwrap_or_else(|| 128 + status.signal().unwrap_or(0))
}

/// A pseudo-terminal of `size`: our end, and the program's.
pub(crate) fn pty(size: Size) -> io::Result<(File, OwnedFd)> {
    let (ours, theirs) = crate::pty::open()?;
    size.or_default().apply(&theirs)?;
    Ok((ours, theirs))
}

/// `command` as a listing shows it: the program's file name and its
/// arguments.
fn command_line(command: &Command) -> String {
    let program = Path::new(command.get_program());
    let program = program.file_name().unwrap_or(program.as_os_str());
    std::iter::once(program)
        .chain(command.get_args())
        .map(|a| a.to_string_lossy())
        .collect::<Vec<_>>()
        .join(" ")
}

impl Host {
    /// Start `program` on a new pty of `size`, in its own session with the
    /// pty as its controlling terminal, and serve clients on `listener`
    /// (bound at session `id`'s socket in `dir`, removed when the program
    /// ends).
    pub fn start(
        listener: UnixListener,
        dir: PathBuf,
        id: String,
        mut program: Command,
        size: Size,
        control: Control,
    ) -> io::Result<Self> {
        let (master, slave) = pty(size)?;
        let (control, theirs) = match control {
            #[cfg(test)]
            Control::Given(w) => (w, None),
            Control::Pipe => {
                let (r, w) = std::io::pipe()?;
                (OwnedFd::from(w), Some(OwnedFd::from(r)))
            }
        };
        rustix::io::ioctl_fionbio(&control, true)?;
        if let Some(r) = &theirs {
            program
                .env(crate::CONTROL_VAR, r.as_raw_fd().to_string())
                .env(crate::PARENT_VAR, std::process::id().to_string());
        }
        let their_control = theirs.as_ref().map(|r| r.as_raw_fd());
        program
            .stdin(Stdio::from(slave.try_clone()?))
            .stdout(Stdio::from(slave.try_clone()?))
            .stderr(Stdio::from(slave));
        // In the child, between fork and exec: only system calls.
        unsafe {
            std::os::unix::process::CommandExt::pre_exec(&mut program, move || {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(0))?;
                if let Some(fd) = their_control {
                    rustix::io::fcntl_setfd(
                        BorrowedFd::borrow_raw(fd),
                        rustix::io::FdFlags::empty(),
                    )?;
                }
                Ok(())
            });
        }
        let command = command_line(&program);
        let child = program.spawn()?;
        // Our copies of the program's ends close, so its end is the end.
        drop(program);
        drop(theirs);
        let current = Arc::new(AtomicU64::new(0));
        let (input, typed) = sync_channel(IN_QUEUE);
        {
            let pty = master.try_clone()?;
            let current = Arc::clone(&current);
            std::thread::spawn(move || relay_input(pty, &typed, &current));
        }
        let shared = Arc::new(Shared {
            client: Mutex::new(None),
            current,
            next: AtomicU64::new(1),
            pty: master.try_clone()?,
            input,
            control,
            watchers: Mutex::new(Vec::new()),
            early: Mutex::new(Some(Vec::new())),
            early_gone: Condvar::new(),
            heir: AtomicU64::new(0),
            dropped: AtomicU64::new(0),
            stopped: AtomicBool::new(false),
            pid: rustix::process::Pid::from_child(&child),
            started: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_secs()),
            command,
            closing: AtomicBool::new(false),
            at: (dir.clone(), id.clone()),
        });
        let (done, drained) = std::sync::mpsc::channel();
        {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                relay_output(master, &shared);
                let _ = done.send(());
            });
        }
        let listening = {
            let shared = Arc::clone(&shared);
            std::thread::spawn(move || {
                for conn in listener.incoming() {
                    if shared.closing.load(SeqCst) {
                        break;
                    }
                    if let Ok(conn) = conn {
                        let shared = Arc::clone(&shared);
                        std::thread::spawn(move || serve(conn, &shared));
                    }
                }
            })
        };
        Ok(Self {
            child,
            shared,
            drained,
            listening,
            dir,
            id,
        })
    }

    /// Wait for the program to end, then take the session's files away
    /// (socket, status, label, log), hand the client its status and return
    /// it (`128 + n` for signal `n`; 1 when it cannot be had).
    pub fn wait(mut self) -> i32 {
        let code = loop {
            match self.child.wait() {
                Ok(s) => break exit_code(s),
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                // How it ended is not known: not as a success.
                Err(_) => break 1,
            }
        };
        // What it wrote last reaches the client first; something it left
        // behind holding the terminal open does not hold us up for long.
        let _ = self.drained.recv_timeout(LAST_WORDS);
        // Ended before its first client was ready, unasked (an error at
        // the start, say): what it said waits for that client a while.
        {
            let early = self.shared.early.lock().unwrap_or_else(|e| e.into_inner());
            let _ = self
                .shared
                .early_gone
                .wait_timeout_while(early, FIRST, |e| {
                    e.is_some() && !self.shared.stopped.load(SeqCst)
                });
        }
        // The listener stops at its next connection, this one, and closes
        // the socket; then nothing answers on it and its files can go.
        self.shared.closing.store(true, SeqCst);
        let path = crate::dir::socket(&self.dir, &self.id);
        if UnixStream::connect(&path).is_ok() {
            let _ = self.listening.join();
            // A child forked meanwhile holds a copy until it execs.
            let closed = std::time::Instant::now();
            while crate::dir::alive(&path) && closed.elapsed() < LAST_WORDS {
                std::thread::sleep(RETRY);
            }
        }
        let _ = crate::dir::clear(&self.dir, &self.id);
        // Watchers, and `kill`s waiting, hear the end.
        self.shared
            .watchers
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .clear();
        let last = {
            let mut c = self.shared.client.lock().unwrap_or_else(|e| e.into_inner());
            self.shared.current.store(0, SeqCst);
            c.take()
        };
        if let Some(c) = last {
            // After its output; its writer ends with the queue.
            c.out.finish(wire::frame(wire::EXIT, &code.to_be_bytes()));
            let _ = c.writer.join();
        }
        code
    }
}

/// Copy the program's output to whoever is attached. With nobody there
/// it is kept for the first client ([`Shared::keep_early`]), and dropped
/// after that (the repaint on attach brings the screen back). A client
/// new since the last chunk, and not given the output from the start,
/// gets output only from where the stream is between escape sequences
/// and characters, after a [`wire::SCREEN`] saying where the screen is.
/// Each chunk is read into the frame it goes in.
fn relay_output(master: File, shared: &Shared) {
    let mut scan = Scan::default();
    // The client the last chunk went to.
    let mut sent_to = 0;
    loop {
        let mut frame = wire::begin(CHUNK);
        let n = match rustix::io::read(&master, rustix::buffer::spare_capacity(&mut frame)) {
            Ok(0) => break,
            Ok(n) => n,
            Err(rustix::io::Errno::INTR) => continue,
            // A pty whose program is gone reads as an error.
            Err(_) => break,
        };
        // A short read keeps no more room than it needs while queued.
        if 2 * frame.len() < frame.capacity() {
            frame.shrink_to_fit();
        }
        let bytes = &frame[wire::HEAD..];
        let cut = scan.cut(bytes);
        let to = {
            let c = shared.client.lock().unwrap_or_else(|e| e.into_inner());
            let to = c.as_ref().map(|c| (c.id, Arc::clone(&c.out)));
            if to.is_none() {
                shared.keep_early(bytes);
            }
            to
        };
        let Some((id, out)) = to else { continue };
        if id == sent_to || id == shared.heir.load(SeqCst) {
            sent_to = id;
            shared.send_to(id, &out, wire::seal(wire::OUTPUT, frame));
            continue;
        }
        let Some((from, alt)) = cut else { continue };
        sent_to = id;
        let from = char_start(bytes, from);
        if shared.send_to(id, &out, wire::frame(wire::SCREEN, &[u8::from(alt)])) && from < n {
            frame.drain(wire::HEAD..wire::HEAD + from);
            shared.send_to(id, &out, wire::seal(wire::OUTPUT, frame));
        }
    }
}

/// Write what is typed to the pty, if whoever typed it is still the
/// client. A program not reading holds up only this thread.
fn relay_input(mut pty: File, typed: &Receiver<(u64, Vec<u8>)>, current: &AtomicU64) {
    for (id, bytes) in typed {
        if current.load(SeqCst) == id {
            let _ = pty.write_all(&bytes);
        }
    }
}

/// Send a client its frames, in order, until its queue closes; one it
/// cannot take within [`STALL`] is let go.
fn relay_to_client(sock: UnixStream, frames: Arc<Queue>, id: u64, shared: Arc<Shared>) {
    while let Some(frame) = frames.pop() {
        if wire::send_all(&sock, &frame).is_err() {
            shared.drop_client(id);
            break;
        }
    }
    frames.close();
    let _ = sock.shutdown(std::net::Shutdown::Both);
}

/// One connection: a question, a kill, a watcher, or a client attaching.
fn serve(mut conn: UnixStream, shared: &Arc<Shared>) {
    let _ = conn.set_write_timeout(Some(STALL));
    let Ok(Some((tag, body))) = wire::read_frame(&mut conn) else {
        return;
    };
    match tag {
        wire::INFO => {
            let mut reply = shared.pid.as_raw_nonzero().get().to_be_bytes().to_vec();
            reply.push(u8::from(shared.current.load(SeqCst) != 0));
            reply.extend_from_slice(&shared.started.to_be_bytes());
            reply.extend_from_slice(shared.command.as_bytes());
            let _ = wire::write_frame(&conn, wire::INFO_REPLY, &reply);
        }
        wire::KILL => {
            use rustix::process::{kill_process, Signal};
            // The program's own stop path, as its quit key; or outright.
            let force = body.first() == Some(&1);
            shared.stopped.store(true, SeqCst);
            shared.stop_early_wait();
            let _ = kill_process(shared.pid, if force { Signal::KILL } else { Signal::TERM });
            if !force {
                shared.wake();
            }
            // Held until the host ends, for the asker to wait on.
            if wire::write_frame(&conn, wire::OK, &[]).is_ok() && conn.set_nonblocking(true).is_ok()
            {
                let mut w = shared.watchers.lock().unwrap_or_else(|e| e.into_inner());
                w.push(conn);
            }
        }
        wire::WATCH => {
            if conn.set_nonblocking(true).is_err() {
                return;
            }
            // Under the lock, so no word is missed between the two.
            let mut w = shared.watchers.lock().unwrap_or_else(|e| e.into_inner());
            let now = if shared.current.load(SeqCst) != 0 {
                ATTACHED
            } else {
                DETACHED
            };
            if wire::send_all(&conn, &[now]).is_ok() {
                w.push(conn);
            }
        }
        wire::HELLO => attach(conn, &body, shared),
        _ => {}
    }
}

/// A client attaching: once it is ready, size the pty to its terminal,
/// make it the client (letting any other go) and ask for a repaint.
fn attach(mut conn: UnixStream, hello: &[u8], shared: &Arc<Shared>) {
    if hello.first() != Some(&wire::VERSION) {
        return;
    }
    // Applied only once this client is the one attached.
    let mut size = Size::decode(&hello[1..]);
    let env = wire::hello_env(hello);
    let mut id = None;
    while let Ok(Some((tag, body))) = wire::read_frame(&mut conn) {
        let current = id.is_some() && shared.current.load(SeqCst) == id.unwrap_or(0);
        match tag {
            wire::READY if id.is_none() => {
                let Ok(writer) = conn.try_clone() else { break };
                let Ok(sock) = conn.try_clone() else { break };
                if let Some(size) = size.take() {
                    shared.resize(size);
                }
                let me = shared.next.fetch_add(1, SeqCst);
                let out = Arc::new(Queue::default());
                let writer = {
                    let shared = Arc::clone(shared);
                    let frames = Arc::clone(&out);
                    std::thread::spawn(move || relay_to_client(writer, frames, me, shared))
                };
                let old = {
                    let mut c = shared.client.lock().unwrap_or_else(|e| e.into_inner());
                    // The first client ready: what was written before it,
                    // first, under the lock the output thread keeps it
                    // under, so nothing comes between.
                    let early = shared
                        .early
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .take();
                    if let Some(early) = early {
                        shared.heir.store(me, SeqCst);
                        shared.early_gone.notify_all();
                        if !early.is_empty() {
                            // The queue is new: there is room.
                            out.push(wire::frame(wire::OUTPUT, &early), || true);
                        }
                    }
                    shared.current.store(me, SeqCst);
                    // Under the lock that makes it current, so the file is
                    // the current client's when two attach at once.
                    let (dir, sid) = &shared.at;
                    let _ = crate::dir::write_terminal_env(dir, sid, &env);
                    c.replace(Client {
                        id: me,
                        out,
                        sock,
                        writer,
                    })
                };
                if let Some(old) = old {
                    // After what it was sent, however long its queue (its
                    // writer lets it go within STALL); then the writer
                    // closes it. Output waiting for room there gives up.
                    old.out.finish(wire::frame(wire::ELSEWHERE, &[]));
                }
                id = Some(me);
                // After the environment is written: the program reads it as
                // it repaints.
                shared.tell(ATTACHED);
                shared.wake();
            }
            wire::INPUT if current => {
                // A program that is not reading loses what it is typed
                // past its queue; the socket is read on, so the client is
                // never held up and its keys (the detach key) still work.
                if let Err(TrySendError::Full((_, lost))) =
                    shared.input.try_send((id.unwrap_or(0), body))
                {
                    shared.dropped.fetch_add(lost.len() as u64, SeqCst);
                }
            }
            wire::RESIZE => match Size::decode(&body) {
                Some(s) if current => shared.resize(s),
                // Not attached yet: the size to start with.
                Some(s) if id.is_none() => size = Some(s),
                _ => {}
            },
            _ => {}
        }
    }
    if let Some(me) = id {
        shared.drop_client(me);
    }
}

/// The listening socket the launching client bound and passed on, named
/// in the environment ([`crate::LISTEN_VAR`]); None if it is not a socket
/// open here.
fn passed_listener() -> Option<UnixListener> {
    crate::inherited_fd(crate::LISTEN_VAR, rustix::fs::FileType::Socket).map(UnixListener::from)
}

/// Run as a session's host: called in a process the launching client
/// started, with the session's name. Leaves the user's session (so a
/// hangup there cannot reach us), starts the program (this executable
/// again, with the same arguments) and serves it until it ends.
pub(crate) fn serve_session(name: &str) -> ! {
    // A client gone mid-write must be an error here, not the end of the
    // host, whatever the tool set SIGPIPE to before calling us. A handler,
    // not SIG_IGN, so the program we start gets the default back.
    let _ = signal_hook::flag::register(signal_hook::consts::SIGPIPE, Arc::default());
    // The launching client did this before exec; a host started otherwise
    // leaves the user's session here.
    let _ = rustix::process::setsid();
    let code = match host_session(name) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("legume-tui: session {name}: {e}");
            std::process::exit(1)
        }
    };
    std::process::exit(code)
}

fn host_session(name: &str) -> io::Result<i32> {
    crate::dir::check_name(name)?;
    let dir = crate::dir::runtime_dir()?;
    let path = crate::dir::socket(&dir, name);
    let listener = match passed_listener() {
        Some(l) => l,
        None => crate::dir::bind(&path)?,
    };
    let size = std::env::var(crate::SIZE_VAR)
        .ok()
        .and_then(|s| Size::from_env(&s))
        .unwrap_or_default();
    let mut program = Command::new(std::env::current_exe()?);
    program
        .args(std::env::args_os().skip(1))
        .env_remove(crate::HOST_VAR)
        .env_remove(crate::SIZE_VAR)
        .env_remove(crate::LISTEN_VAR)
        .env(crate::SESSION_VAR, name);
    let host = Host::start(
        listener,
        dir,
        name.to_string(),
        program,
        size,
        Control::Pipe,
    )?;
    // It leaves no socket, log, status or label behind.
    Ok(host.wait())
}

#[cfg(test)]
#[path = "tests/host.rs"]
mod tests;
