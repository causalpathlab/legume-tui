//! Histograms in text: scales and equal-width bins on them ([`Scale`],
//! [`Binning`], [`Binned`]), the [`Axis`] values sit on, and two plots
//! drawn into a ratatui [`Buffer`] with a y gutter, an x axis and marks:
//! [`HistPlot`], bars over bins with a subset in front, and
//! [`MirrorPlot`], two series either side of one zero line. Their styles
//! are a [`Look`]'s.
//!
//! Ported from data-beans (`src/interactive/ui.rs`, `src/qc/axis.rs`).

mod axis;

pub use axis::Axis;

use crate::ink;
use crate::look::Look;
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::Style;

/// Set a cell outright, rather than layering `style` over what was there.
fn put(buf: &mut Buffer, x: u16, y: u16, symbol: &str, style: Style) {
    buf[(x, y)]
        .set_symbol(symbol)
        .set_style(Style::reset().patch(style));
}

/// Width of the y-axis gutter left of each histogram, in columns.
pub const GUTTER: u16 = 6;

/// Bins across the range on every scale.
const TARGET_BINS: f64 = 50.0;

/// Most bins [`Binned::new`] makes.
const MAX_BINS: i64 = 1024;

/// How a histogram axis is drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scale {
    /// Logarithmic.
    Log,
    /// Square root.
    Sqrt,
    /// As the values are.
    Linear,
}

impl Scale {
    /// The next scale a key steps to: log, sqrt, linear, log.
    pub fn next(self) -> Self {
        match self {
            Scale::Log => Scale::Sqrt,
            Scale::Sqrt => Scale::Linear,
            Scale::Linear => Scale::Log,
        }
    }

    /// The scale's name, as a view shows it.
    pub fn name(self) -> &'static str {
        match self {
            Scale::Log => "log",
            Scale::Sqrt => "sqrt",
            Scale::Linear => "linear",
        }
    }

    fn apply(self, v: f64) -> f64 {
        match self {
            Scale::Log => (v + 1.0).log10(),
            Scale::Sqrt => v.max(0.0).sqrt(),
            Scale::Linear => v,
        }
    }

    fn invert(self, t: f64) -> f64 {
        match self {
            Scale::Log => 10f64.powf(t) - 1.0,
            Scale::Sqrt => t * t,
            Scale::Linear => t,
        }
    }
}

/// Equal-width bins on a scale, keyed by integers from 0 at 0. The log
/// scale is the [`Axis`] every histogram reads values on: `ln(shift + x)`,
/// the shift 1 for counts and smaller for values mostly below 1.
#[derive(Debug, Clone, Copy)]
pub struct Binning {
    pub scale: Scale,
    /// Bin width on the scaled axis.
    width: f64,
    /// What the log scale adds before its log.
    shift: f64,
}

impl Binning {
    /// Bins spanning `0..=max`. Whole counts (`integer`) get linear bins at
    /// least one count wide, so no bin falls between two integers. The log
    /// scale shifts by 1 for whole counts or a `max` of 20 and over, else
    /// by a thousandth of `max`; [`Binned::new`] shifts as the data says.
    pub fn new(scale: Scale, max: f64, integer: bool) -> Self {
        let shift = if integer || max >= 20.0 {
            1.0
        } else {
            (max / 1e3).clamp(1e-9, 1.0)
        };
        Self::shifted(scale, 0.0, max, integer, shift)
    }

    /// Bins about [`TARGET_BINS`] across `min..=max` stretched to take in
    /// 0 (where the keys start), the log scale shifted by `shift`. For
    /// non-negative data that is `0..=max`, as [`Binning::new`]. A range
    /// with no width on the scale (one value, or negatives the log and
    /// sqrt scales put all at 0) gets bins one unit wide.
    fn shifted(scale: Scale, min: f64, max: f64, integer: bool, shift: f64) -> Self {
        let (lo, hi) = (min.min(0.0), max.max(0.0));
        let top = if integer { hi + 1.0 } else { hi };
        let mut bins = Self {
            scale,
            width: 1.0,
            shift,
        };
        let width = match scale {
            Scale::Linear if integer => ((top - lo) / TARGET_BINS).ceil().max(1.0),
            _ => (bins.t(top) - bins.t(lo)) / TARGET_BINS,
        };
        if width.is_finite() && width > 0.0 {
            bins.width = width;
        }
        bins
    }

