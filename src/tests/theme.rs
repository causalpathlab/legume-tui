//! The page a map is drawn on: senna's paper and pinto's light and dark
//! themes, the colours they keep and how they read.

use super::*;
use crate::map::colour::{to_linear, Srgb};

/// WCAG contrast ratio between two sRGB colours.
fn contrast(a: Srgb, b: Srgb) -> f32 {
    let (x, y) = (luminance(a), luminance(b));
    (x.max(y) + 0.05) / (x.min(y) + 0.05)
}

#[test]
fn menu_text_and_hints_read_clearly_on_the_page() {
    for t in [Theme::PAPER, Theme::LIGHT, Theme::DARK] {
        let (text, hint) = (
            contrast(t.text, t.background),
            contrast(t.hint, t.background),
        );
        assert!(text >= 7.0, "{t:?}: {text}");
        assert!(hint >= 4.5, "{t:?}: {hint}");
        // Hints stay a step below the text they explain.
        assert!(hint < text, "{t:?}");
        // Muted points are a backdrop, not text.
        assert!(contrast(t.muted, t.background) < 2.0, "{t:?}");
    }
}

#[test]
fn paper_keeps_senna_s_colours() {
    let p = Theme::PAPER;
    assert_eq!(p.background, [246, 245, 242]);
    assert_eq!(p.muted, [200, 199, 196]);
    assert_eq!(p.text, [30, 30, 32]);
    assert_eq!(p.hint, [104; 3]);
    assert_eq!(p.ink, [92; 3]);
    assert_eq!(p.palette, Categorical::SENNA);
    assert_eq!(p.ink_lightness, 0.42);
    assert!(!p.is_dark());
}

#[test]
fn light_and_dark_keep_pinto_s_colours() {
    let (l, d) = (Theme::LIGHT, Theme::DARK);
    assert_eq!(l.background, [238, 238, 236]);
    assert_eq!(d.background, [17, 17, 20]);
    assert_eq!(l.dimmed, [212, 212, 216]);
    assert_eq!(d.dimmed, [40, 40, 46]);
    assert_eq!(l.no_group, [175, 175, 181]);
    assert_eq!(d.no_group, [70, 70, 76]);
    assert_eq!(l.bar, ([0; 3], [255; 3]));
    assert_eq!(d.bar, ([255; 3], [0; 3]));
    assert_eq!(l.palette, Categorical::PINTO_LIGHT);
    assert_eq!(d.palette, Categorical::PINTO_DARK);
    assert!(d.is_dark() && !l.is_dark());
}

#[test]
fn both_themes_give_distinct_palettes_and_read_their_background() {
    for theme in [Theme::DARK, Theme::LIGHT] {
        let p = theme.palette.srgb_all(40);
        for i in 0..p.len() {
            for j in i + 1..p.len() {
                assert_ne!(p[i], p[j], "{theme:?}: {i} and {j}");
            }
        }
        assert_eq!(Theme::for_background(theme.background), theme);
    }
    assert_eq!(Theme::for_background([255, 255, 255]), Theme::LIGHT);
    assert_eq!(Theme::for_background([30, 30, 30]), Theme::DARK);
    assert_eq!(Theme::for_background(Theme::PAPER.background), Theme::LIGHT);
}

#[test]
fn ramps_start_on_the_background_side() {
    // Low values fade toward the background: dark ramps start dark on a
    // dark theme and light on a light one.
    let lum = |c: [u8; 3]| c.iter().map(|&v| v as u32).sum::<u32>();
    for (theme, starts_light) in [(Theme::DARK, false), (Theme::LIGHT, true)] {
        for ramp in [theme.magma(), theme.levels()] {
            let (low, high) = (lum(ramp.at(0.)), lum(ramp.at(1.)));
            assert_eq!(low > high, starts_light, "{theme:?}");
        }
    }
    // Feature levels start on the dimmed points.
    assert_eq!(Theme::LIGHT.levels().at(0.), Theme::LIGHT.dimmed);
    assert_eq!(Theme::DARK.levels().at(0.), Theme::DARK.dimmed);
}

#[test]
fn the_theme_key_flips_between_dark_and_light() {
    assert_eq!(Theme::DARK.other(), Theme::LIGHT);
    assert_eq!(Theme::LIGHT.other(), Theme::DARK);
    assert_eq!(Theme::PAPER.other(), Theme::DARK);
    // A light map, for paper, is drawn on a light background.
    assert_eq!(
        Theme::for_background(Theme::DARK.other().background),
        Theme::LIGHT
    );
}

#[test]
fn label_ink_follows_the_theme() {
    let c = to_linear([200, 80, 60]);
    let (light, dark) = (Theme::PAPER.ink_for(c), Theme::DARK.ink_for(c));
    let l = |c| crate::map::colour::oklab(c)[0];
    assert!(l(light) < l(c) && l(dark) > l(light));
}
