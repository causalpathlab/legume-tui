//! The file browser: folders then wanted files, listed on a worker,
//! narrowed by typing, typed paths followed, refusals, marks, a best file,
//! and the plain [`Simple`] browser.

use super::*;

fn key(code: KeyCode) -> KeyEvent {
    KeyEvent::new(code, KeyModifiers::NONE)
}

fn ctrl(c: char) -> KeyEvent {
    KeyEvent::new(KeyCode::Char(c), KeyModifiers::CONTROL)
}

/// Type `text` into the browser a key at a time.
fn type_in<W: Wanted>(b: &mut Browser<W>, text: &str) {
    for c in text.chars() {
        assert_eq!(b.key(key(KeyCode::Char(c))), Outcome::Moved, "{c}");
    }
}

fn names<W: Wanted>(b: &Browser<W>) -> Vec<String> {
    b.shown().iter().map(|e| e.name().to_string()).collect()
}

/// The files a key took; none when it took none.
fn taken(o: Outcome) -> Vec<PathBuf> {
    match o {
        Outcome::Chosen(c) => c.files(),
        _ => Vec::new(),
    }
}

fn text(lines: &[Line]) -> Vec<String> {
    lines.iter().map(ToString::to_string).collect()
}

fn write(dir: &Path, name: &str) {
    std::fs::write(dir.join(name), "").unwrap();
}

/// Text files, several at once when `many`; `.zarr` stores too.
#[derive(Clone)]
struct Txt {
    many: bool,
}

impl Wanted for Txt {
    type About = String;

    fn header(&self) -> Header {
        Header::new("Text", "text files")
    }

    fn file(&self, path: &Path, name: &str) -> Option<String> {
        name.ends_with(".txt")
            .then(|| size(path).unwrap_or_default())
    }

    fn store(&self, _path: &Path, _name: &str) -> Option<String> {
        Some(String::new())
    }

    fn describe<'a>(&self, about: &'a String) -> Cow<'a, str> {
        Cow::Borrowed(about)
    }

    fn many(&self) -> bool {
        self.many
    }
}

/// Text files, refusing any named `no.txt`, several at once when `many`.
#[derive(Clone)]
struct Picky {
    many: bool,
}

impl Wanted for Picky {
    type About = ();

    fn header(&self) -> Header {
        Header::new("Text", "text files")
    }

    fn file(&self, _path: &Path, name: &str) -> Option<()> {
        name.ends_with(".txt").then_some(())
    }

    fn describe<'a>(&self, (): &'a ()) -> Cow<'a, str> {
        "".into()
    }

    fn refuse(&self, name: &str, (): &()) -> Option<Cow<'static, str>> {
        (name == "no.txt").then(|| format!("{name} will not do").into())
    }

    fn many(&self) -> bool {
        self.many
    }
}

#[test]
fn folders_come_before_wanted_files_and_stores_are_files() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path();
    write(d, "b.txt");
    write(d, "a.txt");
    write(d, "skip.csv");
    write(d, ".hidden.txt");
    std::fs::create_dir(d.join("sub")).unwrap();
    std::fs::create_dir(d.join("s.zarr")).unwrap();
    std::fs::create_dir(d.join(".hidden")).unwrap();
    let mut b = Browser::open(d, Txt { many: false }, None);
    assert_eq!(names(&b), ["..", "sub", "a.txt", "b.txt", "s.zarr"]);
    // Hidden entries show only when asked for with a leading `.`, their
    // files read then.
    type_in(&mut b, ".h");
    assert_eq!(names(&b), ["..", ".hidden", ".hidden.txt"]);
    // Every file on request, the unwanted ones refused.
    b.key(key(KeyCode::Esc));
    b.toggle_all();
    assert_eq!(
        names(&b),
        ["..", "sub", "a.txt", "b.txt", "s.zarr", "skip.csv"]
    );
    assert!(b.select("skip.csv"));
    assert_eq!(b.key(key(KeyCode::Enter)), Outcome::Moved);
    assert!(b
        .refused
        .as_deref()
        .unwrap()
        .contains("not among the text files"));
}

