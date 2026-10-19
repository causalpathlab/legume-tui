//! Anti-aliased text drawn straight into the raster, so labels sit on the map
//! under every graphics protocol, block-character fallback included.
//!
//! Glyphs come pre-rasterized from Noto Sans Mono. Each label gets a halo in
//! the page colour, which keeps it legible over dense points without a box.
//! Text can instead be kept as [`TextRun`]s, for a vector page that sets it
//! in a real font over the raster.
//!
//! Ported from senna (`src/view/text.rs`), unchanged. [`draw_tiny`] is
//! pinto's 5×7 scale-bar font (`src/view/scalebar.rs`), for a label burnt
//! into a picture with no font at all.

use super::colour::Lin;
use super::raster::{Canvas, Frame};
use noto_sans_mono_bitmap::{get_raster, get_raster_width, FontWeight, RasterHeight};

#[derive(Clone, Copy, Debug)]
pub struct Font {
    pub height: RasterHeight,
    pub weight: FontWeight,
    /// Past the largest raster, glyphs are resampled up by this factor.
    pub scale: f32,
}

impl Font {
    /// The largest raster size that fits `px` (16 at the least); past 32,
    /// the 32 raster scaled up to `px`.
    #[must_use]
    pub fn for_cell_height(px: f32, bold: bool) -> Self {
        let height = if px >= 32.0 {
            RasterHeight::Size32
        } else if px >= 24.0 {
            RasterHeight::Size24
        } else if px >= 20.0 {
            RasterHeight::Size20
        } else {
            RasterHeight::Size16
        };
        let weight = if bold {
            FontWeight::Bold
        } else {
            FontWeight::Regular
        };
        let scale = (px / 32.0).clamp(1.0, 4.0);
        Self {
            height,
            weight,
            scale,
        }
    }

    #[must_use]
    pub fn advance(&self) -> usize {
        (get_raster_width(self.weight, self.height) as f32 * self.scale).round() as usize
    }

    #[must_use]
    pub fn line_height(&self) -> usize {
        (self.height.val() as f32 * self.scale).round() as usize
    }

    /// Pixel width of `s`.
    #[must_use]
    pub fn width(&self, s: &str) -> usize {
        s.chars().count() * self.advance()
    }

    #[must_use]
    pub fn is_bold(&self) -> bool {
        matches!(self.weight, FontWeight::Bold)
    }
}

/// A line of text kept as text rather than drawn: where its box is on the
/// raster, and how it looks. A vector export sets it in a real font.
#[derive(Clone, Debug)]
pub struct TextRun {
    pub text: String,
    /// Top left of the box the raster font would fill, in pixels.
    pub x: f32,
    pub y: f32,
    /// That box's width and line height, in pixels.
    pub w: f32,
    pub line: f32,
    pub bold: bool,
    /// Centred on its box (a map label) rather than starting at its left.
    pub centred: bool,
    /// Read bottom to top, turned a quarter left: `x` is then the left of
    /// the column the text stands in and `y` its bottom.
    pub vertical: bool,
    pub ink: Lin,
    pub bg: Lin,
}

/// Draw `s` with its top-left corner at `(x, y)`, over a `halo` of the page
/// colour `bg`.
pub fn draw<C: Canvas>(canvas: &mut C, font: Font, s: &str, x: i32, y: i32, ink: Lin, bg: Lin) {
    draw_aligned(canvas, font, s, (x, y), false, ink, bg);
}

/// `draw`, saying whether the text is `centred` on its box: the pixels are
/// the same, but kept text stays centred when set in a font of other widths.
pub fn draw_aligned<C: Canvas>(
    canvas: &mut C,
    font: Font,
    s: &str,
    (x, y): (i32, i32),
    centred: bool,
    ink: Lin,
    bg: Lin,
) {
    if let Some(sink) = canvas.text_sink() {
        sink.push(TextRun {
            text: s.to_string(),
            x: x as f32,
            y: y as f32,
            w: font.width(s) as f32,
            line: font.line_height() as f32,
            bold: font.is_bold(),
            centred,
            vertical: false,
            ink,
            bg,
        });
        return;
    }
    let adv = font.advance() as i32;
    stamp(
        canvas,
        font,
        s,
        |k, col, row| (x + k * adv + col, y + row),
        ink,
        bg,
    );
}

