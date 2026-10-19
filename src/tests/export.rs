//! Export: senna's PDF pages (text kept as text over one raster), its deck
//! geometry, and the page sizes a save offers.

use super::*;
use crate::map::colour::{to_linear, Srgb};
use crate::map::text::{self, Font};
use crate::map::theme::Theme;

const PAGE: Srgb = [246, 245, 242];

fn run(text: &str) -> TextRun {
    TextRun {
        text: text.into(),
        x: 2.0,
        y: 3.0,
        w: 30.0,
        line: 10.0,
        bold: true,
        centred: true,
        vertical: false,
        ink: [0.1; 3],
        bg: [0.9; 3],
    }
}

/// The objects a reader finds through the cross-reference table: each
/// offset it gives starts `k 0 obj`, and the trailer points at the table.
/// Offsets are in bytes, so the file is read as bytes, not as text.
fn objects(pdf: &[u8]) -> usize {
    let find = |at: usize, what: &[u8]| -> bool { pdf[at..].starts_with(what) };
    let tail = String::from_utf8_lossy(&pdf[pdf.len().saturating_sub(64)..]).into_owned();
    let at = tail.rfind("startxref").expect("startxref");
    let xref: usize = tail[at + 9..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    assert!(find(xref, b"xref"), "startxref points at the table");
    let table = String::from_utf8_lossy(&pdf[xref..]).into_owned();
    let mut lines = table.lines().skip(1);
    let head: Vec<usize> = lines
        .next()
        .unwrap()
        .split_whitespace()
        .map(|t| t.parse().unwrap())
        .collect();
    let (first, n) = (head[0], head[1]);
    let mut found = 0;
    for (k, line) in lines.take(n).enumerate() {
        let id = first + k;
        if !line.trim_end().ends_with('n') {
            continue;
        }
        let off: usize = line[..10].parse().unwrap();
        assert!(
            find(off, format!("{id} 0 obj").as_bytes()),
            "object {id} at {off}"
        );
        found += 1;
    }
    found
}

#[test]
fn text_stays_text_and_the_raster_is_one_image() {
    let img = RgbaImage::from_pixel(40, 20, image::Rgba([10, 20, 30, 255]));
    let bytes = pdf_bytes(
        &[Page {
            img,
            text: vec![run("CT1 · GENE1")],
        }],
        144.0,
    )
    .unwrap();
    let s = String::from_utf8_lossy(&bytes);
    assert!(s.starts_with("%PDF-"));
    assert!(s.contains("/Helvetica-Bold"));
    // "CT1 · GENE1" in WinAnsi, as a hex string: halo, then letters.
    assert_eq!(s.matches("<43543120B72047454E4531> Tj").count(), 2);
    assert!(s.contains("/Subtype /Image"));
    // 40 × 20 px at 144 dpi is 20 × 10 pt.
    assert!(s.contains("/MediaBox [0 0 20 10]"));
}

#[test]
fn widths_follow_the_font() {
    assert!((font::helvetica_width(b"ii", false) - 0.444).abs() < 1e-6);
    assert!(font::helvetica_width(b"W", true) > font::helvetica_width(b"i", true));
    assert_eq!(font::win_ansi("a·b✓"), vec![b'a', 0xb7, b'b', b'?']);
}

#[test]
fn typographic_marks_take_their_winansi_places_and_widths() {
    assert_eq!(
        font::win_ansi("•–—…‘’“”é"),
        vec![0x95, 0x96, 0x97, 0x85, 0x91, 0x92, 0x93, 0x94, 0xe9]
    );
    assert!((font::helvetica_width(&[0x97], false) - 1.0).abs() < 1e-6);
    assert!((font::helvetica_width(&[0x95], true) - 0.35).abs() < 1e-6);
    // Latin-1 letters past ASCII take a letter's width.
    assert!((font::helvetica_width(&[0xe9], false) - 0.556).abs() < 1e-6);
}

#[test]
fn centred_and_turned_runs_are_set_where_their_boxes_are() {
    let img = RgbaImage::from_pixel(72, 72, image::Rgba([255; 4]));
    let mut turned = run("ab");
    turned.vertical = true;
    turned.centred = false;
    let mut left = run("ab");
    left.centred = false;
    let page = Page {
        img,
        text: vec![run("ab"), left, turned],
    };
    // At 72 dpi a pixel is a point.
    let s = String::from_utf8_lossy(&pdf_bytes(&[page], 72.0).unwrap()).into_owned();
    let size = 10.0 * 0.74;
    let width = font::helvetica_width(b"ab", true) * size;
    let y = 72.0 - (3.0 + 0.78 * 10.0);
    let centred = 2.0 + 0.5 * (30.0 - width);
    assert!(
        s.contains(&format!("1 0 0 1 {} {} Tm", num(centred), num(y))),
        "{s}"
    );
    assert!(s.contains(&format!("1 0 0 1 2 {} Tm", num(y))));
    assert!(s.contains(&format!("0 1 -1 0 {} 69 Tm", num(2.0 + 0.78 * 10.0))));
}

/// A number as pdf-writer writes it.
fn num(v: f32) -> String {
    let mut c = pdf_writer::Content::new();
    c.set_line_width(v);
    let s = String::from_utf8(c.finish().to_vec()).unwrap();
    s.trim_end().trim_end_matches(" w").to_string()
}

#[test]
fn a_frame_with_kept_text_is_a_page_a_reader_parses() {
    let mut f = Frame::blank(120, 60, PAGE);
    f.keep_text();
    let font = Font::for_cell_height(16.0, false);
    text::draw(&mut f, font, "CT1", 10, 8, [0.0; 3], to_linear(PAGE));
    let blank = Frame::blank(120, 60, PAGE).to_image();
    let page = Page {
        img: f.to_image(),
        text: f.take_text(),
    };
    // The text is not in the pixels.
    assert_eq!(page.img, blank);
    assert_eq!(page.text.len(), 1);
    let bytes = pdf_bytes(&[page], 300.0).unwrap();
    // Catalog, page tree, two fonts, and a page, its content and image.
    assert_eq!(objects(&bytes), 7);
    let s = String::from_utf8_lossy(&bytes);
    // The page names its image, its content draws it, and sets the text
    // over it twice: halo, then letters.
    let resources = &s[s.find("/XObject").unwrap()..];
    assert!(resources
        .split_whitespace()
        .take(5)
        .eq(["/XObject", "<<", "/Im", "7", "0"]));
    assert!(s.contains("/Im Do"));
    assert_eq!(s.matches("(CT1) Tj").count(), 2);
    assert!(s.contains("/Width 120"));
    assert!(s.contains("/Height 60"));
    assert!(s.contains("/Filter /FlateDecode"));
}

#[test]
fn pages_are_written_to_a_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("two.pdf");
    let page = || Page {
        img: RgbaImage::from_pixel(8, 8, image::Rgba([0, 0, 0, 255])),
        text: Vec::new(),
    };
    write_pdf(&[page(), page()], 72.0, &path).unwrap();
    let bytes = std::fs::read(&path).unwrap();
    assert_eq!(bytes, pdf_bytes(&[page(), page()], 72.0).unwrap());
    assert!(String::from_utf8_lossy(&bytes).contains("/Count 2"));
    assert_eq!(objects(&bytes), 4 + 2 * 3);
}

