//! Label offsets and the file that keeps them per layout.

use super::*;

fn dir() -> tempfile::TempDir {
    tempfile::tempdir().unwrap()
}

#[test]
fn nudges_add_up_and_a_reset_forgets() {
    let mut o: Offsets<u32> = Offsets::default();
    assert!(o.is_empty());
    assert_eq!(o.get(&1), None);
    o.nudge(1, (0.5, 0.0));
    o.nudge(1, (0.5, -2.0));
    assert_eq!(o.get(&1), Some((1.0, -2.0)));
    o.nudge(2, (0.0, 0.5));
    assert_eq!(o.len(), 2);
    o.reset(&2);
    assert_eq!(o.get(&2), None);
    // Nudged back to where it was, a label is not moved.
    o.nudge(1, (-1.0, 2.0));
    assert_eq!(o.get(&1), None);
    assert!(o.is_empty());
}

#[test]
fn offsets_by_name_map_onto_other_keys() {
    let mut by_name: Offsets<String> = Offsets::default();
    by_name.nudge("CT1".into(), (1.0, 0.0));
    by_name.nudge("CT9".into(), (0.0, 1.0));
    let ids = ["CT0", "CT1"];
    let by_id: Offsets<u32> = by_name
        .iter()
        .filter_map(|(n, d)| Some((ids.iter().position(|i| i == n)? as u32, d)))
        .collect();
    assert_eq!(by_id.get(&1), Some((1.0, 0.0)));
    assert_eq!(by_id.len(), 1);
}

#[test]
fn a_missing_book_is_empty() {
    let d = dir();
    let path = d.path().join("run.view_labels.json");
    let book = OffsetBook::load(&path).unwrap();
    assert!(book.is_empty());
    assert!(book.layout("umap").is_empty());
    assert_eq!(book.path(), path);
}

#[test]
fn a_book_that_cannot_be_read_is_refused_and_left_alone() {
    let d = dir();
    let path = d.path().join("run.view_labels.json");
    for text in [
        "not json",
        r#"{"version":2,"layouts":{"umap":{"CT1":[1,2]}}}"#,
    ] {
        std::fs::write(&path, text).unwrap();
        let e = OffsetBook::load(&path).unwrap_err();
        assert_eq!(e.kind(), std::io::ErrorKind::InvalidData, "{text}");
        // Nothing was written over it.
        assert_eq!(std::fs::read_to_string(&path).unwrap(), text);
    }
}

#[test]
fn a_nudge_that_is_not_a_number_is_ignored() {
    let mut o: Offsets<u32> = Offsets::default();
    o.nudge(1, (1.0, 0.0));
    o.nudge(1, (f32::NAN, 0.0));
    o.nudge(1, (0.0, f32::INFINITY));
    assert_eq!(o.get(&1), Some((1.0, 0.0)));
    o.nudge(2, (f32::NEG_INFINITY, 1.0));
    assert_eq!(o.get(&2), None);
    let o: Offsets<u32> = [(1, (f32::NAN, 0.0)), (2, (0.5, 0.5))]
        .into_iter()
        .collect();
    assert_eq!(o.get(&1), None);
    assert_eq!(o.get(&2), Some((0.5, 0.5)));
}

#[test]
fn one_bad_nudge_does_not_cost_the_other_layouts() {
    let d = dir();
    let path = d.path().join("labels.json");
    let mut book = OffsetBook::load(&path).unwrap();
    let mut bad = Offsets::default();
    bad.nudge("CT2".to_string(), (f32::NAN, 0.0));
    bad.nudge("CT3".to_string(), (2.0, 0.0));
    let mut good = Offsets::default();
    good.nudge("CT1".to_string(), (1.0, 1.0));
    book.set_layout("bad", &bad);
    book.set_layout("good", &good);
    book.save().unwrap();
    let back = OffsetBook::load(&path).unwrap();
    assert_eq!(back.layout("good"), good);
    assert_eq!(back.layout("bad"), bad);
    assert_eq!(back.layout("bad").len(), 1);
}

#[test]
fn a_book_round_trips_per_layout() {
    let d = dir();
    let path = d.path().join("view").join("labels.json");
    let mut book = OffsetBook::load(&path).unwrap();
    let mut umap = Offsets::default();
    umap.nudge("CT1".to_string(), (1.5, -0.5));
    umap.nudge("L1-0".to_string(), (0.0, 2.0));
    let mut phate = Offsets::default();
    phate.nudge("CT1".to_string(), (-1.0, 0.0));
    book.set_layout("umap-r3", &umap);
    book.set_layout("phate-r3", &phate);
    book.save().unwrap();
    // Only the book, no temporary file left beside it.
    let names: Vec<_> = std::fs::read_dir(path.parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["labels.json"]);

    let back = OffsetBook::load(&path).unwrap();
    assert_eq!(back.layout("umap-r3"), umap);
    assert_eq!(back.layout("phate-r3"), phate);
    assert!(back.layout("umap-r4").is_empty());

    let json: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(
        json["layouts"]["umap-r3"]["CT1"],
        serde_json::json!([1.5, -0.5])
    );

    // A layout with nothing moved is dropped.
    let mut back = back;
    back.set_layout("phate-r3", &Offsets::default());
    back.save().unwrap();
    let again = OffsetBook::load(&path).unwrap();
    assert!(again.layout("phate-r3").is_empty());
    assert_eq!(again.layout("umap-r3"), umap);
}
