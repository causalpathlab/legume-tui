//! The save dialog: a name that starts selected, names that never replace
//! a file unasked, and the fields of what is saved and how large.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::KeyModifiers;
use ratatui::Terminal;

fn press(f: &mut Field, code: KeyCode) -> bool {
    f.key(KeyEvent::new(code, KeyModifiers::NONE))
}

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

// pinto `src/tui/field.rs`.

#[test]
fn typing_replaces_the_selection() {
    let mut f = Field::new("view-001");
    press(&mut f, KeyCode::Char('a'));
    press(&mut f, KeyCode::Char('b'));
    assert_eq!(f.text, "ab");
}

#[test]
fn an_arrow_keeps_the_text_to_edit() {
    let mut f = Field::new("fig");
    press(&mut f, KeyCode::End);
    press(&mut f, KeyCode::Char('2'));
    assert_eq!(f.text, "fig2");
    press(&mut f, KeyCode::Backspace);
    assert_eq!(f.text, "fig");
}

#[test]
fn backspace_clears_the_selection_and_ctrl_a_reselects() {
    let mut f = Field::new("fig");
    press(&mut f, KeyCode::Backspace);
    assert_eq!(f.text, "");
    let mut f = Field::new("fig");
    press(&mut f, KeyCode::End);
    f.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL));
    press(&mut f, KeyCode::Char('x'));
    assert_eq!(f.text, "x");
}

#[test]
fn a_field_says_whether_its_text_changed() {
    let mut f = Field::new("fig");
    // Selection and cursor keys leave the text as it is.
    assert!(!press(&mut f, KeyCode::Left));
    assert!(!f.key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::CONTROL)));
    assert!(!press(&mut f, KeyCode::Up));
    // A character typed over a selection of one changes it, though the
    // length stays.
    let mut f = Field::new("a");
    assert!(press(&mut f, KeyCode::Char('b')));
    assert!(!Field::unselected(String::new()).key(key(KeyCode::Backspace)));
}

#[test]
fn a_selected_field_shows_reversed_else_a_cursor_at_its_end() {
    let sel = Style::default().add_modifier(ratatui::style::Modifier::REVERSED);
    let plain = Style::default();
    let f = Field::new("abcdef");
    let lines = f.lines(4, true, plain, sel);
    let text: Vec<&str> = lines.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(text, ["abcd", "ef"]);
    assert!(lines.iter().all(|s| s.style == sel));
    let f = Field::unselected("ab".into());
    let lines = f.lines(4, true, plain, sel);
    assert_eq!(lines[0].content, "ab▏");
    // Out of focus: neither.
    assert_eq!(
        Field::new("ab").lines(4, false, plain, sel)[0].content,
        "ab"
    );
    assert_eq!(Field::new("").lines(4, true, plain, sel).len(), 1);
}

// senna `src/view/tui/grid.rs` and pinto `src/view/tui/save.rs`.

#[test]
fn a_proposed_name_never_collides_with_a_saved_file() {
    let dir = tempfile::tempdir().unwrap();
    let stem = dir.path().join("run.view.layout.cells");
    let stem = stem.to_string_lossy();
    let pdf = [".pdf"];
    assert_eq!(free_stem(&stem, &pdf, Numbering::Suffix), stem);
    std::fs::write(format!("{stem}.pdf"), b"%PDF").unwrap();
    assert_eq!(
        free_stem(&stem, &pdf, Numbering::Suffix),
        format!("{stem}-2")
    );
    std::fs::write(format!("{stem}-2.pdf"), b"%PDF").unwrap();
    assert_eq!(
        free_stem(&stem, &pdf, Numbering::Suffix),
        format!("{stem}-3")
    );
}

