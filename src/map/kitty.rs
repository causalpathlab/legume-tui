//! pinto's own ways of putting a map on screen: kitty graphics ([`Kitty`]),
//! coloured block characters ([`cellart`]), and the `--graphics` choice
//! between them and ratatui-image's protocols ([`MapGraphics`]).
//!
//! Kitty graphics show a frame as an image placed over a block of
//! terminal cells. Frames alternate between two image ids. The new frame
//! is placed first and the old one deleted after, so the map never
//! flashes empty between frames.
//!
//! Locally the pixels travel through a temporary file that kitty reads and
//! deletes (`t=t`), so a full-screen frame costs a file write, not tens of
//! MB of escape codes. Each file is made new under a random name, so a
//! link planted in the shared temporary directory is never written
//! through. The terminal reads a file when it gets to the command, which
//! may be frames later, so each image keeps its last two files and only
//! older ones are removed; all go when the map is cleared or the [`Kitty`]
//! dropped. Over ssh the terminal cannot read our files, so pixels are
//! sent inline, zlib-compressed (`o=z`) and base64 encoded in 4 KB chunks. Every command carries `q=2` so kitty never answers into
//! our input stream.
//!
//! Inside tmux, each command is framed for tmux's passthrough
//! (ratatui-image's framing), which tmux forwards to the terminal when its
//! `allow-passthrough` option is on. tmux does not know about the image,
//! so this is best effort; a viewer uses it only when asked for kitty
//! explicitly.
//!
//! Ported from pinto: `Kitty` from `src/view/kitty.rs` and `Graphics`
//! from `src/view/mod.rs` (here [`MapGraphics`]). The frame is a
//! [`Frame`] (pinto's bytes enter through [`Frame::from_srgba`] and come
//! back unchanged), errors are `io::Error`s, the temporary file is
//! named for the tool ([`Kitty::named`]) and made new for each frame
//! rather than at a fixed name, and too few bytes are an error.

pub mod cellart;

use super::raster::Frame;
use crate::graphics::{Graphics, Transport};
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;
use cellart::Glyphs;
use ratatui_image::picker::cap_parser::Parser;
use ratatui_image::picker::ProtocolType;
use std::collections::VecDeque;
use std::io::{self, Write};
use std::path::PathBuf;

/// Image ids, alternated frame to frame.
const IDS: [u32; 2] = [0x7069_0001, 0x7069_0002];

/// Base64 payload bytes per escape sequence (protocol limit is 4096).
const CHUNK: usize = 4096;

/// Files kept per image id for the terminal to read.
const KEEP: usize = 2;

/// Shows frames with kitty's graphics protocol, two images in turn.
#[derive(Debug)]
pub struct Kitty {
    transport: Transport,
    /// Wrap commands for tmux's passthrough.
    tmux: bool,
    /// The tool, in the temporary file's name.
    tool: String,
    /// Index into `IDS` of the image on screen.
    shown: Option<usize>,
    /// The files each id's last [`KEEP`] frames went through, oldest first.
    files: [VecDeque<PathBuf>; 2],
    /// Bytes written for the last frame.
    pub last_bytes: usize,
}

impl Kitty {
    #[must_use]
    pub fn new(transport: Transport) -> Self {
        Kitty {
            transport,
            tmux: false,
            tool: "legume".into(),
            shown: None,
            files: [VecDeque::new(), VecDeque::new()],
            last_bytes: 0,
        }
    }

    #[must_use]
    pub fn transport(&self) -> Transport {
        self.transport
    }

    /// Wrap commands for tmux's passthrough, or not.
    #[must_use]
    pub fn through_tmux(mut self, on: bool) -> Self {
        self.tmux = on;
        self
    }

    /// Whether commands go through tmux's passthrough.
    #[must_use]
    pub fn via_tmux(&self) -> bool {
        self.tmux
    }

    /// Name the temporary files for `tool` (`legume` by default):
    /// `tty-graphics-protocol-{tool}-{random}.rgba`.
    #[must_use]
    pub fn named(mut self, tool: &str) -> Self {
        self.tool = tool.into();
        self
    }

