//! Which image protocol the attached terminal speaks: the one the user
//! named ([`Graphics`]), else asked of the terminal ([`Fallback::Probe`])
//! or guessed from its environment ([`Fallback::Env`]), else block
//! characters.
//!
//! A hosted program may be attached from a terminal other than the one it
//! started on, so the answer is had again after every attach (in the hook
//! [`Session::poll_with`] calls, before the session's modes are on again:
//! a mouse report would land amid the terminal's answer). The query and
//! its reply travel through the relay; the environment guess reads what
//! the attaching client passed on ([`Session::terminal_env`]).
//!
//! Whether the attached terminal is inside tmux comes from what it passed
//! on too ([`in_tmux`]), and the query is wrapped for tmux by that. A
//! picker's own pictures are wrapped as ratatui-image decides once, from
//! this process's `TERM` and `TERM_PROGRAM` (fixed at launch, and with no
//! setter): when the two disagree after an attach, [`Graphics::Auto`]
//! takes block characters, which need no wrapping.
//!
//! The query is the crate's own, not ratatui-image's: that one reads stdin
//! on a thread that outlives its timeout, so a terminal that never answers
//! leaves a read behind that takes the next keys. Here the answer is read
//! on the calling thread, a byte at a time and only while the terminal
//! has one to give within [`PROBE_TIMEOUT`]; once it returns nothing is
//! left reading. An answer that comes after the timeout is read by the
//! program as input, as any late reply would be.

use crate::Session;
use ratatui_image::picker::{Picker, ProtocolType};
use std::time::Duration;

/// How long a terminal has to answer the query.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// A cell's size in pixels when a protocol is named and the terminal does
/// not say: a common one.
const DEFAULT_CELL: (u16, u16) = (10, 20);

/// The image protocol the user asks for (`--graphics`).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum Graphics {
    /// The terminal's own image protocol, if it has one; block characters
    /// otherwise.
    #[default]
    Auto,
    Kitty,
    Sixel,
    Iterm2,
    /// Coloured half-block characters: works everywhere, lowest resolution.
    Blocks,
}

impl Graphics {
    /// The protocol named; None for [`Graphics::Auto`].
    pub fn protocol(self) -> Option<ProtocolType> {
        match self {
            Self::Auto => None,
            Self::Kitty => Some(ProtocolType::Kitty),
            Self::Sixel => Some(ProtocolType::Sixel),
            Self::Iterm2 => Some(ProtocolType::Iterm2),
            Self::Blocks => Some(ProtocolType::Halfblocks),
        }
    }
}

/// How [`Graphics::Auto`] finds the protocol.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Fallback {
    /// Ask the terminal ([`query`]), with [`PROBE_TIMEOUT`]. Kitty or
    /// sixel as it answers, but not from WezTerm or Konsole, whose kitty
    /// and sixel fall short; else iTerm2 where the environment names a
    /// terminal that has it; else block characters.
    /// Where the terminal cannot be asked (a build without sessions), as
    /// [`Fallback::Env`].
    Probe,
    /// Guess from the terminal's environment ([`env_guess`]), without a
    /// word to the terminal.
    Env,
}

/// What the terminal said to the query.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Answer {
    /// Kitty, else sixel, if it has either.
    pub protocol: Option<ProtocolType>,
    /// A cell's size in pixels, width and height.
    pub cell: Option<(u16, u16)>,
    /// Its background colour (OSC 11), red, green and blue.
    pub background: Option<(u8, u8, u8)>,
}

/// A picker for the terminal `session` is attached to now. A protocol
/// `graphics` names is taken as it is ([`Graphics::Blocks`] without a word
/// to the terminal; another with the cell size the terminal gives, if it
/// does). [`Graphics::Auto`] finds one as `fallback` says, and takes block
/// characters while nobody is attached, when the cell's size in pixels is
/// not known, when the attached terminal is inside tmux and this process
/// was not started there or the other way round, or when nothing better
/// is found. Call it with the terminal set up (raw), whenever
/// [`Session::take_repaint`] is true, and rebuild the image protocols made
/// with the old one ([`reprobe`] says which need it).
pub fn pick(session: &Session, graphics: Graphics, fallback: Fallback) -> Picker {
    pick_with(session, graphics, fallback).0
}

