//! A traced gallery: every file of a save with its size, time and hash,
//! checked against the disk, and nothing moved or deleted on the word of a
//! log that is not this user's alone (lupin `src/tui/tests/gallery.rs`).

use super::*;

/// What a figure was drawn from, as a viewer keeps it beside each entry.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Trace {
    panel: String,
    manifest: PathBuf,
}

fn trace(panel: &str, dir: &Path) -> Trace {
    Trace {
        panel: panel.into(),
        manifest: dir.join("run.senna.json"),
    }
}

fn open(dir: &Path) -> Gallery<Trace> {
    Gallery::open(dir, Policy::traced())
}

fn figure_set(dir: &Path, base: &str) -> Vec<PathBuf> {
    let svg = dir.join(format!("{base}.svg"));
    let pdf = dir.join(format!("{base}.pdf"));
    std::fs::write(&svg, "<svg/>").unwrap();
    std::fs::write(&pdf, "%PDF-1.4 fake").unwrap();
    vec![svg, pdf]
}

#[test]
fn an_export_is_logged_checked_moved_and_removed() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let dir = dir.as_path();
    let log_dir = dir.join(".lupin-view");
    let files = figure_set(dir, "run.trajectory.order");
    let pdf = files[1].clone();
    let mut g = open(&log_dir);
    g.add(&files, "order · 7 in", None, trace("order", dir))
        .unwrap();
    assert_eq!(g.entries().len(), 1);
    assert_eq!(g.entries()[0].path, pdf);
    assert_eq!(g.entries()[0].files.len(), 2);
    assert_eq!(g.entries()[0].extra, trace("order", dir));
    assert_eq!(g.entries()[0].files[0].hash.len(), 64);
    assert_eq!(g.check(), vec![Status::Intact]);

    // Same bytes under a new mtime: still ok; other bytes of the same size,
    // or another size: changed.
    std::fs::write(&files[1], "%PDF-1.4 fake").unwrap();
    assert_eq!(open(&log_dir).check(), vec![Status::Intact]);
    let mut g2 = open(&log_dir);
    g2.entries[0].files[1].mtime = 1;
    std::fs::write(&files[1], "%PDF-1.4 faKe").unwrap();
    assert_eq!(g2.check(), vec![Status::Changed]);
    std::fs::write(&files[1], "%PDF-1.4 other").unwrap();
    assert_eq!(open(&log_dir).check(), vec![Status::Changed]);

    // A changed file is neither moved nor deleted on the log's word.
    let mut g = open(&log_dir);
    assert!(g.relocate(&pdf, &dir.join("figure1")).is_err());
    assert!(g.remove(&pdf, true).is_err());
    std::fs::write(&files[1], "%PDF-1.4 fake").unwrap();

    // Moving renames every file of the set and refuses to replace.
    let mut g = open(&log_dir);
    let to = g.relocate(&pdf, &dir.join("figure1")).unwrap();
    assert_eq!(to, dir.join("figure1.pdf"));
    assert!(dir.join("figure1.svg").is_file() && dir.join("figure1.pdf").is_file());
    assert!(!files[0].exists());
    figure_set(dir, "taken");
    assert!(g
        .relocate(&dir.join("figure1.pdf"), &dir.join("taken"))
        .is_err());

    // A missing file is reported, not dropped.
    std::fs::remove_file(dir.join("figure1.pdf")).unwrap();
    let mut g = open(&log_dir);
    assert_eq!(g.check(), vec![Status::Missing]);
    assert_eq!(g.entries().len(), 1);
    assert!(
        g.remove(&dir.join("figure1.pdf"), true).is_err(),
        "a missing set is not deleted blindly"
    );
    g.remove(&dir.join("figure1.pdf"), false).unwrap();
    assert!(g.entries().is_empty() && dir.join("figure1.svg").exists());
}

#[test]
fn a_file_that_is_not_of_the_set_is_neither_moved_nor_deleted() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let log_dir = dir.join(".lupin-view");
    let mut files = figure_set(&dir, "run.trajectory.order");
    let other = dir.join("other.svg");
    std::fs::write(&other, "<svg/>").unwrap();
    files.push(other.clone());
    let mut g = open(&log_dir);
    g.add(&files, "order", None, trace("order", &dir)).unwrap();
    assert!(g.remove(&files[1], true).is_err());
    assert!(g.relocate(&files[1], &dir.join("elsewhere")).is_err());
    assert!(files.iter().all(|f| f.is_file()));
}