/// `s` read bottom to top, turned a quarter left, standing in a column
/// whose left is `x` and whose bottom is `y` (a heatmap's column names).
pub fn draw_vertical<C: Canvas>(
    canvas: &mut C,
    font: Font,
    s: &str,
    (x, y): (i32, i32),
    ink: Lin,
    bg: Lin,
) {
    if let Some(sink) = canvas.text_sink() {
        sink.push(TextRun {
            text: s.to_string(),
            x: x as f32,
            y: y as f32,
            w: font.width(s) as f32,
            line: font.line_height() as f32,
            bold: font.is_bold(),
            centred: false,
            vertical: true,
            ink,
            bg,
        });
        return;
    }
    let adv = font.advance() as i32;
    stamp(
        canvas,
        font,
        s,
        |k, col, row| (x + row, y - 1 - k * adv - col),
        ink,
        bg,
    );
}

/// Blend the glyphs of `s` into `canvas`, glyph `k`'s pixel `(col, row)`
/// going where `place` puts it: a halo in `bg` first, then the ink.
fn stamp<C: Canvas>(
    canvas: &mut C,
    font: Font,
    s: &str,
    place: impl Fn(i32, i32, i32) -> (i32, i32),
    ink: Lin,
    bg: Lin,
) {
    let (w, h) = canvas.size();
    let glyphs: Vec<Option<Vec<Vec<u8>>>> = s
        .chars()
        .map(|ch| {
            get_raster(ch, font.weight, font.height)
                .or_else(|| get_raster('?', font.weight, font.height))
                .map(|g| resample(g.raster(), font.scale))
        })
        .collect();

    // Halo: the glyph coverage dilated by 2 px, laid down in the page colour
    // first so the ink never has to fight the points beneath it.
    let halo = (2.0 * font.scale).round() as i32;
    for pass in 0..2 {
        for (k, g) in glyphs.iter().enumerate() {
            let Some(g) = g else { continue };
            for (row, line) in g.iter().enumerate() {
                for (col, &v) in line.iter().enumerate() {
                    if v < 24 {
                        continue;
                    }
                    let a = f32::from(v) / 255.0;
                    let (px, py) = place(k as i32, col as i32, row as i32);
                    if pass == 0 {
                        for dy in -halo..=halo {
                            for dx in -halo..=halo {
                                if dx * dx + dy * dy > halo * halo + 1 {
                                    continue;
                                }
                                let (qx, qy) = (px + dx, py + dy);
                                if qx >= 0 && qy >= 0 && (qx as usize) < w && (qy as usize) < h {
                                    canvas.blend(qx as usize, qy as usize, bg, 0.8 * a);
                                }
                            }
                        }
                    } else if px >= 0 && py >= 0 && (px as usize) < w && (py as usize) < h {
                        canvas.blend(px as usize, py as usize, ink, a);
                    }
                }
            }
        }
    }
}

/// A glyph's coverage scaled by `k` (bilinear); as it is when `k` is 1.
fn resample<R: AsRef<[u8]>>(raster: &[R], k: f32) -> Vec<Vec<u8>> {
    let src: Vec<&[u8]> = raster.iter().map(AsRef::as_ref).collect();
    if k <= 1.0 || src.is_empty() {
        return src.iter().map(|r| r.to_vec()).collect();
    }
    let (sw, sh) = (src[0].len(), src.len());
    let (dw, dh) = (
        (sw as f32 * k).round() as usize,
        (sh as f32 * k).round() as usize,
    );
    let at = |x: isize, y: isize| -> f32 {
        let x = x.clamp(0, sw as isize - 1) as usize;
        let y = y.clamp(0, sh as isize - 1) as usize;
        f32::from(src[y][x])
    };
    (0..dh)
        .map(|j| {
            let fy = (j as f32 + 0.5) / k - 0.5;
            let (y0, ty) = (fy.floor() as isize, fy - fy.floor());
            (0..dw)
                .map(|i| {
                    let fx = (i as f32 + 0.5) / k - 0.5;
                    let (x0, tx) = (fx.floor() as isize, fx - fx.floor());
                    let top = at(x0, y0) * (1.0 - tx) + at(x0 + 1, y0) * tx;
                    let bottom = at(x0, y0 + 1) * (1.0 - tx) + at(x0 + 1, y0 + 1) * tx;
                    (top * (1.0 - ty) + bottom * ty).round() as u8
                })
                .collect()
        })
        .collect()
}