/// [`pick`], and what the terminal answered if it was asked (its
/// background colour among it), so the one round trip serves both.
pub fn pick_with(
    session: &Session,
    graphics: Graphics,
    fallback: Fallback,
) -> (Picker, Option<Answer>) {
    let named = match graphics {
        Graphics::Blocks => return (Picker::halfblocks(), None),
        g => g.protocol(),
    };
    let fallback = if cfg!(sessions) {
        fallback
    } else {
        Fallback::Env
    };
    let attached = session.attached();
    let tmux = in_tmux(session);
    // Made first: it finds tmux as its pictures will be wrapped, and lets
    // them through it.
    let blocks = Picker::halfblocks();
    let answer = match fallback {
        Fallback::Probe if attached => ask_terminal(tmux),
        _ => None,
    };
    let cell = answer.and_then(|a| a.cell).or_else(window_cell);
    let wrapped_right = tmux == blocks.tmux_detected();
    let protocol = match (named, fallback) {
        (Some(p), _) => p,
        (None, _) if !attached || cell.is_none() || !wrapped_right => ProtocolType::Halfblocks,
        (None, Fallback::Env) => env_guess(session),
        (None, Fallback::Probe) => probe_protocol(answer.and_then(|a| a.protocol), tmux, |k| {
            session.terminal_env(k)
        }),
    };
    if protocol == ProtocolType::Halfblocks {
        return (blocks, answer);
    }
    // The cell's size known (or a protocol named outright): the picker for
    // it, its protocol set below.
    #[allow(deprecated)] // in favour of ratatui-image's query, avoided here
    let mut picker = Picker::from_fontsize(cell.unwrap_or(DEFAULT_CELL).into());
    picker.set_protocol_type(protocol);
    (picker, answer)
}

/// The protocol [`Fallback::Probe`] takes, given the protocol the
/// terminal answered with (`answered`), whether it is inside tmux, and
/// its environment (`var`). As ratatui-image's own query decides: an
/// answer of kitty or sixel from WezTerm or Konsole is not taken; then
/// iTerm2 inside tmux when the outer terminal is iTerm2 or WezTerm, or
/// when `TERM_PROGRAM` or `LC_TERMINAL` names a terminal that has it;
/// else block characters.
pub(crate) fn probe_protocol(
    answered: Option<ProtocolType>,
    tmux: bool,
    var: impl Fn(&str) -> Option<String>,
) -> ProtocolType {
    let set = |k: &str| var(k).is_some_and(|v| !v.is_empty());
    let short = set("WEZTERM_EXECUTABLE") || set("KONSOLE_VERSION");
    let outer_iterm2 = tmux && (set("ITERM_SESSION_ID") || set("WEZTERM_EXECUTABLE"));
    answered
        .filter(|_| !short)
        .or_else(|| outer_iterm2.then_some(ProtocolType::Iterm2))
        .or_else(|| iterm2_from_env(&var))
        .unwrap_or(ProtocolType::Halfblocks)
}

/// iTerm2, where `TERM_PROGRAM` or `LC_TERMINAL` names a terminal that
/// has it (ratatui-image's list).
fn iterm2_from_env(var: impl Fn(&str) -> Option<String>) -> Option<ProtocolType> {
    const PROGRAMS: [&str; 9] = [
        "iTerm",
        "WezTerm",
        "mintty",
        "vscode",
        "Tabby",
        "Hyper",
        "rio",
        "Bobcat",
        "WarpTerminal",
    ];
    let program = var("TERM_PROGRAM").unwrap_or_default();
    let has = PROGRAMS.iter().any(|p| program.contains(p))
        || var("LC_TERMINAL").is_some_and(|t| t.contains("iTerm"));
    has.then_some(ProtocolType::Iterm2)
}

