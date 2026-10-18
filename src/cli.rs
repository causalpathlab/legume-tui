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
//!     #[command(long_about = format!("Show a view.\n\n{}", cli::SESSION_HELP))]
//!     View {
//!         #[command(flatten)]
//!         session: cli::SessionArgs,
//!     },
//!     /// Return to a detached session
//!     #[command(long_about = cli::about("tool").attach)]
//!     Attach(cli::AttachArgs),
//!     /// List the live sessions
//!     #[command(long_about = cli::about("tool").sessions)]
//!     Sessions,
//!     /// Stop a session
//!     #[command(long_about = cli::about("tool").kill)]
//!     Kill(cli::KillArgs),
//!     /// Give a session a label
//!     #[command(long_about = cli::about("tool").rename)]
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
//! that takes a session finds it by its label or by its id.

use crate::dir::Patience;
use crate::{client, dir, picker, Options, Tool};
use std::io;
use std::time::Duration;

/// How long `kill` waits for a program to wrap up and end (its own work
/// may take a while to stop), saying so after a few seconds and then
/// every so often.
const STOP_WAIT: Patience = Patience {
    limit: Duration::from_secs(60),
    say: Some((Duration::from_secs(3), Duration::from_secs(10))),
};
/// How long `kill --force` waits for it to be gone, saying nothing.
const FORCE_WAIT: Patience = Patience {
    limit: Duration::from_secs(3),
    say: None,
};

/// What a TUI subcommand's long help says about its session, after what
/// the subcommand does.
pub const SESSION_HELP: &str = "\
In an interactive terminal it runs as a detachable session, so its work
goes on when the connection drops. ctrl+alt+d detaches (the first time,
it asks for a name; LEGUME_TUI_DETACH_KEY sets another key); ctrl+alt+s
switches to another session (LEGUME_TUI_SWITCH_KEY). Started again with
nothing named, it offers the live sessions to return to; `attach`,
`sessions`, `kill` and `rename` manage them. --no-detach (or
LEGUME_TUI_NO_DETACH=1) runs it in this terminal only.";

/// The long help of the session subcommands, for [`about`].
#[derive(Clone, Debug)]
pub struct About {
    pub attach: String,
    pub sessions: String,
    pub kill: String,
    pub rename: String,
}

/// The long help of `tool`'s `attach`, `sessions`, `kill` and `rename`.
pub fn about(tool: &str) -> About {
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
detached, age, pid, command line and what it is doing."
        ),
        kill: format!(
            "\
Stop a `{tool}` session as its quit key does, letting it wrap up its work
first; waits up to a minute for it to end. --force kills it at once, and
what it started may then keep running."
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
pub fn attach(tool: &Tool, args: &AttachArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
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
/// separated. Stale sockets are removed on the way.
pub fn sessions(tool: &Tool) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let live = dir::live(&dir, &tool.prefix())?;
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
    Ok(())
}

/// Stop one of `tool`'s sessions: SIGTERM, which the program takes as its
/// quit key, or with `--force` SIGKILL. Waits for it to end, up to a
/// minute, saying it does after a few seconds.
pub fn kill(tool: &Tool, args: &KillArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let found = dir::pick(&dir, &tool.prefix(), args.name.as_deref())?;
    let path = dir::socket(&dir, &found.name);
    let patience = if args.force { FORCE_WAIT } else { STOP_WAIT };
    let waiting = || println!("waiting for {} to wrap up", found.name);
    match dir::stop_waiting(&path, args.force, patience, waiting)? {
        true => println!("{} stopped", found.name),
        false if args.force => println!("{} was killed; its host has not closed yet", found.name),
        false => println!(
            "asked {} to stop; it is still wrapping up (`{} kill --force {}` to end it now)",
            found.name,
            tool.name(),
            found.name
        ),
    }
    Ok(())
}

/// Give one of `tool`'s sessions a new label.
pub fn rename(tool: &Tool, args: &RenameArgs) -> io::Result<()> {
    let dir = crate::runtime_dir()?;
    let found = dir::pick(&dir, &tool.prefix(), Some(&args.session))?;
    dir::set_label(&dir, &tool.prefix(), &found.name, &args.label)?;
    println!("{} is now {}", found.name, args.label);
    Ok(())
}
