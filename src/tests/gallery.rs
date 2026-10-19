//! The gallery: saves listed newest first and kept for next time, their
//! thumbnails, and the logs the viewers wrote before it was shared.

use super::*;

fn picture(w: u32, h: u32) -> RgbaImage {
    RgbaImage::from_fn(w, h, |x, y| {
        image::Rgba([(x % 256) as u8, (y % 256) as u8, 90, 255])
    })
}

/// A saved file in `dir` with some bytes in it.
fn file(dir: &Path, name: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, b"%PDF").unwrap();
    p
}

fn open(store: &Path) -> Gallery {
    Gallery::open(store, Policy::plain())
}

fn add(g: &mut Gallery, path: &Path, what: &str, pic: &RgbaImage) {
    g.add(&[path], what, Some(pic), ()).unwrap();
}

fn names<X>(g: &Gallery<X>) -> Vec<String> {
    g.entries().iter().map(Entry::name).collect()
}

fn thumbs(store: &Path) -> usize {
    std::fs::read_dir(store.join("thumbs")).unwrap().count()
}

// senna `src/view/saved.rs`.

#[test]
fn saves_are_listed_newest_first_and_kept_for_next_time() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    assert!(g.entries().is_empty());
    add(&mut g, &a, "this view · 7 in · 300 dpi", &picture(800, 600));
    add(&mut g, &b, "all 2 runs, one grid", &picture(1200, 400));
    assert_eq!(names(&g), ["b.pdf", "a.pdf"]);
    // Each has a small thumbnail on disk, at the picture's aspect.
    let thumb = thumb::read_png(&g.thumb(&g.entries()[1]).unwrap()).unwrap();
    assert_eq!(thumb.width(), THUMB_WIDTH);
    assert_eq!(thumb.height(), THUMB_WIDTH * 3 / 4);
    // A new viewer in the same directory sees the same list.
    let again = open(&store);
    assert_eq!(names(&again), ["b.pdf", "a.pdf"]);
    assert_eq!(again.entries()[0].what, "all 2 runs, one grid");
}

#[test]
fn saving_a_file_again_moves_it_to_the_top_without_a_copy() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    add(&mut g, &a, "first", &picture(100, 100));
    add(&mut g, &b, "second", &picture(100, 100));
    add(&mut g, &a, "first, redone", &picture(100, 100));
    let listed: Vec<(String, &str)> = g
        .entries()
        .iter()
        .map(|e| (e.name(), e.what.as_str()))
        .collect();
    assert_eq!(
        listed,
        [
            ("a.pdf".into(), "first, redone"),
            ("b.pdf".into(), "second")
        ]
    );
    // One thumbnail per entry: the replaced one is gone.
    assert_eq!(thumbs(&store), 2);
}

#[test]
fn a_saved_file_deleted_since_is_dropped_on_open() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    add(&mut g, &a, "a", &picture(100, 100));
    add(&mut g, &b, "b", &picture(100, 100));
    std::fs::remove_file(&a).unwrap();
    assert_eq!(names(&open(&store)), ["b.pdf"]);
}

#[test]
fn an_unreadable_log_starts_an_empty_gallery() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    std::fs::create_dir_all(&store).unwrap();
    std::fs::write(store.join("saved.json"), b"not json").unwrap();
    let g = open(&store);
    assert!(g.entries().is_empty());
    assert!(g.set_aside().is_none());
}

#[test]
fn times_read_as_how_long_ago() {
    let now = 1_000_000;
    assert_eq!(ago(now - 20, now), "just now");
    assert_eq!(ago(now - 300, now), "5 min ago");
    assert_eq!(ago(now - 2 * 3600 - 100, now), "2 h ago");
    assert_eq!(ago(now - 3 * 86_400, now), "3 d ago");
    // A clock that went back is not "in the future".
    assert_eq!(ago(now + 50, now), "just now");
}

#[test]
fn a_deleted_files_thumbnail_goes_with_it() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    add(&mut g, &a, "a", &picture(100, 100));
    add(&mut g, &b, "b", &picture(100, 100));
    std::fs::remove_file(&a).unwrap();
    let again = open(&store);
    assert_eq!(thumbs(&store), 1);
    assert!(again.thumb(&again.entries()[0]).unwrap().exists());
}

