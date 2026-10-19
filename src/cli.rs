//! The `attach`, `sessions`, `kill` and `rename` subcommands, and the
//! flags a TUI subcommand takes, as clap arguments, with their help. A
//! tool wires them in with a few lines:
//!
//! ```no_run
//! use legume_tui::{cli, Tool};
//! const TOOL: Tool = Tool::new("tool");
//!
//! #[derive(clap::Subcommand)]
//! enum Commands {
//!     /// Show a view
//!     #[command(long_about = format!(
//!         "Show a view.\n\n{}",
//!         cli::session_help(
//!             &TOOL,
//!             cli::SessionHelp::new().resume_hint(Some("`tool view` with no file")),
//!         )
//!     ))]
//!     View {
//!         #[command(flatten)]
//!         session: cli::SessionArgs,
//!     },
//!     /// Return to a detached session
//!     #[command(long_about = cli::about(&TOOL, cli::QUIT_KEY).attach)]
//!     Attach(cli::AttachArgs),
//!     /// List the live sessions
//!     #[command(long_about = cli::about(&TOOL, cli::QUIT_KEY).sessions)]
//!     Sessions,
//!     /// Stop a session
//!     #[command(long_about = cli::about(&TOOL, cli::QUIT_KEY).kill)]
//!     Kill(cli::KillArgs),
//!     /// Give a session a label
//!     #[command(long_about = cli::about(&TOOL, cli::QUIT_KEY).rename)]
//!     Rename(cli::RenameArgs),
//! }
//!
//! fn run(cmd: Commands) -> std::io::Result<()> {
//!     match cmd {
//!         Commands::View { session } => {
//!             let _session = TOOL.session("view", &session.options(false))?;
//!             // ...
//!         }
//!         Commands::Attach(a) => cli::attach(&TOOL, &a)?,
//!         Commands::Sessions => cli::sessions(&TOOL)?,
//!         Commands::Kill(k) => cli::kill(&TOOL, &k)?,
//!         Commands::Rename(r) => cli::rename(&TOOL, &r)?,
//!     }
//!     Ok(())
//! }
//! ```
//!
//! Sessions of a tool are those whose ids are `<tool>-...`. Each command
//! that takes a session finds it by its label or by its id. `attach` and
//! `sessions` first say, once, what the tool's ended sessions left
//! ([`crate::Session::leave`]), and `kill` what the one it ended left
//! unless a terminal attached to it was there to say it.
//! Where sessions are not supported, the handlers say so and fail.

#[cfg(sessions)]
use crate::dir::{self, Patience};
#[cfg(sessions)]
use crate::{client, picker};
use crate::{Options, Tool};
use std::io;
#[cfg(sessions)]
use std::time::Duration;

/// How long `kill` waits for a program to wrap up and end (its own work
/// may take a while to stop), saying so after a few seconds and then
/// every so often.
#[cfg(sessions)]
const STOP_WAIT: Patience = Patience {
    limit: Duration::from_secs(60),
    say: Some((Duration::from_secs(3), Duration::from_secs(10))),
};
/// How long `kill --force` waits for it to be gone, saying nothing.
#[cfg(sessions)]
const FORCE_WAIT: Patience = Patience {
    limit: Duration::from_secs(3),
    say: None,
};

/// The width [`session_help`] wraps to by default.
pub const HELP_WIDTH: usize = 70;

/// What a TUI subcommand's long help says about its session, in the
/// tool's words ([`session_help`]): the defaults, each changed in turn
/// (`SessionHelp::new().resume_hint(None)`).
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SessionHelp<'a> {
    /// A sentence before the rest, for a TUI behind a flag: `With
    /// --interactive, the view opens first.`
    pub lead: Option<&'a str>,
    /// What goes on when the connection drops, with its verb: `the fits
    /// go on`; `its work goes on` when None.
    pub what_goes_on: Option<&'a str>,
    /// Who offers the live sessions to return to: `` `tool view` with no
    /// file ``, or by default `Started again with nothing named, it`. None
    /// for a TUI that always opens on something named: each start is a new
    /// session, and `attach` offers the live ones.
    pub resume_hint: Option<&'a str>,
    /// Sentences added at the end, as `` `?` shows the keys in effect.``
    pub more: Option<&'a str>,
    /// The longest line ([`HELP_WIDTH`] by default).
    pub width: usize,
}