#[test]
fn a_numbered_name_is_free_of_every_file_the_save_writes() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("tool-view");
    let prefix = prefix.to_string_lossy();
    let writes = [".png", ".pdf", "-structure.png"];
    let counter = Numbering::Counter { digits: 3 };
    assert_eq!(
        free_stem(&prefix, &writes, counter),
        format!("{prefix}-001")
    );
    // Any one companion file takes the name.
    std::fs::write(format!("{prefix}-001-structure.png"), b"").unwrap();
    assert_eq!(
        free_stem(&prefix, &writes, counter),
        format!("{prefix}-002")
    );
    assert_eq!(
        taken(&format!("{prefix}-001"), &writes),
        [std::path::PathBuf::from(format!(
            "{prefix}-001-structure.png"
        ))]
    );
}

#[test]
fn a_typed_extension_is_dropped_and_home_expanded() {
    let exts = ["png", "pdf", "txt"];
    assert_eq!(stem_of(" fig.pdf ", &exts), "fig");
    assert_eq!(stem_of("fig.v2", &exts), "fig.v2");
    assert_eq!(stem_of("t.tsv", &["tsv", "txt"]), "t");
    let home = std::env::var("HOME").unwrap();
    assert_eq!(stem_of("~/a/fig.png", &exts), format!("{home}/a/fig"));
    // Home alone is a folder: it keeps its slash, so it asks for a name.
    let folder = format!("{}/", home.trim_end_matches('/'));
    assert_eq!(stem_of("~", &exts), folder);
    assert_eq!(stem_of("~/", &exts), folder);
}

#[test]
fn a_name_that_is_a_folder_asks_for_a_file_name() {
    let (_dir, v) = viewer(1);
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    d.key(key(KeyCode::Backspace), &v);
    for c in v.dir.to_string_lossy().chars() {
        d.key(key(KeyCode::Char(c)), &v);
    }
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert_eq!(d.status(), Some("type a file name"));
}