#[test]
fn enter_chooses_one_file_and_opens_folders() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("sub")).unwrap();
    write(&dir.path().join("sub"), "a.txt");
    let mut b = Browser::open(dir.path(), Txt { many: false }, None);
    assert_eq!(b.current().map(Entry::name), Some("sub"));
    assert_eq!(b.key(key(KeyCode::Enter)), Outcome::Moved);
    assert_eq!(b.current().map(Entry::name), Some("a.txt"));
    // Space and chords are the caller's in a single-choice browser.
    for k in [key(KeyCode::Char(' ')), ctrl('r'), key(KeyCode::Tab)] {
        assert_eq!(b.key(k), Outcome::Ignored, "{k:?}");
    }
    // → only opens folders; Enter takes a file.
    assert_eq!(b.key(key(KeyCode::Right)), Outcome::Ignored);
    assert_eq!(
        taken(b.key(key(KeyCode::Enter))),
        [dir.path().join("sub/a.txt")]
    );
    // Up lands on the folder left; Esc with nothing typed cancels.
    b.go_up();
    assert_eq!(b.current().map(Entry::name), Some("sub"));
    assert_eq!(b.key(key(KeyCode::Esc)), Outcome::Cancelled);
}

#[test]
fn marked_files_in_several_folders_are_taken_together() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "a.txt");
    std::fs::create_dir(dir.path().join("more")).unwrap();
    write(&dir.path().join("more"), "b.txt");
    let mut b = Browser::open(dir.path(), Txt { many: true }, None);
    assert_eq!(names(&b), ["..", "more", "a.txt"]);
    b.row = 2;
    b.key(key(KeyCode::Char(' ')));
    b.row = 1;
    // Space on a folder is the caller's; Enter still opens it.
    assert_eq!(b.key(key(KeyCode::Char(' '))), Outcome::Ignored);
    assert_eq!(b.key(key(KeyCode::Enter)), Outcome::Moved);
    assert_eq!(b.dir, dir.path().join("more"));
    b.row = 1;
    b.key(key(KeyCode::Char(' ')));
    let lines = text(&b.lines(30, 80, &Look::terminal()));
    assert!(lines.iter().any(|l| l.contains("● b.txt")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("2 marked")), "{lines:?}");
    b.row = 1;
    assert_eq!(
        taken(b.key(key(KeyCode::Enter))),
        [dir.path().join("a.txt"), dir.path().join("more/b.txt")]
    );
    assert!(b.marks.is_empty());
}

#[test]
fn marks_keep_their_order_and_toggle() {
    let mut m = Marks::default();
    assert!(m.toggle("b".into()));
    assert!(m.toggle("a".into()));
    assert!(!m.insert("b".into()));
    assert_eq!(
        m.iter().collect::<Vec<_>>(),
        [Path::new("b"), Path::new("a")]
    );
    assert!(!m.toggle("b".into()));
    assert!(!m.contains(Path::new("b")));
    assert_eq!(m.len(), 1);
    assert_eq!(m.take(), [PathBuf::from("a")]);
    assert!(m.is_empty());
}