    /// Write one graphics command, `ESC _ G {parts…} ESC \`, framed for
    /// tmux's passthrough when in tmux. The bodies are plain ASCII (keys and
    /// base64), so nothing inside needs escaping.
    fn command(&self, out: &mut impl Write, parts: &[&[u8]]) -> io::Result<()> {
        let (start, esc, end) = Parser::tmux_start_escape_end(self.tmux);
        write!(out, "{start}{esc}_G")?;
        for part in parts {
            out.write_all(part)?;
        }
        write!(out, "{esc}\\{end}")
    }

    /// Place `frame` with its top-left at cell `(col, row)`, scaled to
    /// `cols × rows` cells, replacing the previous frame.
    pub fn show(
        &mut self,
        out: &mut impl Write,
        frame: &Frame,
        at: (u16, u16),
        size: (u16, u16),
    ) -> io::Result<()> {
        let image = frame.to_image();
        self.show_rgba(out, (frame.w, frame.h), image.as_raw(), at, size)
    }

    /// [`Kitty::show`] for `w × h` sRGBA bytes, `rgba`, as a renderer
    /// writing bytes has them.
    ///
    /// # Errors
    /// `InvalidInput` when `rgba` holds fewer than `w × h × 4` bytes, and
    /// any error writing the file or the commands.
    pub fn show_rgba(
        &mut self,
        out: &mut impl Write,
        (w, h): (usize, usize),
        rgba: &[u8],
        (col, row): (u16, u16),
        (cols, rows): (u16, u16),
    ) -> io::Result<()> {
        let Some(rgba) = w
            .checked_mul(h)
            .and_then(|n| n.checked_mul(4))
            .and_then(|n| rgba.get(..n))
        else {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "{w} × {h} frame needs {w} × {h} × 4 bytes, got {}",
                    rgba.len()
                ),
            ));
        };
        let slot = self.shown.map_or(0, |s| 1 - s);
        let id = IDS[slot];
        // 1-based cursor position; kitty places the image at the cursor.
        write!(out, "\x1b7\x1b[{};{}H", row + 1, col + 1)?;
        let keys = format!("a=T,f=32,s={w},v={h},i={id},p=1,c={cols},r={rows},C=1,q=2");
        self.last_bytes = match self.transport {
            // Inline, compressed.
            Transport::Inline => {
                let mut z = flate2::write::ZlibEncoder::new(
                    Vec::with_capacity(rgba.len() / 4),
                    flate2::Compression::fast(),
                );
                z.write_all(rgba)?;
                let payload = B64.encode(z.finish()?);
                let chunks: Vec<&[u8]> = payload.as_bytes().chunks(CHUNK).collect();
                for (i, chunk) in chunks.iter().enumerate() {
                    let more = u8::from(i + 1 < chunks.len());
                    let head = if i == 0 {
                        format!("{keys},t=d,o=z,m={more};")
                    } else {
                        format!("m={more};")
                    };
                    self.command(out, &[head.as_bytes(), chunk])?;
                }
                payload.len()
            }
            // Through a temporary file: the terminal shares our files.
            _ => {
                let path = self.write_file(slot, rgba)?;
                let payload = B64.encode(path.to_string_lossy().as_bytes());
                self.command(out, &[format!("{keys},t=t;{payload}").as_bytes()])?;
                rgba.len()
            }
        };
        if let Some(old) = self.shown {
            self.command(out, &[format!("a=d,d=I,i={},q=2", IDS[old]).as_bytes()])?;
        }
        // Restore the cursor saved above.
        write!(out, "\x1b8")?;
        out.flush()?;
        self.shown = Some(slot);
        Ok(())
    }

    /// Remove our images from the screen and free them.
    pub fn clear(&mut self, out: &mut impl Write) -> io::Result<()> {
        for id in IDS {
            self.command(out, &[format!("a=d,d=I,i={id},q=2").as_bytes()])?;
        }
        self.shown = None;
        out.flush()?;
        self.remove_files();
        for files in &mut self.files {
            files.clear();
        }
        Ok(())
    }

    /// Remove the files the pixels went through. Kitty deletes each once
    /// read; one it never read (a terminal that ignored the command, or
    /// none attached to read it) would otherwise stay behind.
    pub fn remove_files(&self) {
        for path in self.files.iter().flatten() {
            let _ = std::fs::remove_file(path);
        }
    }

    /// Write `rgba` to a new file for `slot`'s image, removing the files
    /// it had before its last [`KEEP`] − 1: the terminal may not yet have
    /// read those. Kitty only deletes files whose name says they are for
    /// it. The file is created new (`O_EXCL`) under a random name, so
    /// nothing already at a name is followed or truncated.
    fn write_file(&mut self, slot: usize, rgba: &[u8]) -> io::Result<PathBuf> {
        while self.files[slot].len() >= KEEP {
            if let Some(old) = self.files[slot].pop_front() {
                let _ = std::fs::remove_file(old);
            }
        }
        let mut file = tempfile::Builder::new()
            .prefix(&format!("tty-graphics-protocol-{}-", self.tool))
            .suffix(".rgba")
            .tempfile_in(std::env::temp_dir())?;
        file.write_all(rgba)?;
        let (_, path) = file.keep().map_err(|e| e.error)?;
        self.files[slot].push_back(path.clone());
        Ok(path)
    }
}