    /// Bins of an explicit `width` on the scaled axis. On the linear scale
    /// with width 1, bin `k` holds exactly the value `k`, so a histogram of
    /// bin indices draws one bar per category.
    pub fn with_width(scale: Scale, width: f64) -> Self {
        Self {
            scale,
            width: width.max(f64::MIN_POSITIVE),
            shift: 1.0,
        }
    }

    /// `x` on the scaled axis, 0 at 0.
    fn t(&self, x: f64) -> f64 {
        match self.scale {
            Scale::Log => (x.max(0.0) + self.shift).ln() - self.shift.ln(),
            _ => self.scale.apply(x),
        }
    }

    /// The value at `t` on the scaled axis.
    fn value(&self, t: f64) -> f64 {
        match self.scale {
            Scale::Log => ((t + self.shift.ln()).exp() - self.shift).max(0.0),
            _ => self.scale.invert(t),
        }
    }

    /// The bin `x` falls in.
    pub fn key(&self, x: f64) -> i32 {
        (self.t(x) / self.width).floor() as i32
    }

    /// Scaled position where bin `k` starts.
    fn start(&self, k: i32) -> f64 {
        k as f64 * self.width
    }

    /// Smallest whole count in bin `k` or above: the cutoff that drops every
    /// bin left of `k`.
    pub fn lower_edge(&self, k: i32) -> usize {
        if k <= 0 {
            return 0;
        }
        // Start from the exact inverse, then settle rounding either way.
        let mut x = self.value(self.start(k)).ceil().max(0.0) as usize;
        while self.key(x as f64) < k {
            x += 1;
        }
        while x > 0 && self.key((x - 1) as f64) >= k {
            x -= 1;
        }
        x
    }

    /// The value where bin `k` starts, its tick's label.
    fn tick_value(&self, k: i32) -> f64 {
        self.value(self.start(k))
    }

    /// About six ticks across.
    fn tick_every(&self, nbins: usize) -> i32 {
        (nbins as i32 / 6).max(1)
    }

    /// The ticks over bins `kmin..=kmax`, as bin and value: on the log
    /// scale at the axis' 1-2-5 values, so labels read round; else every
    /// [`Binning::tick_every`] bins.
    fn ticks(&self, kmin: i32, kmax: i32) -> Vec<(i32, f64)> {
        match self.scale {
            Scale::Log => {
                let axis = Axis {
                    lo: self.tick_value(kmin),
                    hi: self.value(self.start(kmax + 1)),
                    log: true,
                    shift: self.shift,
                };
                axis.ticks().into_iter().map(|v| (self.key(v), v)).collect()
            }
            _ => {
                let every = self.tick_every((kmax - kmin + 1).max(1) as usize);
                (kmin..=kmax)
                    .filter(|k| k % every == 0)
                    .map(|k| (k, self.tick_value(k)))
                    .collect()
            }
        }
    }
}

/// A sorted statistic binned on a scale: the bins and their counts.
pub struct Binned {
    /// The bins.
    pub bins: Binning,
    /// The first bin's key.
    pub kmin: i32,
    /// How many values fall in each bin, from `kmin` on.
    pub counts: Vec<usize>,
}

impl Binned {
    /// Bin `sorted`: finite, ascending. Its ends are the range the bins
    /// span. All-whole data gets whole-count bins.
    pub fn new(sorted: &[f32], scale: Scale) -> Self {
        let (min, max) = match (sorted.first(), sorted.last()) {
            (Some(&lo), Some(&hi)) => (lo as f64, hi as f64),
            _ => (0.0, 0.0),
        };
        let integer = sorted.iter().all(|v| v.fract() == 0.0);
        // The log scale shifts as the axis does for these values.
        let shift = match integer {
            true => 1.0,
            false => Axis::fit(sorted, true).shift,
        };
        let mut bins = Binning::shifted(scale, min, max, integer, shift);
        // Bins sized to the range number about TARGET_BINS; widen them
        // should rounding ever make many more.
        let span = |b: &Binning| i64::from(b.key(max)) - i64::from(b.key(min)) + 1;
        while span(&bins) > MAX_BINS {
            bins.width *= 2.0;
        }
        let kmin = bins.key(min);
        let nbins = span(&bins).max(1) as usize;
        let counts = count(&bins, kmin, nbins, sorted.iter().copied());
        Self { bins, kmin, counts }
    }

