//! The strip of saved figures: how many fit, scrolling to the one chosen,
//! the keys that act on it, and what it draws (senna
//! `src/view/tui/grid.rs`).

use super::*;
use crate::gallery::{now, Policy};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyModifiers, MouseButton, MouseEventKind};
use ratatui::Terminal;

/// Thumbnails as their size in cells, drawn as `#`.
struct Sized;

impl ThumbPainter for Sized {
    type Thumb = (u32, u32);
    fn make(&mut self, img: RgbaImage, _cols: u16, _rows: u16) -> Option<(u32, u32)> {
        Some(img.dimensions())
    }
    fn draw(&self, _: &(u32, u32), f: &mut ratatui::Frame, area: Rect) {
        let fill = "#".repeat(usize::from(area.width));
        let lines: Vec<Line> = (0..area.height).map(|_| Line::from(fill.clone())).collect();
        f.render_widget(Paragraph::new(lines), area);
    }
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

/// Ten saves in a gallery under `work`.
fn ten(work: &std::path::Path) -> Gallery {
    let mut g = Gallery::open(&work.join(".senna-view"), Policy::plain());
    let picture = RgbaImage::from_pixel(40, 30, image::Rgba([1, 2, 3, 255]));
    for k in 0..10 {
        let pdf = work.join(format!("{k}.pdf"));
        std::fs::write(&pdf, b"%PDF").unwrap();
        g.add(&[&pdf], "a view", Some(&picture), ()).unwrap();
    }
    g
}

#[test]
fn only_thumbnails_that_fit_the_strip_are_made() {
    // Each takes its rows, two lines of text and a gap; one row on top.
    assert_eq!(thumbs_that_fit(0, 6), 0);
    assert_eq!(thumbs_that_fit(8, 6), 0);
    assert_eq!(thumbs_that_fit(9, 6), 1);
    assert_eq!(thumbs_that_fit(17, 6), 1);
    assert_eq!(thumbs_that_fit(18, 6), 2);
    assert_eq!(thumbs_that_fit(40, 6), 4);
}

#[test]
fn the_strip_shows_on_the_left_of_a_wide_enough_screen() {
    let work = tempfile::tempdir().unwrap();
    let g = ten(work.path());
    let mut s = Strip::new(Sized, (8.0, 16.0));
    let body = Rect::new(0, 0, 160, 50);
    assert_eq!(s.area(body, &g), Some(Rect::new(0, 0, WIDTH, 50)));
    assert_eq!(s.content(body, &g), Rect::new(WIDTH, 0, 160 - WIDTH, 50));
    // Too narrow, hidden, or nothing saved: none.
    assert_eq!(s.area(Rect::new(0, 0, 3 * WIDTH - 1, 50), &g), None);
    let empty: Gallery = Gallery::open(&work.path().join("none"), Policy::plain());
    assert_eq!(s.area(body, &empty), None);
    assert!(!s.focus(&empty));
    s.toggle();
    assert_eq!(s.area(body, &g), None);
    assert_eq!(s.content(body, &g), body);
    // Choosing a save shows it again.
    assert!(s.focus(&g));
    assert!(s.area(body, &g).is_some());
}

#[test]
fn the_strip_scrolls_to_the_chosen_save_and_drops_it() {
    let work = tempfile::tempdir().unwrap();
    let mut g = ten(work.path());
    let mut s = Strip::new(Sized, (8.0, 16.0));
    let strip = s.area(Rect::new(0, 0, 160, 50), &g).unwrap();
    let shown = s.shown(strip);
    assert!((1..10).contains(&shown), "{shown} of 10 fit");
    let keys = Keymap::default();

    assert!(s.focus(&g));
    assert_eq!(s.chosen(), Some(0));
    // Down past the last one shown scrolls it into view.
    for _ in 0..shown {
        s.key(strip, key(KeyCode::Down), &g, &keys);
    }
    assert_eq!(s.chosen(), Some(shown));
    assert_eq!(s.top(), 1);
    s.key(strip, key(KeyCode::End), &g, &keys);
    assert_eq!(s.chosen(), Some(9));
    assert_eq!(s.top(), 10 - shown);
    // The wheel stops at either end.
    assert!(!s.scroll(99, shown, &g));
    assert!(s.scroll(0, shown, &g));

    // x asks to take the last off the list; the viewer does, and the choice
    // stays in range.
    s.key(strip, key(KeyCode::End), &g, &keys);
    let last = g.entries()[9].path.clone();
    let act = s.key(strip, key(KeyCode::Char('x')), &g, &keys);
    assert_eq!(
        act,
        Action::Remove {
            at: 9,
            delete: false
        }
    );
    g.remove(&last, false).unwrap();
    s.after_change(strip, &g);
    assert_eq!(g.entries().len(), 9);
    assert!(last.exists());
    assert_eq!(s.chosen(), Some(8));
    // X deletes, m moves, the quit key quits.
    let act = s.key(strip, key(KeyCode::Char('X')), &g, &keys);
    assert_eq!(
        act,
        Action::Remove {
            at: 8,
            delete: true
        }
    );
    assert_eq!(
        s.key(strip, key(KeyCode::Char('m')), &g, &keys),
        Action::Move(8)
    );
    assert_eq!(
        s.key(strip, key(KeyCode::Char('q')), &g, &keys),
        Action::Quit
    );
    // Esc hands the keys back.
    assert_eq!(s.key(strip, key(KeyCode::Esc), &g, &keys), Action::Leave);
    assert_eq!(s.chosen(), None);
    assert_eq!(s.key(strip, key(KeyCode::Down), &g, &keys), Action::None);
}

#[test]
fn the_mouse_scrolls_the_strip_and_chooses_a_save() {
    let work = tempfile::tempdir().unwrap();
    let g = ten(work.path());
    let mut s = Strip::new(Sized, (8.0, 16.0));
    let strip = s.area(Rect::new(0, 0, 160, 50), &g).unwrap();
    let mouse = |kind, column, row| MouseEvent {
        kind,
        column,
        row,
        modifiers: KeyModifiers::NONE,
    };
    // Off the strip: not its event.
    assert_eq!(
        s.mouse(strip, &mouse(MouseEventKind::ScrollDown, 100, 5), &g),
        None
    );
    assert_eq!(
        s.mouse(strip, &mouse(MouseEventKind::ScrollDown, 3, 5), &g),
        Some(true)
    );
    assert_eq!(s.top(), 1);
    assert_eq!(
        s.mouse(strip, &mouse(MouseEventKind::ScrollUp, 3, 5), &g),
        Some(true)
    );
    assert_eq!(
        s.mouse(strip, &mouse(MouseEventKind::ScrollUp, 3, 5), &g),
        Some(false)
    );
    // A click on the second save chooses it.
    let each = s.rows_per_thumb() + 3;
    let row = 2 + each;
    let down = mouse(MouseEventKind::Down(MouseButton::Left), 3, row);
    assert_eq!(s.mouse(strip, &down, &g), Some(true));
    assert_eq!(s.chosen(), Some(1));
}

#[test]
fn the_strip_draws_each_save_and_how_many_more() {
    let work = tempfile::tempdir().unwrap();
    let g = ten(work.path());
    let mut s = Strip::new(Sized, (8.0, 16.0));
    let full = Rect::new(0, 0, 100, 40);
    let strip = s.area(full, &g).unwrap();
    s.prepare(strip, &g);
    let shown = s.shown(strip);
    assert_eq!(s.made(), shown, "only the thumbnails shown are made");
    let mut t = Terminal::new(TestBackend::new(100, 40)).unwrap();
    t.draw(|f| s.draw(f, strip, &g, &Look::default(), &Keymap::default(), now()))
        .unwrap();
    let buf = t.backend().buffer();
    let screen: Vec<String> = (0..buf.area.height)
        .map(|y| (0..WIDTH).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let all = screen.join("\n");
    for want in [
        "saved (10) · f hides",
        "9.pdf",
        "just now · a view",
        "####",
        "more",
        "F choose",
    ] {
        assert!(all.contains(want), "{want} not in\n{all}");
    }
}
