//! The standard font a page's text is set in: Helvetica, which every PDF
//! reader has, in its WinAnsi encoding, with its advance widths.
//!
//! Ported from senna (`src/view/pdf.rs`), unchanged.

/// `s` in WinAnsi, the standard fonts' encoding: ASCII and Latin-1 as they
/// are, a few typographic marks moved, anything else a `?`.
#[must_use]
pub fn win_ansi(s: &str) -> Vec<u8> {
    s.chars()
        .map(|ch| match ch {
            ' '..='~' | '\u{a0}'..='\u{ff}' => ch as u8,
            '•' => 0x95,
            '–' => 0x96,
            '—' => 0x97,
            '…' => 0x85,
            '‘' => 0x91,
            '’' => 0x92,
            '“' => 0x93,
            '”' => 0x94,
            _ => b'?',
        })
        .collect()
}

/// Width of WinAnsi `bytes` in Helvetica at size 1 (from the fonts' AFM).
#[must_use]
pub fn helvetica_width(bytes: &[u8], bold: bool) -> f32 {
    let table = if bold { &HELVETICA_BOLD } else { &HELVETICA };
    let units: u32 = bytes
        .iter()
        .map(|&b| match b {
            32..=126 => u32::from(table[usize::from(b - 32)]),
            0x95 => 350,
            0x96 => 556,
            0x97 => 1000,
            0x85 => 1000,
            _ => 556,
        })
        .sum();
    units as f32 / 1000.0
}

/// Advance widths of ASCII 32..=126, in thousandths of the size.
#[rustfmt::skip]
const HELVETICA: [u16; 95] = [
    278, 278, 355, 556, 556, 889, 667, 191, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 278, 278, 584, 584, 584, 556,
    1015, 667, 667, 722, 722, 667, 611, 778, 722, 278, 500, 667, 556, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 278, 278, 278, 469, 556,
    333, 556, 556, 500, 556, 556, 278, 556, 556, 222, 222, 500, 222, 833, 556, 556,
    556, 556, 333, 500, 278, 556, 500, 722, 500, 500, 500, 334, 260, 334, 584,
];

#[rustfmt::skip]
const HELVETICA_BOLD: [u16; 95] = [
    278, 333, 474, 556, 556, 889, 722, 238, 333, 333, 389, 584, 278, 333, 278, 278,
    556, 556, 556, 556, 556, 556, 556, 556, 556, 556, 333, 333, 584, 584, 584, 611,
    975, 722, 722, 722, 722, 667, 611, 778, 722, 278, 556, 722, 611, 833, 722, 778,
    667, 778, 722, 667, 611, 722, 667, 944, 667, 667, 611, 333, 278, 333, 584, 556,
    333, 556, 611, 556, 611, 556, 333, 611, 611, 278, 278, 556, 278, 889, 611, 611,
    611, 611, 389, 556, 333, 611, 556, 778, 556, 556, 500, 389, 280, 389, 584,
];
