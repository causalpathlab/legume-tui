//! A whole map drawn with no label offsets: points in two layers, a ring,
//! an edge, a legend and labels, then an activity view with its ramp key,
//! vertical text and a scaled-up font. The pinned numbers are what
//! senna's own code (`src/view/{render,text,style,color}.rs`, compiled
//! apart from this crate) gives for the same scene, so a viewer moving
//! onto these modules keeps its pixels.

use super::colour::{activity_ramp, to_linear, Lin, Srgb};
use super::labels::{place, Label, Placed, Rect};
use super::legend::{draw_legend, draw_ramp_key};
use super::offsets::Offsets;
use super::points::{by_level, group_medians, ranked, Job, Mark, PointLayer};
use super::raster::{edge, ring, Shape};
use super::text::{self, Font};
use super::viewport::{Viewport, YAxis};
use std::sync::Arc;

const PAGE: Srgb = [246, 245, 242];
const MUTED: Srgb = [200, 199, 196];
const INK: Srgb = [92, 92, 92];

struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (self.0 >> 33) as u32
    }
    fn unit(&mut self) -> f32 {
        (self.next() % 10_000) as f32 / 10_000.0
    }
}

fn fnv(b: &[u8]) -> u64 {
    let mut h = 0xcbf2_9ce4_8422_2325_u64;
    for &x in b {
        h ^= u64::from(x);
        h = h.wrapping_mul(0x0100_0000_01b3);
    }
    h
}

const CENTRES: [[f32; 2]; 5] = [[2.0, 2.0], [8.0, 2.5], [5.0, 8.0], [2.5, 7.5], [5.0, 5.0]];
const COLOURS: [Lin; 4] = [
    [0.8, 0.2, 0.1],
    [0.1, 0.5, 0.8],
    [0.2, 0.7, 0.2],
    [0.6, 0.1, 0.6],
];
const INKS: [Lin; 4] = [
    [0.3, 0.05, 0.02],
    [0.02, 0.15, 0.3],
    [0.05, 0.25, 0.05],
    [0.2, 0.02, 0.2],
];
const SHAPES: [Shape; 4] = [Shape::Circle, Shape::Square, Shape::Diamond, Shape::Cross];
const ALPHAS: [f32; 4] = [1.0, 1.0, 0.6, 1.0];
const SIZES: [f32; 4] = [1.0, 1.5, 1.0, 0.8];

struct Points {
    xy: Vec<[f32; 2]>,
    shuffled: Vec<u32>,
    groups: Vec<Option<u32>>,
}

/// senna's `Paint` for what the scene uses: a muted backdrop, groups in
/// their styles, or activity on a ramp.
struct Paint<'a> {
    points: &'a Points,
    muted: bool,
    size: f32,
    levels: Option<(&'a [f32], &'a [Lin])>,
    order: Arc<Vec<u32>>,
}

/// senna's `Levels::t` for levels in `[0, 1]`, off at zero and below.
fn t(v: f32) -> f32 {
    if !v.is_finite() || v <= 0.0 {
        -1.0
    } else {
        v.clamp(0.0, 1.0)
    }
}

impl<'a> Paint<'a> {
    fn new(
        points: &'a Points,
        muted: bool,
        size: f32,
        levels: Option<(&'a [f32], &'a [Lin])>,
    ) -> Self {
        let order = match levels {
            Some((v, _)) => by_level(&points.shuffled, |i| t(v[i])),
            None => ranked(
                &points.shuffled,
                |i| u8::from(!muted && points.groups[i].is_some()),
                4,
            ),
        };
        Self {
            points,
            muted,
            size,
            levels,
            order: Arc::new(order),
        }
    }
}

impl PointLayer for Paint<'_> {
    fn xy(&self, i: usize) -> [f32; 2] {
        self.points.xy[i]
    }
    fn shuffled(&self) -> &[u32] {
        &self.points.shuffled
    }
    fn order(&self) -> Arc<Vec<u32>> {
        self.order.clone()
    }
    fn mark(&self, i: usize) -> Option<Mark> {
        let plain = |colour| Mark {
            colour,
            alpha: 1.0,
            shape: Shape::Circle,
            size: 1.0,
        };
        let muted = to_linear(MUTED);
        if self.muted {
            return Some(plain(muted));
        }
        if let Some((v, ramp)) = self.levels {
            let t = t(v[i]);
            if t < 0.0 {
                return Some(plain(muted));
            }
            let k = ((ramp.len() - 1) as f32 * t).round() as usize;
            return Some(plain(ramp[k]));
        }
        let Some(g) = self.points.groups[i] else {
            return Some(plain(muted));
        };
        let g = g as usize;
        Some(Mark {
            colour: COLOURS[g],
            alpha: ALPHAS[g],
            shape: SHAPES[g],
            size: SIZES[g],
        })
    }
    fn size(&self) -> f32 {
        self.size
    }
}

fn points() -> Points {
    let mut r = Lcg(7);
    let n = 3000;
    let groups = (0..n)
        .map(|i| (i % 5 != 4).then_some((i % 5) as u32))
        .collect();
    let xy = (0..n)
        .map(|i| {
            let c = CENTRES[i % 5];
            [c[0] + 3.0 * (r.unit() - 0.5), c[1] + 3.0 * (r.unit() - 0.5)]
        })
        .collect();
    let mut shuffled: Vec<u32> = (0..n as u32).collect();
    for i in (1..n).rev() {
        let j = r.next() as usize % (i + 1);
        shuffled.swap(i, j);
    }
    Points {
        xy,
        shuffled,
        groups,
    }
}

