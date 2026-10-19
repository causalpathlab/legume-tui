//! What a host and its clients say over the socket: frames of a tag byte,
//! a big-endian `u32` length, then that many bytes.
//!
//! A connection opens with one of [`HELLO`] (attach), [`INFO`], [`KILL`]
//! or [`WATCH`]. An attaching client then sends [`READY`] once its
//! terminal is raw, and from there [`INPUT`] and [`RESIZE`]; the host
//! answers with [`SCREEN`] and [`OUTPUT`] and, at the end, [`EXIT`] or
//! [`ELSEWHERE`].
//!
//! Frames go out with `send(MSG_NOSIGNAL)` (`SO_NOSIGPIPE` on macOS): a
//! peer that is gone is an error to handle, never a SIGPIPE, whatever the
//! tool set SIGPIPE to.

use std::io::{self, Read};
use std::os::unix::net::UnixStream;

/// Bumped when a frame changes meaning; a host refuses other versions.
pub(crate) const VERSION: u8 = 5;

// Client to host.
/// Attach: the version, the terminal's [`Size`], then what of its
/// environment says which terminal it is ([`encode_env`]).
pub(crate) const HELLO: u8 = 1;
/// The terminal is raw: output and the repaint request may start.
pub(crate) const READY: u8 = 2;
/// Bytes typed, for the program.
pub(crate) const INPUT: u8 = 3;
/// The terminal changed size: a [`Size`].
pub(crate) const RESIZE: u8 = 4;
/// Ask for the program's pid and whether someone is attached.
pub(crate) const INFO: u8 = 5;
/// Ask the program to stop: one byte, 1 to kill it outright.
pub(crate) const KILL: u8 = 6;
/// Follow the session's attach state: the host answers with no frames,
/// only single bytes, [`crate::host::ATTACHED`] or
/// [`crate::host::DETACHED`], the current state first. For a TUI that a
/// hosted program started on the same terminal.
pub(crate) const WATCH: u8 = 7;

// Host to client.
/// Bytes the program wrote.
pub(crate) const OUTPUT: u8 = 16;
/// The program ended: its status as an `i32`.
pub(crate) const EXIT: u8 = 17;
/// Answer to [`INFO`]: pid (`u32`), 1 if attached, the start time in
/// seconds since the epoch (`u64`), then the command line.
pub(crate) const INFO_REPLY: u8 = 18;
/// Another client attached; this one is let go.
pub(crate) const ELSEWHERE: u8 = 19;
/// Done as asked.
pub(crate) const OK: u8 = 20;
/// Before a client's first output when it joins a running stream: one
/// byte, 1 if the program's screen is the alternate one there (the client
/// enters it too). A first client given the stream from its start is not
/// sent one.
pub(crate) const SCREEN: u8 = 21;

/// The longest frame read: anything longer is not ours.
const MAX: usize = 1 << 22;

/// A frame's head: its tag and its body's length.
pub(crate) const HEAD: usize = 5;

/// Write one frame, head and body in one buffer, so a frame is never
/// split by another writer between the two.
pub(crate) fn write_frame(s: &UnixStream, tag: u8, body: &[u8]) -> io::Result<()> {
    send_all(s, &frame(tag, body))
}

/// One frame as it goes on the wire.
pub(crate) fn frame(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut buf = begin(body.len());
    buf.extend_from_slice(body);
    seal(tag, buf)
}

/// A frame to write a body of up to `len` bytes into, after the room
/// kept for its head.
pub(crate) fn begin(len: usize) -> Vec<u8> {
    let mut buf = Vec::with_capacity(HEAD + len);
    buf.resize(HEAD, 0);
    buf
}

/// A frame from [`begin`], its body written: its head filled in.
pub(crate) fn seal(tag: u8, mut buf: Vec<u8>) -> Vec<u8> {
    let len = (buf.len() - HEAD) as u32;
    buf[0] = tag;
    buf[1..HEAD].copy_from_slice(&len.to_be_bytes());
    buf
}

/// What a client says [`HELLO`] with: the version, its terminal's size
/// and `env`.
pub(crate) fn hello_body(size: Size, env: &[(String, String)]) -> Vec<u8> {
    let mut body = vec![VERSION];
    body.extend_from_slice(&size.encode());
    body.extend_from_slice(&encode_env(env));
    body
}