#[test]
fn a_save_taken_off_the_list_keeps_its_pdf_unless_deleted() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    add(&mut g, &a, "a", &picture(100, 100));
    add(&mut g, &b, "b", &picture(100, 100));
    let listed_a = g.entries()[1].path.clone();
    g.remove(&listed_a, false).unwrap();
    assert!(a.exists());
    let listed_b = g.entries()[0].path.clone();
    g.remove(&listed_b, true).unwrap();
    assert!(!b.exists());
    assert!(open(&store).entries().is_empty());
    assert_eq!(thumbs(&store), 0);
}

#[test]
fn a_moved_save_keeps_its_place_and_its_thumbnail() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut g = open(&store);
    add(&mut g, &a, "a", &picture(100, 100));
    add(&mut g, &b, "b", &picture(100, 100));
    let listed_a = g.entries()[1].path.clone();

    // Into a folder that is not there yet: the name stays.
    let figs = work.path().join("figs");
    let to = g
        .relocate(&listed_a, Path::new(&format!("{}/", figs.display())))
        .unwrap();
    assert_eq!(to, figs.canonicalize().unwrap().join("a.pdf"));
    assert!(!a.exists() && to.exists());

    // A new name gets `.pdf`; an existing file is never replaced.
    let to = g.relocate(&to, &figs.join("final")).unwrap();
    assert_eq!(to.file_name().unwrap(), "final.pdf");
    let listed_b = g.entries()[0].path.clone();
    assert!(g.relocate(&listed_b, &to).is_err());
    assert!(b.exists());

    let again = open(&store);
    assert_eq!(names(&again), ["b.pdf", "final.pdf"]);
    assert!(again.thumb(&again.entries()[1]).unwrap().exists());
}

#[test]
fn two_viewers_in_one_directory_keep_each_others_saves() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let (a, b) = (file(work.path(), "a.pdf"), file(work.path(), "b.pdf"));
    let mut first = open(&store);
    let mut second = open(&store);
    add(&mut first, &a, "from the first", &picture(100, 100));
    add(&mut second, &b, "from the second", &picture(100, 100));
    assert_eq!(names(&open(&store)), ["b.pdf", "a.pdf"]);
}

#[test]
fn only_so_many_saves_are_kept() {
    let work = tempfile::tempdir().unwrap();
    let store = work.path().join(".senna-view");
    let mut g = open(&store);
    for k in 0..KEEP + 2 {
        let f = file(work.path(), &format!("{k}.pdf"));
        g.add(&[&f], "a view", None, ()).unwrap();
    }
    assert_eq!(g.entries().len(), KEEP);
    assert_eq!(g.entries()[0].name(), format!("{}.pdf", KEEP + 1));
}

// pinto `src/view/tests/saved.rs`.

#[test]
fn saves_are_listed_newest_first_with_a_thumbnail_each() {
    let dir = tempfile::tempdir().unwrap();
    let store = dir.path().join(".pinto-view");
    let (a, b) = (dir.path().join("a.pdf"), dir.path().join("b.png"));
    std::fs::write(&a, "a").unwrap();
    std::fs::write(&b, "b").unwrap();
    let frame = |w, h| {
        RgbaImage::from_fn(w, h, |x, y| {
            image::Rgba([((x + y * w) % 256) as u8, 0, 0, 255])
        })
    };
    let mut g = open(&store);
    add(&mut g, &a, "map", &frame(480, 360));
    add(&mut g, &b, "structure", &frame(100, 50));
    add(&mut g, &a, "map again", &frame(480, 360));

    let g = open(&store);
    assert_eq!(names(&g), ["a.pdf", "b.png"]);
    assert_eq!(g.entries()[0].what, "map again");
    // Shrunk to the thumbnail width at the picture's aspect; never enlarged.
    let t = thumb::read_png(&g.thumb(&g.entries()[0]).unwrap()).unwrap();
    assert_eq!(t.dimensions(), (THUMB_WIDTH, THUMB_WIDTH * 3 / 4));
    let t = thumb::read_png(&g.thumb(&g.entries()[1]).unwrap()).unwrap();
    assert_eq!(t.dimensions(), (100, 50));
    // One thumbnail per entry: the replaced one is gone.
    assert_eq!(thumbs(&store), 2);

    // A deleted file leaves the list, its thumbnail with it.
    std::fs::remove_file(&b).unwrap();
    let g = open(&store);
    assert_eq!(g.entries().len(), 1);
    assert_eq!(thumbs(&store), 1);
}