#[test]
fn a_resolution_or_page_that_cannot_be_set_is_refused() {
    let page = |w, h| Page {
        img: RgbaImage::from_pixel(w, h, image::Rgba([0, 0, 0, 255])),
        text: Vec::new(),
    };
    for dpi in [0.0, -72.0, f32::NAN, f32::INFINITY] {
        let e = pdf_bytes(&[page(4, 4)], dpi).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{dpi}");
    }
    for (w, h) in [(0, 0), (4, 0), (0, 4)] {
        let e = pdf_bytes(&[page(4, 4), page(w, h)], 72.0).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{w} × {h}");
    }
    // Refused before the file is touched.
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("bad.pdf");
    let e = write_pdf(&[page(4, 4)], 0.0, &path).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::InvalidInput);
    assert!(!path.exists());
}

#[test]
fn the_grid_keeps_tiles_as_large_as_it_can() {
    assert_eq!(shape(1, 100.0, 100.0), (1, 1));
    assert_eq!(shape(4, 100.0, 100.0), (2, 2));
    assert_eq!(shape(3, 300.0, 100.0), (3, 1));
    assert_eq!(shape(3, 100.0, 300.0), (1, 3));
    assert_eq!(shape(5, 160.0, 100.0), (3, 2));
    assert_eq!(shape(0, 10.0, 10.0), (1, 1));
}

