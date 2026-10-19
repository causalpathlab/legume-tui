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
//! program as input, as any late reply would be. The same round trip asks
//! for kitty's keyboard enhancement, which the session keeps for the
//! modes that depend on it ([`crate::Keyboard::IfSupported`]).

use crate::Session;
use ratatui_image::picker::{Picker, ProtocolType};
use std::time::Duration;

/// How long a terminal has to answer the query.
pub const PROBE_TIMEOUT: Duration = crate::ASK_TIMEOUT;

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
    /// [`Fallback::Probe`], and where the terminal does not answer in
    /// time, [`Fallback::Env`]'s guess.
    ProbeThenEnv,
}

impl Fallback {
    /// Whether the terminal is asked: never where it cannot be.
    fn asks(self) -> bool {
        cfg!(sessions) && matches!(self, Self::Probe | Self::ProbeThenEnv)
    }
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
    /// Whether it has kitty's progressive keyboard enhancement (it
    /// answered `CSI ? u`). Not asked inside tmux: false there.
    pub keyboard_enhancement: bool,
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
    match graphics {
        Graphics::Blocks => (Picker::halfblocks(), None),
        g => pick_protocol(session, g.protocol(), fallback),
    }
}

/// [`pick_with`] for a tool's own choice of protocol, `wanted` (None for
/// auto), so values of its own map onto it: the protocol it wants is
/// taken as it is, with the cell size the terminal gives where it is
/// asked. Unlike [`Graphics::Blocks`], block characters wanted here still
/// ask the terminal when `fallback` does, for its answer (the cell's shape,
/// the background).
pub fn pick_protocol(
    session: &Session,
    wanted: Option<ProtocolType>,
    fallback: Fallback,
) -> (Picker, Option<Answer>) {
    let attached = session.attached();
    let tmux = in_tmux(session);
    // Made first: it finds tmux as its pictures will be wrapped, and lets
    // them through it.
    let blocks = Picker::halfblocks();
    let answer = if fallback.asks() && attached {
        ask_terminal(session, tmux)
    } else {
        None
    };
    let cell = answer.and_then(|a| a.cell).or_else(window_cell);
    let wrapped_right = tmux == blocks.tmux_detected();
    let env = |k: &str| session.terminal_env(k);
    let protocol = match (wanted, fallback) {
        (Some(p), _) => p,
        (None, _) if !attached || cell.is_none() || !wrapped_right => ProtocolType::Halfblocks,
        (None, Fallback::ProbeThenEnv) if answer.is_none() => env_guess(session),
        (None, Fallback::Probe | Fallback::ProbeThenEnv) if cfg!(sessions) => {
            probe_protocol(answer.and_then(|a| a.protocol), tmux, env)
        }
        (None, _) => env_guess(session),
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

/// Frame pixels per terminal cell under `picker`, across and down, at
/// least one each way: the cell the terminal reported in `answer` where
/// `picker` is for block characters, as the one [`pick_protocol`] falls
/// back to has a stock cell ([`Picker::halfblocks`]); the picker's own
/// otherwise, which [`pick_protocol`] made from the reported cell.
#[must_use]
pub fn cell_size(picker: &Picker, answer: Option<Answer>) -> (f32, f32) {
    let (w, h) = match (picker.protocol_type(), answer.and_then(|a| a.cell)) {
        (ProtocolType::Halfblocks, Some(cell)) => cell,
        _ => {
            let f = picker.font_size();
            (f.width, f.height)
        }
    };
    (f32::from(w.max(1)), f32::from(h.max(1)))
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
    reprobe_with(session, old, graphics, fallback).0
}

/// [`reprobe`], and what the terminal answered if it was asked (its
/// background colour among it).
pub fn reprobe_with(
    session: &Session,
    old: &Picker,
    graphics: Graphics,
    fallback: Fallback,
) -> (Repick, Option<Answer>) {
    let (new, answer) = pick_with(session, graphics, fallback);
    (repick(old, new), answer)
}

/// What the pictures made with `old` need, `new` picked for the terminal
/// attached now.
fn repick(old: &Picker, new: Picker) -> Repick {
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

/// How kitty's pictures reach the terminal ([`kitty_transport`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Transport {
    /// Through a temporary file the terminal reads: it shares our files.
    File,
    /// Inline in the escape codes: over ssh, where the terminal cannot
    /// read our files.
    Inline,
}

/// How kitty's pictures reach the terminal `session` is attached to now:
/// inline when it is reached over ssh (`SSH_CONNECTION`, `SSH_CLIENT` or
/// `SSH_TTY` set and not empty in what it passed on,
/// [`Session::terminal_env`]: it may be reached another way than the one
/// the program started from), through a file otherwise.
pub fn kitty_transport(session: &Session) -> Transport {
    transport_from_env(|k| session.terminal_env(k))
}

/// [`kitty_transport`], from the environment variables (`var`).
pub(crate) fn transport_from_env(var: impl Fn(&str) -> Option<String>) -> Transport {
    let remote = ["SSH_CONNECTION", "SSH_CLIENT", "SSH_TTY"]
        .iter()
        .any(|k| var(k).is_some_and(|v| !v.is_empty()));
    if remote {
        Transport::Inline
    } else {
        Transport::File
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
/// protocol, cell size, background colour and keyboard enhancement. None
/// while nobody is attached, when stdin is not a terminal, when it does
/// not answer within [`PROBE_TIMEOUT`], and where the terminal cannot be
/// asked (a build without sessions). Call it with the terminal set up
/// (raw), before mouse reports are on. Nothing is left reading after it
/// returns.
pub fn query(session: &Session) -> Option<Answer> {
    if !session.attached() {
        return None;
    }
    ask_terminal(session, in_tmux(session))
}

/// [`ask`] the terminal on stdin and stdout, the query wrapped for tmux
/// when `tmux`; what it said of the keyboard (nothing, if it did not
/// answer) is the session's to keep.
#[cfg(sessions)]
fn ask_terminal(session: &Session, tmux: bool) -> Option<Answer> {
    use std::os::fd::AsFd;
    let stdin = std::io::stdin();
    if !rustix::termios::isatty(&stdin) {
        return None;
    }
    let answer = ask(stdin.as_fd(), &mut std::io::stdout(), tmux, PROBE_TIMEOUT)
        .ok()
        .flatten();
    // No answer in time: none to the keyboard's query either, not asked
    // again until the next attach.
    if !tmux {
        session.heard_keyboard(answer.is_some_and(|a| a.keyboard_enhancement));
    }
    answer
}

#[cfg(not(sessions))]
fn ask_terminal(_session: &Session, _tmux: bool) -> Option<Answer> {
    None
}

/// Write the query to `output` and read the terminal's answer from
/// `input`, on this thread, for at most `timeout` ([`crate::ask::ask`]).
/// None if the answer did not end in time. Outside tmux, the keyboard is
/// asked in the same query.
#[cfg(sessions)]
pub(crate) fn ask(
    input: std::os::fd::BorrowedFd<'_>,
    output: &mut impl std::io::Write,
    tmux: bool,
    timeout: Duration,
) -> std::io::Result<Option<Answer>> {
    use ratatui_image::picker::cap_parser::{Parser, QueryStdioOptions, Response};
    let options = QueryStdioOptions {
        timeout,
        terminal_background_color_osc: true,
        ..Default::default()
    };
    let mut query = Parser::query(tmux, options);
    if !tmux {
        // Before the status report it ends with.
        let end = query.rfind("\x1b[5n").unwrap_or(query.len());
        query.insert_str(end, crate::ask::KEYBOARD);
    }
    let mut parser = Parser::new();
    let mut keyboard = crate::ask::KeyboardReply::default();
    let mut answer = Answer::default();
    let ended = crate::ask::ask(input, output, query.as_bytes(), timeout, |b| {
        keyboard.push(b);
        for response in parser.push(char::from(b)) {
            match response {
                Response::Kitty => answer.protocol = Some(ProtocolType::Kitty),
                Response::Sixel => {
                    answer.protocol.get_or_insert(ProtocolType::Sixel);
                }
                Response::CellSize(Some(cell)) => answer.cell = Some(cell),
                Response::Background(r, g, b) => answer.background = Some((r, g, b)),
                _ => {}
            }
        }
    })?;
    answer.keyboard_enhancement = keyboard.heard;
    Ok(ended.then_some(answer))
}

#[cfg(test)]
#[path = "tests/graphics.rs"]
mod tests;
