//! Thumbnails: a picture shrunk in linear light, written and read as PNG.
//!
//! [`shrink`] is pinto's (`src/view/saved.rs`), each pixel the mean in
//! linear light of those it covers, as pinto's average mode mixes colours,
//! and never enlarged; it works on an [`RgbaImage`] here in place of
//! pinto's `Frame`. Its decode and encode tables are pinto's
//! (`src/view/color.rs`: `linear`, `encode_fast`), the same as
//! `map::colour::{linear, encode}`, kept here so a gallery needs no map.
//! PNG goes through the `image` crate, which senna and lupin already use,
//! in place of pinto's `png`.

use image::{ImageFormat, RgbaImage};
use std::io;
use std::path::Path;
use std::sync::LazyLock;

/// A picture's RGBA bytes, borrowed: an [`RgbaImage`] or a viewer's own
/// frame, row by row, four bytes a pixel.
#[derive(Clone, Copy, Debug)]
pub struct Picture<'a> {
    w: u32,
    h: u32,
    rgba: &'a [u8],
}

impl<'a> Picture<'a> {
    /// `w × h` pixels in `rgba`; `None` when it holds fewer bytes.
    #[must_use]
    pub fn new(w: u32, h: u32, rgba: &'a [u8]) -> Option<Self> {
        let need = (w as usize).checked_mul(h as usize)?.checked_mul(4)?;
        (rgba.len() >= need).then_some(Self { w, h, rgba })
    }
}

impl<'a> From<&'a RgbaImage> for Picture<'a> {
    fn from(img: &'a RgbaImage) -> Self {
        Self {
            w: img.width(),
            h: img.height(),
            rgba: img.as_raw(),
        }
    }
}

/// `picture` shrunk to `width` pixels wide at its aspect, each pixel the
/// mean in linear light of the ones it covers (never enlarged), opaque.
#[must_use]
pub fn shrink<'a>(picture: impl Into<Picture<'a>>, width: u32) -> RgbaImage {
    let picture = picture.into();
    let (fw, fh) = (picture.w as usize, picture.h as usize);
    let w = (width as usize).min(fw).max(1);
    let h = ((fh * w) as f32 / fw.max(1) as f32).round().max(1.) as usize;
    let raw = picture.rgba;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for y in 0..h {
        let (y0, y1) = (y * fh / h, ((y + 1) * fh / h).max(y * fh / h + 1));
        for x in 0..w {
            let (x0, x1) = (x * fw / w, ((x + 1) * fw / w).max(x * fw / w + 1));
            let mut sum = [0f32; 3];
            let mut n = 0f32;
            for yy in y0..y1.min(fh) {
                for xx in x0..x1.min(fw) {
                    let o = (yy * fw + xx) * 4;
                    for (s, &v) in sum.iter_mut().zip(&raw[o..o + 3]) {
                        *s += linear(v);
                    }
                    n += 1.;
                }
            }
            let [r, g, b] = sum.map(|s| encode(s / n.max(1.)));
            rgba.extend([r, g, b, 255]);
        }
    }
    RgbaImage::from_raw(w as u32, h as u32, rgba).expect("w × h pixels")
}

/// Write `img` to `path` as an 8-bit RGBA PNG.
pub fn write_png(img: &RgbaImage, path: &Path) -> io::Result<()> {
    use image::codecs::png::{CompressionType, FilterType, PngEncoder};
    use image::ImageEncoder;
    // Fast compression, as pinto's thumbnails were written: they are small.
    let out = io::BufWriter::new(std::fs::File::create(path)?);
    PngEncoder::new_with_quality(out, CompressionType::Fast, FilterType::Adaptive)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(io::Error::other)
}

/// The PNG at `path`, as RGBA; `None` when it cannot be read.
#[must_use]
pub fn read_png(path: &Path) -> Option<RgbaImage> {
    let bytes = std::fs::read(path).ok()?;
    image::load_from_memory_with_format(&bytes, ImageFormat::Png)
        .ok()
        .map(image::DynamicImage::into_rgba8)
}

/// sRGB byte → linear light, by table.
pub(crate) fn linear(v: u8) -> f32 {
    static TABLE: LazyLock<[f32; 256]> = LazyLock::new(|| {
        std::array::from_fn(|v| {
            let c = v as f32 / 255.;
            if c <= 0.040_45 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        })
    });
    TABLE[usize::from(v)]
}

/// Steps of the linear-light table behind [`encode`].
const ENCODE_STEPS: usize = 4096;

/// Linear light → sRGB byte, by table.
pub(crate) fn encode(c: f32) -> u8 {
    static TABLE: LazyLock<Vec<u8>> = LazyLock::new(|| {
        (0..=ENCODE_STEPS)
            .map(|i| {
                let c = i as f32 / ENCODE_STEPS as f32;
                let v = if c <= 0.003_130_8 {
                    12.92 * c
                } else {
                    1.055 * c.powf(1. / 2.4) - 0.055
                };
                (v * 255.).round() as u8
            })
            .collect()
    });
    TABLE[(c.clamp(0., 1.) * ENCODE_STEPS as f32).round() as usize]
}