/// A viewer with one view, or with several to put in a grid.
struct Viewer {
    dir: std::path::PathBuf,
    views: usize,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scope {
    View,
    Grid,
}

impl SaveTarget for Viewer {
    type Scope = Scope;
    fn scopes(&self) -> Vec<Scope> {
        if self.views > 1 {
            vec![Scope::View, Scope::Grid]
        } else {
            vec![Scope::View]
        }
    }
    fn scope_name(&self, s: Scope) -> String {
        match s {
            Scope::View => "this view".into(),
            Scope::Grid => format!("all {} runs, one grid", self.views),
        }
    }
    fn default_stem(&self, s: Scope) -> String {
        let name = match s {
            Scope::View => "run.view",
            Scope::Grid => "run.view.grid",
        };
        self.dir.join(name).to_string_lossy().into_owned()
    }
    fn exts(&self, _: Scope) -> Vec<String> {
        vec!["pdf".into()]
    }
    fn aspect(&self, s: Scope) -> f32 {
        match s {
            Scope::View => 4.0 / 3.0,
            Scope::Grid => 2.0,
        }
    }
}

fn viewer(views: usize) -> (tempfile::TempDir, Viewer) {
    let dir = tempfile::tempdir().unwrap();
    let v = Viewer {
        dir: dir.path().to_path_buf(),
        views,
    };
    (dir, v)
}

fn saved<S>(k: SaveKey<S>) -> SaveRequest<S> {
    match k {
        SaveKey::Save(r) => r,
        _ => panic!("not saved"),
    }
}

#[test]
fn enter_saves_the_name_offered_at_seven_inches_and_300_dpi() {
    let (_dir, v) = viewer(1);
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    assert_eq!(d.name(), v.default_stem(Scope::View));
    let r = saved(d.key(key(KeyCode::Enter), &v));
    assert_eq!(r.scope, Scope::View);
    assert_eq!((r.width_in, r.dpi), (7.0, 300));
    assert_eq!(r.px, (2100, 1575));
    assert_eq!(r.path, v.dir.join("run.view.pdf"));
    assert_eq!(r.files, [v.dir.join("run.view.pdf")]);
}

#[test]
fn arrows_choose_the_field_and_change_it() {
    let (_dir, v) = viewer(2);
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    // What: a new scope brings its own name, while none was typed.
    assert!(matches!(d.key(key(KeyCode::Right), &v), SaveKey::Open));
    assert_eq!(d.scope(), Scope::Grid);
    assert_eq!(d.name(), v.default_stem(Scope::Grid));
    // Width and dpi wrap around their lists.
    d.key(key(KeyCode::Down), &v);
    d.key(key(KeyCode::Right), &v);
    d.key(key(KeyCode::Tab), &v);
    d.key(key(KeyCode::Left), &v);
    d.key(key(KeyCode::Left), &v);
    assert_eq!((d.width_in(), d.dpi()), (10.0, 1200));
    assert_eq!(d.px(&v), (12000, 6000));
    // A typed name stays when the scope changes.
    d.key(key(KeyCode::Char('f')), &v);
    assert_eq!(d.name(), "f");
    d.key(key(KeyCode::BackTab), &v);
    d.key(key(KeyCode::BackTab), &v);
    d.key(key(KeyCode::BackTab), &v);
    d.key(key(KeyCode::Left), &v);
    assert_eq!((d.scope(), d.name()), (Scope::View, "f"));
    assert!(matches!(d.key(key(KeyCode::Esc), &v), SaveKey::Cancelled));
}

#[test]
fn one_scope_does_not_cycle() {
    let (_dir, v) = viewer(1);
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    d.key(key(KeyCode::Right), &v);
    assert_eq!(d.scope(), Scope::View);
}

#[test]
fn replacing_a_file_takes_a_second_enter() {
    let (_dir, v) = viewer(1);
    let taken = v.dir.join("fig.pdf");
    std::fs::write(&taken, b"%PDF").unwrap();
    let type_in = |d: &mut SaveDialog<Scope>, s: &str| {
        d.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), &v);
        for c in s.chars() {
            d.key(key(KeyCode::Char(c)), &v);
        }
    };
    let fig = v.dir.join("fig").to_string_lossy().into_owned();

    // A new name saves at once.
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    type_in(&mut d, &v.dir.join("new").to_string_lossy());
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Save(_)));

    // An existing one asks first, then saves on the second enter.
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    type_in(&mut d, &fig);
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert!(d.status().unwrap().contains("enter again replaces it"));
    assert_eq!(saved(d.key(key(KeyCode::Enter), &v)).path, taken);

    // Moving between fields, or changing the size, keeps the answer.
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    type_in(&mut d, &fig);
    d.key(key(KeyCode::Enter), &v);
    d.key(key(KeyCode::Up), &v);
    d.key(key(KeyCode::Right), &v);
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Save(_)));

    // Any change to the name asks again, even one undone.
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    type_in(&mut d, &fig);
    d.key(key(KeyCode::Enter), &v);
    d.key(key(KeyCode::Char('2')), &v);
    d.key(key(KeyCode::Backspace), &v);
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Save(_)));
}

#[test]
fn a_name_that_cannot_be_written_keeps_the_dialog_open() {
    let (_dir, v) = viewer(1);
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    d.key(key(KeyCode::Backspace), &v);
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert_eq!(d.status(), Some("type a file name"));
    for c in v.dir.join("nowhere/fig").to_string_lossy().chars() {
        d.key(key(KeyCode::Char(c)), &v);
    }
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert!(d.status().unwrap().starts_with("no folder"));
    // Too large: the widest page at the finest dpi.
    let mut d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    d.key(key(KeyCode::Down), &v);
    d.key(key(KeyCode::Right), &v);
    d.key(key(KeyCode::Right), &v);
    d.key(key(KeyCode::Down), &v);
    d.key(key(KeyCode::Left), &v);
    d.key(key(KeyCode::Left), &v);
    assert_eq!((d.width_in(), d.dpi()), (14.0, 1200));
    assert!(d.px(&v).0 * d.px(&v).1 > MAX_PIXELS);
    assert!(matches!(d.key(key(KeyCode::Enter), &v), SaveKey::Open));
    assert!(d.status().unwrap().starts_with("too large"));
    // What went wrong in the save itself is said in the dialog.
    d.say("not saved: disk full");
    assert_eq!(d.status(), Some("not saved: disk full"));
}