#[test]
fn ages_read_in_the_largest_whole_unit() {
    assert_eq!(ago(100, 130), "just now");
    assert_eq!(ago(0, 7200), "2 h ago");
    assert_eq!(ago(0, 3 * 86_400 + 5), "3 d ago");
}

#[test]
fn a_thumbnail_averages_in_linear_light() {
    // Black beside white, one pixel wide: the mean of their light, not of
    // their bytes, and opaque.
    let img = RgbaImage::from_fn(2, 1, |x, _| {
        let v = if x == 0 { 0 } else { 255 };
        image::Rgba([v, v, v, 128])
    });
    let t = thumb::shrink(&img, 1);
    assert_eq!(t.dimensions(), (1, 1));
    assert_eq!(t.get_pixel(0, 0).0, [188, 188, 188, 255]);
    // Never enlarged, never empty.
    assert_eq!(thumb::shrink(&img, 10).dimensions(), (2, 1));
    let tall = RgbaImage::new(1, 50);
    assert_eq!(thumb::shrink(&tall, 1).dimensions(), (1, 50));
    let wide = RgbaImage::new(500, 1);
    assert_eq!(thumb::shrink(&wide, 10).dimensions(), (10, 1));
}

/// The thumbnail's tables are the map's, value for value.
#[cfg(feature = "map")]
#[test]
fn the_thumbnail_tables_are_the_maps() {
    use crate::map::colour;
    for v in 0..=255u8 {
        assert_eq!(thumb::linear(v).to_bits(), colour::linear(v).to_bits());
    }
    for i in 0..=8192 {
        let c = i as f32 / 8192.;
        assert_eq!(thumb::encode(c), colour::encode(c), "{c}");
    }
}

#[test]
fn a_thumbnail_written_reads_back_the_same() {
    let dir = tempfile::tempdir().unwrap();
    let p = dir.path().join("t.png");
    let img = picture(13, 7);
    thumb::write_png(&img, &p).unwrap();
    assert_eq!(thumb::read_png(&p).unwrap(), img);
    std::fs::write(&p, b"not a png").unwrap();
    assert!(thumb::read_png(&p).is_none());
    assert!(thumb::read_png(&dir.path().join("none.png")).is_none());
}

// The logs each viewer wrote before the gallery was shared.

/// `json` with `{dir}` set to `dir`, written as the log in `store`.
fn fixture(store: &Path, dir: &Path, json: &str) {
    std::fs::create_dir_all(store.join("thumbs")).unwrap();
    let json = json.replace("{dir}", &dir.to_string_lossy());
    std::fs::write(store.join("saved.json"), json).unwrap();
}

/// senna's log: path, what, when and thumbnail.
const SENNA: &str = r#"[
  {
    "path": "{dir}/run.view.grid.pdf",
    "what": "all 2 runs, one grid · 7 in · 300 dpi",
    "when": 1760000100,
    "thumb": "1760000100000000001.png"
  },
  {
    "path": "{dir}/run.view.pdf",
    "what": "this view · 3.5 in · 600 dpi",
    "when": 1760000000,
    "thumb": "1760000000000000001.png"
  }
]"#;

/// pinto's log: the same fields, its exports PNG or PDF.
const PINTO: &str = r#"[
  {
    "path": "{dir}/tool-view-001.png",
    "what": "map · final · argmax",
    "when": 1760000200,
    "thumb": "1760000200000000001.png"
  }
]"#;

