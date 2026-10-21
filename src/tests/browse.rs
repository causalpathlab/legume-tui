//! The file browser: folders then kept files, narrowed by typing, typed
//! paths walked, and each file's note and box.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn typed(b: &mut Browser, text: &str) {
    for c in text.chars() {
        assert_eq!(b.key(key(KeyCode::Char(c))), Nav::Moved, "{c}");
    }
}

fn names(b: &Browser) -> Vec<&str> {
    b.list.shown().map(|e| e.name.as_str()).collect()
}

/// `root/{ann/{genes.gtf.gz, notes.txt}, b.gff3, a.txt, .hidden.gtf, m.zarr/}`.
fn tree() -> tempfile::TempDir {
    let tmp = tempfile::tempdir().unwrap();
    let root = tmp.path();
    std::fs::create_dir_all(root.join("ann")).unwrap();
    std::fs::create_dir_all(root.join("m.zarr")).unwrap();
    for f in [
        "ann/genes.gtf.gz",
        "ann/notes.txt",
        "b.gff3",
        "a.txt",
        ".hidden.gtf",
    ] {
        std::fs::write(root.join(f), b"").unwrap();
    }
    tmp
}

fn kept(name: &str) -> bool {
    name.ends_with(".gtf.gz") || name.ends_with(".gff3") || name.ends_with(".zarr")
}

fn browser(root: &Path) -> Browser {
    let note = |p: &Path| {
        p.extension()
            .is_some_and(|e| e == "gff3")
            .then(|| "annotation".to_string())
    };
    Browser::new(root.to_path_buf(), kept, note).opened()
}

#[test]
fn lists_folders_then_the_kept_files_and_picks_a_file() {
    let tmp = tree();
    let root = normalize(tmp.path());
    let mut b = browser(&root);
    // Hidden files are left out, a zarr store is a file, others unless kept.
    assert_eq!(names(&b), ["..", "ann", "b.gff3", "m.zarr"]);
    let notes: Vec<Option<&str>> = b.list.shown().map(|e| e.note.as_deref()).collect();
    assert_eq!(notes, [None, None, Some("annotation"), None]);

    // Into `ann`, where only the annotation is listed, and pick it.
    b.list.at = 1;
    assert_eq!(b.key(key(KeyCode::Enter)), Nav::Moved);
    assert_eq!(b.cwd, root.join("ann"));
    assert_eq!(names(&b), ["..", "genes.gtf.gz"]);
    b.list.at = 1;
    // Right only opens folders; Enter picks a file.
    assert_eq!(b.key(key(KeyCode::Right)), Nav::Moved);
    assert_eq!(
        b.key(key(KeyCode::Enter)),
        Nav::Picked(root.join("ann/genes.gtf.gz"))
    );

    // Up lands back on the folder just left; other keys are the caller's.
    assert_eq!(b.key(key(KeyCode::Backspace)), Nav::Moved);
    assert_eq!(b.cwd, root);
    assert_eq!(b.highlighted().unwrap().name, "ann");
    for code in [KeyCode::Esc, KeyCode::Char(' '), KeyCode::Tab] {
        assert_eq!(b.key(key(code)), Nav::Ignored, "{code:?}");
    }
    // A chord is no letter.
    let ctrl_r = KeyEvent::new(KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(b.key(ctrl_r), Nav::Ignored);
}

#[test]
fn typing_narrows_the_listing_and_esc_restores_it() {
    let tmp = tree();
    let mut b = browser(tmp.path());
    // Any case, the cursor on the first name starting with the text, and
    // `..` hidden.
    typed(&mut b, "AN");
    assert_eq!(names(&b), ["ann"]);
    assert_eq!(b.highlighted().map(|e| e.dir), Some(true));
    assert_eq!(b.key(key(KeyCode::Esc)), Nav::Moved);
    // The whole list again, the cursor still on what was found.
    assert_eq!(b.highlighted().unwrap().name, "ann");
    typed(&mut b, "zz");
    assert!(b.list.is_empty());
    assert_eq!(b.key(key(KeyCode::Esc)), Nav::Moved);
    assert_eq!(names(&b), ["..", "ann", "b.gff3", "m.zarr"]);
    // With nothing typed, Esc is the caller's.
    assert_eq!(b.key(key(KeyCode::Esc)), Nav::Ignored);
}

#[test]
fn typing_a_path_walks_there() {
    let tmp = tree();
    let root = normalize(tmp.path());
    let mut b = browser(&std::env::temp_dir());
    typed(&mut b, &format!("{}/", root.display()));
    assert_eq!(b.cwd, root);
    assert!(b.list.find.text.is_empty());
    // A partial name goes to the highlighted folder; `../` back up.
    typed(&mut b, "an/");
    assert_eq!(b.cwd, root.join("ann"));
    typed(&mut b, "../");
    assert_eq!(b.cwd, root);
}

#[test]
fn a_relative_start_is_made_absolute_so_up_keeps_working() {
    let mut b = Browser::new(PathBuf::from("."), |_| false, |_| None).opened();
    assert!(b.cwd.is_absolute(), "{}", b.cwd.display());
    let start = b.cwd.clone();
    assert_eq!(b.key(key(KeyCode::Left)), Nav::Moved);
    assert_eq!(Some(b.cwd.as_path()), start.parent());
}

#[test]
fn lines_carry_each_files_box_and_note_and_cut_long_names() {
    let tmp = tree();
    let b = browser(tmp.path());
    let look = Look::terminal();
    let mark = |e: &Entry| (e.name == "m.zarr").then(|| "[data]".to_string());
    let text: Vec<String> = b
        .lines(10, 40, &look, 6, mark, "")
        .iter()
        .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
        .collect();
    assert_eq!(text[0].trim_end(), "▸        ../");
    assert!(text[2].starts_with("         b.gff3") && text[2].ends_with("  annotation"));
    assert!(text[3].starts_with("  [data] m.zarr"), "{}", text[3]);
    assert!(text.iter().all(|t| t.chars().count() <= 40), "{text:?}");

    let narrow = b.lines(10, 12, &look, 0, |_| None, "");
    let row: String = narrow[2].spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(row.contains('…'), "{row}");
}

#[test]
fn a_typed_path_is_absolute_from_home() {
    if let Some(home) = home() {
        assert_eq!(typed_path("~/a/../b"), normalize(&home.join("b")));
        assert_eq!(tilde(&home.join("x")), "~/x");
    }
    assert!(typed_path("rel").is_absolute());
}
