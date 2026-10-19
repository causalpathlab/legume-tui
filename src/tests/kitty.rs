//! Kitty graphics: frames inline or through a file, double-buffered ids,
//! tmux's passthrough, and what `--graphics` names. Ported from pinto's
//! `src/view/tests/kitty.rs`.

use super::*;
use crate::map::theme::Theme;
use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine;

fn frame(w: usize, h: usize) -> Frame {
    Frame::from_srgba(w, h, &vec![7; w * h * 4], Theme::DARK.background)
}

fn decode_inline(bytes: &[u8]) -> (String, Vec<u8>) {
    // First escape's keys, and every chunk's payload concatenated.
    let text = String::from_utf8(bytes.to_vec()).unwrap();
    let mut keys = String::new();
    let mut payload = String::new();
    for part in text.split("\x1b_G").skip(1) {
        let body = part.split("\x1b\\").next().unwrap();
        let (k, p) = body.split_once(';').unwrap_or((body, ""));
        if keys.is_empty() {
            keys = k.to_string();
        }
        if !k.starts_with("a=d") {
            payload.push_str(p);
        }
    }
    (keys, B64.decode(payload).unwrap())
}

#[test]
fn direct_frames_round_trip_through_zlib_chunks() {
    let f = frame(300, 200);
    let mut out = Vec::new();
    let mut k = Kitty::new(Transport::Inline);
    k.show(&mut out, &f, (2, 3), (40, 20)).unwrap();

    let (keys, z) = decode_inline(&out);
    assert!(keys.contains("s=300,v=200"));
    assert!(keys.contains("c=40,r=20"));
    assert!(keys.contains("o=z") && keys.contains("q=2"));
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&z[..]), &mut raw).unwrap();
    assert_eq!(raw, f.to_image().into_raw());
}

#[test]
fn a_frame_that_does_not_compress_goes_in_chunks_within_the_limit() {
    // Bytes from a linear congruential sequence: zlib cannot shrink them.
    let mut x = 1u32;
    let rgba: Vec<u8> = (0..64 * 64 * 4)
        .map(|_| {
            x = x.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            (x >> 24) as u8
        })
        .collect();
    let f = Frame::from_srgba(64, 64, &rgba, Theme::DARK.background);
    let mut out = Vec::new();
    Kitty::new(Transport::Inline)
        .show(&mut out, &f, (0, 0), (8, 4))
        .unwrap();

    let (_, z) = decode_inline(&out);
    let mut raw = Vec::new();
    std::io::Read::read_to_end(&mut flate2::read::ZlibDecoder::new(&z[..]), &mut raw).unwrap();
    assert_eq!(raw, f.to_image().into_raw());
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("m=1;") && text.contains("m=0;"));
    assert!(text.contains("m=1;"));
    for part in text.split("\x1b_G").skip(1) {
        let body = part.split("\x1b\\").next().unwrap();
        assert!(body.split_once(';').map_or(0, |(_, p)| p.len()) <= 4096);
    }
}

#[test]
fn frames_alternate_ids_and_delete_the_previous_one() {
    let f = frame(4, 4);
    let mut k = Kitty::new(Transport::Inline);
    let mut first = Vec::new();
    k.show(&mut first, &f, (0, 0), (1, 1)).unwrap();
    let mut second = Vec::new();
    k.show(&mut second, &f, (0, 0), (1, 1)).unwrap();

    let first = String::from_utf8(first).unwrap();
    let second = String::from_utf8(second).unwrap();
    assert!(first.contains(&format!("i={}", IDS[0])) && !first.contains("a=d"));
    assert!(second.contains(&format!("i={}", IDS[1])));
    assert!(second.contains(&format!("a=d,d=I,i={}", IDS[0])));
}