#[test]
fn a_refused_file_is_not_taken_nor_marked_and_the_popup_says_why() {
    let dir = tempfile::tempdir().unwrap();
    write(dir.path(), "no.txt");
    write(dir.path(), "yes.txt");
    let look = Look::terminal();
    let mut b = Browser::open(dir.path(), Picky { many: false }, Some("no.txt"));
    assert_eq!(b.key(key(KeyCode::Enter)), Outcome::Moved);
    let lines = b.lines(20, 80, &look);
    let said = lines
        .iter()
        .find(|l| l.to_string().contains("no.txt will not do"))
        .expect("the refusal");
    assert_eq!(said.spans[0].style, look.accent);
    // The refusal stays through keys that do nothing.
    assert_eq!(b.key(key(KeyCode::Tab)), Outcome::Ignored);
    assert!(b.refused.is_some());
    b.key(key(KeyCode::Down));
    assert!(b.refused.is_none());
    assert_eq!(
        taken(b.key(key(KeyCode::Enter))),
        [dir.path().join("yes.txt")]
    );

    // Taken several at once, the refused one is never marked.
    let mut b = Browser::open(dir.path(), Picky { many: true }, Some("no.txt"));
    b.key(key(KeyCode::Char(' ')));
    assert!(b.marks.is_empty());
    assert_eq!(b.refused.as_deref(), Some("no.txt will not do"));
    b.key(ctrl('a'));
    assert_eq!(
        b.marks.iter().collect::<Vec<_>>(),
        [dir.path().join("yes.txt")]
    );
    assert!(b.refused.is_some(), "the refused one is named");
}

#[test]
fn the_key_hints_fit_however_short_the_popup_and_only_rows_near_the_cursor_are_drawn() {
    let dir = tempfile::tempdir().unwrap();
    for i in 0..40 {
        write(dir.path(), &format!("f{i:02}.txt"));
    }
    let mut b = Browser::open(dir.path(), Txt { many: true }, None);
    let lines = b.lines(20, 60, &Look::terminal());
    assert!(lines.len() <= 20, "{}", lines.len());
    assert!(lines.last().unwrap().to_string().contains("esc cancel"));
    assert!(lines.iter().all(|l| l.width() <= 60));
    b.row = 30;
    let rows = text(&b.rows(5, 40, &Look::terminal()));
    assert_eq!(rows.len(), 5);
    assert!(rows[2].starts_with("▸   f29.txt"), "{rows:?}");
}

#[test]
fn typing_narrows_any_case_and_esc_restores_the_list() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("Ann")).unwrap();
    write(dir.path(), "a.txt");
    write(dir.path(), "b.txt");
    let mut b = Browser::open(dir.path(), Txt { many: false }, None);
    type_in(&mut b, "aN");
    assert_eq!(names(&b), ["..", "Ann"]);
    let said = text(&b.lines(20, 60, &Look::terminal()));
    assert!(
        said.iter().any(|l| l.contains("names with “aN”")),
        "{said:?}"
    );
    // Back to the whole list, the cursor still on what was found.
    assert_eq!(b.key(key(KeyCode::Esc)), Outcome::Moved);
    assert_eq!(b.current().map(Entry::name), Some("Ann"));
    type_in(&mut b, "zz");
    assert_eq!(names(&b), [".."]);
    assert_eq!(b.status().as_deref(), Some(NOTHING_MATCHES));
    // Backspace pops a letter; with nothing typed it goes up.
    b.key(key(KeyCode::Backspace));
    b.key(key(KeyCode::Backspace));
    assert!(b.filter.is_empty());
    assert_eq!(names(&b), ["..", "Ann", "a.txt", "b.txt"]);
    b.key(key(KeyCode::Backspace));
    assert_eq!(Some(b.dir.as_path()), dir.path().parent());
}