#[test]
fn the_offered_name_is_numbered_past_a_saved_file() {
    let (_dir, v) = viewer(1);
    std::fs::write(v.dir.join("run.view.pdf"), b"%PDF").unwrap();
    let d = SaveDialog::new(&v, Scope::View, Numbering::Suffix);
    assert_eq!(d.name(), format!("{}-2", v.default_stem(Scope::View)));
}

#[test]
fn the_dialog_draws_its_fields_size_and_keys() {
    let (_dir, v) = viewer(2);
    let d = SaveDialog::new(&v, Scope::Grid, Numbering::Suffix);
    let mut t = Terminal::new(TestBackend::new(90, 16)).unwrap();
    t.draw(|f| d.render(f, f.area(), &v, &Look::default()))
        .unwrap();
    let buf = t.backend().buffer();
    let screen: Vec<String> = (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect();
    let all = screen.join("\n");
    for want in [
        "Save as PDF",
        "‹ all 2 runs, one grid ›",
        "‹ 7 in ›",
        "‹ 300 ›",
        "2100 × 1050 px",
        "enter save",
    ] {
        assert!(all.contains(want), "{want} not in\n{all}");
    }
}

/// A map whose save writes a structure picture and table beside it, as
/// pinto's does.
struct Map {
    dir: std::path::PathBuf,
}

impl SaveTarget for Map {
    type Scope = ();
    fn scopes(&self) -> Vec<()> {
        vec![()]
    }
    fn scope_name(&self, (): ()) -> String {
        "map".into()
    }
    fn default_stem(&self, (): ()) -> String {
        self.dir.join("tool-view").to_string_lossy().into_owned()
    }
    fn exts(&self, (): ()) -> Vec<String> {
        vec!["png".into(), "pdf".into()]
    }
    fn suffixes(&self, (): ()) -> Vec<String> {
        [".png", ".pdf", "-structure.png", "-structure.txt"]
            .map(String::from)
            .to_vec()
    }
    fn aspect(&self, (): ()) -> f32 {
        1.0
    }
}

#[test]
fn a_companion_file_takes_the_name_in_the_dialog_too() {
    let dir = tempfile::tempdir().unwrap();
    let m = Map {
        dir: dir.path().to_path_buf(),
    };
    let stem = m.default_stem(());
    let counter = Numbering::Counter { digits: 3 };
    std::fs::write(format!("{stem}-001-structure.png"), b"").unwrap();
    // Offered past the companion.
    let mut d = SaveDialog::new(&m, (), counter);
    assert_eq!(d.name(), format!("{stem}-002"));
    // The save writes, and names, every companion.
    let r = saved(d.key(key(KeyCode::Enter), &m));
    let want: Vec<std::path::PathBuf> = [".png", ".pdf", "-structure.png", "-structure.txt"]
        .iter()
        .map(|s| format!("{stem}-002{s}").into())
        .collect();
    assert_eq!(r.files, want);
    assert_eq!(r.path, want[0]);
    // A typed name whose companion alone exists asks before replacing it.
    let mut d = SaveDialog::new(&m, (), counter);
    d.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL), &m);
    for c in format!("{stem}-001.png").chars() {
        d.key(key(KeyCode::Char(c)), &m);
    }
    assert!(matches!(d.key(key(KeyCode::Enter), &m), SaveKey::Open));
    assert!(d.status().unwrap().contains("-001-structure.png"));
    assert_eq!(
        saved(d.key(key(KeyCode::Enter), &m)).stem,
        format!("{stem}-001")
    );
}
