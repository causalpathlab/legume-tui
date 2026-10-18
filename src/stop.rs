//! `kill` as the program hears it: SIGTERM, taken as a request to stop
//! while the program's screen is up (from [`crate::Session::terminal`] to
//! [`crate::Session::restore`]) and while it waits for a child
//! ([`crate::Session::run_child`]). Elsewhere (loading, say) it ends the
//! program as by default. A second one while taken ends it too.

use std::io;
use std::os::unix::net::UnixStream;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering::SeqCst};
use std::sync::{Arc, OnceLock};

/// What SIGTERM does here, set up once per process the first time a stop
/// is taken; None if that could not be arranged.
static STOP: OnceLock<Option<Stop>> = OnceLock::new();

struct Stop {
    /// SIGTERM came while taken.
    heard: Arc<AtomicBool>,
    /// SIGTERM ends the program as by default: nothing takes it.
    by_default: Arc<AtomicBool>,
    /// How many take it now ([`Armed`]).
    arms: AtomicUsize,
    /// What SIGTERM writes to, to wake a wait for a child.
    woken: UnixStream,
}

/// Arrange SIGTERM's handlers, in the order they run: while nothing takes
/// it, and once it has come, the default; else it is heard, wakes a wait
/// for a child, and wakes the program's event loop (crossterm reports a
/// resize, as on an attach).
fn register() -> io::Result<Stop> {
    use signal_hook::consts::{SIGTERM, SIGWINCH};
    use signal_hook::{flag, low_level};
    let heard = Arc::new(AtomicBool::new(false));
    let by_default = Arc::new(AtomicBool::new(true));
    let (woken, wake_in) = UnixStream::pair()?;
    woken.set_nonblocking(true)?;
    flag::register_conditional_default(SIGTERM, Arc::clone(&by_default))?;
    flag::register_conditional_default(SIGTERM, Arc::clone(&heard))?;
    flag::register(SIGTERM, Arc::clone(&heard))?;
    low_level::pipe::register(SIGTERM, wake_in)?;
    // Only a system call, as a handler may.
    unsafe {
        low_level::register(SIGTERM, || {
            let _ = low_level::raise(SIGWINCH);
        })?;
    }
    Ok(Stop {
        heard,
        by_default,
        arms: AtomicUsize::new(0),
        woken,
    })
}

/// SIGTERM taken as a stop while this lives.
pub(crate) struct Armed(());

impl Armed {
    /// None if SIGTERM's handlers could not be set up: it keeps its
    /// default.
    pub fn new() -> Option<Self> {
        let stop = STOP.get_or_init(|| register().ok()).as_ref()?;
        if stop.arms.fetch_add(1, SeqCst) == 0 {
            stop.by_default.store(false, SeqCst);
        }
        Some(Self(()))
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        if let Some(stop) = STOP.get().and_then(Option::as_ref) {
            if stop.arms.fetch_sub(1, SeqCst) == 1 {
                stop.by_default.store(true, SeqCst);
            }
        }
    }
}

/// Whether SIGTERM came while taken.
pub(crate) fn heard() -> bool {
    STOP.get()
        .and_then(Option::as_ref)
        .is_some_and(|s| s.heard.load(SeqCst))
}

/// End the program as SIGTERM does by default.
pub(crate) fn end_as_by_default() -> ! {
    let _ = signal_hook::low_level::emulate_default_handler(signal_hook::consts::SIGTERM);
    // Not ended (it cannot be helped): as a host reports such a death.
    std::process::exit(128 + signal_hook::consts::SIGTERM)
}

/// Wait for `child` to end, as [`Child::wait`] does, without waking
/// until it has: a thread waits for it and says so on a socket, waited on
/// together with the stop's. With `passes`, a stop heard is passed on to
/// the child as SIGTERM, once.
pub(crate) fn wait(mut child: Child, passes: bool) -> io::Result<ExitStatus> {
    use rustix::event::{poll, PollFd, PollFlags};
    use rustix::process::{kill_process, waitid, Pid, Signal, WaitId, WaitIdOptions};
    let pid = Pid::from_child(&child);
    let (gone, gone_in) = UnixStream::pair()?;
    let waiter = std::thread::spawn(move || {
        // Not reaped: its pid stays its own until `child.wait()` below,
        // so a stop passed on reaches it and nothing else.
        let ended = WaitIdOptions::EXITED | WaitIdOptions::NOWAIT;
        while let Err(rustix::io::Errno::INTR) = waitid(WaitId::Pid(pid), ended) {}
        // Closed: `gone` reads as ended.
        drop(gone_in);
    });
    let woken = STOP
        .get()
        .and_then(Option::as_ref)
        .filter(|_| passes)
        .map(|s| &s.woken);
    let mut passed = false;
    loop {
        if let Some(woken) = woken {
            while rustix::io::read(woken, &mut [0u8; 64]).is_ok_and(|n| n > 0) {}
            if !passed && heard() {
                passed = true;
                let _ = kill_process(pid, Signal::TERM);
            }
        }
        let mut fds = vec![PollFd::new(&gone, PollFlags::IN)];
        fds.extend(woken.map(|w| PollFd::new(w, PollFlags::IN)));
        match poll(&mut fds, None) {
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            // Waited for plainly below.
            Err(_) => break,
        }
        if !fds[0].revents().is_empty() {
            break;
        }
    }
    let _ = waiter.join();
    child.wait()
}
