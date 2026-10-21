use super::*;
use std::os::unix::net::UnixListener;

fn private() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    std::fs::set_permissions(d.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    d
}

/// Claim a session of `base` (`tool-view`) for the tool its first part
/// names, asking every host.
fn claimed(dir: &Path, base: &str, name: Option<&str>) -> io::Result<(String, UnixListener)> {
    let tool = base.split('-').next().unwrap();
    claim(dir, &format!("{tool}-"), base, name, &[], false).map(|c| (c.name, c.listener))
}

/// Drop `l`, and wait until nothing listens on `path`: a child another
/// test forks at the same time holds a copy of it until it execs.
fn close(l: UnixListener, path: &Path) {
    drop(l);
    let start = std::time::Instant::now();
    while alive(path) {
        assert!(start.elapsed() < Duration::from_secs(5), "still listening");
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn private_dir_is_made_owner_only_and_others_refused() {
    let uid = rustix::process::getuid().as_raw();
    let top = private();
    let dir = top.path().join("sessions");
    private_dir(&dir, uid).unwrap();
    let mode = std::fs::metadata(&dir).unwrap().permissions().mode();
    assert_eq!(mode & 0o777, 0o700);
    // Again: still fine.
    private_dir(&dir, uid).unwrap();

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(private_dir(&dir, uid).is_err());

    let link = top.path().join("link");
    std::os::unix::fs::symlink(&dir, &link).unwrap();
    assert!(private_dir(&link, uid).is_err());

    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(private_dir(&dir, uid.wrapping_add(1)).is_err());
}

#[test]
fn names_are_checked() {
    for good in ["tool-view-1", "tool-run-a.b_c"] {
        check_name(good).unwrap();
    }
    for bad in ["", ".hidden", "a/b", "a b", &"x".repeat(65)] {
        assert!(check_name(bad).is_err(), "{bad:?}");
    }
}

#[test]
fn stale_sockets_are_removed_and_live_ones_kept() {
    let d = private();
    // Stale: bound, then nobody listening.
    let stale = socket(d.path(), "tool-run-1");
    drop(UnixListener::bind(&stale).unwrap());
    assert!(stale.exists());
    // Live, but answering nothing: kept, not listed.
    let quiet = socket(d.path(), "tool-run-2");
    let _quiet = UnixListener::bind(&quiet).unwrap();
    // Another tool's stale socket is not ours to touch.
    let other = socket(d.path(), "elsewhere-1");
    drop(UnixListener::bind(&other).unwrap());

    let listed = live(d.path(), "tool-").unwrap();
    assert!(listed.is_empty());
    // A child another test forked meanwhile holds a copy of the stale one
    // until it execs, which spares it as busy: asked again, it goes.
    for _ in 0..3 {
        if !stale.exists() {
            break;
        }
        assert!(live(d.path(), "tool-").unwrap().is_empty());
    }
    assert!(!stale.exists());
    assert!(quiet.exists());
    assert!(other.exists());
}

#[test]
fn claim_skips_taken_numbers() {
    let d = private();
    let (one, _l1) = claimed(d.path(), "tool-view", None).unwrap();
    assert_eq!(one, "tool-view-1");
    let (two, l2) = claimed(d.path(), "tool-view", None).unwrap();
    assert_eq!(two, "tool-view-2");
    // A stale socket frees its number.
    close(l2, &socket(d.path(), &two));
    let (again, _l2) = claimed(d.path(), "tool-view", None).unwrap();
    assert_eq!(again, "tool-view-2");
    // A name is taken once.
    let (named, _n) = claimed(d.path(), "tool-view", Some("x")).unwrap();
    assert_eq!(named, "tool-view-x");
    let taken = claimed(d.path(), "tool-view", Some("x")).unwrap_err();
    assert_eq!(taken.kind(), io::ErrorKind::AddrInUse);
    assert!(claimed(d.path(), "tool-view", Some("a/b")).is_err());
    // A name is the session's label, and a label the tool's once.
    assert_eq!(read_label(d.path(), "tool-view-x").as_deref(), Some("x"));
    assert_eq!(read_label(d.path(), "tool-view-1"), None);
    let again = claimed(d.path(), "tool-run", Some("x")).unwrap_err();
    assert_eq!(again.kind(), io::ErrorKind::AddrInUse);
    assert!(!socket(d.path(), "tool-run-x").exists());
    assert_eq!(read_label(d.path(), "tool-run-x"), None);
}

#[test]
fn labels_are_checked_unique_and_go_with_the_socket() {
    let d = private();
    let dir = d.path();
    let (one, l1) = claimed(dir, "tool-view", None).unwrap();
    let (two, _l2) = claimed(dir, "tool-run", None).unwrap();
    set_label(dir, "tool-", &one, "work").unwrap();
    assert_eq!(read_label(dir, &one).as_deref(), Some("work"));
    // Written whole: nothing else is left beside it.
    let mut files: Vec<_> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    assert_eq!(
        files,
        ["tool-run-1.sock", "tool-view-1.label", "tool-view-1.sock"]
    );
    // Relabelled.
    set_label(dir, "tool-", &one, "work2").unwrap();
    assert_eq!(read_label(dir, &one).as_deref(), Some("work2"));
    // Another's label, or another's id in full or after the prefix, is
    // taken; names are checked.
    for taken in ["work2", "tool-view-1", "view-1"] {
        let e = set_label(dir, "tool-", &two, taken).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AddrInUse, "{taken}");
        assert!(e.to_string().contains("taken by tool-view-1"), "{e}");
    }
    for bad in ["", "a b", ".x", "a/b"] {
        let e = set_label(dir, "tool-", &two, bad).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::InvalidInput, "{bad:?}");
    }
    assert_eq!(read_label(dir, &two), None);
    // Another tool's sessions do not count.
    let (_other, _lo) = claimed(dir, "elsewhere-view", Some("shared")).unwrap();
    set_label(dir, "tool-", &two, "shared").unwrap();
    // Once the session is gone, its label is free, and is removed with
    // its stale socket; it cannot be labelled again.
    close(l1, &socket(dir, &one));
    let e = set_label(dir, "tool-", &one, "late").unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::NotFound);
    assert_eq!(read_label(dir, &one).as_deref(), Some("work2"));
    set_label(dir, "tool-", &two, "work2").unwrap();
    live(dir, "tool-").unwrap();
    assert!(!label_file(dir, &one).exists());
    assert!(!socket(dir, &one).exists());
    // A number claimed again starts unlabelled.
    std::fs::write(label_file(dir, "tool-view-1"), "left").unwrap();
    let (again, _l1) = claimed(dir, "tool-view", None).unwrap();
    assert_eq!(again, "tool-view-1");
    assert_eq!(read_label(dir, &again), None);
}