fn draw<L: PointLayer>(vp: Viewport, layers: &[L]) -> super::raster::Frame {
    let mut job = Job::new(vp, PAGE, layers);
    while !job.step(layers) {}
    job.finish()
}

#[test]
fn a_cell_map_with_no_offsets_is_senna_s_to_the_byte() {
    let p = points();
    let vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 240, 180, 0.92, YAxis::Up);
    let mut frame = draw(
        vp,
        &[
            Paint::new(&p, true, 0.8, None),
            Paint::new(&p, false, 1.0, None),
        ],
    );
    let ink = [0.1, 0.0, 0.3];
    let a = vp.to_px(p.xy[0]);
    let b = vp.to_px(p.xy[1]);
    ring(&mut frame, a.0, a.1, 6.0, ink);
    edge(&mut frame, a, b, (6.0, 4.0), ink, 0.7);
    let small = Font::for_cell_height(12.8, false);
    let entries: Vec<(String, Lin, Lin)> = (0..3)
        .map(|g| (format!("CT{g}"), COLOURS[g], INKS[g]))
        .collect();
    let reserved: Vec<Rect> = draw_legend(&mut frame, &entries, small, None)
        .into_iter()
        .collect();
    assert_eq!(reserved, [[0.0, 0.0, 55.0, 64.0]]);

    let bold = Font::for_cell_height(16.0, true);
    let regular = Font::for_cell_height(16.0, false);
    let medians = group_medians(|i| p.xy[i], &p.shuffled, |i| p.groups[i], 4);
    let mut labels = Vec::new();
    for (g, m) in medians.iter().enumerate() {
        let &Some((at, size)) = m else { continue };
        let (x, y) = vp.to_px(at);
        labels.push(Label {
            text: format!("CT{g}"),
            x,
            y,
            ink: INKS[g],
            tier: 0,
            priority: size as f32,
            font: bold,
            key: Some(g as u32),
        });
    }
    let extra = |text: &str, (x, y), ink, tier, priority, font, key| Label {
        text: text.to_string(),
        x,
        y,
        ink,
        tier,
        priority,
        font,
        key,
    };
    // senna's tiers: Map 0, NearFeature 2, Centre 3, Picked 4.
    labels.push(extra("L1-0", (120.0, 90.0), ink, 4, 0.0, bold, None));
    labels.push(extra(
        "GENE1",
        (124.0, 94.0),
        INKS[0],
        2,
        0.0,
        regular,
        None,
    ));
    labels.push(extra("CT9", (3.0, 3.0), ink, 0, 1000.0, bold, Some(9)));
    labels.push(extra("CT8", (20.0, 14.0), ink, 3, 0.0, bold, Some(8)));
    labels.push(extra("CT7", (200.0, 160.0), INKS[1], 0, 5.0, bold, Some(7)));
    labels.push(extra("CT6", (205.0, 162.0), INKS[2], 0, 6.0, bold, Some(6)));
    let placed = place(&mut frame, labels, &reserved, &Offsets::default());

    let want: [(u32, Rect); 5] = [
        (0, [57.31183, 131.95677, 84.31183, 149.95677]),
        (1, [155.80243, 122.82724, 182.80243, 140.82724]),
        (2, [105.88397, 30.738747, 132.88397, 48.738747]),
        (3, [64.002075, 38.71073, 91.002075, 56.71073]),
        (6, [191.5, 153.0, 218.5, 171.0]),
    ];
    let got: Vec<(u32, Rect)> = placed.iter().map(|p| (p.key, p.rect)).collect();
    assert_eq!(got, want);
    assert!(placed.iter().all(|p: &Placed<u32>| !p.moved));
    assert_eq!(fnv(frame.to_image().as_raw()), 0x1db1_ae84_ce6d_9869);
}

#[test]
fn an_activity_view_with_its_key_is_senna_s_to_the_byte() {
    let p = points();
    let values: Vec<f32> = p.xy.iter().map(|q| q[0] / 10.0 - 0.1).collect();
    let ramp = activity_ramp(16);
    let vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 160, 120, 0.92, YAxis::Up);
    let mut f = draw(vp, &[Paint::new(&p, false, 1.0, Some((&values, &ramp)))]);
    let regular = Font::for_cell_height(16.0, false);
    let key = draw_ramp_key(&mut f, "GENE1", &ramp, regular, to_linear(INK));
    assert_eq!(key, [0.0, 0.0, 144.0, 38.0]);
    let bg = f.background();
    text::draw_vertical(&mut f, regular, "CT1", (140, 110), INKS[1], bg);
    let big = Font::for_cell_height(40.0, false);
    let entry = [("CT0".to_string(), COLOURS[0], INKS[0])];
    draw_legend(&mut f, &entry, big, None);
    assert_eq!(fnv(f.to_image().as_raw()), 0x4507_fa57_aec7_0d24);
}

#[test]
fn a_map_saves_as_png() {
    let p = points();
    let vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 60, 40, 0.92, YAxis::Up);
    let img = draw(vp, &[Paint::new(&p, false, 1.0, None)]).to_image();
    let mut png = std::io::Cursor::new(Vec::new());
    img.write_to(&mut png, image::ImageFormat::Png).unwrap();
    let back = image::load_from_memory(png.get_ref()).unwrap().to_rgba8();
    assert_eq!(back, img);
}
