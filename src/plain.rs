//! Where sessions are not supported (see `build.rs`): what a plain
//! [`crate::Session`] needs of the session internals, standing in for
//! them. No host, no control channel, and `kill` is not taken as a stop.

use std::path::PathBuf;

/// The host's word on attaching: never had here.
pub(crate) enum Control {}

impl Control {
    pub(crate) fn attached(&self) -> bool {
        match *self {}
    }

    pub(crate) fn take_repaint(&self) -> bool {
        match *self {}
    }
}

/// Never in a session here.
pub(crate) fn session_at() -> Option<(PathBuf, String)> {
    None
}

/// The session files: none here.
pub(crate) mod dir {
    use std::io;
    use std::path::Path;

    pub(crate) fn write_status(_: &Path, _: &str, _: &str) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn read_terminal_env(_: &Path, _: &str) -> Option<Vec<(String, String)>> {
        None
    }

    pub(crate) fn write_leave(_: &Path, _: &str, _: &str) -> io::Result<()> {
        Ok(())
    }

    pub(crate) fn take_leave(_: &Path, _: &str) -> Option<String> {
        None
    }
}

/// No stop is taken: the program ends as by default.
pub(crate) mod stop {
    use std::io;
    use std::process::{Child, ExitStatus};

    pub(crate) enum Armed {}

    impl Armed {
        pub(crate) fn new() -> Option<Self> {
            None
        }

        pub(crate) fn outer(&self) -> bool {
            match *self {}
        }
    }

    // As the real one, which gives the stop up when dropped.
    impl Drop for Armed {
        fn drop(&mut self) {
            match *self {}
        }
    }

    pub(crate) fn heard() -> bool {
        false
    }

    pub(crate) fn end_as_by_default() {}

    pub(crate) fn wait(mut child: Child, _passes: bool) -> io::Result<ExitStatus> {
        child.wait()
    }

    /// No hangup is passed on here: nothing is hosted.
    #[cfg_attr(not(feature = "child"), allow(dead_code))]
    pub(crate) fn hold_group(_group: i32) -> bool {
        false
    }

    #[cfg_attr(not(feature = "child"), allow(dead_code))]
    pub(crate) fn release_group(_group: i32) {}
}
