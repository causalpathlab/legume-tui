//! The map's pointer fed terminal mouse events: pan, zoom at the pointer,
//! click, hover, label drag and release outside the map; the label-edit
//! keys and the map's navigation keys.

use super::*;
use crate::map::labels::Placed;
use ratatui::crossterm::event::{KeyEventKind, KeyEventState};

/// A map of 20 x 10 cells at (5, 2), each cell 8 x 16 frame pixels.
fn area() -> MapArea {
    MapArea {
        rect: Rect::new(5, 2, 20, 10),
        cell: (8.0, 16.0),
    }
}

fn ev(kind: MouseEventKind, column: u16, row: u16) -> MouseEvent {
    MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    }
}

const LEFT: MouseButton = MouseButton::Left;

/// No label anywhere.
fn nothing(_: f32, _: f32) -> Option<u32> {
    None
}

/// Label 7 over the cells of columns 10..=12 on row 4.
fn label_seven(px: f32, py: f32) -> Option<u32> {
    let a = area();
    let (x0, y0) = a.px(10, 4).unwrap();
    let (x1, _) = a.px(12, 4).unwrap();
    ((x0..=x1).contains(&px) && (py - y0).abs() < 1.0).then_some(7)
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn shift(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::SHIFT)
}

#[test]
fn cells_map_to_the_centres_of_their_pixels_and_only_inside() {
    let a = area();
    assert_eq!(a.px(5, 2), Some((4.0, 8.0)));
    assert_eq!(a.px(6, 3), Some((12.0, 24.0)));
    assert_eq!(a.px(24, 11), Some((156.0, 152.0)));
    assert_eq!(a.px(4, 2), None);
    assert_eq!(a.px(25, 2), None);
    assert_eq!(a.px(5, 12), None);
    // Outside, the nearest cell of the map.
    assert_eq!(a.px_clamped(0, 0), (4.0, 8.0));
    assert_eq!(a.px_clamped(40, 30), (156.0, 152.0));
    assert_eq!(a.px_clamped(6, 3), (12.0, 24.0));
    assert_eq!(a.size_px(), (160, 160));
}

#[test]
fn a_drag_pans_by_the_cells_moved_in_pixels() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Down(LEFT), 10, 5), nothing),
        MapAction::None
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 12, 4), nothing),
        MapAction::Pan {
            dx: 16.0,
            dy: -16.0
        }
    );
    // From where the last step left off, not from the press.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 11, 4), nothing),
        MapAction::Pan { dx: -8.0, dy: 0.0 }
    );
    // A step that stays in its cell moves nothing.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 11, 4), nothing),
        MapAction::None
    );
    // Released after moving: no click.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 11, 4), nothing),
        MapAction::None
    );
    assert!(!p.dragging());
    // A drag with no press first is nothing.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 13, 4), nothing),
        MapAction::None
    );
}

#[test]
fn the_wheel_zooms_at_the_pointer_and_only_over_the_map() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::ScrollUp, 6, 3), nothing),
        MapAction::Zoom {
            factor: 1.25,
            at: (12.0, 24.0)
        }
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::ScrollDown, 6, 3), nothing),
        MapAction::Zoom {
            factor: 0.8,
            at: (12.0, 24.0)
        }
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::ScrollUp, 0, 0), nothing),
        MapAction::None
    );
    let mut p: Pointer<u32> = Pointer::new(PointerOpts {
        wheel: 2.0,
        ..PointerOpts::default()
    });
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::ScrollDown, 5, 2), nothing),
        MapAction::Zoom {
            factor: 0.5,
            at: (4.0, 8.0)
        }
    );
}

#[test]
fn a_press_and_release_without_moving_is_a_click() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 6, 3), nothing);
    assert!(p.dragging());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 6, 3), nothing),
        MapAction::Click {
            px: 12.0,
            py: 24.0,
            add: false
        }
    );
    // Once only.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 6, 3), nothing),
        MapAction::None
    );
    // A press outside the map is not the map's.
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 0, 0), nothing);
    assert!(!p.dragging());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 0, 0), nothing),
        MapAction::None
    );
    // A dragged-and-back press is a drag, not a click.
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 6, 3), nothing);
    p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 7, 3), nothing);
    p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 6, 3), nothing);
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 6, 3), nothing),
        MapAction::None
    );
}