/// Whether the terminal `session` is attached to is inside tmux, by what
/// it passed on ([`Session::terminal_env`]): `TMUX` set, `TERM` starting
/// with `tmux`, or `TERM_PROGRAM` tmux. A tool that writes pictures itself
/// wraps them for tmux by this.
pub fn in_tmux(session: &Session) -> bool {
    tmux_from_env(|k| session.terminal_env(k))
}

/// [`in_tmux`], from the environment variables (`var`).
pub(crate) fn tmux_from_env(var: impl Fn(&str) -> Option<String>) -> bool {
    var("TMUX").is_some_and(|v| !v.is_empty())
        || var("TERM").is_some_and(|t| t.starts_with("tmux"))
        || var("TERM_PROGRAM").is_some_and(|p| p == "tmux")
}

/// What the terminal attached now needs of the pictures made for the
/// last one.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Repick {
    /// Nothing: they draw as they are.
    Same,
    /// The same protocol and cell size, but kitty: the client's reset on
    /// attach deleted the images kitty keeps, so each is sent again
    /// (encoded again with the same picker).
    Resend,
    /// Another protocol or cell size: every picture made again with this.
    Changed(Picker),
}

/// Pick for the terminal attached now ([`pick`]) and say what the
/// pictures made with `old` need. A program can start from
/// [`Picker::halfblocks`] and take the first answer as for any attach:
/// the first [`Session::take_repaint`] is true.
pub fn reprobe(session: &Session, old: &Picker, graphics: Graphics, fallback: Fallback) -> Repick {
    let new = pick(session, graphics, fallback);
    let kind = |p: &Picker| {
        let f = p.font_size();
        (p.protocol_type(), f.width, f.height)
    };
    if kind(&new) != kind(old) {
        Repick::Changed(new)
    } else if new.protocol_type() == ProtocolType::Kitty {
        Repick::Resend
    } else {
        Repick::Same
    }
}

/// The image protocol the attached terminal's environment names
/// ([`Session::terminal_env`]): kitty for kitty and ghostty, iTerm2 for
/// iTerm2 and WezTerm, sixel for foot and mlterm, else block characters.
/// Block characters too inside tmux, where a picture needs passthrough
/// (which naming the protocol asks for).
pub fn env_guess(session: &Session) -> ProtocolType {
    protocol_from_env(|k| session.terminal_env(k))
}

/// The image protocol the environment variables (`var`) name.
pub(crate) fn protocol_from_env(var: impl Fn(&str) -> Option<String>) -> ProtocolType {
    let v = |k: &str| var(k).unwrap_or_default();
    let (term, program) = (v("TERM"), v("TERM_PROGRAM"));
    if var("TMUX").is_some() {
        ProtocolType::Halfblocks
    } else if var("KITTY_WINDOW_ID").is_some()
        || term.contains("kitty")
        || term.contains("ghostty")
        || program.eq_ignore_ascii_case("ghostty")
    {
        ProtocolType::Kitty
    } else if program == "iTerm.app" || program == "WezTerm" || v("LC_TERMINAL") == "iTerm2" {
        ProtocolType::Iterm2
    } else if term.starts_with("foot") || term.starts_with("mlterm") {
        ProtocolType::Sixel
    } else {
        ProtocolType::Halfblocks
    }
}

/// A cell's size in pixels, from the window's, when the terminal reports
/// it.
fn window_cell() -> Option<(u16, u16)> {
    let w = ratatui::crossterm::terminal::window_size().ok()?;
    cell_pixels(w.columns, w.rows, w.width, w.height)
}

/// A cell's size in pixels in a window of `columns` by `rows` cells and
/// `width` by `height` pixels; None when a size is not reported.
pub(crate) fn cell_pixels(columns: u16, rows: u16, width: u16, height: u16) -> Option<(u16, u16)> {
    (columns > 0 && rows > 0 && width > 0 && height > 0).then(|| (width / columns, height / rows))
}

/// Ask the terminal `session` is attached to what it can show: its image
/// protocol, cell size and background colour. None while nobody is
/// attached, when stdin is not a terminal, when it does not answer within
/// [`PROBE_TIMEOUT`], and where the terminal cannot be asked (a build
/// without sessions). Call it with the terminal set up (raw), before mouse
/// reports are on. Nothing is left reading after it returns.
pub fn query(session: &Session) -> Option<Answer> {
    if !session.attached() {
        return None;
    }
    ask_terminal(in_tmux(session))
}