/// What a [`HELLO`] says of the terminal's environment.
pub(crate) fn hello_env(body: &[u8]) -> Vec<(String, String)> {
    decode_env(body.get(1 + 8..).unwrap_or_default())
}

/// This process's values of [`crate::TERMINAL_VARS`], those it has.
pub(crate) fn terminal_env() -> Vec<(String, String)> {
    crate::TERMINAL_VARS
        .iter()
        .filter_map(|&k| Some((k.to_string(), std::env::var(k).ok()?)))
        .collect()
}

/// Environment variables as `NAME=value`, each ended by a NUL.
pub(crate) fn encode_env(vars: &[(String, String)]) -> Vec<u8> {
    let mut out = Vec::new();
    for (k, v) in vars {
        out.extend_from_slice(format!("{k}={v}").as_bytes());
        out.push(0);
    }
    out
}

/// What [`encode_env`] wrote; an entry not of that form is left out.
pub(crate) fn decode_env(bytes: &[u8]) -> Vec<(String, String)> {
    bytes
        .split(|&b| b == 0)
        .filter_map(|e| {
            let (k, v) = std::str::from_utf8(e).ok()?.split_once('=')?;
            (!k.is_empty()).then(|| (k.to_string(), v.to_string()))
        })
        .collect()
}

/// The send flags that make a peer that is gone an error, not a SIGPIPE.
fn no_sigpipe(s: &UnixStream) -> io::Result<rustix::net::SendFlags> {
    #[cfg(any(target_vendor = "apple", target_os = "freebsd", target_os = "netbsd"))]
    {
        rustix::net::sockopt::set_socket_nosigpipe(s, true)?;
        Ok(rustix::net::SendFlags::empty())
    }
    #[cfg(not(any(target_vendor = "apple", target_os = "freebsd", target_os = "netbsd")))]
    {
        let _ = s;
        Ok(rustix::net::SendFlags::NOSIGNAL)
    }
}

/// Send what of `buf` the socket takes now, without waiting; how much
/// that was. [`io::ErrorKind::WouldBlock`] when it takes nothing.
pub(crate) fn send_now(s: &UnixStream, buf: &[u8]) -> io::Result<usize> {
    use rustix::net::{send, SendFlags};
    let flags = no_sigpipe(s)? | SendFlags::DONTWAIT;
    loop {
        match send(s, buf, flags) {
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => return Err(io::ErrorKind::WouldBlock.into()),
            done => return Ok(done?),
        }
    }
}