    /// Counts of the finite `values` in these bins.
    pub fn count(&self, values: impl Iterator<Item = f32>) -> Vec<usize> {
        count(&self.bins, self.kmin, self.counts.len(), values)
    }

    /// The last bin's key.
    pub fn kmax(&self) -> i32 {
        self.kmin + self.counts.len() as i32 - 1
    }
}

/// Counts of the finite `values` per bin, from `kmin` to `kmin + nbins - 1`
/// (values outside land in the end bins).
fn count(bins: &Binning, kmin: i32, nbins: usize, values: impl Iterator<Item = f32>) -> Vec<usize> {
    let mut counts = vec![0; nbins];
    for v in values.filter(|v| v.is_finite()) {
        let i = (bins.key(v as f64) - kmin).clamp(0, nbins as i32 - 1);
        counts[i as usize] += 1;
    }
    counts
}

/// Median of an ascending slice (0 when empty).
pub fn median(sorted: &[f32]) -> f32 {
    let n = sorted.len();
    match n {
        0 => 0.0,
        _ if n.is_multiple_of(2) => (sorted[n / 2 - 1] + sorted[n / 2]) / 2.0,
        _ => sorted[n / 2],
    }
}

/// Compact number for axis labels: 950, 1.2k, 35k, 1.1M; small fractions
/// keep two significant digits.
pub fn compact(v: f64) -> String {
    if v != 0.0 && v.abs() < 10.0 && v.fract() != 0.0 {
        format!("{:.2}", v)
            .trim_end_matches('0')
            .trim_end_matches('.')
            .to_string()
    } else if v < 1e3 {
        format!("{}", v.round() as i64)
    } else if v < 1e4 {
        format!("{:.1}k", v / 1e3)
    } else if v < 1e6 {
        format!("{}k", (v / 1e3).round() as u64)
    } else if v < 1e9 {
        format!("{:.1}M", v / 1e6)
    } else {
        format!("{:.1}G", v / 1e9)
    }
}

/// A bar height [`HistPlot`] can draw: whole counts, or any non-negative
/// real value (a summed signal, a log statistic).
pub trait BarValue: Copy {
    /// The bar's height in count units.
    fn bar(self) -> f64;
}

impl BarValue for usize {
    fn bar(self) -> f64 {
        self as f64
    }
}

impl BarValue for f64 {
    fn bar(self) -> f64 {
        self
    }
}

/// A histogram of `counts` over bins `kmin..`, scaled to them.
pub struct HistPlot<'a, T: BarValue = usize> {
    /// The bins the bars stand on.
    pub bins: Binning,
    /// The first bar's key.
    pub kmin: i32,
    /// Each bar's height, from `kmin` on.
    pub counts: &'a [T],
    /// Style of each bin's bar, by key.
    pub style: &'a dyn Fn(i32) -> Style,
    /// A subset drawn in front, in the bar style; `counts` then draw dimmed
    /// behind it. Counted on the same bins; a bar past `counts`' end has
    /// nothing behind it.
    pub subset: Option<&'a [T]>,
    /// How bar heights are drawn.
    pub y_scale: Scale,
    /// Top of the y axis in count units; `None` scales to the tallest bar.
    /// Set it to put several plots on one scale.
    pub y_max: Option<f64>,
    /// Bin under the accent rule and ▲.
    pub pointer: Option<i32>,
    /// Other symbols on the x axis, by key.
    pub marks: Vec<(i32, &'static str, Style)>,
    /// Tick label at bin `k` in place of the bin's value; `None` from it
    /// drops that tick, so the labels decide where ticks go (e.g. at
    /// category boundaries with `tick_every: Some(1)`). Unset, every tick
    /// shows its value.
    pub x_label: Option<&'a dyn Fn(i32) -> Option<String>>,
    /// Ticks every this many bins, in place of the scale's default.
    pub tick_every: Option<i32>,
    /// The styles of the axes, the dimmed bars, the pointer and its rule.
    pub look: Look,
}

const EIGHTHS: [&str; 8] = ["▁", "▂", "▃", "▄", "▅", "▆", "▇", "█"];