#[test]
fn inside_tmux_commands_are_wrapped_with_escapes_doubled() {
    let f = frame(2, 2);
    let mut out = Vec::new();
    let mut k = Kitty::new(Transport::Inline).through_tmux(true);
    k.show(&mut out, &f, (0, 0), (1, 1)).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.contains("\x1bPtmux;\x1b\x1b_G"));
    assert!(text.contains("\x1b\x1b\\\x1b\\"));
    // No graphics command escapes tmux unwrapped.
    assert!(!text.replace("\x1b\x1b_G", "").contains("\x1b_G"));
}

#[test]
fn a_file_frame_is_named_for_its_tool_and_removed_when_cleared() {
    let f = frame(3, 2);
    let mut k = Kitty::new(Transport::File).named("lt6-test");
    let mut out = Vec::new();
    k.show(&mut out, &f, (1, 1), (2, 1)).unwrap();

    let (keys, named) = decode_inline(&out);
    assert!(keys.contains("t=t") && keys.contains("q=2"));
    let path = PathBuf::from(String::from_utf8(named).unwrap());
    assert_eq!(path.parent(), Some(std::env::temp_dir().as_path()));
    let name = path.file_name().unwrap().to_string_lossy().into_owned();
    assert!(name.starts_with("tty-graphics-protocol-lt6-test-"));
    assert!(name.ends_with(".rgba"));
    assert_eq!(std::fs::read(&path).unwrap(), f.to_image().into_raw());
    assert_eq!(k.last_bytes, 3 * 2 * 4);

    let mut out = Vec::new();
    k.clear(&mut out).unwrap();
    let text = String::from_utf8(out).unwrap();
    for id in IDS {
        assert!(text.contains(&format!("a=d,d=I,i={id},q=2")));
    }
    assert!(!path.exists());
}

/// The file a file-transport frame's command names.
fn named_file(out: &[u8]) -> PathBuf {
    let (_, named) = decode_inline(out);
    PathBuf::from(String::from_utf8(named).unwrap())
}

#[test]
fn file_frames_are_removed_when_the_kitty_is_dropped() {
    let mut k = Kitty::new(Transport::File).named("lt6-drop");
    let paths: Vec<PathBuf> = (0..2)
        .map(|_| {
            let mut out = Vec::new();
            k.show(&mut out, &frame(2, 2), (0, 0), (1, 1)).unwrap();
            named_file(&out)
        })
        .collect();
    assert!(paths.iter().all(|p| p.exists()));
    drop(k);
    assert!(paths.iter().all(|p| !p.exists()));
}

#[test]
fn each_image_keeps_its_last_two_files_for_the_terminal_to_read() {
    // Six frames alternate the two ids: the terminal may not yet have read
    // the files of the last two frames of each, so those stay; older go.
    let mut k = Kitty::new(Transport::File).named("lt6-queue");
    let paths: Vec<PathBuf> = (0..6)
        .map(|_| {
            let mut out = Vec::new();
            k.show(&mut out, &frame(2, 2), (0, 0), (1, 1)).unwrap();
            named_file(&out)
        })
        .collect();
    for (i, path) in paths.iter().enumerate() {
        assert_eq!(path.exists(), i >= 2, "frame {i}");
    }
    k.clear(&mut Vec::new()).unwrap();
    assert!(paths.iter().all(|p| !p.exists()));
}

#[cfg(unix)]
#[test]
fn a_file_frame_does_not_write_through_a_planted_link() {
    // Links at the names a frame's file could be guessed to have, pointing
    // at a file of the user's: showing frames must leave it alone.
    let dir = tempfile::tempdir().unwrap();
    let precious = dir.path().join("precious");
    std::fs::write(&precious, b"precious").unwrap();
    let tool = format!("lt6-link-{}", std::process::id());
    let planted: Vec<PathBuf> = (0..2)
        .map(|slot| {
            std::env::temp_dir().join(format!(
                "tty-graphics-protocol-{tool}-{}-{slot}.rgba",
                std::process::id()
            ))
        })
        .collect();
    for link in &planted {
        let _ = std::fs::remove_file(link);
        std::os::unix::fs::symlink(&precious, link).unwrap();
    }

    let mut k = Kitty::new(Transport::File).named(&tool);
    for _ in 0..2 {
        k.show_rgba(&mut Vec::new(), (1, 1), &[1, 2, 3, 4], (0, 0), (1, 1))
            .unwrap();
    }
    k.clear(&mut Vec::new()).unwrap();
    for link in &planted {
        let _ = std::fs::remove_file(link);
    }
    assert_eq!(std::fs::read(&precious).unwrap(), b"precious");
}