impl Default for SessionHelp<'_> {
    fn default() -> Self {
        Self::new()
    }
}

impl<'a> SessionHelp<'a> {
    pub const fn new() -> Self {
        Self {
            lead: None,
            what_goes_on: None,
            resume_hint: Some("Started again with nothing named, it"),
            more: None,
            width: HELP_WIDTH,
        }
    }

    pub const fn lead(self, lead: &'a str) -> Self {
        Self {
            lead: Some(lead),
            ..self
        }
    }

    pub const fn what_goes_on(self, what_goes_on: &'a str) -> Self {
        Self {
            what_goes_on: Some(what_goes_on),
            ..self
        }
    }

    pub const fn resume_hint(self, resume_hint: Option<&'a str>) -> Self {
        Self {
            resume_hint,
            ..self
        }
    }

    pub const fn more(self, more: &'a str) -> Self {
        Self {
            more: Some(more),
            ..self
        }
    }

    pub const fn width(self, width: usize) -> Self {
        Self { width, ..self }
    }
}

/// The paragraph a TUI subcommand's long help ends with, about its
/// session, said as `help` has it and wrapped ([`wrap`]): how it runs, the
/// detach and switch keys, how to return, the subcommands, and how to run
/// it plain.
pub fn session_help(tool: &Tool, help: SessionHelp<'_>) -> String {
    let name = tool.name();
    let resume = match help.resume_hint {
        Some(hint) => format!(
            "{hint} offers the live sessions to return to; `{name} attach`, \
             `sessions`, `kill` and `rename` manage them."
        ),
        None => format!(
            "Starting it again starts a new session, naming the detached \
             ones; `{name} attach` offers them to return to, and `sessions`, \
             `kill` and `rename` manage them."
        ),
    };
    let mut text = help.lead.map_or(String::new(), |lead| format!("{lead} "));
    text.push_str(&format!(
        "In an interactive terminal it runs as a detachable session, so {} \
         when the connection drops. ctrl+alt+d detaches (the first time, it \
         asks for a name; LEGUME_TUI_DETACH_KEY sets another key); ctrl+alt+s \
         switches to another session (LEGUME_TUI_SWITCH_KEY). {resume} \
         --no-detach (or LEGUME_TUI_NO_DETACH=1) runs it in this terminal \
         only.",
        help.what_goes_on.unwrap_or("its work goes on"),
    ));
    if let Some(more) = help.more {
        text.push(' ');
        text.push_str(more);
    }
    wrap(&text, help.width)
}