#[test]
fn a_typed_path_is_followed_a_folder_it_reaches_entered_and_one_not_there_stays() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().canonicalize().unwrap();
    for sub in ["data", "data2", "other"] {
        std::fs::create_dir(d.join(sub)).unwrap();
    }
    write(&d.join("data2"), "a.txt");
    let mut b = Browser::open(d.join("other"), Txt { many: false }, None);
    // The whole path typed as it is: each folder it passes is listed.
    let root = format!("{}/", d.display());
    type_in(&mut b, &root);
    assert_eq!(b.dir, d);
    assert_eq!(b.filter, root);
    // `data` is a folder, but `data2` begins with it too: wait.
    type_in(&mut b, "data");
    assert_eq!(b.dir, d);
    assert_eq!(names(&b), ["..", "data", "data2"]);
    // `data2` is the only one: its folder is listed at once, and what was
    // typed stays as typed.
    type_in(&mut b, "2");
    assert_eq!(b.dir, d.join("data2"));
    assert_eq!(b.filter, format!("{root}data2"));
    assert_eq!(names(&b), ["..", "a.txt"]);
    // The `/` typed next is just that: the same folder.
    type_in(&mut b, "/");
    assert_eq!(b.filter, format!("{root}data2/"));
    assert_eq!(b.dir, d.join("data2"));
    // Backspace back past the folder lists its parent again.
    b.key(key(KeyCode::Backspace));
    b.key(key(KeyCode::Backspace));
    assert_eq!(b.dir, d);
    assert_eq!(names(&b), ["..", "data", "data2"]);
    // Enter takes the file the path ends on.
    type_in(&mut b, "2/a");
    assert_eq!(taken(b.key(key(KeyCode::Enter))), [d.join("data2/a.txt")]);

    // Letters without a leading slash narrow names; a name then `/` goes
    // into the folder under the cursor, `../` back up.
    let mut b = Browser::open(&d, Txt { many: false }, None);
    type_in(&mut b, "ata2");
    assert_eq!(names(&b), ["..", "data2"]);
    type_in(&mut b, "/");
    assert_eq!(b.dir, d.join("data2"));
    assert!(b.filter.is_empty());
    type_in(&mut b, "../");
    assert_eq!(b.dir, d);

    // A typed path that is not there stays where it is.
    let mut b = Browser::open(&d, Txt { many: false }, None);
    type_in(&mut b, &format!("{}/nowhere/x", d.display()));
    assert_eq!(b.dir, d);
    assert!(!b.completed());
    assert_eq!(names(&b), [".."]);
}

#[test]
fn a_typed_path_through_dot_dot_lists_the_folder_by_its_one_name() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().canonicalize().unwrap();
    for sub in ["data", "data2"] {
        std::fs::create_dir(d.join(sub)).unwrap();
    }
    let mut b = Browser::open(&d, Txt { many: false }, None);
    type_in(&mut b, &format!("{}/data2/../", d.display()));
    assert_eq!(b.dir, d);
    assert_eq!(names(&b), ["..", "data", "data2"]);
    // Completed through the `..`, as typed.
    type_in(&mut b, "data2");
    assert_eq!(b.dir, d.join("data2"));
    assert!(b.completed());
    // Up from there is up by name.
    b.key(key(KeyCode::Esc));
    b.go_up();
    assert_eq!(b.dir, d);
    assert_eq!(b.current().map(Entry::name), Some("data2"));
}

#[test]
fn a_typed_path_into_a_folder_not_listed_yet_is_followed_once_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().canonicalize().unwrap();
    for sub in ["data", "data2"] {
        std::fs::create_dir(d.join(sub)).unwrap();
    }
    write(&d.join("data2"), "a.txt");
    // No wait for any listing: each folder is followed as its names come.
    let mut b = Browser::open_within(&d, Slow, None, Duration::ZERO);
    type_in(&mut b, &format!("{}/data2", d.display()));
    let until = Instant::now() + Duration::from_secs(30);
    while (b.listing().is_some() || b.unsure) && Instant::now() < until {
        b.poll();
        std::thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(b.dir, d.join("data2"));
    assert_eq!(names(&b), ["..", "a.txt"]);
}

#[test]
fn a_relative_start_is_made_absolute_so_up_keeps_working() {
    let mut b = Browser::open(".", Simple::keeping(Header::new("", ""), |_| false), None);
    assert!(b.dir.is_absolute(), "{}", b.dir.display());
    let start = b.dir.clone();
    assert_eq!(b.key(key(KeyCode::Left)), Outcome::Moved);
    assert_eq!(Some(b.dir.as_path()), start.parent());
}

