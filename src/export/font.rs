//! The font a page's text is set in: Helvetica, which every PDF reader
//! has, in its WinAnsi encoding, with its advance widths; or Arial, named
//! for the reader to find, with the same widths (Arial's are Helvetica's).
//!
//! Ported from senna (`src/view/pdf.rs`), unchanged; [`Face::Arial`] is
//! new.

use pdf_writer::types::FontFlags;
use pdf_writer::{Finish, Name, Pdf, Rect, Ref};

/// The face a PDF's text is set in. Neither is embedded: Helvetica is one
/// of the standard fonts every reader has; Arial is named (`ArialMT`,
/// `Arial-BoldMT`) with Helvetica's widths, which are Arial's, so a reader
/// or editor with Arial sets it in Arial and one without in a font of the
/// same widths.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[cfg_attr(feature = "clap", derive(clap::ValueEnum))]
#[cfg_attr(feature = "map", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "map", serde(rename_all = "lowercase"))]
pub enum Face {
    #[default]
    Helvetica,
    Arial,
}

/// The regular and bold fonts of `face` as objects `fonts`, with
/// descriptors at `descriptors` for a face that needs them.
pub(crate) fn write_fonts(pdf: &mut Pdf, face: Face, fonts: [Ref; 2], descriptors: [Ref; 2]) {
    if face == Face::Helvetica {
        for (id, name) in fonts
            .into_iter()
            .zip([&b"Helvetica"[..], b"Helvetica-Bold"])
        {
            pdf.type1_font(id)
                .base_font(Name(name))
                .encoding_predefined(Name(b"WinAnsiEncoding"));
        }
        return;
    }
    // Arial's metrics, from its own tables, in thousandths of the size.
    let arial = [
        (
            &b"ArialMT"[..],
            false,
            [-665.0, -325.0, 2000.0, 1006.0],
            88.0,
        ),
        (
            b"Arial-BoldMT",
            true,
            [-628.0, -376.0, 2000.0, 1018.0],
            165.0,
        ),
    ];
    for ((id, desc), (name, bold, [x0, y0, x1, y1], stem)) in
        fonts.into_iter().zip(descriptors).zip(arial)
    {
        let mut font = pdf.indirect(id).dict();
        font.pair(Name(b"Type"), Name(b"Font"))
            .pair(Name(b"Subtype"), Name(b"TrueType"))
            .pair(Name(b"BaseFont"), Name(name))
            .pair(Name(b"FirstChar"), 32)
            .pair(Name(b"LastChar"), 255)
            .pair(Name(b"Encoding"), Name(b"WinAnsiEncoding"))
            .pair(Name(b"FontDescriptor"), desc);
        font.insert(Name(b"Widths"))
            .array()
            .items((32..=255u8).map(|b| 1000.0 * helvetica_width(&[b], bold)));
        font.finish();
        pdf.font_descriptor(desc)
            .name(Name(name))
            .flags(FontFlags::NON_SYMBOLIC)
            .bbox(Rect::new(x0, y0, x1, y1))
            .italic_angle(0.0)
            .ascent(905.0)
            .descent(-212.0)
            .cap_height(716.0)
            .stem_v(stem);
    }
}

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