/// `text` with each paragraph (a blank line apart) broken into lines of
/// at most `width` columns, between words and never inside a `` `quoted
/// command` ``. A word longer than `width` has a line of its own.
pub fn wrap(text: &str, width: usize) -> String {
    text.split("\n\n")
        .map(|p| wrap_paragraph(p, width))
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn wrap_paragraph(paragraph: &str, width: usize) -> String {
    // Words, those inside backticks kept together.
    let mut units: Vec<String> = Vec::new();
    for word in paragraph.split_whitespace() {
        match units.last_mut() {
            Some(open) if open.matches('`').count() % 2 == 1 => {
                open.push(' ');
                open.push_str(word);
            }
            _ => units.push(word.to_string()),
        }
    }
    let mut out = String::new();
    let mut line = 0;
    for unit in units {
        let n = unit.chars().count();
        if line > 0 && line + 1 + n > width {
            out.push('\n');
            line = 0;
        } else if line > 0 {
            out.push(' ');
            line += 1;
        }
        out.push_str(&unit);
        line += n;
    }
    out
}

/// The long help of the session subcommands, for [`about`].
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct About {
    pub attach: String,
    pub sessions: String,
    pub kill: String,
    pub rename: String,
}

/// What `kill` stops a session as, for [`about`]: its quit key.
pub const QUIT_KEY: &str = "its quit key";

/// The long help of `tool`'s `attach`, `sessions`, `kill` and `rename`;
/// `kill` stops a session as `quit` does ([`QUIT_KEY`], or `ctrl+c in it`).
pub fn about(tool: &Tool, quit: &str) -> About {
    let tool = tool.name();
    About {
        attach: format!(
            "\
Return to a detached `{tool}` session in this terminal.

A session keeps running when the terminal goes (a dropped connection, or
the detach key: ctrl+alt+d unless LEGUME_TUI_DETACH_KEY sets another) and
is picked up again here, or by starting it again with nothing named. The
first detach by the key asks for a label (Enter keeps the id); `{tool}
rename` or `r` in the picker changes it later. A session is found by its
label or its id; with one live the name is optional, with several a
picker opens. `{tool} sessions` lists them. Inside one, ctrl+alt+s (or
LEGUME_TUI_SWITCH_KEY) switches to another."
        ),
        sessions: format!(
            "\
List the live `{tool}` sessions, one a line: label and id, attached or
detached, age, pid, command line and what it is doing. What ended
sessions left (a summary, an error) follows, said once."
        ),
        kill: wrap(
            &format!(
                "Stop a `{tool}` session as {quit} does, letting it wrap up its \
                 work first; waits up to a minute for it to end, and says what \
                 it left unless a terminal attached to it said it. --force \
                 kills it at once, and what it started may then keep running."
            ),
            HELP_WIDTH,
        ),
        rename: format!(
            "\
Give a `{tool}` session a label, found by its label or its id. The label
is unique among the live sessions; `attach`, `kill` and `rename` take it
in place of the id, and `sessions` shows it."
        ),
    }
}

/// The session flags of a TUI subcommand.
#[derive(clap::Args, Clone, Debug, Default)]
pub struct SessionArgs {
    /// Run in this terminal only, not as a detachable session (also:
    /// LEGUME_TUI_NO_DETACH=1)
    #[arg(long)]
    pub no_detach: bool,

    /// Label the session, and name its id after the label (after the tool
    /// and screen) instead of numbering it
    #[arg(long, value_name = "NAME")]
    pub session: Option<String>,
}

impl SessionArgs {
    /// The session options; `target` when the user named what to open
    /// (no resume picker then).
    pub fn options(&self, target: bool) -> Options {
        Options {
            no_detach: self.no_detach,
            name: self.session.clone(),
            target,
            ..Options::default()
        }
    }
}

#[derive(clap::Args, Clone, Debug, Default)]
pub struct AttachArgs {
    /// The session: its label, its id in full or without the tool's
    /// prefix, or the end of its id (`foo` for `tool-view-foo`). Without
    /// it, the only live session, or a picker when there are several
    pub name: Option<String>,
}

#[derive(clap::Args, Clone, Debug, Default)]
pub struct KillArgs {
    /// The session, as `attach` takes it; optional when only one is live
    pub name: Option<String>,

    /// Kill the program at once instead of asking it to stop. What it
    /// started in process groups of their own may keep running.
    #[arg(long)]
    pub force: bool,
}

#[derive(clap::Args, Clone, Debug, Default)]
pub struct RenameArgs {
    /// The session, as `attach` takes it
    pub session: String,

    /// Its new label: letters, digits, '-', '_' or '.', and not another
    /// live session's label or id
    pub label: String,
}

/// Attach this terminal to one of `tool`'s sessions; with none named and
/// several live, the one picked. Returns after a detach; when the program
/// ends with a failure, exits with its status.
#[cfg(sessions)]
pub fn attach(tool: &Tool, args: &AttachArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    for (id, text) in dir::take_left(&dir, &tool.prefix()) {
        eprintln!("{}", dir::ended(&id, &text));
    }
    let terminal = rustix::termios::isatty(io::stdin()) && rustix::termios::isatty(io::stdout());
    let live = dir::live(&dir, &tool.prefix())?;
    let id = match args.name.as_deref() {
        // Several, and a terminal to pick on.
        None if terminal && live.len() > 1 => {
            match picker::choose(&dir, tool, false, None, live)? {
                picker::Choice::Attach(id) => id,
                _ => return Ok(()),
            }
        }
        wanted => dir::pick_from(&live, &tool.prefix(), wanted)?.name,
    };
    if !terminal {
        return Err(io::Error::other("attach needs a terminal"));
    }
    match client::attach_and_report(&dir, tool, &id)? {
        0 => Ok(()),
        code => std::process::exit(code),
    }
}

/// Print `tool`'s live sessions, one a line: label and id (`label (id)`,
/// or the id alone), state, age, pid, command line and status, tab
/// separated; then what ended sessions left, once. Stale sockets are
/// removed on the way.
#[cfg(sessions)]
pub fn sessions(tool: &Tool) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let live = dir::live(&dir, &tool.prefix())?;
    let left = dir::take_left(&dir, &tool.prefix());
    if live.is_empty() {
        println!("no live {} sessions", tool.name());
    }
    for l in live {
        println!(
            "{}\t{}\t{}\tpid {}\t{}\t{}",
            l.shown(),
            l.state(),
            l.age(),
            l.pid,
            l.command,
            l.status.as_deref().unwrap_or("")
        );
    }
    for (id, text) in left {
        println!("{}", dir::ended(&id, &text));
    }
    Ok(())
}

