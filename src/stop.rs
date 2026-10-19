//! `kill` as the program hears it: SIGTERM, taken as a request to stop
//! while the program's screen is up (from [`crate::Session::terminal`] to
//! [`crate::Session::restore`]) and while it waits for a child
//! ([`crate::Session::run_child`]). Elsewhere (loading, say) it ends the
//! program as by default. A second one while taken ends it too.
//!
//! Also a hangup, passed on to the children the program holds in process
//! groups of their own: the kernel sends it to the terminal's foreground
//! group only, so without this they would run on after their host is gone.

use std::io;
use std::os::unix::net::UnixStream;
use std::process::{Child, ExitStatus};
use std::sync::atomic::{AtomicBool, AtomicI32, AtomicUsize, Ordering::SeqCst};
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
pub(crate) struct Armed {
    /// Others took it already as this one was made.
    outer: bool,
}

impl Armed {
    /// None if SIGTERM's handlers could not be set up: it keeps its
    /// default.
    pub fn new() -> Option<Self> {
        let stop = STOP.get_or_init(|| register().ok()).as_ref()?;
        let outer = stop.arms.fetch_add(1, SeqCst) > 0;
        if !outer {
            stop.by_default.store(false, SeqCst);
        }
        Some(Self { outer })
    }

    /// Whether another [`Armed`] took the stop as this one was made: what
    /// it guards decides what a stop does once this one is gone.
    pub fn outer(&self) -> bool {
        self.outer
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

/// The most process groups a hangup is passed on to.
const GROUPS: usize = 64;

/// The process groups of the children held now ([`hold_group`]); 0 for a
/// free slot. Atomics, as the signal handler reads them.
static HELD: [AtomicI32; GROUPS] = [const { AtomicI32::new(0) }; GROUPS];

/// Pass a hangup of the program on to process group `group` (a child that
/// leads its own) until [`release_group`]; false if it cannot be, as when
/// SIGHUP's handlers could not be set up or too many are held.
#[cfg_attr(not(feature = "child"), allow(dead_code))]
pub(crate) fn hold_group(group: i32) -> bool {
    static PASSED: OnceLock<bool> = OnceLock::new();
    if group <= 0 || !*PASSED.get_or_init(|| pass_hangups().is_ok()) {
        return false;
    }
    HELD.iter()
        .any(|slot| slot.compare_exchange(0, group, SeqCst, SeqCst).is_ok())
}

/// No more hangups to process group `group`: its child is waited for.
#[cfg_attr(not(feature = "child"), allow(dead_code))]
pub(crate) fn release_group(group: i32) {
    for slot in &HELD {
        let _ = slot.compare_exchange(group, 0, SeqCst, SeqCst);
    }
}

/// SIGHUP's handlers, in the order they run: the groups held hear it too,
/// then the program ends as by default.
fn pass_hangups() -> io::Result<()> {
    use signal_hook::consts::SIGHUP;
    // Only system calls, as a handler may.
    unsafe {
        signal_hook::low_level::register(SIGHUP, || {
            use rustix::process::{kill_process_group, Pid, Signal};
            for slot in &HELD {
                if let Some(group) = Pid::from_raw(slot.load(SeqCst)) {
                    let _ = kill_process_group(group, Signal::HUP);
                }
            }
        })?;
    }
    signal_hook::flag::register_conditional_default(SIGHUP, Arc::new(AtomicBool::new(true)))?;
    Ok(())
}