#[test]
fn a_grid_page_tiles_its_frames_titled_with_rules_between() {
    let theme = Theme::PAPER;
    let pages: [Srgb; 3] = [[200, 30, 30], [30, 200, 30], [30, 30, 200]];
    let (w, h) = (300, 100);
    let (cols, rows) = shape(3, w as f32, h as f32);
    assert_eq!((cols, rows), (3, 1));
    let (tw, th) = (w / cols, h / rows);
    let tiles: Vec<(Frame, String)> = pages
        .iter()
        .enumerate()
        .map(|(k, &bg)| {
            let mut f = Frame::blank(tw, th, bg);
            f.keep_text();
            (f, format!("L1-{k}"))
        })
        .collect();
    let cell_px = 20.0;
    let page = grid_page(tiles, (w, h), cell_px, &theme);
    assert_eq!(page.img.dimensions(), (w as u32, h as u32));

    for (k, &bg) in pages.iter().enumerate() {
        let want = Frame::blank(1, 1, bg).to_image().get_pixel(0, 0).0;
        assert_eq!(
            page.img.get_pixel((k * tw + 50) as u32, 30).0,
            want,
            "tile {k}"
        );
    }
    let [r, g, b] = theme.muted;
    for x in [tw, 2 * tw] {
        for y in [0, 50, h - 1] {
            assert_eq!(page.img.get_pixel(x as u32, y as u32).0, [r, g, b, 255]);
        }
    }

    // The titles stay text, at each tile's bottom left, in the theme's ink.
    let lh = Font::for_cell_height(cell_px, true).line_height() as f32;
    assert_eq!(page.text.len(), 3);
    for (k, t) in page.text.iter().enumerate() {
        assert_eq!(t.text, format!("L1-{k}"));
        assert!(t.bold);
        assert_eq!(t.x, (k * tw) as f32 + (lh as i32 / 2) as f32);
        let lh = lh as i32;
        assert_eq!(t.y, (th as i32 - lh - lh / 4) as f32);
        assert_eq!(t.ink, to_linear(theme.ink));
        assert_eq!(t.bg, to_linear(pages[k]));
    }
}

#[test]
fn a_grid_page_keeps_text_the_tiles_already_held() {
    let theme = Theme::PAPER;
    let font = Font::for_cell_height(16.0, false);
    let tiles: Vec<(Frame, String)> = (0..4)
        .map(|k| {
            let mut f = Frame::blank(50, 40, PAGE);
            f.keep_text();
            text::draw(&mut f, font, "CT1", 5, 4, [0.0; 3], to_linear(PAGE));
            (f, format!("T{k}"))
        })
        .collect();
    let page = grid_page(tiles, (100, 80), 16.0, &theme);
    let labels: Vec<(f32, f32)> = page
        .text
        .iter()
        .filter(|t| t.text == "CT1")
        .map(|t| (t.x, t.y))
        .collect();
    assert_eq!(labels, [(5.0, 4.0), (55.0, 4.0), (5.0, 44.0), (55.0, 44.0)]);
    // Rules at the tiles' edges, on both axes.
    let [r, g, b] = theme.muted;
    assert_eq!(page.img.get_pixel(50, 10).0, [r, g, b, 255]);
    assert_eq!(page.img.get_pixel(10, 40).0, [r, g, b, 255]);
}

#[test]
fn an_empty_grid_page_has_no_rules_to_draw() {
    let theme = Theme::PAPER;
    let tiles = || -> Vec<(Frame, String)> {
        (0..3)
            .map(|k| (Frame::blank(0, 0, PAGE), format!("T{k}")))
            .collect()
    };
    for (w, h) in [(30, 0), (0, 30), (0, 0)] {
        let page = grid_page(tiles(), (w, h), 16.0, &theme);
        assert_eq!(page.img.dimensions(), (w as u32, h as u32));
    }
}

