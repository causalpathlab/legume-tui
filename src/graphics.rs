//! Which image protocol the attached terminal speaks, asked of the
//! terminal itself, on request.
//!
//! A hosted program may be attached from a terminal other than the one it
//! started on, so the answer is asked again after every attach (in the
//! hook [`Session::poll_with`] calls, before the session's modes are on
//! again: a mouse report would land amid the terminal's answer). The
//! query and its reply travel through the relay; a terminal that does not
//! answer in time gets block characters.

use crate::Session;
use ratatui_image::picker::cap_parser::QueryStdioOptions;
use ratatui_image::picker::{Picker, ProtocolType};
use std::time::Duration;

/// How long a terminal has to answer the probe.
pub const PROBE_TIMEOUT: Duration = Duration::from_millis(500);

/// A picker for the terminal `session` is attached to now: block
/// characters while nobody is attached (no terminal to ask) or when the
/// terminal does not answer within [`PROBE_TIMEOUT`]. Call it with the
/// terminal set up (raw), whenever [`Session::take_repaint`] is true, and
/// rebuild the image protocols made with the old one ([`reprobe`] says
/// which need it).
pub fn probe(session: &Session) -> Picker {
    if !session.attached() {
        return Picker::halfblocks();
    }
    let options = QueryStdioOptions {
        timeout: PROBE_TIMEOUT,
        ..Default::default()
    };
    Picker::from_query_stdio_with_options(options).unwrap_or_else(|_| Picker::halfblocks())
}

/// [`probe`], with the protocol `forced` where the user named one: block
/// characters are taken without asking the terminal, another protocol
/// with the font size the terminal reports.
pub fn probe_as(session: &Session, forced: Option<ProtocolType>) -> Picker {
    let mut picker = if forced == Some(ProtocolType::Halfblocks) {
        Picker::halfblocks()
    } else {
        probe(session)
    };
    if let Some(protocol) = forced {
        picker.set_protocol_type(protocol);
    }
    picker
}

/// What the terminal attached now needs of the pictures made for the
/// last one.
#[derive(Clone, Debug)]
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

/// Ask the terminal attached now ([`probe_as`]) and say what the pictures
/// made with `old` need. A program can start from
/// [`Picker::halfblocks`] and take the first answer as for any attach:
/// the first [`Session::take_repaint`] is true.
pub fn reprobe(session: &Session, old: &Picker, forced: Option<ProtocolType>) -> Repick {
    let new = probe_as(session, forced);
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
