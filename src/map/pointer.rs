//! The mouse and keys on a map: what a terminal event means for its camera
//! and its labels, with the viewer left to act on it.
//!
//! [`Pointer`] is senna's map mouse (`src/view/tui/input.rs`, `mouse`;
//! the cell to pixel map from `cell_to_px` and `map_px` in
//! `src/view/tui/{input,mod}.rs`): a drag pans, the wheel zooms at the
//! pointer, a press released without moving clicks, motion hovers. From
//! pinto (`src/view/tui.rs`, `mouse`): any release ends a drag, even off
//! the map, a drag goes on past the map's edge, and the right button or a
//! held modifier adds to a pick. New here: a press on a label drags the
//! label instead of the map.
//!
//! Terminal mouse events come in whole cells, so a dragged label moves a
//! cell at a time; [`LabelEdit`] is the keyboard's finer way, in half
//! line heights. [`nav_key`] is senna's zoom, pan and fit keys.

use super::labels::Placed;
use super::offsets::Offsets;
use super::viewport::{Limits, Viewport};
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseButton, MouseEvent, MouseEventKind,
};
use ratatui::layout::Rect;
use std::hash::Hash;

/// Where a map is on the screen, and how many of its frame's pixels a
/// terminal cell holds.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MapArea {
    pub rect: Rect,
    /// Frame pixels per cell, across and down.
    pub cell: (f32, f32),
}

impl MapArea {
    /// Whether cell `(col, row)` is on the map.
    #[must_use]
    pub fn contains(&self, col: u16, row: u16) -> bool {
        let r = self.rect;
        col >= r.x && row >= r.y && col < r.right() && row < r.bottom()
    }

    /// The frame pixel at the centre of cell `(col, row)`, when it is on
    /// the map.
    #[must_use]
    pub fn px(&self, col: u16, row: u16) -> Option<(f32, f32)> {
        self.contains(col, row).then(|| self.centre(col, row))
    }

    /// The frame pixel at the centre of the map's cell nearest `(col, row)`.
    #[must_use]
    pub fn px_clamped(&self, col: u16, row: u16) -> (f32, f32) {
        let r = self.rect;
        let last = |from: u16, n: u16| from + n.max(1) - 1;
        self.centre(
            col.clamp(r.x, last(r.x, r.width)),
            row.clamp(r.y, last(r.y, r.height)),
        )
    }

    /// The frame's size in pixels, a whole number of cells.
    #[must_use]
    pub fn size_px(&self) -> (usize, usize) {
        (
            (f32::from(self.rect.width) * self.cell.0) as usize,
            (f32::from(self.rect.height) * self.cell.1) as usize,
        )
    }

    fn centre(&self, col: u16, row: u16) -> (f32, f32) {
        (
            (f32::from(col.saturating_sub(self.rect.x)) + 0.5) * self.cell.0,
            (f32::from(row.saturating_sub(self.rect.y)) + 0.5) * self.cell.1,
        )
    }
}

/// How a viewer's mouse behaves.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct PointerOpts {
    /// The zoom of one wheel step: in on the way up, out by its inverse.
    pub wheel: f32,
    /// A drag goes on when the pointer leaves the map (pinto); otherwise
    /// motion off the map is ignored. Either way any release ends it.
    pub drag_outside: bool,
    /// A right press on the map is a click that adds (pinto).
    pub right_click_adds: bool,
    /// Held on release, these make a click one that adds (pinto: Ctrl or
    /// Alt; Shift is the terminal's, for selecting text).
    pub add_mods: KeyModifiers,
}

/// senna's wheel, pinto's drag past the edge, and no click that adds.
impl Default for PointerOpts {
    fn default() -> Self {
        Self {
            wheel: 1.25,
            drag_outside: true,
            right_click_adds: false,
            add_mods: KeyModifiers::NONE,
        }
    }
}

/// What a mouse event asks of the map, in frame pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum MapAction<K> {
    None,
    /// The pointer is over pixel `(px, py)`.
    Hover {
        px: f32,
        py: f32,
    },
    /// The pointer has left the map: whatever it hovered is not.
    Away,
    /// Move the picture by `(dx, dy)`, as [`Viewport::pan_px`] takes it.
    Pan {
        dx: f32,
        dy: f32,
    },
    /// Zoom by `factor` (over one is in) keeping pixel `at` fixed.
    Zoom {
        factor: f32,
        at: (f32, f32),
    },
    /// Move label `key` by `(dx, dy)` pixels, down the screen for `dy`;
    /// over its font's line height, that is a nudge of its offset in em.
    MoveLabel {
        key: K,
        dx: f32,
        dy: f32,
    },
    /// A press released where it was, at pixel `(px, py)`; `add` when it
    /// adds to the pick rather than replacing it.
    Click {
        px: f32,
        py: f32,
        add: bool,
    },
}

impl<K> MapAction<K> {
    /// Move camera `vp` as a pan or a zoom asks, within `lim`. Returns
    /// whether it was one of those.
    pub fn apply(&self, vp: &mut Viewport, lim: &Limits) -> bool {
        match *self {
            Self::Pan { dx, dy } => vp.pan_px(dx, dy, lim),
            Self::Zoom { factor, at } => vp.zoom_at(factor, at.0, at.1, lim),
            _ => return false,
        }
        true
    }
}