/// Width of a [`tiny_glyph`], in glyph pixels.
pub const TINY_W: i64 = 5;
/// Height of a [`tiny_glyph`], in glyph pixels.
pub const TINY_H: i64 = 7;

/// Width of `text` in the tiny font, each glyph pixel `s` frame pixels,
/// with a glyph pixel's gap after each character.
#[must_use]
pub fn tiny_width(text: &str, s: i64) -> i64 {
    text.chars().count() as i64 * (TINY_W + 1) * s
}

/// Burn `text` in the tiny font into `frame` with its top left at pixel
/// `(x, y)`, each glyph pixel `s` frame pixels: first an outline `s`
/// wide in `edge`, then the glyphs in `fill`, so it reads over anything.
/// Characters without a glyph are left as gaps.
pub fn draw_tiny(
    frame: &mut Frame,
    (x, y): (i64, i64),
    s: i64,
    (fill, edge): (Lin, Lin),
    text: &str,
) {
    for (pad, colour) in [(s, edge), (0, fill)] {
        write_tiny(frame, (x, y), s, pad, colour, text);
    }
}

/// One pass of `text`: each glyph pixel grown by `pad` on every side.
fn write_tiny(frame: &mut Frame, (x0, y): (i64, i64), s: i64, pad: i64, colour: Lin, text: &str) {
    let mut x = x0;
    for ch in text.chars() {
        if let Some(rows) = tiny_glyph(ch) {
            for (gy, row) in rows.iter().enumerate() {
                for (gx, bit) in row.bytes().enumerate() {
                    if bit == b'#' {
                        let (px, py) = (x + gx as i64 * s, y + gy as i64 * s);
                        fill_clipped(frame, px - pad, py - pad, s + 2 * pad, s + 2 * pad, colour);
                    }
                }
            }
        }
        x += (TINY_W + 1) * s;
    }
}

/// [`Frame::fill`] from a corner that may lie off the frame.
fn fill_clipped(frame: &mut Frame, x: i64, y: i64, w: i64, h: i64, c: Lin) {
    let (x0, y0) = (x.max(0), y.max(0));
    let (x1, y1) = ((x + w).min(frame.w as i64), (y + h).min(frame.h as i64));
    if x0 < x1 && y0 < y1 {
        frame.fill([x0 as usize, y0 as usize, x1 as usize, y1 as usize], c);
    }
}

/// 5×7 glyphs for the characters a scale bar's label can hold.
#[must_use]
pub fn tiny_glyph(c: char) -> Option<[&'static str; 7]> {
    Some(match c {
        '0' => [
            " ### ", "#   #", "#  ##", "# # #", "##  #", "#   #", " ### ",
        ],
        '1' => [
            "  #  ", " ##  ", "  #  ", "  #  ", "  #  ", "  #  ", " ### ",
        ],
        '2' => [
            " ### ", "#   #", "    #", "   # ", "  #  ", " #   ", "#####",
        ],
        '3' => [
            "#####", "   # ", "  #  ", "   # ", "    #", "#   #", " ### ",
        ],
        '4' => [
            "   # ", "  ## ", " # # ", "#  # ", "#####", "   # ", "   # ",
        ],
        '5' => [
            "#####", "#    ", "#### ", "    #", "    #", "#   #", " ### ",
        ],
        '6' => [
            "  ## ", " #   ", "#    ", "#### ", "#   #", "#   #", " ### ",
        ],
        '7' => [
            "#####", "    #", "   # ", "  #  ", " #   ", " #   ", " #   ",
        ],
        '8' => [
            " ### ", "#   #", "#   #", " ### ", "#   #", "#   #", " ### ",
        ],
        '9' => [
            " ### ", "#   #", "#   #", " ####", "    #", "   # ", " ##  ",
        ],
        '.' => [
            "     ", "     ", "     ", "     ", "     ", " ##  ", " ##  ",
        ],
        'µ' => [
            "     ", "     ", "#   #", "#   #", "#   #", "#### ", "#    ",
        ],
        'm' => [
            "     ", "     ", "## # ", "# # #", "# # #", "# # #", "# # #",
        ],
        'p' => [
            "     ", "     ", "#### ", "#   #", "#### ", "#    ", "#    ",
        ],
        'x' => [
            "     ", "     ", "#   #", " # # ", "  #  ", " # # ", "#   #",
        ],
        ' ' => ["     "; 7],
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../tests/text.rs"]
mod tests;