#[test]
fn page_sizes_are_the_width_and_dpi_offered() {
    assert_eq!(PageSize::WIDTHS_IN, [3.5, 5.0, 7.0, 10.0, 14.0]);
    assert_eq!(PageSize::DPIS, [150, 300, 600, 1200]);
    let p = PageSize {
        width_in: 7.0,
        dpi: 300,
    };
    assert_eq!(p.px(1.0), (2100, 2100));
    assert_eq!(p.px(0.75), (2100, 1575));
    // The aspect is held between 0.4 and 2.5.
    assert_eq!(p.px(0.1), (2100, 840));
    assert_eq!(p.px(10.0), (2100, 5250));
    assert!((p.cell_px() - 300.0 * 11.0 / 72.0).abs() < 1e-4);
}

#[test]
fn a_page_past_max_pixels_is_refused() {
    let ok = PageSize {
        width_in: 7.0,
        dpi: 300,
    };
    assert_eq!(ok.check(1.0), Ok((2100, 2100)));
    let large = PageSize {
        width_in: 14.0,
        dpi: 1200,
    };
    let (w, h) = large.px(1.0);
    assert!(w * h > MAX_PIXELS);
    let e = large.check(1.0).unwrap_err();
    assert!(e.contains("16800 × 16800 px is too large"), "{e}");
    // Exactly MAX_PIXELS is kept: 10,000 × 8,000.
    let edge = PageSize {
        width_in: 10.0,
        dpi: 1000,
    };
    assert_eq!(edge.check(0.8), Ok((10_000, 8_000)));
    for bad in [
        PageSize {
            width_in: 0.0,
            dpi: 300,
        },
        PageSize {
            width_in: -1.0,
            dpi: 300,
        },
        PageSize {
            width_in: f32::NAN,
            dpi: 300,
        },
        PageSize {
            width_in: 7.0,
            dpi: 0,
        },
    ] {
        assert!(bad.check(1.0).is_err(), "{bad:?}");
    }
    // Every size offered at a square page, against the limit.
    for &width_in in &PageSize::WIDTHS_IN {
        for &dpi in &PageSize::DPIS {
            let p = PageSize { width_in, dpi };
            let (w, h) = p.px(1.0);
            assert_eq!(p.check(1.0).is_ok(), w * h <= MAX_PIXELS, "{p:?}");
        }
    }
}

#[test]
fn a_grid_page_is_sized_as_senna_sizes_it() {
    // 4:3 tiles in a roughly 4:3 page, its aspect not held: two runs side
    // by side make a page wider than any single map's.
    let p = PageSize {
        width_in: 7.0,
        dpi: 300,
    };
    for n in 1..=12 {
        let (cols, rows) = shape(n, 4.0, 3.0);
        let w = 2100usize;
        let h = (w as f32 * 0.75 * rows as f32 / cols as f32).round() as usize;
        assert_eq!(p.grid_px(n), (w, h), "{n}");
        assert_eq!(p.check_grid(n), Ok((w, h)), "{n}");
    }
    assert_eq!(p.grid_px(2), (2100, 788));
    assert!(PageSize {
        width_in: 14.0,
        dpi: 1200
    }
    .check_grid(4)
    .is_err());
    assert!(PageSize {
        width_in: 7.0,
        dpi: 0
    }
    .check_grid(2)
    .is_err());
}

#[test]
fn a_page_too_wide_to_count_is_refused() {
    for width_in in [1e10, f32::MAX, f32::INFINITY, f32::NEG_INFINITY] {
        let p = PageSize {
            width_in,
            dpi: 1200,
        };
        assert!(p.check(1.0).is_err(), "{p:?}");
        assert!(p.check_grid(2).is_err(), "{p:?}");
    }
}

#[cfg(feature = "clap")]
#[test]
fn pdf_args_are_sennas_flags_and_defaults() {
    use clap::Parser;
    #[derive(Parser)]
    struct Cli {
        #[command(flatten)]
        pdf: PdfArgs,
    }
    let a = Cli::try_parse_from(["tool"]).unwrap().pdf;
    assert_eq!((a.pdf, a.width, a.dpi), (None, 7.0, 300));
    let a = Cli::try_parse_from(["tool", "--pdf", "out.pdf", "--width", "3.5", "--dpi", "600"])
        .unwrap()
        .pdf;
    assert_eq!(a.pdf.as_deref(), Some(std::path::Path::new("out.pdf")));
    assert_eq!(
        a.size(),
        PageSize {
            width_in: 3.5,
            dpi: 600
        }
    );
}
