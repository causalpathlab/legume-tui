//! Colour: the sRGB tables, OKLCH, the palettes and the ramps, with the
//! numbers senna and pinto drew before they shared them.

use super::*;

fn distinct(p: &[Srgb]) {
    for i in 0..p.len() {
        for j in i + 1..p.len() {
            assert_ne!(p[i], p[j], "{i} and {j}");
        }
    }
}

#[test]
fn srgb_round_trips() {
    for v in 0..=255u8 {
        assert_eq!(encode_exact(decode(v)), v);
    }
}

#[test]
fn every_byte_round_trips_through_the_tables() {
    // What a style file keeps in sRGB comes back as it was.
    for v in 0..=255u8 {
        assert_eq!(linear(v), decode(v));
        assert_eq!(to_srgb(to_linear([v, v, v])), [v, v, v], "{v}");
        assert_eq!(encode(linear(v)), v, "{v}");
    }
}

#[test]
fn table_encoder_matches_the_formula_within_one_step() {
    for i in 0..=1000 {
        let c = i as f32 / 1000.;
        assert!(
            (encode(c) as i32 - encode_exact(c) as i32).abs() <= 1,
            "{c}"
        );
    }
    assert_eq!(encode(-1.0), 0);
    assert_eq!(encode(2.0), 255);
}

#[test]
fn senna_s_encoder_keeps_its_bytes() {
    // senna's table truncates into 4096 steps: a few dark bytes come back
    // one lower, and the middle of the range one below the formula.
    assert_eq!(encode_senna(linear(3)), 2);
    assert_eq!(encode_senna(linear(17)), 16);
    assert_eq!(encode_senna(linear(18)), 18);
    assert_eq!(encode_senna(0.5), 187);
    assert_eq!(encode(0.5), 188);
    assert_eq!(encode_senna(0.000_13), 0);
    assert_eq!(encode_senna(-1.0), 0);
    assert_eq!(encode_senna(2.0), 255);
    assert_eq!(to_srgb_senna([0.5, 1.0, 0.0]), [187, 255, 0]);
}

#[test]
fn palette_colours_are_distinct() {
    distinct(&Categorical::PINTO_DARK.srgb_all(60));
}

#[test]
fn both_pinto_palettes_are_distinct() {
    for p in [Categorical::PINTO_DARK, Categorical::PINTO_LIGHT] {
        distinct(&p.srgb_all(40));
    }
}

#[test]
fn pinto_palettes_keep_their_bytes() {
    let light = [
        [195, 78, 84],
        [0, 113, 57],
        [159, 133, 229],
        [175, 104, 0],
        [0, 109, 134],
        [213, 114, 168],
    ];
    let dark = [
        [235, 129, 130],
        [35, 151, 97],
        [200, 183, 255],
        [214, 150, 58],
        [0, 148, 171],
        [245, 170, 208],
    ];
    assert_eq!(Categorical::PINTO_LIGHT.srgb_all(6), light);
    assert_eq!(Categorical::PINTO_DARK.srgb_all(6), dark);
}

#[test]
fn senna_categories_keep_their_colours() {
    let p = Categorical::SENNA;
    assert_eq!(p.colour(0, 5), [0.687_268, 0.121_939_12, 0.107_462_82]);
    assert_eq!(p.colour(1, 5), [0.0, 0.387_798_07, 0.162_886_87]);
    assert_eq!(p.colour(2, 5), [0.327_382_62, 0.174_712_58, 0.691_845_66]);
    // Past a dozen groups lightness alternates.
    assert_eq!(p.colour(0, 20), p.colour(0, 5));
    assert_eq!(p.colour(1, 20), [0.038_938_567, 0.617_429_8, 0.297_616_45]);
    assert_eq!(p.colour(2, 20), [0.185_758_59, 0.079_815_52, 0.439_652_8]);
}