/// Stop one of `tool`'s sessions: SIGTERM, which the program takes as its
/// quit key, or with `--force` SIGKILL. Waits for it to end, up to a
/// minute, saying it does after a few seconds.
#[cfg(sessions)]
pub fn kill(tool: &Tool, args: &KillArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let found = dir::pick(&dir, &tool.prefix(), args.name.as_deref())?;
    let path = dir::socket(&dir, &found.name);
    let patience = if args.force { FORCE_WAIT } else { STOP_WAIT };
    // `<id> stopped` is one of the lines tools may read (README).
    let id = &found.name;
    let label = found.labelled();
    let waiting = || println!("waiting for {id}{label} to wrap up");
    match dir::stop_waiting(&path, args.force, patience, waiting)? {
        true => {
            println!("{id} stopped{label}");
            if let Some(text) = dir::take_leave(&dir, id) {
                println!("{text}");
            }
        }
        false if args.force => println!("{id}{label} was killed; its host has not closed yet"),
        false => {
            let (name, handle) = (tool.name(), found.handle());
            println!(
                "asked {id}{label} to stop; it is still wrapping up (`{name} kill --force {handle}` to end it now)"
            );
        }
    }
    Ok(())
}

/// Give one of `tool`'s sessions a new label.
#[cfg(sessions)]
pub fn rename(tool: &Tool, args: &RenameArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let found = dir::pick(&dir, &tool.prefix(), Some(&args.session))?;
    dir::set_label(&dir, &tool.prefix(), &found.name, &args.label)?;
    println!("{} is now {}", found.name, args.label);
    Ok(())
}

/// Where sessions are not supported: the session subcommands say so.
#[cfg(not(sessions))]
mod unsupported {
    use super::*;

    fn unsupported(tool: &Tool) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            format!(
                "{} sessions are not supported on this platform",
                tool.name()
            ),
        ))
    }

    pub fn attach(tool: &Tool, _: &AttachArgs) -> io::Result<()> {
        unsupported(tool)
    }

    pub fn sessions(tool: &Tool) -> io::Result<()> {
        unsupported(tool)
    }

    pub fn kill(tool: &Tool, _: &KillArgs) -> io::Result<()> {
        unsupported(tool)
    }

    pub fn rename(tool: &Tool, _: &RenameArgs) -> io::Result<()> {
        unsupported(tool)
    }
}
#[cfg(not(sessions))]
pub use unsupported::{attach, kill, rename, sessions};

#[cfg(test)]
#[path = "tests/cli.rs"]
mod tests;