#[test]
fn an_unreadable_log_is_set_aside_not_emptied() {
    let tmp = tempfile::tempdir().unwrap();
    let log_dir = tmp.path().join(".lupin-view");
    std::fs::create_dir_all(&log_dir).unwrap();
    std::fs::write(log_dir.join("saved.json"), "{ not json").unwrap();
    std::fs::create_dir_all(log_dir.join("thumbs")).unwrap();
    std::fs::write(log_dir.join("thumbs").join("1.png"), b"").unwrap();
    let g = open(&log_dir);
    assert!(g.entries().is_empty());
    assert!(g.set_aside().is_some_and(Path::is_file));
    assert!(!log_dir.join("saved.json").exists());
    // The thumbnails stay with the log set aside, to be recovered.
    assert!(log_dir.join("thumbs").join("1.png").exists());
}

#[test]
fn a_thumbnail_goes_beside_a_traced_save_too() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let log_dir = dir.join(".lupin-view");
    let files = figure_set(&dir, "run.view");
    let mut g = open(&log_dir);
    let pic = RgbaImage::from_pixel(480, 360, image::Rgba([10, 20, 30, 255]));
    g.add(&files, "map", Some(&pic), trace("map", &dir))
        .unwrap();
    let t = thumb::read_png(&g.thumb(&g.entries()[0]).unwrap()).unwrap();
    assert_eq!(t.dimensions(), (THUMB_WIDTH, 180));
    g.remove(&files[1], false).unwrap();
    assert_eq!(
        std::fs::read_dir(log_dir.join("thumbs")).unwrap().count(),
        0
    );
}

#[cfg(unix)]
#[test]
fn a_log_others_can_write_moves_and_deletes_nothing() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let log_dir = dir.join(".lupin-view");
    let files = figure_set(&dir, "run.trajectory.order");
    let mut g = open(&log_dir);
    g.add(&files, "order", None, trace("order", &dir)).unwrap();
    let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
    assert_eq!(
        mode(&log_dir),
        0o700,
        "the log's directory is created private"
    );
    assert_eq!(mode(&log_dir.join(LOG)) & 0o077, 0, "so is the log");

    let open_to_all = std::fs::Permissions::from_mode(0o666);
    std::fs::set_permissions(log_dir.join(LOG), open_to_all).unwrap();
    let mut g = open(&log_dir);
    assert!(g.remove(&files[1], true).is_err());
    assert!(g.relocate(&files[1], &dir.join("elsewhere")).is_err());
    assert!(files.iter().all(|f| f.is_file()));
    // Nor is an entry taken off a log others can write, nor is the log
    // taken over by saving a new export over it.
    assert!(g.remove(&files[1], false).is_err());
    let more = figure_set(&dir, "run.trajectory.layout");
    assert!(g.add(&more, "layout", None, trace("layout", &dir)).is_err());
    assert_eq!(mode(&log_dir.join(LOG)), 0o666, "the log is left as it was");
}

#[cfg(unix)]
#[test]
fn a_log_that_is_a_link_moves_and_deletes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let files = figure_set(&dir, "run.trajectory.order");
    // A genuine log of this user's elsewhere, linked in as this one.
    let other = dir.join("elsewhere");
    let mut g = open(&other);
    g.add(&files, "order", None, trace("order", &dir)).unwrap();
    let log_dir = dir.join(".lupin-view");
    std::fs::create_dir(&log_dir).unwrap();
    std::os::unix::fs::symlink(other.join(LOG), log_dir.join(LOG)).unwrap();
    let mut g = open(&log_dir);
    assert!(g.remove(&files[1], true).is_err());
    assert!(files.iter().all(|f| f.is_file()));
}

#[test]
fn a_file_is_recorded_with_its_size_and_sha256() {
    let tmp = tempfile::tempdir().unwrap();
    let f = tmp.path().join("abc.txt");
    std::fs::write(&f, "abc").unwrap();
    let r = FileRecord::of(&f).unwrap();
    assert_eq!(r.size, 3);
    assert_eq!(
        r.hash,
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    assert!(FileRecord::of(&tmp.path().join("gone")).is_err());
}

#[test]
fn a_set_whose_second_file_cannot_move_is_put_back() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().canonicalize().unwrap();
    let log_dir = dir.join(".lupin-view");
    let (pdf, json) = (dir.join("run.pdf"), dir.join("run.json"));
    std::fs::write(&pdf, "%PDF-1.4 fake").unwrap();
    std::fs::write(&json, "{}").unwrap();
    let mut g = open(&log_dir);
    g.add(&[&pdf, &json], "order", None, trace("order", &dir))
        .unwrap();
    // A name the PDF fits under and the table, one byte longer, does not.
    let long = dir.join("x".repeat(255 - ".pdf".len()));
    assert!(g.relocate(&pdf, &long).is_err());
    assert!(pdf.is_file() && json.is_file(), "the moved PDF is put back");
    assert!(!long.with_extension("pdf").exists());
    assert_eq!(open(&log_dir).entries()[0].path, pdf);
}