#[test]
fn ids_claimed_later_skip_labels_already_given() {
    let d = private();
    let dir = d.path();
    let (run, _lr) = claimed(dir, "tool-run", None).unwrap();
    // The short form of the next view id, and the full id of the one
    // after: both numbers are skipped.
    set_label(dir, "tool-", &run, "view-1").unwrap();
    let (other, _lo) = claimed(dir, "tool-run", None).unwrap();
    set_label(dir, "tool-", &other, "tool-view-2").unwrap();
    let (view, _lv) = claimed(dir, "tool-view", None).unwrap();
    assert_eq!(view, "tool-view-3");
    // A named id another session has as its label is refused.
    set_label(dir, "tool-", &run, "view-x").unwrap();
    let e = claimed(dir, "tool-view", Some("x")).unwrap_err();
    assert_eq!(e.kind(), io::ErrorKind::AddrInUse);
    assert!(e.to_string().contains("taken by tool-run-1"), "{e}");
    assert!(!socket(dir, "tool-view-x").exists());
    // Another tool's labels do not count.
    let (_e, _le) = claimed(dir, "elsewhere-run", Some("view-y")).unwrap();
    claimed(dir, "tool-view", Some("y")).unwrap();
}

#[test]
fn listed_sessions_show_their_label_with_their_id() {
    let mut l = Listed {
        name: "tool-view-1".into(),
        label: None,
        pid: 1,
        attached: false,
        started: 0,
        command: "tool view".into(),
        status: None,
    };
    assert_eq!(
        (l.handle(), l.shown().as_str()),
        ("tool-view-1", "tool-view-1")
    );
    assert_eq!(l.labelled(), "");
    l.label = Some("work".into());
    assert_eq!(l.handle(), "work");
    assert_eq!(l.shown(), "work (tool-view-1)");
    assert_eq!(l.labelled(), " (labelled work)");
}

#[test]
fn launches_at_once_never_share_a_name() {
    let d = private();
    let path = d.path().to_path_buf();
    let claims: Vec<_> = (0..8)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || claimed(&path, "tool-run", None).unwrap())
        })
        .collect();
    let mut names: Vec<_> = claims.into_iter().map(|t| t.join().unwrap()).collect();
    names.sort_by(|a, b| natural(&a.0, &b.0));
    let names: Vec<_> = names.iter().map(|(n, _)| n.as_str()).collect();
    let want: Vec<_> = (1..=8).map(|n| format!("tool-run-{n}")).collect();
    assert_eq!(names, want);
}