#[test]
fn held_modifiers_or_the_right_button_add_to_the_pick_when_asked() {
    let a = area();
    let pinto = PointerOpts {
        right_click_adds: true,
        add_mods: KeyModifiers::CONTROL | KeyModifiers::ALT,
        ..PointerOpts::default()
    };
    let mut p: Pointer<u32> = Pointer::new(pinto);
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 6, 3), nothing);
    let mut up = ev(MouseEventKind::Up(LEFT), 6, 3);
    up.modifiers = KeyModifiers::CONTROL;
    assert_eq!(
        p.handle(&a, &up, nothing),
        MapAction::Click {
            px: 12.0,
            py: 24.0,
            add: true
        }
    );
    assert_eq!(
        p.handle(
            &a,
            &ev(MouseEventKind::Down(MouseButton::Right), 6, 3),
            nothing
        ),
        MapAction::Click {
            px: 12.0,
            py: 24.0,
            add: true
        }
    );
    assert_eq!(
        p.handle(
            &a,
            &ev(MouseEventKind::Down(MouseButton::Right), 0, 0),
            nothing
        ),
        MapAction::None
    );
    // By default neither adds: the right button does nothing, Ctrl is a
    // plain click.
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    assert_eq!(
        p.handle(
            &a,
            &ev(MouseEventKind::Down(MouseButton::Right), 6, 3),
            nothing
        ),
        MapAction::None
    );
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 6, 3), nothing);
    assert_eq!(
        p.handle(&a, &up, nothing),
        MapAction::Click {
            px: 12.0,
            py: 24.0,
            add: false
        }
    );
}

#[test]
fn hover_follows_the_pointer_and_says_once_when_it_leaves() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Moved, 0, 0), nothing),
        MapAction::None
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Moved, 6, 3), nothing),
        MapAction::Hover { px: 12.0, py: 24.0 }
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Moved, 30, 3), nothing),
        MapAction::Away
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Moved, 31, 3), nothing),
        MapAction::None
    );
}

#[test]
fn a_press_on_a_label_drags_the_label_and_not_the_map() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 11, 4), label_seven);
    assert_eq!(p.label(), Some(7));
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 13, 6), label_seven),
        MapAction::MoveLabel {
            key: 7,
            dx: 16.0,
            dy: 32.0
        }
    );
    // The label goes on with the pointer wherever it is now.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 12, 6), label_seven),
        MapAction::MoveLabel {
            key: 7,
            dx: -8.0,
            dy: 0.0
        }
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 12, 6), label_seven),
        MapAction::None
    );
    assert_eq!(p.label(), None);
    // A press on a label that never moves is a click there.
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 10, 4), label_seven);
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 10, 4), label_seven),
        MapAction::Click {
            px: 44.0,
            py: 40.0,
            add: false
        }
    );
    // Beside the label the map pans.
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 14, 4), label_seven);
    assert_eq!(p.label(), None);
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 11, 4), label_seven),
        MapAction::Pan { dx: -24.0, dy: 0.0 }
    );
}

#[test]
fn any_release_ends_a_drag_even_off_the_map() {
    let a = area();
    let mut p: Pointer<u32> = Pointer::new(PointerOpts::default());
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 23, 5), nothing);
    // Dragged past the map's edge, the pan goes on.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 27, 5), nothing),
        MapAction::Pan { dx: 32.0, dy: 0.0 }
    );
    // Released off the map: the drag ends, no click.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 27, 5), nothing),
        MapAction::None
    );
    assert!(!p.dragging());
    // So back over the map, a plain motion pans nothing.
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 20, 5), nothing),
        MapAction::None
    );

    // Kept to the map, motion off it is ignored but any release still ends
    // the drag, whatever its button.
    let mut p: Pointer<u32> = Pointer::new(PointerOpts {
        drag_outside: false,
        ..PointerOpts::default()
    });
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 23, 5), nothing);
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 27, 5), nothing),
        MapAction::None
    );
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Drag(LEFT), 24, 5), nothing),
        MapAction::Pan { dx: 8.0, dy: 0.0 }
    );
    assert_eq!(
        p.handle(
            &a,
            &ev(MouseEventKind::Up(MouseButton::Middle), 40, 5),
            nothing
        ),
        MapAction::None
    );
    assert!(!p.dragging());
    // A cancelled drag is gone too.
    p.handle(&a, &ev(MouseEventKind::Down(LEFT), 23, 5), nothing);
    p.cancel();
    assert!(!p.dragging());
    assert_eq!(
        p.handle(&a, &ev(MouseEventKind::Up(LEFT), 23, 5), nothing),
        MapAction::None
    );
}

fn placed(key: u32, anchor: (f32, f32)) -> Placed<u32> {
    Placed {
        key,
        rect: [
            anchor.0 - 10.0,
            anchor.1 - 5.0,
            anchor.0 + 10.0,
            anchor.1 + 5.0,
        ],
        anchor,
        moved: false,
    }
}