/// A plot's areas in `area`: the y gutter and the chart right of it over
/// the last two rows, which hold the x axis and its labels.
fn frame(area: Rect) -> [Rect; 4] {
    let [plot, axis, labels] = Layout::vertical([
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let [gutter, chart] =
        Layout::horizontal([Constraint::Length(GUTTER), Constraint::Min(1)]).areas(plot);
    [gutter, chart, axis, labels]
}

/// Bar lengths on a y scale, in whole `cells` units of its top.
#[derive(Clone, Copy)]
struct YScale {
    scale: Scale,
    /// The top: the tallest bar, or `y_max` if higher, on the scale.
    max_h: f64,
    cells: usize,
}

impl YScale {
    fn new<T: BarValue>(
        scale: Scale,
        y_max: Option<f64>,
        counts: impl IntoIterator<Item = T>,
        cells: usize,
    ) -> Self {
        let tallest = counts
            .into_iter()
            .map(|c| scale.apply(c.bar().max(0.0)))
            .fold(0.0, f64::max);
        let max_h = y_max.map_or(tallest, |m| scale.apply(m.max(0.0)).max(tallest));
        Self {
            scale,
            max_h,
            cells,
        }
    }

    /// `c`'s length in units: none for nothing, else at least one.
    fn units<T: BarValue>(self, c: T) -> usize {
        if c.bar() <= 0.0 || self.max_h <= 0.0 {
            0
        } else {
            let h = self.scale.apply(c.bar().max(0.0));
            ((h / self.max_h * self.cells as f64).round() as usize).clamp(1, self.cells)
        }
    }

    /// The count at `t` of the top.
    fn value_at(self, t: f64) -> f64 {
        self.scale.invert(self.max_h * t)
    }
}

/// The axis lines: the y axis down the gutter's last column, which it
/// returns, and the x axis from there along `axis`.
fn axes(buf: &mut Buffer, gutter: Rect, axis: Rect, dim: Style) -> u16 {
    let gx = gutter.right() - 1;
    for y in gutter.top()..gutter.bottom() {
        put(buf, gx, y, "│", dim);
    }
    for x in axis.left()..axis.right() {
        let sym = match x.cmp(&gx) {
            std::cmp::Ordering::Less => " ",
            std::cmp::Ordering::Equal => "└",
            std::cmp::Ordering::Greater => "─",
        };
        put(buf, x, axis.y, sym, dim);
    }
    gx
}

/// `s` on the y axis at row `y`, right-aligned in the gutter.
fn y_label(buf: &mut Buffer, gutter: Rect, y: u16, s: &str, dim: Style) {
    let gx = gutter.right() - 1;
    let x = gx.saturating_sub(1 + s.len() as u16).max(gutter.x);
    buf.set_string(x, y, s, dim);
    put(buf, gx, y, "┤", dim);
}

/// A tick at column `x` of `axis`, labelled `s` below where it fits clear
/// of the label before (which ended before `next_free`).
fn x_tick(
    buf: &mut Buffer,
    x: u16,
    axis: Rect,
    labels: Rect,
    next_free: &mut u16,
    s: &str,
    dim: Style,
) {
    put(buf, x, axis.y, "┴", dim);
    if x >= *next_free && x + (s.len() as u16) <= labels.right() {
        buf.set_string(x, labels.y, s, dim);
        *next_free = x + s.len() as u16 + 1;
    }
}

impl<T: BarValue> HistPlot<'_, T> {
    /// Draw into `area`: bars over the rows above the last two, which hold
    /// the x axis and its labels; the left [`GUTTER`] columns hold the y axis.
    pub fn render(&self, buf: &mut Buffer, area: Rect) {
        let look = &self.look;
        let (dim, accented, highlight) = (look.hint, ink::accent(look), ink::highlight(look));
        let [gutter, chart, axis, labels] = frame(area);
        if chart.width == 0 || chart.height == 0 {
            return;
        }
        let nbins = self.counts.len();
        let bw = (chart.width / nbins.max(1) as u16).clamp(1, 4);
        // The bars that fit in the chart, and those of them `counts` has.
        let fit = usize::from(chart.width.div_ceil(bw));
        let shown = nbins.min(fit);
        let x_of = |k: i32| -> Option<u16> {
            let i = k - self.kmin;
            (i >= 0 && (i as usize) < shown).then(|| chart.x + i as u16 * bw)
        };
        let y = YScale::new(
            self.y_scale,
            self.y_max,
            self.counts.iter().copied(),
            chart.height as usize * 8,
        );

        if let Some(x) = self.pointer.and_then(x_of) {
            for y in chart.top()..chart.bottom() {
                put(buf, x, y, "┊", accented);
            }
        }

        let mut bars = |counts: &[T], behind: Option<&[T]>, dimmed: bool| {
            for (i, &c) in counts.iter().enumerate().take(fit) {
                let x0 = chart.x + i as u16 * bw;
                let style = if dimmed {
                    dim
                } else {
                    (self.style)(self.kmin + i as i32)
                };
                // A bar `behind` lacks (a subset counted on other bins) is 0.
                let under = behind.and_then(|b| b.get(i)).map_or(0, |&b| y.units(b));
                let top = y.units(c);
                for (j, y) in (chart.top()..chart.bottom()).rev().enumerate() {
                    let mut fill = top.saturating_sub(j * 8).min(8);
                    if fill == 0 {
                        break;
                    }
                    // A partial top in front of a taller bar would show a gap
                    // above it (the terminal's foreground cannot be a
                    // background), so it rounds up to a whole cell.
                    if under >= (j + 1) * 8 {
                        fill = 8;
                    }
                    for x in x0..(x0 + bw).min(chart.right()) {
                        put(buf, x, y, EIGHTHS[fill - 1], style);
                    }
                }
            }
        };
        bars(self.counts, None, self.subset.is_some());
        if let Some(subset) = self.subset {
            bars(subset, Some(self.counts), false);
        }

        // y axis: count at the top and at half height on the y scale.
        axes(buf, gutter, axis, dim);
        if y.max_h > 0.0 {
            y_label(buf, gutter, gutter.top(), &compact(y.value_at(1.0)), dim);
            if gutter.height >= 6 {
                let mid = gutter.top() + gutter.height / 2;
                y_label(buf, gutter, mid, &compact(y.value_at(0.5)), dim);
            }
        }

        // x axis: ticks, labels below, then the marks.
        let mut next_free = labels.x;
        let kmax = self.kmin + nbins as i32 - 1;
        // Labels given, or a spacing, tick every so many bins that fit;
        // else the bins' own ticks (round values on the log scale).
        let ticks: Vec<(i32, Option<f64>)> = match (self.x_label, self.tick_every) {
            (None, None) => self
                .bins
                .ticks(self.kmin, kmax)
                .into_iter()
                .map(|(k, v)| (k, Some(v)))
                .collect(),
            _ => {
                let every = self
                    .tick_every
                    .unwrap_or_else(|| self.bins.tick_every(nbins))
                    .max(1);
                (self.kmin..self.kmin + shown as i32)
                    .filter(|k| k % every == 0)
                    .map(|k| (k, None))
                    .collect()
            }
        };
        for (k, value) in ticks {
            let Some(x) = x_of(k) else { continue };
            let s = match (self.x_label, value) {
                (Some(label), _) => match label(k) {
                    Some(s) => s,
                    None => continue,
                },
                (None, Some(v)) => compact(v),
                (None, None) => compact(self.bins.tick_value(k)),
            };
            x_tick(buf, x, axis, labels, &mut next_free, &s, dim);
        }
        let pointer = self.pointer.map(|k| (k, "▲", highlight));
        for &(k, sym, style) in self.marks.iter().chain(pointer.iter()) {
            if let Some(x) = x_of(k) {
                put(buf, x, axis.y, sym, style);
            }
        }
    }
}

/// One side of a [`MirrorPlot`].
pub struct MirrorSide<'a, T: BarValue = f64> {
    /// Each bar's length.
    pub counts: &'a [T],
    /// A subset drawn in front, in `style`; `counts` then draw dimmed
    /// behind it. A bar past `counts`' end has nothing behind it.
    pub subset: Option<&'a [T]>,
    /// The bars' style.
    pub style: Style,
    /// Named in the side's outer corner; empty for none.
    pub name: &'a str,
}