/// Send all of `buf`, with no SIGPIPE if the peer is gone. A write
/// timeout on `s` holds as for a plain write.
pub(crate) fn send_all(s: &UnixStream, mut buf: &[u8]) -> io::Result<()> {
    use rustix::net::send;
    let flags = no_sigpipe(s)?;
    while !buf.is_empty() {
        match send(s, buf, flags) {
            Ok(0) => return Err(io::ErrorKind::WriteZero.into()),
            Ok(n) => buf = &buf[n..],
            Err(rustix::io::Errno::INTR) => {}
            Err(rustix::io::Errno::AGAIN) => return Err(io::ErrorKind::WouldBlock.into()),
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}

/// Read one frame; None at the end of the stream.
pub(crate) fn read_frame(r: &mut impl Read) -> io::Result<Option<(u8, Vec<u8>)>> {
    let mut head = [0u8; HEAD];
    match r.read_exact(&mut head) {
        Ok(()) => {}
        Err(e) if e.kind() == io::ErrorKind::UnexpectedEof => return Ok(None),
        Err(e) => return Err(e),
    }
    let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
    if len > MAX {
        return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
    }
    let mut body = vec![0u8; len];
    r.read_exact(&mut body)?;
    Ok(Some((head[0], body)))
}

/// Frames read from a socket as they come, without waiting for the rest of
/// one: each is taken once it is whole.
#[derive(Default)]
pub(crate) struct Inbox {
    buf: Vec<u8>,
    /// Where the frames not taken yet start.
    at: usize,
}

impl Inbox {
    /// Read what `s` has now, without waiting; false at the end of the
    /// stream.
    pub fn fill(&mut self, s: &UnixStream) -> io::Result<bool> {
        use rustix::net::{recv, RecvFlags};
        if self.at > 0 {
            self.buf.drain(..self.at);
            self.at = 0;
        }
        self.buf.reserve(1 << 16);
        loop {
            match recv(
                s,
                rustix::buffer::spare_capacity(&mut self.buf),
                RecvFlags::DONTWAIT,
            ) {
                Ok((n, _)) => return Ok(n > 0),
                Err(rustix::io::Errno::INTR) => {}
                Err(rustix::io::Errno::AGAIN) => return Ok(true),
                Err(e) => return Err(e.into()),
            }
        }
    }

    /// The next whole frame read, if there is one.
    pub fn next(&mut self) -> io::Result<Option<(u8, &[u8])>> {
        let rest = &self.buf[self.at..];
        let Some(head) = rest.get(..HEAD) else {
            return Ok(None);
        };
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > MAX {
            return Err(io::Error::new(io::ErrorKind::InvalidData, "frame too long"));
        }
        if rest.len() < HEAD + len {
            return Ok(None);
        }
        let start = self.at + HEAD;
        self.at = start + len;
        Ok(Some((
            self.buf[start - HEAD],
            &self.buf[start..start + len],
        )))
    }
}

/// A terminal's size: cells, and pixels where the terminal reports them
/// (image scaling and the graphics probe need both).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Size {
    /// Rows of cells.
    pub rows: u16,
    /// Columns of cells.
    pub cols: u16,
    /// Width in pixels; 0 when the terminal does not say.
    pub width: u16,
    /// Height in pixels; 0 when the terminal does not say.
    pub height: u16,
}

impl Size {
    /// The size of the terminal on `fd`.
    pub fn of(fd: impl std::os::fd::AsFd) -> io::Result<Self> {
        let w = rustix::termios::tcgetwinsize(fd)?;
        Ok(Self {
            rows: w.ws_row,
            cols: w.ws_col,
            width: w.ws_xpixel,
            height: w.ws_ypixel,
        })
    }

    /// Give the terminal on `fd` this size; the program on it hears of a
    /// change by SIGWINCH.
    pub(crate) fn apply(self, fd: impl std::os::fd::AsFd) -> io::Result<()> {
        rustix::termios::tcsetwinsize(
            fd,
            rustix::termios::Winsize {
                ws_row: self.rows,
                ws_col: self.cols,
                ws_xpixel: self.width,
                ws_ypixel: self.height,
            },
        )?;
        Ok(())
    }

    /// A terminal that reports no size is taken as 80 by 24.
    pub(crate) fn or_default(self) -> Self {
        if self.rows == 0 || self.cols == 0 {
            Self {
                rows: 24,
                cols: 80,
                ..self
            }
        } else {
            self
        }
    }

    pub(crate) fn encode(self) -> [u8; 8] {
        let mut b = [0u8; 8];
        for (i, v) in [self.rows, self.cols, self.width, self.height]
            .into_iter()
            .enumerate()
        {
            b[2 * i..2 * i + 2].copy_from_slice(&v.to_be_bytes());
        }
        b
    }

    pub(crate) fn decode(b: &[u8]) -> Option<Self> {
        let v = |i: usize| Some(u16::from_be_bytes([*b.get(2 * i)?, *b.get(2 * i + 1)?]));
        Some(Self {
            rows: v(0)?,
            cols: v(1)?,
            width: v(2)?,
            height: v(3)?,
        })
    }

    /// As an environment value, `rows,cols,width,height`.
    pub(crate) fn to_env(self) -> String {
        format!("{},{},{},{}", self.rows, self.cols, self.width, self.height)
    }

    pub(crate) fn from_env(s: &str) -> Option<Self> {
        let mut v = s.split(',').map(|x| x.trim().parse::<u16>().ok());
        let size = Self {
            rows: v.next()??,
            cols: v.next()??,
            width: v.next()??,
            height: v.next()??,
        };
        v.next().is_none().then_some(size)
    }
}
