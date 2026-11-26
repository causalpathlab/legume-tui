//! A [`Page`] as an SVG: the raster as one embedded PNG image, the text
//! over it as SVG text in the page's face, each run over a halo of its
//! background, so labels and keys stay editable in a vector editor while
//! the points stay one image.

use super::font::Face;
use super::{Page, BASELINE, EM_PER_LINE};
use crate::map::colour::{to_srgb_senna, Lin};
use base64::Engine;
use std::fmt::Write;
use std::io;
use std::path::Path;

/// `page` as an SVG document, `dpi` raster pixels to the inch: its size in
/// points, the raster stretched over it.
///
/// # Errors
/// [`io::ErrorKind::InvalidInput`] for a `dpi` that is not a positive
/// number or an empty page; an image that cannot be encoded.
pub fn svg_string(page: &Page, dpi: f32, face: Face) -> io::Result<String> {
    let invalid = |e: String| io::Error::new(io::ErrorKind::InvalidInput, e);
    if !(dpi.is_finite() && dpi > 0.0) {
        return Err(invalid(format!("{dpi} dpi is not a resolution")));
    }
    let (w, h) = (page.img.width(), page.img.height());
    if w == 0 || h == 0 {
        return Err(invalid("the page is empty".into()));
    }
    let png = super::png_at_dpi(&page.img, dpi)?;
    let pt = 72.0 / dpi;
    let (wpt, hpt) = (w as f32 * pt, h as f32 * pt);
    let family = match face {
        Face::Arial => "Arial, Helvetica, sans-serif",
        _ => "Helvetica, Arial, sans-serif",
    };
    let mut s = String::new();
    let _ = writeln!(
        s,
        r#"<svg xmlns="http://www.w3.org/2000/svg" xmlns:xlink="http://www.w3.org/1999/xlink" width="{wpt}pt" height="{hpt}pt" viewBox="0 0 {wpt} {hpt}">"#
    );
    let _ = writeln!(
        s,
        // xlink's link, which SVG 1.1 editors (Inkscape 1.x, Illustrator)
        // read and SVG 2 readers still take.
        r#"<image width="{wpt}" height="{hpt}" preserveAspectRatio="none" xlink:href="data:image/png;base64,{}"/>"#,
        base64::engine::general_purpose::STANDARD.encode(png)
    );
    let _ = writeln!(
        s,
        r#"<g font-family="{family}" stroke-linejoin="round" paint-order="stroke">"#
    );
    for run in &page.text {
        let size = run.line * EM_PER_LINE * pt;
        let (x, anchor) = if run.centred {
            ((run.x + 0.5 * run.w) * pt, "middle")
        } else {
            (run.x * pt, "start")
        };
        let place = if run.vertical {
            let (bx, by) = ((run.x + BASELINE * run.line) * pt, run.y * pt);
            format!(r#"transform="translate({bx} {by}) rotate(-90)""#)
        } else {
            let y = (run.y + BASELINE * run.line) * pt;
            format!(r#"x="{x}" y="{y}""#)
        };
        let weight = if run.bold {
            r#" font-weight="bold""#
        } else {
            ""
        };
        let _ = writeln!(
            s,
            r#"<text {place} font-size="{size}"{weight} text-anchor="{anchor}" fill="{}" stroke="{}" stroke-width="{}">{}</text>"#,
            hex(run.ink),
            hex(run.bg),
            0.22 * size,
            escape(&run.text)
        );
    }
    s.push_str("</g>\n</svg>\n");
    Ok(s)
}

/// [`svg_string`] written to `path`.
///
/// # Errors
/// As [`svg_string`], or when the file cannot be written.
pub fn write_svg(page: &Page, dpi: f32, face: Face, path: &Path) -> io::Result<()> {
    std::fs::write(path, svg_string(page, dpi, face)?)
}

fn hex(c: Lin) -> String {
    let [r, g, b] = to_srgb_senna(c);
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::map::raster::Frame;
    use crate::map::text::{self, Font};

    #[test]
    fn an_svg_keeps_the_text_as_text_over_one_image() {
        let mut f = Frame::blank(200, 100, [255, 255, 255]);
        f.keep_text();
        let font = Font::for_cell_height(16.0, false);
        text::draw(&mut f, font, "a < b & c", 10, 10, [0.0; 3], [1.0; 3]);
        let page = Page {
            img: f.to_image(),
            text: f.take_text(),
        };
        let svg = svg_string(&page, 300.0, Face::Arial).unwrap();
        assert!(svg.starts_with("<svg"));
        assert_eq!(svg.matches("<image").count(), 1);
        assert!(svg.contains(">a &lt; b &amp; c</text>"));
        assert!(svg.contains("Arial"));
        assert!(svg_string(&page, 0.0, Face::Arial).is_err());
    }
}