#[test]
fn status_lines_are_one_line_and_go_with_a_stale_socket() {
    let d = private();
    write_status(d.path(), "tool-run-1", "two\nlines").unwrap();
    assert_eq!(
        read_status(d.path(), "tool-run-1").as_deref(),
        Some("two lines")
    );
    write_status(d.path(), "tool-run-1", &"x".repeat(500)).unwrap();
    assert_eq!(read_status(d.path(), "tool-run-1").unwrap().len(), 200);
    // Nothing else is left beside it.
    let files = std::fs::read_dir(d.path()).unwrap().count();
    assert_eq!(files, 1);
    drop(UnixListener::bind(socket(d.path(), "tool-run-1")).unwrap());
    live(d.path(), "tool-").unwrap();
    assert!(!status_file(d.path(), "tool-run-1").exists());
}

#[test]
fn natural_order_counts_numbers() {
    let mut v = vec!["x-10", "x-2", "w-3", "x-1"];
    v.sort_by(|a, b| natural(a, b));
    assert_eq!(v, ["w-3", "x-1", "x-2", "x-10"]);
}

#[test]
fn hosts_slow_to_answer_are_asked_all_at_once() {
    let d = private();
    // Listening, but never answering: each takes the whole of ASK.
    let silent: Vec<_> = (1..=3)
        .map(|n| UnixListener::bind(socket(d.path(), &format!("tool-view-{n}"))).unwrap())
        .collect();
    let start = std::time::Instant::now();
    assert!(live(d.path(), "tool-").unwrap().is_empty());
    assert!(start.elapsed() < 2 * ASK, "{:?}", start.elapsed());
    // Spared: a host busy enough not to answer still accepts.
    for n in 1..=3 {
        assert!(socket(d.path(), &format!("tool-view-{n}")).exists());
    }
    drop(silent);
}

#[test]
fn sessions_listed_just_now_are_taken_as_live_without_asking() {
    let d = private();
    let dir = d.path();
    // A socket nobody listens on, but listed live a moment ago, with a
    // label: its id and its label are skipped all the same.
    drop(UnixListener::bind(socket(dir, "tool-view-1")).unwrap());
    std::fs::write(label_file(dir, "tool-view-1"), "work").unwrap();
    let listed = Listed {
        name: "tool-view-1".into(),
        label: Some("work".into()),
        pid: 1,
        attached: false,
        started: 0,
        command: "tool view".into(),
        status: None,
    };
    let known = [listed];
    let c = claim(dir, "tool-", "tool-view", None, &known, false).unwrap();
    assert_eq!((c.name.as_str(), c.left), ("tool-view-2", None));
    let e = claim(dir, "tool-", "tool-run", Some("work"), &known, false).unwrap_err();
    assert!(e.to_string().contains("taken by tool-view-1"), "{e}");
    // Not listed, it is asked, found stale, and its number is free; what
    // it left as it ended is handed over to be said, not lost.
    write_leave(dir, "tool-view-1", "the run's summary").unwrap();
    let c = claim(dir, "tool-", "tool-view", None, &[], false).unwrap();
    assert_eq!(c.name, "tool-view-1");
    assert_eq!(c.left.as_deref(), Some("the run's summary"));
    assert!(!leave_file(dir, "tool-view-1").exists(), "taken");
}

#[test]
fn a_host_asked_is_answered_busy_closing_stale_or_failed_and_says_which() {
    use std::io::Write;
    let dir = private();
    let path = dir.path().join("tool-view-1.sock");
    // Nothing behind the socket: stale.
    let l = UnixListener::bind(&path).unwrap();
    drop(l);
    assert!(matches!(asked(&path), Asked::Stale));
    let _ = std::fs::remove_file(&path);
    // It takes the question and never answers: busy.
    let l = UnixListener::bind(&path).unwrap();
    let held = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        std::thread::sleep(ASK + Duration::from_millis(500));
        drop(s);
        l
    });
    assert!(matches!(asked(&path), Asked::Busy));
    let l = held.join().unwrap();
    // It takes the question and lets it go: it is ending.
    let closing = std::thread::spawn(move || {
        let (s, _) = l.accept().unwrap();
        drop(s);
        l
    });
    assert!(matches!(asked(&path), Asked::Closing));
    let l = closing.join().unwrap();
    // It answers what is not an answer: failed, and why.
    let garbled = std::thread::spawn(move || {
        let (mut s, _) = l.accept().unwrap();
        let mut head = [0u8; 5];
        let _ = std::io::Read::read_exact(&mut s, &mut head);
        s.write_all(&wire::frame(b'?', b"x")).unwrap();
        l
    });
    match asked(&path) {
        Asked::Failed(e) => assert_eq!(e.kind(), io::ErrorKind::InvalidData, "{e}"),
        _ => panic!("a garbled answer is a failure"),
    }
    drop(garbled.join().unwrap());
}