#[test]
fn a_dozen_senna_groups_share_one_lightness() {
    let l = |c: Lin| oklab(c)[0];
    let first = l(Categorical::SENNA.colour(0, 12));
    for i in 1..12 {
        let c = Categorical::SENNA.colour(i, 12);
        // Clamping into sRGB moves a few a little.
        assert!((l(c) - first).abs() < 0.06, "{i}");
    }
}

#[test]
fn oklab_inverts_oklch_inside_the_gamut() {
    let c = oklch(0.64, 0.08, 200.0);
    let [l, a, b] = oklab(c);
    assert!((l - 0.64).abs() < 1e-3);
    assert!((a.hypot(b) - 0.08).abs() < 1e-3);
    assert!((b.atan2(a).to_degrees().rem_euclid(360.0) - 200.0).abs() < 0.5);
}

#[test]
fn ink_keeps_the_hue_and_reads_darker() {
    let c = oklch(0.7, 0.1, 140.0);
    let ink = ink_for(c, 0.42);
    let ([l0, a0, b0], [l1, a1, b1]) = (oklab(c), oklab(ink));
    assert!(l1 < l0);
    assert!((l1 - 0.42).abs() < 0.02);
    assert!(a1.hypot(b1) <= 0.12 + 1e-3);
    let hue = |a: f32, b: f32| b.atan2(a).to_degrees();
    assert!((hue(a0, b0) - hue(a1, b1)).abs() < 3.0);
}

#[test]
fn the_activity_ramp_runs_light_to_deep() {
    let r = activity_ramp(5);
    assert_eq!(r.len(), 5);
    let l: Vec<f32> = r.iter().map(|&c| oklab(c)[0]).collect();
    assert!(l.windows(2).all(|w| w[0] > w[1]), "{l:?}");
    assert_eq!(activity_ramp(1).len(), 1);
    assert!(oklab(highlight_ink())[0] < 0.4);
}

#[test]
fn ramp_ends_are_the_first_and_last_stops() {
    let v = viridis();
    assert_eq!(v.at(0.), [0x44, 0x01, 0x54]);
    assert_eq!(v.at(1.), [0xfd, 0xe7, 0x25]);
    assert_eq!(v.at_u8(255), v.at(1.));
    assert_eq!(v.at(f32::NAN), v.at(0.));
    let m = magma();
    assert_eq!(m.at(0.), [0x00, 0x00, 0x04]);
    assert_eq!(m.at(1.), [0xfc, 0xfd, 0xbf]);
}

#[test]
fn ramps_from_a_function_and_in_steps() {
    let grey = Ramp::from_fn(|t| [t; 3]);
    assert_eq!(grey.at(0.), [0; 3]);
    assert_eq!(grey.at(1.), [255; 3]);
    // Evenly spaced in linear light, so the middle is light in sRGB.
    assert_eq!(grey.at(0.5), to_srgb([128.0 / 255.0; 3]));
    let steps = viridis().steps(3);
    assert_eq!(steps.len(), 3);
    assert_eq!(to_srgb(steps[0]), viridis().at(0.));
    assert_eq!(to_srgb(steps[2]), viridis().at(1.));
    assert_eq!(viridis().steps(1).len(), 1);
}

#[test]
fn diverging_is_pale_at_zero_and_equally_light_either_side() {
    let zero = diverging(0.0);
    assert!(zero.iter().all(|&v| v > 240), "{zero:?}");
    let (blue, red) = (diverging(-1.0), diverging(1.0));
    assert!(blue[2] > blue[0] && red[0] > red[2]);
    let l = |c: Srgb| oklab(to_linear(c))[0];
    for t in [0.3, 0.7, 1.0] {
        assert!((l(diverging(-t)) - l(diverging(t))).abs() < 0.02, "{t}");
    }
    assert_eq!(diverging(f32::NAN), zero);
    assert_eq!(diverging(5.0), red);
}

#[test]
fn luminance_runs_from_black_to_white() {
    assert_eq!(luminance([0; 3]), 0.0);
    assert!((luminance([255; 3]) - 1.0).abs() < 1e-6);
    assert!(luminance([0, 255, 0]) > luminance([255, 0, 0]));
}
