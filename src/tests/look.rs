//! A look's styles: the terminal's by default, built whole in a constant.

use super::*;

/// A look built at compile time, as an app's constant.
const PLAIN: Look = Look::terminal().with_accent(Style::new().fg(Color::Red));

#[test]
fn the_terminal_look_is_the_default_and_its_accent_stands_out() {
    let t = Look::terminal();
    assert_eq!(Look::default(), t);
    assert_eq!(t.border, t.hint);
    assert_ne!(t.accent, t.bold);
    assert_eq!(t.accent.fg, Some(Color::Yellow));
    assert_eq!(PLAIN.accent.fg, Some(Color::Red));
    assert_eq!(PLAIN.page, t.page);
}

#[test]
fn new_takes_each_style_in_order() {
    let s = |c| Style::new().fg(c);
    let l = Look::new(
        s(Color::Red),
        s(Color::Green),
        s(Color::Blue),
        s(Color::Cyan),
        s(Color::White),
        s(Color::Magenta),
    );
    let got = [l.page, l.bold, l.hint, l.selected, l.border, l.accent].map(|x| x.fg);
    let want = [
        Color::Red,
        Color::Green,
        Color::Blue,
        Color::Cyan,
        Color::White,
        Color::Magenta,
    ]
    .map(Some);
    assert_eq!(got, want);
}

#[cfg(feature = "map")]
#[test]
fn a_chrome_gives_its_styles_and_the_terminal_accent() {
    let c = crate::chrome::Chrome::TERMINAL;
    let l = Look::from(c);
    assert_eq!(l.selected, c.selected());
    assert_eq!(l.accent, Look::terminal().accent);
}
