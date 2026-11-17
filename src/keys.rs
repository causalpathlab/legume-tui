//! The keys every list and view of the tools takes alike: moving a
//! selection, the wheel as arrows, the keys a view never sees, and the
//! clipboard.
//!
//! senna's `list_delta` and `list_move` (`src/tui/mod.rs`) and lupin's
//! `step` (`src/tui/app.rs`) in one, with [`PAGE`] rows a page; faba's
//! `wheel_key`, `is_click_or_wheel` and stray keys (`src/tui.rs`); and the
//! OSC 52 `copy` of senna, pinto and data-beans.

use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use std::io::{self, Write};

/// Rows a page key moves in a list.
pub const PAGE: usize = 10;

/// How far a list key moves: ↑↓ (and k/j with `vi`) by one, PgUp/PgDn by
/// [`PAGE`], Home/End to either end (as far as any list goes). `None` for
/// any other key, or one held with Ctrl or Alt, so the caller can give it
/// its own meaning.
#[must_use]
pub fn list_delta(k: &KeyEvent, vi: bool) -> Option<isize> {
    if k.modifiers
        .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let page = PAGE as isize;
    Some(match k.code {
        KeyCode::Up => -1,
        KeyCode::Down => 1,
        KeyCode::Char('k') if vi => -1,
        KeyCode::Char('j') if vi => 1,
        KeyCode::PageUp => -page,
        KeyCode::PageDown => page,
        KeyCode::Home => isize::MIN / 2,
        KeyCode::End => isize::MAX / 2,
        _ => return None,
    })
}

/// Item `at` moved by `d`, kept within `0..=last`.
#[must_use]
pub fn moved(at: usize, d: isize, last: usize) -> usize {
    at.saturating_add_signed(d).min(last)
}

/// The row [`list_delta`] leads to from `row` in a list of `len` rows;
/// `None` when `k` moves nothing.
#[must_use]
pub fn list_move(k: &KeyEvent, row: usize, len: usize, vi: bool) -> Option<usize> {
    list_delta(k, vi).map(|d| moved(row, d, len.saturating_sub(1)))
}

/// Move `sel` in a list of `len` rows by `k`, as [`list_move`] does;
/// `false`, `sel` as it was, for a key that moves nothing.
pub fn list_step(sel: &mut usize, len: usize, k: &KeyEvent, vi: bool) -> bool {
    match list_move(k, *sel, len, vi) {
        Some(to) => {
            *sel = to;
            true
        }
        None => false,
    }
}

/// The ↑ or ↓ a turn of the wheel stands for; `None` for other mouse
/// events.
#[must_use]
pub fn wheel_key(m: &MouseEvent) -> Option<KeyEvent> {
    let code = match m.kind {
        MouseEventKind::ScrollUp => KeyCode::Up,
        MouseEventKind::ScrollDown => KeyCode::Down,
        _ => return None,
    };
    Some(KeyEvent::new(code, KeyModifiers::NONE))
}

/// Whether a mouse event is one a view acts on: a left click or a turn of
/// the wheel.
#[must_use]
pub fn is_click_or_wheel(m: &MouseEvent) -> bool {
    matches!(
        m.kind,
        MouseEventKind::Down(MouseButton::Left)
            | MouseEventKind::ScrollUp
            | MouseEventKind::ScrollDown
    )
}

/// Whether `k`, as the terminal sent it, is held with what makes it
/// another key (Ctrl, Alt, Super, Meta, Hyper, or Shift on Enter) and is
/// not the run key ([`crate::is_run_key`]): a view that drops these never
/// takes Shift+Enter or Ctrl+Enter for Enter, nor Ctrl+S for `s`.
#[must_use]
pub fn is_stray(k: &KeyEvent) -> bool {
    let chord = KeyModifiers::CONTROL
        | KeyModifiers::ALT
        | KeyModifiers::SUPER
        | KeyModifiers::META
        | KeyModifiers::HYPER;
    let held = k.modifiers.intersects(chord)
        || (k.code == KeyCode::Enter && k.modifiers.contains(KeyModifiers::SHIFT));
    held && !crate::is_run_key(k)
}

/// Put `text` on the clipboard: through `pbcopy` where there is one,
/// else by asking the terminal (OSC 52) when one is `attached` (a
/// session's output reaches it through the host). Says which: `pbcopy`
/// or `terminal clipboard`. Whether a terminal took it cannot be known:
/// kitty, iTerm2, WezTerm and others do (tmux when its `set-clipboard`
/// lets it through); the rest ignore it.
pub fn copy(text: &str, attached: bool) -> io::Result<&'static str> {
    if let Ok(mut c) = std::process::Command::new("pbcopy")
        .stdin(std::process::Stdio::piped())
        .spawn()
    {
        if let Some(mut stdin) = c.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        if c.wait()?.success() {
            return Ok("pbcopy");
        }
    }
    if !attached {
        return Err(io::Error::new(
            io::ErrorKind::NotConnected,
            "no terminal attached",
        ));
    }
    let mut out = io::stdout();
    out.write_all(osc52(text).as_bytes())?;
    out.flush()?;
    Ok("terminal clipboard")
}

/// The OSC 52 sequence that puts `text` on the terminal's clipboard.
fn osc52(text: &str) -> String {
    format!("\x1b]52;c;{}\x07", base64(text.as_bytes()))
}

/// `bytes` in standard base64, padded.
fn base64(bytes: &[u8]) -> String {
    const ABC: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |n, &b| n << 8 | u32::from(b)) << (8 * (3 - chunk.len()));
        for k in 0..4 {
            out.push(if k <= chunk.len() {
                char::from(ABC[(n >> (18 - 6 * k) & 63) as usize])
            } else {
                '='
            });
        }
    }
    out
}

#[cfg(test)]
#[path = "tests/keys.rs"]
mod tests;