/// Three labels, placed in another order than they read.
fn three() -> Vec<Placed<u32>> {
    vec![
        placed(2, (50.0, 80.0)),
        placed(1, (90.0, 10.0)),
        placed(3, (10.0, 80.0)),
    ]
}

#[test]
fn only_shift_l_starts_label_editing() {
    assert!(LabelEdit::<u32>::starts(&key(KeyCode::Char('L'))));
    assert!(LabelEdit::<u32>::starts(&shift(KeyCode::Char('L'))));
    assert!(!LabelEdit::<u32>::starts(&key(KeyCode::Char('l'))));
    assert!(!LabelEdit::<u32>::starts(&KeyEvent::new(
        KeyCode::Char('L'),
        KeyModifiers::CONTROL | KeyModifiers::SHIFT
    )));
    let mut release = key(KeyCode::Char('L'));
    release.kind = KeyEventKind::Release;
    assert!(!LabelEdit::<u32>::starts(&release));
}

#[test]
fn tab_cycles_the_labels_in_reading_order() {
    let p = three();
    let mut o = Offsets::default();
    let mut e = LabelEdit::enter(&p);
    // The first as read: top, then left to right.
    assert_eq!(e.at, Some(1));
    assert_eq!(e.key(&key(KeyCode::Tab), &p, &mut o), EditKey::Taken);
    assert_eq!(e.at, Some(3));
    e.key(&key(KeyCode::Tab), &p, &mut o);
    assert_eq!(e.at, Some(2));
    e.key(&key(KeyCode::Tab), &p, &mut o);
    assert_eq!(e.at, Some(1));
    e.key(&key(KeyCode::BackTab), &p, &mut o);
    assert_eq!(e.at, Some(2));
    // A moved label keeps its place in the cycle: it goes by its anchor.
    let mut moved = p.clone();
    moved[0].moved = true;
    moved.swap(0, 2);
    e.key(&key(KeyCode::Tab), &moved, &mut o);
    assert_eq!(e.at, Some(1));
    // With no labels there is nothing to pick.
    assert_eq!(LabelEdit::<u32>::enter(&[]).at, None);
    assert!(o.is_empty());
}

#[test]
fn arrows_nudge_half_an_em_or_two_with_shift_and_backspace_resets() {
    let p = three();
    let mut o = Offsets::default();
    let mut e = LabelEdit::enter(&p);
    assert_eq!(e.key(&key(KeyCode::Right), &p, &mut o), EditKey::Changed);
    assert_eq!(e.key(&key(KeyCode::Up), &p, &mut o), EditKey::Changed);
    assert_eq!(o.get(&1), Some((0.5, -0.5)));
    e.key(&shift(KeyCode::Down), &p, &mut o);
    e.key(&shift(KeyCode::Left), &p, &mut o);
    assert_eq!(o.get(&1), Some((-1.5, 1.5)));
    e.key(&key(KeyCode::Tab), &p, &mut o);
    e.key(&key(KeyCode::Down), &p, &mut o);
    assert_eq!(o.get(&3), Some((0.0, 0.5)));
    // Backspace puts the selected label back, and only that one.
    assert_eq!(
        e.key(&key(KeyCode::Backspace), &p, &mut o),
        EditKey::Changed
    );
    assert_eq!(o.get(&3), None);
    assert_eq!(o.get(&1), Some((-1.5, 1.5)));
    // Nothing to put back: taken, unchanged.
    assert_eq!(e.key(&key(KeyCode::Backspace), &p, &mut o), EditKey::Taken);
}

#[test]
fn esc_leaves_and_other_keys_pass_through() {
    let p = three();
    let mut o = Offsets::default();
    let mut e = LabelEdit::enter(&p);
    assert_eq!(e.key(&key(KeyCode::Char('q')), &p, &mut o), EditKey::Pass);
    assert_eq!(
        e.key(
            &KeyEvent::new(KeyCode::Char('c'), KeyModifiers::CONTROL),
            &p,
            &mut o
        ),
        EditKey::Pass
    );
    // Ctrl or Alt with an arrow is not a nudge.
    assert_eq!(
        e.key(&KeyEvent::new(KeyCode::Up, KeyModifiers::ALT), &p, &mut o),
        EditKey::Pass
    );
    let mut release = key(KeyCode::Right);
    release.kind = KeyEventKind::Release;
    release.state = KeyEventState::NONE;
    assert_eq!(e.key(&release, &p, &mut o), EditKey::Pass);
    assert!(o.is_empty());
    assert_eq!(e.key(&key(KeyCode::Esc), &p, &mut o), EditKey::Leave);
    assert_eq!(e.at, None);
}