#[test]
fn too_few_bytes_for_the_frame_are_an_error() {
    for transport in [Transport::Inline, Transport::File] {
        let mut k = Kitty::new(transport);
        let mut out = Vec::new();
        let err = k
            .show_rgba(&mut out, (4, 4), &[0u8; 8], (0, 0), (1, 1))
            .unwrap_err();
        assert_eq!(err.kind(), std::io::ErrorKind::InvalidInput);
        assert!(out.is_empty());
        assert_eq!(k.last_bytes, 0);
    }
}

#[test]
fn the_cursor_is_saved_placed_and_restored() {
    let mut k = Kitty::new(Transport::Inline);
    let mut out = Vec::new();
    k.show(&mut out, &frame(1, 1), (4, 2), (1, 1)).unwrap();
    let text = String::from_utf8(out).unwrap();
    assert!(text.starts_with("\x1b7\x1b[3;5H"));
    assert!(text.ends_with("\x1b8"));
}

#[test]
fn map_graphics_name_the_crate_protocol_where_there_is_one() {
    use crate::graphics::Graphics;
    let to = |g: MapGraphics| Option::<Graphics>::from(g);
    assert_eq!(to(MapGraphics::Auto), Some(Graphics::Auto));
    assert_eq!(to(MapGraphics::Kitty), Some(Graphics::Kitty));
    assert_eq!(to(MapGraphics::KittyInline), Some(Graphics::Kitty));
    assert_eq!(to(MapGraphics::Sixel), Some(Graphics::Sixel));
    assert_eq!(to(MapGraphics::Iterm2), Some(Graphics::Iterm2));
    assert_eq!(to(MapGraphics::Blocks), Some(Graphics::Blocks));
    assert_eq!(to(MapGraphics::Quadrants), None);
    assert_eq!(to(MapGraphics::Symbols), None);
    assert_eq!(MapGraphics::default(), MapGraphics::Auto);
}

#[test]
fn map_graphics_want_the_protocol_pinto_asked_for() {
    use ratatui_image::picker::ProtocolType;
    assert_eq!(MapGraphics::Auto.protocol(), None);
    assert_eq!(
        MapGraphics::KittyInline.protocol(),
        Some(ProtocolType::Kitty)
    );
    assert_eq!(MapGraphics::Iterm2.protocol(), Some(ProtocolType::Iterm2));
    for g in [
        MapGraphics::Quadrants,
        MapGraphics::Symbols,
        MapGraphics::Blocks,
    ] {
        assert_eq!(g.protocol(), Some(ProtocolType::Halfblocks));
    }
    assert_eq!(MapGraphics::Quadrants.glyphs(), Some(Glyphs::Quadrants));
    assert_eq!(MapGraphics::Symbols.glyphs(), Some(Glyphs::Symbols));
    assert_eq!(MapGraphics::Blocks.glyphs(), Some(Glyphs::HalfBlocks));
    assert_eq!(MapGraphics::Kitty.glyphs(), None);
}

#[cfg(feature = "clap")]
#[test]
fn map_graphics_parse_as_pinto_named_them() {
    use clap::ValueEnum;
    let names: Vec<String> = MapGraphics::value_variants()
        .iter()
        .map(|g| g.to_possible_value().unwrap().get_name().to_string())
        .collect();
    assert_eq!(
        names,
        [
            "auto",
            "kitty",
            "kitty-inline",
            "sixel",
            "iterm2",
            "quadrants",
            "symbols",
            "blocks"
        ]
    );
}
