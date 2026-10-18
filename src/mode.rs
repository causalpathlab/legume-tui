//! Whether a TUI starting now is hosted, and why not when it is not.

use crate::Options;

/// Whether a TUI in an interactive terminal is hosted unless asked not to
/// be. The one place to change to make hosting opt-in
/// ([`Options::detach`] then asks for it).
pub fn hosts_by_default() -> bool {
    true
}

/// Why a session runs plain, in the terminal it started in, as before.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bypass {
    /// `--no-detach` ([`Options::no_detach`]).
    Flag,
    /// `LEGUME_TUI_NO_DETACH` is set.
    Env,
    /// Already inside a session: a hosted tool started another TUI.
    Nested,
    /// stdin or stdout is not a terminal.
    NotTerminal,
    /// Inside tmux or screen: hosts do not stack on a multiplexer.
    Multiplexer,
    /// Hosting is opt-in and was not asked for.
    NotAsked,
    /// A host was wanted but could not start.
    NoHost,
}

/// What `session()` does.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Mode {
    /// This process is the program a host started.
    Hosted,
    /// Start a host and be its first client.
    Launch,
    /// Run in this terminal as before.
    Plain(Bypass),
}

/// What of the environment bears on the [`Mode`].
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Env {
    /// `LEGUME_TUI_NO_DETACH` is set to something other than empty or `0`.
    pub no_detach: bool,
    /// `LEGUME_TUI_SESSION` is set.
    pub marker: bool,
    /// The control channel a host hands its own program, valid here: set,
    /// and our parent is the host that set it.
    pub control: bool,
    pub stdin_tty: bool,
    pub stdout_tty: bool,
    /// `TMUX` or `STY` is set, or `TERM` names screen or tmux.
    pub multiplexer: bool,
}

impl Env {
    /// The environment of this process.
    pub fn read() -> Self {
        let set = |k: &str| std::env::var_os(k).is_some_and(|v| !v.is_empty());
        let term = std::env::var("TERM").unwrap_or_default();
        let parent = rustix::process::getppid().map(|p| p.as_raw_nonzero().get() as u32);
        let host = std::env::var(crate::PARENT_VAR)
            .ok()
            .and_then(|v| v.parse::<u32>().ok());
        Self {
            no_detach: std::env::var_os(crate::NO_DETACH_VAR)
                .is_some_and(|v| !v.is_empty() && v != "0"),
            marker: set(crate::SESSION_VAR),
            control: set(crate::CONTROL_VAR) && host.is_some() && host == parent,
            stdin_tty: rustix::termios::isatty(std::io::stdin()),
            stdout_tty: rustix::termios::isatty(std::io::stdout()),
            multiplexer: set("TMUX")
                || set("STY")
                || term.starts_with("screen")
                || term.starts_with("tmux"),
        }
    }
}

/// The mode for `opts` in `env`.
pub(crate) fn mode(opts: &Options, env: &Env) -> Mode {
    if env.control && env.marker {
        return Mode::Hosted;
    }
    let bypass = if opts.no_detach {
        Some(Bypass::Flag)
    } else if env.no_detach {
        Some(Bypass::Env)
    } else if env.marker {
        Some(Bypass::Nested)
    } else if !env.stdin_tty || !env.stdout_tty {
        Some(Bypass::NotTerminal)
    } else if env.multiplexer {
        Some(Bypass::Multiplexer)
    } else if !(opts.detach || hosts_by_default()) {
        Some(Bypass::NotAsked)
    } else {
        None
    };
    bypass.map_or(Mode::Launch, Mode::Plain)
}

#[cfg(test)]
#[path = "tests/mode.rs"]
mod tests;