/// Text files, each described slowly: a folder that takes a while to list.
#[derive(Clone)]
struct Slow;

impl Wanted for Slow {
    type About = ();

    fn header(&self) -> Header {
        Header::new("Slow", "text files").star("the one to start on")
    }

    fn file(&self, _path: &Path, name: &str) -> Option<()> {
        std::thread::sleep(Duration::from_millis(80));
        name.ends_with(".txt").then_some(())
    }

    fn describe<'a>(&self, (): &'a ()) -> Cow<'a, str> {
        Cow::Borrowed("")
    }

    fn best(&self, _dir: &Path, files: &[(&str, &())]) -> Option<String> {
        files.iter().map(|(n, _)| n.to_string()).max()
    }
}

#[test]
fn a_slow_folder_is_listed_on_the_side_and_shown_as_it_comes() {
    let dir = tempfile::tempdir().unwrap();
    for f in ["a.txt", "b.txt", "c.txt"] {
        write(dir.path(), f);
    }
    let mut b = Browser::open_within(dir.path(), Slow, Some("b.txt"), Duration::ZERO);
    // Too slow to wait for: the browser is up at once, listing.
    assert!(b.listing().is_some());
    assert!(!names(&b).contains(&"c.txt".to_string()));
    let lines = text(&b.lines(30, 80, &Look::terminal()));
    assert!(
        lines.iter().any(|l| l.contains("listing this folder")),
        "{lines:?}"
    );
    while b.listing().is_some() {
        b.poll();
        std::thread::sleep(Duration::from_millis(10));
    }
    assert_eq!(names(&b), ["..", "a.txt", "b.txt", "c.txt"]);
    // The cursor goes where it was asked, once the folder is there; the
    // best file is starred.
    assert_eq!(b.current().map(Entry::name), Some("b.txt"));
    assert_eq!(b.best(), Some("c.txt"));
    let lines = text(&b.lines(30, 80, &Look::terminal()));
    assert!(lines.iter().any(|l| l.contains("★ c.txt")), "{lines:?}");
    assert!(lines.iter().any(|l| l.contains("★ the one to start on")));
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

/// faba's: kept by name, annotations noted, folders noted as such.
fn simple(root: &Path) -> Browser<Simple> {
    let note = |p: &Path, dir: bool| -> Option<Cow<'static, str>> {
        if dir {
            return Some("a folder".into());
        }
        p.extension()
            .is_some_and(|e| e == "gff3")
            .then_some(Cow::Borrowed("annotation"))
    };
    Browser::open(
        root,
        Simple::new(Header::new("Annotations", "annotations"), kept, note),
        None,
    )
}

#[test]
fn a_simple_browser_keeps_files_by_name_and_notes_them() {
    let tmp = tree();
    let root = normalize(tmp.path());
    let mut b = simple(&root);
    // Hidden files are left out, a zarr store is a file, others unless kept.
    assert_eq!(names(&b), ["..", "ann", "b.gff3", "m.zarr"]);
    let notes: Vec<String> = b
        .shown()
        .iter()
        .map(|e| match e {
            Entry::Dir(_, Some(a)) | Entry::File(_, a) => b.want.describe(a).into_owned(),
            _ => String::new(),
        })
        .collect();
    assert_eq!(notes, ["", "a folder", "annotation", ""]);
    let rows = text(&b.rows(10, 40, &Look::terminal()));
    assert!(
        rows[1].starts_with("▸ ann/") && rows[1].ends_with("  a folder"),
        "{rows:?}"
    );
    assert!(rows.iter().all(|t| t.chars().count() <= 40), "{rows:?}");
    let narrow = text(&b.rows(10, 10, &Look::terminal()));
    assert!(narrow[2].contains('…'), "{narrow:?}");

    // Into `ann`, where only the annotation is listed, and pick it.
    b.key(key(KeyCode::Enter));
    assert_eq!(b.dir, root.join("ann"));
    assert_eq!(names(&b), ["..", "genes.gtf.gz"]);
    assert_eq!(
        taken(b.key(key(KeyCode::Enter))),
        [root.join("ann/genes.gtf.gz")]
    );
    assert_eq!(b.current_path(), Some(root.join("ann/genes.gtf.gz")));
}