/// [`ask`] the terminal on stdin and stdout, the query wrapped for tmux
/// when `tmux`.
#[cfg(sessions)]
fn ask_terminal(tmux: bool) -> Option<Answer> {
    use std::os::fd::AsFd;
    let stdin = std::io::stdin();
    if !rustix::termios::isatty(&stdin) {
        return None;
    }
    ask(stdin.as_fd(), &mut std::io::stdout(), tmux, PROBE_TIMEOUT)
        .ok()
        .flatten()
}

#[cfg(not(sessions))]
fn ask_terminal(_tmux: bool) -> Option<Answer> {
    None
}

/// Write the query to `output` and read the terminal's answer from
/// `input`, on this thread, for at most `timeout`: a byte at a time, each
/// only once `input` has one, up to the end of the answer (the device
/// status report the query ends with). None if the answer did not end in
/// time. Echo and line mode are off while it reads, and put back after.
#[cfg(sessions)]
pub(crate) fn ask(
    input: std::os::fd::BorrowedFd<'_>,
    output: &mut impl std::io::Write,
    tmux: bool,
    timeout: Duration,
) -> std::io::Result<Option<Answer>> {
    use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions};
    use rustix::termios::{tcgetattr, tcsetattr, LocalModes, OptionalActions, SpecialCodeIndex};
    let before = tcgetattr(input)?;
    let mut quiet = before.clone();
    quiet.local_modes &= !(LocalModes::ICANON | LocalModes::ECHO);
    // Each read returns what has come, however little.
    quiet.special_codes[SpecialCodeIndex::VMIN] = 1;
    quiet.special_codes[SpecialCodeIndex::VTIME] = 0;
    tcsetattr(input, OptionalActions::Now, &quiet)?;
    let answered = (|| {
        let options = QueryStdioOptions {
            timeout,
            terminal_background_color_osc: true,
            ..Default::default()
        };
        output.write_all(Parser::query(tmux, options).as_bytes())?;
        output.flush()?;
        read_answer(input, timeout)
    })();
    tcsetattr(input, OptionalActions::Now, &before)?;
    answered
}

/// The answer read from `input` as [`ask`] says.
#[cfg(sessions)]
fn read_answer(
    input: std::os::fd::BorrowedFd<'_>,
    timeout: Duration,
) -> std::io::Result<Option<Answer>> {
    use ratatui_image::picker::cap_parser::{Parser, Response};
    use rustix::event::{poll, PollFd, PollFlags, Timespec};
    let start = std::time::Instant::now();
    let mut parser = Parser::new();
    let mut answer = Answer::default();
    loop {
        let left = timeout.saturating_sub(start.elapsed());
        if left.is_zero() {
            return Ok(None);
        }
        let left = Timespec::try_from(left).map_err(std::io::Error::other)?;
        let mut fds = [PollFd::new(&input, PollFlags::IN)];
        match poll(&mut fds, Some(&left)) {
            Ok(0) => continue,
            Ok(_) => {}
            Err(rustix::io::Errno::INTR) => continue,
            Err(e) => return Err(e.into()),
        }
        let mut byte = [0u8];
        match rustix::io::read(input, &mut byte) {
            Ok(0) => return Ok(None),
            Ok(_) => {}
            Err(rustix::io::Errno::INTR | rustix::io::Errno::AGAIN) => continue,
            Err(e) => return Err(e.into()),
        }
        for response in parser.push(char::from(byte[0])) {
            match response {
                Response::Status => return Ok(Some(answer)),
                Response::Kitty => answer.protocol = Some(ProtocolType::Kitty),
                Response::Sixel => {
                    answer.protocol.get_or_insert(ProtocolType::Sixel);
                }
                Response::CellSize(Some(cell)) => answer.cell = Some(cell),
                Response::Background(r, g, b) => answer.background = Some((r, g, b)),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/graphics.rs"]
mod tests;