#[test]
fn a_log_senna_wrote_loads() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    let store = dir.join(".senna-view");
    fixture(&store, dir, SENNA);
    for (f, t) in [
        ("run.view.grid.pdf", "1760000100000000001.png"),
        ("run.view.pdf", "1760000000000000001.png"),
    ] {
        file(dir, f);
        thumb::write_png(&picture(4, 3), &store.join("thumbs").join(t)).unwrap();
    }
    let g = open(&store);
    assert_eq!(names(&g), ["run.view.grid.pdf", "run.view.pdf"]);
    let e = &g.entries()[1];
    assert_eq!(
        (e.what.as_str(), e.when),
        ("this view · 3.5 in · 600 dpi", 1760000000)
    );
    assert!(e.files.is_empty());
    assert!(g.thumb(e).unwrap().exists());
    assert_eq!(g.check(), [Status::Intact, Status::Intact]);

    // Written again as senna wrote it: the same fields, no more.
    let mut g = g;
    add(&mut g, &file(dir, "new.pdf"), "this view", &picture(10, 10));
    let log: serde_json::Value =
        serde_json::from_slice(&std::fs::read(store.join("saved.json")).unwrap()).unwrap();
    for e in log.as_array().unwrap() {
        let mut keys: Vec<&str> = e.as_object().unwrap().keys().map(String::as_str).collect();
        keys.sort_unstable();
        assert_eq!(keys, ["path", "thumb", "what", "when"]);
    }
}

#[test]
fn a_log_pinto_wrote_loads_and_drops_what_is_gone() {
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    let store = dir.join(".pinto-view");
    fixture(&store, dir, PINTO);
    file(dir, "tool-view-001.png");
    // The thumbnail is missing: the entry goes, as pinto drops it.
    assert!(open(&store).entries().is_empty());
    fixture(&store, dir, PINTO);
    thumb::write_png(
        &picture(4, 3),
        &store.join("thumbs").join("1760000200000000001.png"),
    )
    .unwrap();
    let g = open(&store);
    assert_eq!(names(&g), ["tool-view-001.png"]);
    assert_eq!(g.entries()[0].what, "map · final · argmax");
}

#[test]
fn a_log_lupin_wrote_loads_its_files_and_trace() {
    #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
    struct Trace {
        panel: String,
        manifest: PathBuf,
    }
    let work = tempfile::tempdir().unwrap();
    let dir = work.path();
    let store = dir.join(".lupin-view");
    fixture(&store, dir, LUPIN);
    // A file of the set is gone: the entry stays, and says so.
    file(dir, "run.trajectory.order.pdf");
    let g: Gallery<Trace> = Gallery::open(
        &store,
        Policy {
            missing: Missing::Keep,
            ..Policy::plain()
        },
    );
    let e = &g.entries()[0];
    assert_eq!(e.name(), "run.trajectory.order.pdf");
    assert_eq!(e.extra.panel, "order");
    assert_eq!(e.extra.manifest, dir.join("run.senna.json"));
    assert_eq!(e.files.len(), 2);
    // An older log, before the mtime was kept.
    assert_eq!(e.files[1].mtime, 0);
    assert!(e.thumb.is_none() && g.thumb(e).is_none());
    assert_eq!(g.check(), [Status::Missing]);
    // A viewer that keeps no trace still lists it.
    let plain = open(&store);
    assert_eq!(names(&plain), ["run.trajectory.order.pdf"]);
}

/// lupin's log: a figure set, each file with its size, time and hash, and
/// what the figure was drawn from.
const LUPIN: &str = r#"[
  {
    "path": "{dir}/run.trajectory.order.pdf",
    "what": "order · 7 in",
    "when": 1760000300,
    "panel": "order",
    "manifest": "{dir}/run.senna.json",
    "files": [
      {
        "path": "{dir}/run.trajectory.order.svg",
        "size": 6,
        "mtime": 1760000300,
        "hash": "0000000000000000000000000000000000000000000000000000000000000000"
      },
      {
        "path": "{dir}/run.trajectory.order.pdf",
        "size": 4,
        "hash": "0000000000000000000000000000000000000000000000000000000000000000"
      }
    ]
  }
]"#;
