//! Figures as terminal images: which terminals are asked, the raster's
//! size, and when a plot area's image is made again.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// A `w` x `h` figure with a box in it.
fn svg(w: f64, h: f64) -> String {
    format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w}\" height=\"{h}\" \
         viewBox=\"0 0 {w} {h}\"><rect x=\"2\" y=\"2\" width=\"8\" height=\"8\" \
         fill=\"#d97757\"/></svg>"
    )
}

fn env(pairs: &'static [(&'static str, &'static str)]) -> impl Fn(&str) -> Option<String> {
    move |k: &str| {
        pairs
            .iter()
            .find(|(key, _)| *key == k)
            .map(|(_, v)| v.to_string())
    }
}

#[test]
fn only_known_image_terminals_are_queried() {
    assert!(likely_images(env(&[("TERM_PROGRAM", "iTerm.app")])));
    assert!(likely_images(env(&[("TERM", "xterm-kitty")])));
    assert!(likely_images(env(&[("KITTY_WINDOW_ID", "3")])));
    assert!(likely_images(env(&[
        ("TERM", "tmux-256color"),
        ("LC_TERMINAL", "iTerm2")
    ])));
    assert!(!likely_images(env(&[
        ("TERM_PROGRAM", "Apple_Terminal"),
        ("TERM", "xterm-256color")
    ])));
    assert!(!likely_images(env(&[("TERM", "screen-256color")])));
    assert!(!likely_images(env(&[("KITTY_WINDOW_ID", "")])));
}

#[test]
fn the_override_says_whether_the_terminal_is_asked() {
    assert!(
        !asks(Some("0"), env(&[("TERM", "xterm-kitty")])),
        "0: never"
    );
    assert!(
        asks(Some("1"), env(&[("TERM", "xterm-256color")])),
        "1: always"
    );
    assert!(asks(None, env(&[("TERM_PROGRAM", "ghostty")])));
    assert!(!asks(None, env(&[])), "else by the terminal");
}

#[test]
fn a_raster_has_the_scaled_size() {
    let img = raster(&svg(100.0, 40.0), 2.0).unwrap();
    assert_eq!((img.width(), img.height()), (200, 80));
    assert!(png(&svg(10.0, 10.0), 1.0).unwrap().starts_with(b"\x89PNG"));
    assert!(raster("not a figure", 1.0).is_err());
}

#[test]
fn a_plot_image_is_made_again_only_when_stale_or_resized() {
    let picker = Picker::halfblocks();
    let mut plot = PlotImage::default();
    let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
    let mut calls = 0;
    let mut frame = |plot: &mut PlotImage, area: Rect, calls: &mut i32| {
        let mut drawn = false;
        term.draw(|f| {
            drawn = plot.render_at(f, area, &picker, 0, |w, h| {
                *calls += 1;
                svg(w, h)
            });
        })
        .unwrap();
        drawn
    };
    let area = Rect::new(0, 0, 20, 5);
    assert!(frame(&mut plot, area, &mut calls), "the first made at once");
    assert!(frame(&mut plot, area, &mut calls));
    assert_eq!(calls, 1, "unchanged view: kept");
    plot.invalidate();
    assert!(plot.is_stale());
    assert!(frame(&mut plot, area, &mut calls));
    assert_eq!(calls, 2, "stale: made again");
    // A resize under way: text until a frame finds the same size again.
    for size in [30, 34, 36] {
        let resized = Rect::new(0, 0, size, 6);
        assert!(!frame(&mut plot, resized, &mut calls), "{size}: text");
        assert!(plot.is_stale(), "a redraw owed");
    }
    assert_eq!(calls, 2, "not made while the size changes");
    assert!(frame(&mut plot, Rect::new(0, 0, 36, 6), &mut calls));
    assert_eq!(calls, 3, "resized: made again once settled");
    assert!(!plot.is_stale());
    assert!(!frame(&mut plot, Rect::new(0, 0, 30, 6), &mut calls));
    assert!(frame(&mut plot, Rect::new(0, 0, 36, 6), &mut calls));
    assert_eq!(calls, 3, "back to the made size: kept");
    assert!(!plot.is_stale());
}

#[test]
fn a_plot_image_is_made_again_after_an_attach_or_for_another_picker() {
    let blocks = Picker::halfblocks();
    let mut kitty = Picker::halfblocks();
    kitty.set_protocol_type(ProtocolType::Kitty);
    let mut plot = PlotImage::default();
    let mut term = Terminal::new(TestBackend::new(40, 10)).unwrap();
    let area = Rect::new(0, 0, 20, 5);
    let mut calls = 0;
    let mut frame = |picker: &Picker, attach: u64, calls: &mut i32| {
        term.draw(|f| {
            plot.render_at(f, area, picker, attach, |w, h| {
                *calls += 1;
                svg(w, h)
            });
        })
        .unwrap();
    };
    frame(&blocks, 0, &mut calls);
    frame(&blocks, 0, &mut calls);
    assert_eq!(calls, 1, "same terminal: kept");
    frame(&blocks, 1, &mut calls);
    assert_eq!(calls, 2, "a terminal attached: made again");
    frame(&kitty, 1, &mut calls);
    assert_eq!(calls, 3, "another protocol: made again");
    frame(&kitty, 1, &mut calls);
    assert_eq!(calls, 3);
}

#[test]
fn an_image_that_cannot_be_made_is_not_tried_again_each_frame() {
    let mut terminal = Terminal::new(TestBackend::new(40, 12)).unwrap();
    let picker = Picker::halfblocks();
    let mut plot = PlotImage::default();
    let mut drawn = 0;
    let mut frame = |plot: &mut PlotImage, drawn: &mut usize| {
        let mut shown = true;
        terminal
            .draw(|f| {
                shown = plot.render_at(f, f.area(), &picker, 0, |_, _| {
                    *drawn += 1;
                    "not a figure".to_string()
                })
            })
            .unwrap();
        shown
    };
    assert!(!frame(&mut plot, &mut drawn));
    assert!(!frame(&mut plot, &mut drawn));
    assert_eq!(drawn, 1);
    plot.invalidate();
    assert!(!frame(&mut plot, &mut drawn));
    assert_eq!(drawn, 2, "a change to the view tries again");
}