/// Two bar series on one scale around a zero line, one growing up and the
/// other down, as a Miami plot, in half cells. Signed values are a mirror of
/// their positive and negative parts. Axes as [`HistPlot`]'s, one column
/// per bar.
pub struct MirrorPlot<'a, T: BarValue = f64> {
    /// The side above the zero line.
    pub up: MirrorSide<'a, T>,
    /// The side below it.
    pub down: MirrorSide<'a, T>,
    /// How bar lengths are drawn.
    pub y_scale: Scale,
    /// Top of either side in count units; `None` scales to the tallest bar.
    pub y_max: Option<f64>,
    /// Labels at the top, the zero line and the bottom, in place of the
    /// scale's own.
    pub y_labels: Option<[String; 3]>,
    /// Bar under the accent rule and ▲.
    pub pointer: Option<usize>,
    /// Tick label at bar `i`; `None` from it drops that tick. Unset, there
    /// are no ticks.
    pub x_label: Option<&'a dyn Fn(usize) -> Option<String>>,
    /// The styles of the axes, the dimmed bars, the pointer and its rule.
    pub look: Look,
}

impl<T: BarValue> MirrorPlot<'_, T> {
    /// Draw into `area`: the halves over the rows above the last two, which
    /// hold the x axis and its labels; the left [`GUTTER`] columns hold the
    /// y axis.
    pub fn render(&self, buf: &mut Buffer, area: Rect) {
        let look = &self.look;
        let (dim, accented, highlight) = (look.hint, ink::accent(look), ink::highlight(look));
        let [gutter, chart, axis, labels] = frame(area);
        if chart.width == 0 || chart.height < 3 {
            return;
        }
        let half = (chart.height - 1) / 2;
        let zero = chart.top() + half;
        let x_of = |i: usize| Some(chart.x + i as u16).filter(|&x| x < chart.right());
        let all = self.up.counts.iter().chain(self.down.counts).copied();
        let y = YScale::new(self.y_scale, self.y_max, all, half as usize * 2);

        if let Some(x) = self.pointer.and_then(x_of) {
            for y in chart.top()..chart.top() + 2 * half + 1 {
                put(buf, x, y, "┊", accented);
            }
        }
        for x in chart.left()..chart.right() {
            put(buf, x, zero, "─", dim);
        }
        for (side, up) in [(&self.up, true), (&self.down, false)] {
            let (whole, part) = if up { ("█", "▄") } else { ("█", "▀") };
            let mut bars = |counts: &[T], behind: Option<&[T]>, style: Style| {
                for (i, &c) in counts.iter().enumerate() {
                    let Some(x) = x_of(i) else { break };
                    // A bar `behind` lacks is 0, as in HistPlot.
                    let under = behind.and_then(|b| b.get(i)).map_or(0, |&b| y.units(b));
                    let top = y.units(c);
                    for k in 0..top.div_ceil(2) {
                        let y = if up {
                            zero - 1 - k as u16
                        } else {
                            zero + 1 + k as u16
                        };
                        // As in HistPlot, a partial end in front of a longer
                        // bar rounds up to a whole cell.
                        let full = 2 * k + 2 <= top || under >= 2 * k + 2;
                        put(buf, x, y, if full { whole } else { part }, style);
                    }
                }
            };
            match side.subset {
                Some(subset) => {
                    bars(side.counts, None, dim);
                    bars(subset, Some(side.counts), side.style);
                }
                None => bars(side.counts, None, side.style),
            }
        }
        buf.set_string(chart.x, chart.top(), self.up.name, dim);
        buf.set_string(chart.x, chart.top() + 2 * half, self.down.name, dim);

        // y axis: the scale's top on both sides of the zero line.
        axes(buf, gutter, axis, dim);
        let own = || {
            let top = compact(y.value_at(1.0));
            [top.clone(), "0".to_string(), top]
        };
        let ys = [chart.top(), zero, chart.top() + 2 * half];
        for (y, s) in ys
            .into_iter()
            .zip(self.y_labels.clone().unwrap_or_else(own))
        {
            y_label(buf, gutter, y, &s, dim);
        }

        // x axis: ticks, labels below, then the pointer.
        let n = self.up.counts.len().max(self.down.counts.len());
        let mut next_free = labels.x;
        for i in 0..n {
            let Some(x) = x_of(i) else { break };
            if let Some(s) = self.x_label.and_then(|label| label(i)) {
                x_tick(buf, x, axis, labels, &mut next_free, &s, dim);
            }
        }
        if let Some(x) = self.pointer.and_then(x_of) {
            put(buf, x, axis.y, "▲", highlight);
        }
    }
}

/// The x axis of a text plot in `area` without its tick marks: the axis
/// line runs through where the plot put a `┴` under each label. For labels
/// that name a span (a region), where a tick would read as a position.
pub fn plain_axis(buf: &mut Buffer, area: Rect) {
    for y in area.top()..area.bottom() {
        for x in area.left()..area.right() {
            if buf[(x, y)].symbol() == "┴" {
                buf[(x, y)].set_symbol("─");
            }
        }
    }
}

#[cfg(test)]
#[path = "tests/hist.rs"]
mod tests;
