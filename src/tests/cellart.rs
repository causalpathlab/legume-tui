//! A frame as coloured block characters. Ported from pinto's
//! `src/view/tests/cellart.rs`.

use super::*;
use crate::map::colour::{encode, encode_senna, linear};
use crate::map::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Color;
use ratatui::widgets::Widget;

const RED: [u8; 3] = [200, 30, 30];
const BLUE: [u8; 3] = [30, 30, 200];

fn lin(c: [u8; 3]) -> [f32; 3] {
    c.map(linear)
}

/// Mean of sRGB colours, taken in linear light as the fit does.
fn mean(cs: &[[u8; 3]]) -> [u8; 3] {
    [0, 1, 2].map(|ch| encode(cs.iter().map(|c| linear(c[ch])).sum::<f32>() / cs.len() as f32))
}

fn quadrant(px: [[u8; 3]; 4]) -> (char, [u8; 3], [u8; 3]) {
    fit(&px.map(lin), Glyphs::Quadrants)
}

fn symbol(px: &[[u8; 3]; 64]) -> (char, [u8; 3], [u8; 3]) {
    fit(&px.map(lin), Glyphs::Symbols)
}

/// Cells of 1:2 terminal characters.
fn fit_cells(frame: &Frame, glyphs: Glyphs, cols: usize, rows: usize) -> Cells {
    Cells::fit(frame, glyphs, glyphs.pixels_per_cell(2.), (cols, rows))
}

fn from_bytes(w: usize, h: usize, rgba: &[u8]) -> Frame {
    Frame::from_srgba(w, h, rgba, Theme::DARK.background)
}

#[test]
fn a_uniform_cell_is_blank_background() {
    assert_eq!(quadrant([RED; 4]), (' ', RED, RED));
}

#[test]
fn two_colour_splits_are_shown_exactly() {
    // Pixels are top-left, top-right, bottom-left, bottom-right; the
    // bottom-right pixel always takes the background.
    assert_eq!(quadrant([RED, BLUE, RED, BLUE]), ('▌', RED, BLUE));
    assert_eq!(quadrant([RED, RED, BLUE, BLUE]), ('▀', RED, BLUE));
    assert_eq!(quadrant([RED, BLUE, BLUE, RED]), ('▞', BLUE, RED));
    assert_eq!(quadrant([BLUE, RED, RED, RED]), ('▘', BLUE, RED));
}

#[test]
fn the_odd_pixel_out_joins_the_closer_group() {
    // A near-red pixel belongs with the reds, not alone.
    let near_red = [180, 40, 40];
    let (glyph, fg, _) = quadrant([RED, near_red, BLUE, BLUE]);
    assert_eq!(glyph, '▀');
    assert_eq!(fg, mean(&[RED, near_red]));
}