/// A press being held: where it last was, whether it has moved, and the
/// label it took hold of.
#[derive(Clone, Copy, Debug)]
struct Drag<K> {
    col: u16,
    row: u16,
    moved: bool,
    label: Option<K>,
}

/// The map's mouse: terminal events in, [`MapAction`]s out.
#[derive(Clone, Debug)]
pub struct Pointer<K> {
    opts: PointerOpts,
    drag: Option<Drag<K>>,
    over: bool,
}

impl<K: Copy> Pointer<K> {
    #[must_use]
    pub fn new(opts: PointerOpts) -> Self {
        Self {
            opts,
            drag: None,
            over: false,
        }
    }

    /// What `m` asks of the map at `a`. A left press over a label (`hit`
    /// on its pixel gives the label's key) starts dragging that label,
    /// anywhere else the map; a press released without moving a cell is a
    /// click.
    pub fn handle(
        &mut self,
        a: &MapArea,
        m: &MouseEvent,
        hit: impl Fn(f32, f32) -> Option<K>,
    ) -> MapAction<K> {
        let at = a.px(m.column, m.row);
        match m.kind {
            MouseEventKind::Moved => match at {
                Some((px, py)) => {
                    self.over = true;
                    MapAction::Hover { px, py }
                }
                None if std::mem::take(&mut self.over) => MapAction::Away,
                None => MapAction::None,
            },
            MouseEventKind::ScrollUp | MouseEventKind::ScrollDown => {
                let Some(at) = at else {
                    return MapAction::None;
                };
                let factor = if m.kind == MouseEventKind::ScrollUp {
                    self.opts.wheel
                } else {
                    1.0 / self.opts.wheel
                };
                MapAction::Zoom { factor, at }
            }
            MouseEventKind::Down(MouseButton::Left) => {
                if let Some((px, py)) = at {
                    self.drag = Some(Drag {
                        col: m.column,
                        row: m.row,
                        moved: false,
                        label: hit(px, py),
                    });
                }
                MapAction::None
            }
            MouseEventKind::Down(MouseButton::Right) if self.opts.right_click_adds => match at {
                Some((px, py)) => MapAction::Click { px, py, add: true },
                None => MapAction::None,
            },
            MouseEventKind::Drag(MouseButton::Left) => {
                let Some(d) = self.drag.as_mut() else {
                    return MapAction::None;
                };
                if (at.is_none() && !self.opts.drag_outside) || (m.column, m.row) == (d.col, d.row)
                {
                    return MapAction::None;
                }
                let dx = (f32::from(m.column) - f32::from(d.col)) * a.cell.0;
                let dy = (f32::from(m.row) - f32::from(d.row)) * a.cell.1;
                (d.col, d.row, d.moved) = (m.column, m.row, true);
                match d.label {
                    Some(key) => MapAction::MoveLabel { key, dx, dy },
                    None => MapAction::Pan { dx, dy },
                }
            }
            // Any release ends a drag, wherever it is; a left press that
            // never moved, released over the map, is a click.
            MouseEventKind::Up(button) => match (self.drag.take(), at) {
                (Some(d), Some((px, py))) if button == MouseButton::Left && !d.moved => {
                    let add = m.modifiers.intersects(self.opts.add_mods);
                    MapAction::Click { px, py, add }
                }
                _ => MapAction::None,
            },
            _ => MapAction::None,
        }
    }

    /// Let go of any press being held, as when the map goes away under it.
    pub fn cancel(&mut self) {
        self.drag = None;
    }

    /// Whether a press is being held.
    #[must_use]
    pub fn dragging(&self) -> bool {
        self.drag.is_some()
    }

    /// The label being dragged, if one is.
    #[must_use]
    pub fn label(&self) -> Option<K> {
        self.drag.and_then(|d| d.label)
    }
}

/// The key that enters label editing: Shift+l.
pub const EDIT_KEY: char = 'L';

/// How far an arrow moves a label, in em.
pub const NUDGE: f32 = 0.5;
/// How far an arrow with Shift moves a label, in em.
pub const NUDGE_FAR: f32 = 2.0;

/// What a key did in label editing.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditKey {
    /// Not a key of the mode: the viewer's, as ever.
    Pass,
    /// Taken; the offsets are as they were (the selected label may not be).
    Taken,
    /// An offset changed: draw again, and keep the offsets.
    Changed,
    /// Esc: the mode is over.
    Leave,
}

/// Moving labels from the keyboard, one selected at a time. New here.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LabelEdit<K> {
    /// The selected label.
    pub at: Option<K>,
}

impl<K> Default for LabelEdit<K> {
    fn default() -> Self {
        Self { at: None }
    }
}

impl<K: Copy + Eq + Hash> LabelEdit<K> {
    /// Whether `k` starts label editing: [`EDIT_KEY`], Ctrl and Alt up.
    #[must_use]
    pub fn starts(k: &KeyEvent) -> bool {
        k.kind != KeyEventKind::Release
            && k.code == KeyCode::Char(EDIT_KEY)
            && !k
                .modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    }

