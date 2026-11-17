//! The screen loop's pieces that need no terminal: how events reach a
//! view, the session keys on the title row, the chrome, and a stop told
//! apart from other errors.

use super::*;
use ratatui::backend::TestBackend;
use ratatui::Terminal;

/// A view that counts what reaches it and draws a title over its top row.
#[derive(Default)]
struct Counter {
    keys: Vec<KeyCode>,
    interrupts: usize,
    quits: usize,
    width_seen: u16,
}

impl Screen for Counter {
    fn render(&mut self, frame: &mut Frame) {
        self.width_seen = title_keys_width();
        let title = header("VIEW", "a title that runs long", "extra", &Look::default());
        frame.render_widget(title, frame.area());
    }
    fn handle_key(&mut self, key: KeyEvent) {
        self.keys.push(key.code);
    }
    fn interrupt(&mut self) {
        self.interrupts += 1;
    }
    fn done(&self) -> bool {
        false
    }
    fn quit(&mut self) {
        self.quits += 1;
    }
}

fn press(code: KeyCode, modifiers: KeyModifiers) -> Event {
    Event::Key(KeyEvent::new(code, modifiers))
}

fn rows(t: &Terminal<TestBackend>) -> Vec<String> {
    let buf = t.backend().buffer();
    (0..buf.area.height)
        .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect())
        .collect()
}

#[test]
fn ctrl_c_interrupts_and_other_presses_go_to_the_key_handler() {
    let mut view = Counter::default();
    let mut on_key = |s: &mut Counter, k: KeyEvent| s.handle_key(k);
    assert!(feed(
        &mut view,
        &mut on_key,
        press(KeyCode::Char('c'), KeyModifiers::CONTROL)
    ));
    assert!(feed(
        &mut view,
        &mut on_key,
        press(KeyCode::Char('q'), KeyModifiers::NONE)
    ));
    let mut release = KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE);
    release.kind = KeyEventKind::Release;
    assert!(!feed(&mut view, &mut on_key, Event::Key(release)));
    assert!(feed(&mut view, &mut on_key, Event::Resize(10, 10)));
    assert!(!feed(&mut view, &mut on_key, Event::FocusGained));
    assert_eq!(view.interrupts, 1);
    assert_eq!(view.keys, vec![KeyCode::Char('q')]);
}

#[test]
fn a_framed_view_ends_its_title_row_with_the_session_keys() {
    let mut view = Counter::default();
    let keys = vec![("ctrl+alt+d".to_string(), "detach".to_string())];
    let mut framed = Framed {
        screen: &mut view,
        keys,
        look: Look::default(),
    };
    let mut t = Terminal::new(TestBackend::new(30, 2)).unwrap();
    t.draw(|f| framed.render(f)).unwrap();
    let top = &rows(&t)[0];
    assert!(top.trim_end().ends_with("ctrl+alt+d detach"), "{top}");
    assert!(top.starts_with(" VIEW "), "{top}");
    framed.quit();
    assert_eq!(view.quits, 1, "a stop is the view's own quit");
    assert_eq!(view.width_seen, " ctrl+alt+d detach  ".len() as u16 + 1);
    assert_eq!(title_keys_width(), 0, "outside a draw");
}

#[test]
fn a_title_bar_keeps_the_keys_whole_however_narrow() {
    let keys = vec![("ctrl+alt+d".to_string(), "detach".to_string())];
    let look = Look::default();
    let mut t = Terminal::new(TestBackend::new(24, 1)).unwrap();
    t.draw(|f| {
        let title = header("VIEW", "a long title", "", &look);
        title_bar(f, f.area(), title, &keys, &look);
    })
    .unwrap();
    assert!(rows(&t)[0].contains("ctrl+alt+d detach"), "{:?}", rows(&t));
}

#[test]
fn help_and_input_lines_light_the_keys() {
    let look = Look::default();
    let line = help_line(&[("q", "quit"), ("s", "save")], &look);
    let keys: Vec<_> = line
        .spans
        .iter()
        .filter(|s| s.style == ink::highlight(&look))
        .map(|s| s.content.to_string())
        .collect();
    assert_eq!(keys, ["q", "s"]);
    let typed = input_line("name: ", "abc", &[("Enter", "keep")], &look);
    let text: String = typed.spans.iter().map(|s| s.content.as_ref()).collect();
    assert!(
        text.starts_with(" name: abc▏") && text.contains("Enter keep"),
        "{text}"
    );
}

#[test]
fn a_stop_is_told_apart_wrapped_or_not() {
    let tool = Tool::new("tool");
    let stopped = Stopped::new(&tool);
    assert_eq!(stopped.to_string(), "stopped by `tool kill`");
    assert!(is_stopped(&stopped));
    let io: io::Error = stopped.into();
    assert!(is_stopped(&io));
    assert!(!is_stopped(&io::Error::other("something else")));

    /// An error that wraps another, as a context does.
    #[derive(Debug)]
    struct Context(io::Error);
    impl std::fmt::Display for Context {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str("while showing the view")
        }
    }
    impl std::error::Error for Context {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            Some(&self.0)
        }
    }
    assert!(is_stopped(&Context(Stopped::new(&tool).into())));
    assert!(!is_stopped(&Context(io::Error::other("no"))));
}

#[test]
fn a_view_asks_for_the_mouse_only_when_it_takes_it() {
    let modes = modes_of(&Counter::default());
    assert_eq!(
        modes,
        Modes::new()
            .mouse(Mouse::Off)
            .keyboard_enhancement(Keyboard::Off)
    );
}