#[test]
fn quadrant_cells_cover_twice_the_width_of_half_blocks() {
    // Two cells of 2×4 square frame pixels: left cell red, right cell blue.
    let mut rgba = Vec::new();
    for _y in 0..4 {
        for x in 0..4 {
            let c = if x < 2 { RED } else { BLUE };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let frame = from_bytes(4, 4, &rgba);
    let area = Rect::new(0, 0, 2, 1);
    let mut buf = Buffer::empty(area);
    (&fit_cells(&frame, Glyphs::Quadrants, 2, 1)).render(area, &mut buf);
    let bg = |x| buf[(x, 0)].bg;
    assert_eq!(bg(0), Color::Rgb(RED[0], RED[1], RED[2]));
    assert_eq!(bg(1), Color::Rgb(BLUE[0], BLUE[1], BLUE[2]));
}

#[test]
fn quadrant_pixels_average_the_two_frame_rows_they_cover() {
    // One cell, 2×4 frame pixels: rows alternate red and blue, so every
    // glyph pixel is their average and the cell is uniform.
    let mut rgba = Vec::new();
    for y in 0..4 {
        for _x in 0..2 {
            let c = if y % 2 == 0 { RED } else { BLUE };
            rgba.extend_from_slice(&[c[0], c[1], c[2], 255]);
        }
    }
    let frame = from_bytes(2, 4, &rgba);
    let area = Rect::new(0, 0, 1, 1);
    let mut buf = Buffer::empty(area);
    (&fit_cells(&frame, Glyphs::Quadrants, 1, 1)).render(area, &mut buf);
    assert_eq!(buf[(0, 0)].symbol(), " ");
    let [r, g, b] = mean(&[RED, BLUE]);
    assert_eq!(buf[(0, 0)].bg, Color::Rgb(r, g, b));
}

#[test]
fn past_the_frame_reads_as_its_page() {
    // A 1×2 red frame under two cells of 2×4 pixels: the top-left glyph
    // pixel is red, the rest is the page.
    let frame = from_bytes(1, 2, &[RED[0], RED[1], RED[2], 255].repeat(2));
    let area = Rect::new(0, 0, 2, 1);
    let mut buf = Buffer::empty(area);
    (&fit_cells(&frame, Glyphs::Quadrants, 2, 1)).render(area, &mut buf);
    let [r, g, b] = Theme::DARK.background;
    assert_eq!(buf[(1, 0)].symbol(), " ");
    assert_eq!(buf[(1, 0)].bg, Color::Rgb(r, g, b));
    assert_eq!(buf[(0, 0)].symbol(), "▘");
    assert_eq!(buf[(0, 0)].fg, Color::Rgb(RED[0], RED[1], RED[2]));
}

#[test]
fn cells_past_the_area_are_left_alone() {
    let frame = from_bytes(1, 1, &[RED[0], RED[1], RED[2], 255]);
    let area = Rect::new(0, 0, 1, 1);
    let mut buf = Buffer::empty(Rect::new(0, 0, 3, 1));
    (&fit_cells(&frame, Glyphs::HalfBlocks, 3, 1)).render(area, &mut buf);
    assert_eq!(buf[(1, 0)], ratatui::buffer::Cell::default());
}

/// An 8×8 cell, `RED` where `blue(x, y)` is false.
fn cell(blue: impl Fn(usize, usize) -> bool) -> [[u8; 3]; 64] {
    std::array::from_fn(|i| if blue(i % 8, i / 8) { BLUE } else { RED })
}

#[test]
fn symbols_place_a_horizontal_boundary_at_an_eighth() {
    assert_eq!(symbol(&cell(|_, y| y >= 5)), ('▃', BLUE, RED));
    // A blue top eighth is the red lower seven-eighths.
    assert_eq!(symbol(&cell(|_, y| y < 1)), ('▇', RED, BLUE));
}

#[test]
fn symbols_place_a_vertical_boundary_at_an_eighth() {
    assert_eq!(symbol(&cell(|x, _| x < 5)), ('▋', BLUE, RED));
    // A blue right eighth is the red left seven-eighths.
    assert_eq!(symbol(&cell(|x, _| x >= 7)), ('▉', RED, BLUE));
}

#[test]
fn symbols_fall_back_to_quadrants_for_corners() {
    let (glyph, fg, bg) = symbol(&cell(|x, y| x >= 4 && y < 4));
    assert_eq!(glyph, '▝');
    assert_eq!((fg, bg), (BLUE, RED));
}

#[test]
fn a_uniform_symbol_cell_is_blank() {
    assert_eq!(symbol(&cell(|_, _| false)), (' ', RED, RED));
}

#[test]
fn frame_pixels_per_cell_follow_the_font_aspect() {
    assert_eq!(Glyphs::Symbols.pixels_per_cell(2.), (8, 16));
    assert_eq!(Glyphs::Quadrants.pixels_per_cell(2.), (2, 4));
    assert_eq!(Glyphs::HalfBlocks.pixels_per_cell(2.), (1, 2));
    // A font 2.3 times as tall as wide gets more rows, never fewer than
    // the glyph grid has.
    assert_eq!(Glyphs::Quadrants.pixels_per_cell(2.3), (2, 5));
    assert_eq!(Glyphs::HalfBlocks.pixels_per_cell(1.), (1, 2));
}

#[test]
fn glyph_sets_keep_their_names() {
    assert_eq!(Glyphs::HalfBlocks.name(), "half-blocks");
    assert_eq!(Glyphs::Quadrants.name(), "quadrants");
    assert_eq!(Glyphs::Symbols.name(), "symbols");
}

#[test]
fn cells_show_the_bytes_a_drawn_frame_encodes_to() {
    // A linear value that senna's table, which a frame drawn from scratch
    // encodes through, takes one byte lower than the exact table: the cell
    // shows the frame's byte, as kitty would be sent it.
    let c = (0..4096)
        .map(|i| i as f32 / 4096.)
        .find(|&c| encode_senna(c) != encode(c))
        .expect("a value the tables disagree on");
    let frame = Frame::from_canvas(1, 2, vec![[c, c, c]; 2], [c, c, c]);
    let byte = frame.to_image().into_raw()[0];
    assert_eq!(byte, encode_senna(c));
    let cells = fit_cells(&frame, Glyphs::HalfBlocks, 1, 1);
    let mut buf = Buffer::empty(Rect::new(0, 0, 1, 1));
    (&cells).render(buf.area, &mut buf);
    let cell = &buf[(0, 0)];
    assert_eq!(cell.bg, Color::Rgb(byte, byte, byte));
    assert_eq!(cell.fg, Color::Rgb(byte, byte, byte));
}