#[test]
fn with_nothing_selected_an_arrow_selects_before_it_moves() {
    let p = three();
    let mut o = Offsets::default();
    let mut e = LabelEdit::<u32>::default();
    assert_eq!(e.key(&key(KeyCode::Left), &p, &mut o), EditKey::Taken);
    assert_eq!(e.at, Some(1));
    assert!(o.is_empty());
    // With no labels at all the mode's keys are still taken, doing nothing.
    let mut e = LabelEdit::<u32>::default();
    assert_eq!(e.key(&key(KeyCode::Left), &[], &mut o), EditKey::Taken);
    assert_eq!(e.key(&key(KeyCode::Tab), &[], &mut o), EditKey::Taken);
    assert_eq!(e.at, None);
}

#[test]
fn nav_keys_zoom_pan_and_fit() {
    let n = |k: KeyEvent| nav_key(&k, 1.4, 30.0);
    assert_eq!(n(key(KeyCode::Char('+'))), Some(Nav::Zoom(1.4)));
    assert_eq!(n(key(KeyCode::Char('='))), Some(Nav::Zoom(1.4)));
    assert_eq!(n(shift(KeyCode::Char('+'))), Some(Nav::Zoom(1.4)));
    assert_eq!(n(key(KeyCode::Char('-'))), Some(Nav::Zoom(1.0 / 1.4)));
    assert_eq!(n(key(KeyCode::Char('_'))), Some(Nav::Zoom(1.0 / 1.4)));
    // As senna's arrows: the picture moves against the arrow, so the view
    // goes its way.
    assert_eq!(n(key(KeyCode::Left)), Some(Nav::Pan(30.0, 0.0)));
    assert_eq!(n(key(KeyCode::Right)), Some(Nav::Pan(-30.0, 0.0)));
    assert_eq!(n(key(KeyCode::Up)), Some(Nav::Pan(0.0, 30.0)));
    assert_eq!(n(key(KeyCode::Down)), Some(Nav::Pan(0.0, -30.0)));
    assert_eq!(n(key(KeyCode::Char('0'))), Some(Nav::Fit));
    // Everything else, and these with Ctrl or Alt or Shift on an arrow, is
    // the viewer's.
    assert_eq!(n(key(KeyCode::Char('z'))), None);
    assert_eq!(n(key(KeyCode::Tab)), None);
    assert_eq!(n(shift(KeyCode::Left)), None);
    assert_eq!(
        n(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::CONTROL)),
        None
    );
    assert_eq!(n(KeyEvent::new(KeyCode::Left, KeyModifiers::ALT)), None);
    let mut release = key(KeyCode::Char('0'));
    release.kind = KeyEventKind::Release;
    assert_eq!(n(release), None);
}

#[test]
fn nav_and_pointer_actions_move_a_viewport_as_senna_does() {
    use crate::map::viewport::{Limits, Viewport, YAxis};
    let lim = Limits::default();
    let vp = Viewport::fit([0.0, 0.0, 10.0, 10.0], 100, 100, 1.0, YAxis::Up);
    let mut a = vp;
    Nav::Pan(30.0, 0.0).apply(&mut a, &lim, || vp);
    let mut b = vp;
    b.pan_px(30.0, 0.0, &lim);
    assert_eq!(a, b);
    let mut a = vp;
    Nav::Zoom(2.0).apply(&mut a, &lim, || vp);
    let mut b = vp;
    b.zoom_at(2.0, 50.0, 50.0, &lim);
    assert_eq!(a, b);
    let mut a = b;
    Nav::Fit.apply(&mut a, &lim, || vp);
    assert_eq!(a, vp);

    let mut a = vp;
    assert!(MapAction::<u32>::Zoom {
        factor: 1.25,
        at: (10.0, 20.0)
    }
    .apply(&mut a, &lim));
    let mut b = vp;
    b.zoom_at(1.25, 10.0, 20.0, &lim);
    assert_eq!(a, b);
    let mut a = vp;
    assert!(MapAction::<u32>::Pan { dx: 3.0, dy: -4.0 }.apply(&mut a, &lim));
    let mut b = vp;
    b.pan_px(3.0, -4.0, &lim);
    assert_eq!(a, b);
    // The rest leave the camera alone.
    let mut a = vp;
    assert!(!MapAction::<u32>::Click {
        px: 1.0,
        py: 1.0,
        add: false
    }
    .apply(&mut a, &lim));
    assert!(!MapAction::MoveLabel {
        key: 1u32,
        dx: 1.0,
        dy: 1.0
    }
    .apply(&mut a, &lim));
    assert_eq!(a, vp);
}