#[test]
fn a_click_on_a_line_finds_the_row_drawn_there() {
    let tmp = tree();
    let root = normalize(tmp.path());
    let b = simple(&root);
    let (h, w) = (20, 40);
    let lines = text(&b.lines(h, w, &Look::terminal()));
    let shown = names(&b);
    for (k, l) in lines.iter().enumerate() {
        match b.row_at(h, w, k) {
            // The row found is the one drawn on that line.
            Some(i) => assert!(l.contains(shown[i].as_str()), "{k}: {l:?}"),
            None => assert!(
                !shown
                    .iter()
                    .any(|n| l.starts_with(&format!("▸ {n}")) || l.starts_with(&format!("  {n}"))),
                "{k}: {l:?}"
            ),
        }
    }
    let found: Vec<usize> = (0..h).filter_map(|k| b.row_at(h, w, k)).collect();
    assert_eq!(found, (0..shown.len()).collect::<Vec<_>>());
    assert_eq!(b.row_at(h, w, h + 5), None);
    assert!(!b.has_mark_column());
}

#[test]
fn a_simple_browser_marks_when_asked() {
    let tmp = tree();
    let root = normalize(tmp.path());
    let want = Simple::keeping(Header::new("", "annotations"), kept).marking();
    let mut b = Browser::open(&root, want, Some("b.gff3"));
    b.key(key(KeyCode::Char(' ')));
    b.key(key(KeyCode::Char(' ')));
    assert_eq!(
        b.marks.iter().collect::<Vec<_>>(),
        [root.join("b.gff3"), root.join("m.zarr")]
    );
}

#[test]
fn a_typed_path_is_absolute_from_home() {
    if let Some(home) = home() {
        assert_eq!(typed_path("~/a/../b"), normalize(&home.join("b")));
        assert_eq!(tilde(&home.join("x")), "~/x");
    }
    assert!(typed_path("rel").is_absolute());
}

#[test]
fn a_size_reads_in_its_unit_and_a_missing_file_has_none() {
    let tmp = tempfile::tempdir().unwrap();
    let small = tmp.path().join("small");
    let big = tmp.path().join("big");
    std::fs::write(&small, [0u8; 12]).unwrap();
    std::fs::write(&big, vec![0u8; 12_300]).unwrap();
    assert_eq!(size(&small).as_deref(), Some("12 B"));
    assert_eq!(size(&big).as_deref(), Some("12.3 kB"));
    assert_eq!(size(&tmp.path().join("none")), None);
}

#[test]
fn a_path_under_the_working_folder_is_shown_relative() {
    let cwd = std::env::current_dir().unwrap();
    assert_eq!(shown(&cwd.join("a/b.zarr")), "a/b.zarr");
    // The folder itself and a path elsewhere stay as given.
    assert_eq!(relative(&cwd), cwd);
    let elsewhere = Path::new("/somewhere/else");
    assert_eq!(relative(elsewhere), elsewhere);
}

#[test]
fn a_find_list_narrows_and_jumps() {
    let mut l: FindList<String> = FindList::default();
    l.set(vec!["..".into(), "Alpha".into(), "beta".into()], 0);
    for c in "ET".chars() {
        assert!(l.key(&key(KeyCode::Char(c))));
    }
    assert_eq!(l.shown().collect::<Vec<_>>(), ["beta"]);
    assert!(l.key(&key(KeyCode::Esc)));
    assert_eq!(l.highlighted().map(String::as_str), Some("beta"));
    let root = Path::new("/a/b");
    assert_eq!(
        l.find.jump(&key(KeyCode::Char('/')), root, None),
        Some(PathBuf::from("/"))
    );
}
