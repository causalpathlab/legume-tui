//! Pseudo-terminals: one a session's program runs on, one a child's
//! stderr goes to so its progress bars draw, one a test types into.

use std::fs::File;
use std::io;
use std::os::fd::OwnedFd;

/// A new pseudo-terminal: our end, and the other, both closed on exec.
pub(crate) fn open() -> io::Result<(File, OwnedFd)> {
    use rustix::fs::{Mode, OFlags};
    use rustix::io::{fcntl_setfd, FdFlags};
    use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
    let ours = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY)?;
    fcntl_setfd(&ours, FdFlags::CLOEXEC)?;
    grantpt(&ours)?;
    unlockpt(&ours)?;
    let name = ptsname(&ours, Vec::new())?;
    let theirs = rustix::fs::open(
        name.as_c_str(),
        OFlags::RDWR | OFlags::NOCTTY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    Ok((File::from(ours), theirs))
}
