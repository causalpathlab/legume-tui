//! Keys drawn into the frame's top-left corner: a legend of groups, and a
//! ramp under its title for an activity view. Each returns the area it
//! covers, for labels to keep clear of.
//!
//! Ported from senna (`src/view/render.rs`: `draw_legend`,
//! `draw_ramp_key`), with lupin's `+k more` line past a number of rows
//! (`src/figures/mod.rs` `legend_svg`). The ramp key's title ink is a
//! parameter here, not senna's constant.

use super::colour::Lin;
use super::raster::{blend_round, Canvas, Frame};
use super::text::{self, Font};

/// A key in the top-left corner: a dot in the point colour and the name in
/// the label ink, one per line, no frame. With `max_rows` set to `(m, ink)`
/// and more than `m` entries, the first `m - 1` are listed and a last line
/// in `ink` counts the rest (`+k more`). Returns the area it covers, so map
/// labels can keep clear of it.
pub fn draw_legend(
    frame: &mut Frame,
    entries: &[(String, Lin, Lin)],
    font: Font,
    max_rows: Option<(usize, Lin)>,
) -> Option<[f32; 4]> {
    if entries.is_empty() || max_rows.is_some_and(|(m, _)| m == 0) {
        return None;
    }
    let (shown, more) = match max_rows {
        Some((m, ink)) if entries.len() > m => (
            &entries[..m - 1],
            Some((format!("+{} more", entries.len() - (m - 1)), ink)),
        ),
        _ => (entries, None),
    };
    let bg = frame.background();
    let lh = font.line_height() as i32;
    let r = lh as f32 * 0.22;
    let (x0, mut y) = (lh, lh / 2);
    let text_x = x0 + lh / 2 + 2;
    let widest = shown
        .iter()
        .map(|(n, ..)| font.width(n))
        .chain(more.iter().map(|(n, _)| font.width(n)))
        .max()
        .unwrap_or(0) as i32;
    let rows = shown.len() + usize::from(more.is_some());
    let area = [
        0.0,
        0.0,
        (text_x + widest + lh / 2) as f32,
        (y + lh * rows as i32 + lh / 2) as f32,
    ];
    for (name, dot, ink) in shown {
        if y + lh > frame.h as i32 {
            return Some(area);
        }
        let (cx, cy) = (x0 as f32, y as f32 + 0.5 * lh as f32);
        blend_round(frame, cx, cy, r + 1.0, *dot, |d| r + 0.5 - d);
        text::draw(frame, font, name, text_x, y, *ink, bg);
        y += lh;
    }
    if let Some((line, ink)) = more {
        if y + lh <= frame.h as i32 {
            text::draw(frame, font, &line, text_x, y, ink, bg);
        }
    }
    Some(area)
}

/// A key for an activity view in the top-left corner: the title in `ink`,
/// then the ramp as a thin bar. Returns the area it covers.
pub fn draw_ramp_key(
    frame: &mut Frame,
    title: &str,
    ramp: &[Lin],
    font: Font,
    ink: Lin,
) -> [f32; 4] {
    let bg = frame.background();
    let lh = font.line_height() as i32;
    let (x0, y0) = (lh / 2, lh / 2);
    text::draw(frame, font, title, x0, y0, ink, bg);
    let (bw, bh) = ((8 * lh).max(font.width(title) as i32 / 2), (lh / 4).max(3));
    let by = y0 + lh + 2;
    for dx in 0..bw {
        let k = (dx as usize * (ramp.len() - 1)) / (bw as usize - 1).max(1);
        for dy in 0..bh {
            let (x, y) = ((x0 + dx) as usize, (by + dy) as usize);
            if x < frame.w && y < frame.h {
                frame.blend(x, y, ramp[k], 1.0);
            }
        }
    }
    [
        0.0,
        0.0,
        (x0 + bw.max(font.width(title) as i32) + lh / 2) as f32,
        (by + bh + lh / 2) as f32,
    ]
}

#[cfg(test)]
#[path = "../tests/legend.rs"]
mod tests;