    /// Editing begun, the first of the `placed` labels as read selected.
    #[must_use]
    pub fn enter(placed: &[Placed<K>]) -> Self {
        Self {
            at: reading(placed).first().copied(),
        }
    }

    /// Key `k` in label editing, over the labels last `placed`: Tab and
    /// BackTab select the next or previous label as read (top to bottom,
    /// then left to right, by anchor, so moving one keeps its turn), the
    /// arrows nudge the selected one [`NUDGE`] em (Shift: [`NUDGE_FAR`]),
    /// Backspace puts it back on its anchor, Esc leaves. With none
    /// selected, an arrow selects the first.
    pub fn key(&mut self, k: &KeyEvent, placed: &[Placed<K>], o: &mut Offsets<K>) -> EditKey {
        if k.kind == KeyEventKind::Release
            || k.modifiers
                .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
        {
            return EditKey::Pass;
        }
        let step = if k.modifiers.contains(KeyModifiers::SHIFT) {
            NUDGE_FAR
        } else {
            NUDGE
        };
        let d = match k.code {
            KeyCode::Esc => {
                self.at = None;
                return EditKey::Leave;
            }
            KeyCode::Tab | KeyCode::BackTab => {
                let order = reading(placed);
                let n = order.len();
                if n > 0 {
                    let i = self.at.and_then(|a| order.iter().position(|&k| k == a));
                    let next = match (i, k.code == KeyCode::BackTab) {
                        (Some(i), false) => (i + 1) % n,
                        (Some(i), true) => (i + n - 1) % n,
                        (None, false) => 0,
                        (None, true) => n - 1,
                    };
                    self.at = Some(order[next]);
                }
                return EditKey::Taken;
            }
            KeyCode::Backspace => {
                return match self.at {
                    Some(a) if o.get(&a).is_some() => {
                        o.reset(&a);
                        EditKey::Changed
                    }
                    _ => EditKey::Taken,
                };
            }
            KeyCode::Left => (-step, 0.0),
            KeyCode::Right => (step, 0.0),
            KeyCode::Up => (0.0, -step),
            KeyCode::Down => (0.0, step),
            _ => return EditKey::Pass,
        };
        match self.at {
            Some(a) => {
                o.nudge(a, d);
                EditKey::Changed
            }
            None => {
                *self = Self::enter(placed);
                EditKey::Taken
            }
        }
    }
}

/// The placed labels' keys as read: by anchor, top to bottom, then left to
/// right.
fn reading<K: Copy>(placed: &[Placed<K>]) -> Vec<K> {
    let mut by: Vec<&Placed<K>> = placed.iter().collect();
    by.sort_by(|a, b| {
        a.anchor
            .1
            .total_cmp(&b.anchor.1)
            .then(a.anchor.0.total_cmp(&b.anchor.0))
    });
    by.into_iter().map(|p| p.key).collect()
}

/// A camera move a key asks for.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Nav {
    /// Zoom by this (over one is in) about the frame's centre.
    Zoom(f32),
    /// Move the picture by these pixels, as [`Viewport::pan_px`] takes them.
    Pan(f32, f32),
    /// Back to the whole map.
    Fit,
}

impl Nav {
    /// Move camera `vp` within `lim`; [`Nav::Fit`] takes the camera `fit`
    /// gives.
    pub fn apply(self, vp: &mut Viewport, lim: &Limits, fit: impl FnOnce() -> Viewport) {
        match self {
            Self::Zoom(f) => vp.zoom_at(f, 0.5 * vp.w as f32, 0.5 * vp.h as f32, lim),
            Self::Pan(dx, dy) => vp.pan_px(dx, dy, lim),
            Self::Fit => *vp = fit(),
        }
    }
}

/// senna's map keys: `+`/`=` zoom in by `zoom`, `-`/`_` out by it, an
/// arrow pans `pan` pixels (the picture moves against the arrow, so the
/// view goes its way), `0` fits the whole map. Any other key, any with
/// Ctrl or Alt, and Shift with an arrow, are the viewer's.
#[must_use]
pub fn nav_key(k: &KeyEvent, zoom: f32, pan: f32) -> Option<Nav> {
    if k.kind == KeyEventKind::Release
        || k.modifiers
            .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
    {
        return None;
    }
    let shift = k.modifiers.contains(KeyModifiers::SHIFT);
    Some(match k.code {
        KeyCode::Char('+' | '=') => Nav::Zoom(zoom),
        KeyCode::Char('-' | '_') => Nav::Zoom(1.0 / zoom),
        KeyCode::Char('0') => Nav::Fit,
        KeyCode::Left if !shift => Nav::Pan(pan, 0.0),
        KeyCode::Right if !shift => Nav::Pan(-pan, 0.0),
        KeyCode::Up if !shift => Nav::Pan(0.0, pan),
        KeyCode::Down if !shift => Nav::Pan(0.0, -pan),
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../tests/pointer.rs"]
mod tests;