/// A Kitty replaced or abandoned on an error takes its files with it.
impl Drop for Kitty {
    fn drop(&mut self) {
        self.remove_files();
    }
}

/// How a map is drawn in the terminal (`--graphics`): ratatui-image's
/// protocols, kitty graphics of the map's own ([`Kitty`]), or coloured
/// block characters ([`cellart`]).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "clap", clap(rename_all = "kebab-case"))]
#[non_exhaustive]
pub enum MapGraphics {
    /// Ask the terminal; quadrants when it has no graphics protocol, and
    /// inside tmux.
    #[default]
    Auto,
    /// Kitty graphics; pixels go through a temp file, or inline when over
    /// ssh (kitty, Ghostty, WezTerm).
    Kitty,
    /// Kitty graphics, always inline (in tmux, through passthrough; best
    /// effort).
    KittyInline,
    /// Sixel graphics (iTerm2, WezTerm, foot, xterm).
    Sixel,
    /// iTerm2 inline images.
    Iterm2,
    /// Block characters, 2×2 pixels per character; any terminal.
    Quadrants,
    /// Block characters with boundaries at eighths of a character: finer
    /// shapes, mixed colours averaged.
    Symbols,
    /// Half-blocks, 1×2 pixels per character.
    Blocks,
}

impl MapGraphics {
    /// The protocol to ask [`crate::graphics::pick_protocol`] for: None
    /// for auto, and half-blocks for block characters, so the terminal is
    /// still asked for its cell's shape and its background.
    #[must_use]
    pub fn protocol(self) -> Option<ProtocolType> {
        match self {
            Self::Auto => None,
            Self::Kitty | Self::KittyInline => Some(ProtocolType::Kitty),
            Self::Sixel => Some(ProtocolType::Sixel),
            Self::Iterm2 => Some(ProtocolType::Iterm2),
            Self::Quadrants | Self::Symbols | Self::Blocks => Some(ProtocolType::Halfblocks),
        }
    }

    /// The block characters named, if any.
    #[must_use]
    pub fn glyphs(self) -> Option<Glyphs> {
        match self {
            Self::Quadrants => Some(Glyphs::Quadrants),
            Self::Symbols => Some(Glyphs::Symbols),
            Self::Blocks => Some(Glyphs::HalfBlocks),
            _ => None,
        }
    }
}

/// The crate's own choice of protocol where there is one; None for
/// quadrants and symbols, which only [`cellart`] draws.
impl From<MapGraphics> for Option<Graphics> {
    fn from(g: MapGraphics) -> Self {
        match g {
            MapGraphics::Auto => Some(Graphics::Auto),
            MapGraphics::Kitty | MapGraphics::KittyInline => Some(Graphics::Kitty),
            MapGraphics::Sixel => Some(Graphics::Sixel),
            MapGraphics::Iterm2 => Some(Graphics::Iterm2),
            MapGraphics::Blocks => Some(Graphics::Blocks),
            MapGraphics::Quadrants | MapGraphics::Symbols => None,
        }
    }
}

#[cfg(test)]
#[path = "../tests/kitty.rs"]
mod tests;
