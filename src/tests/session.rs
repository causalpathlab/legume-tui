//! What a session tells a tool about itself: the keys it offers, the
//! environment and process group of what it starts, what it leaves, and
//! the modes its screen asks for.

use super::*;

/// The environment `session` sets on a command.
fn env_of(session: &Session) -> Vec<(String, Option<String>)> {
    let mut cmd = Command::new("true");
    session
        .command(&mut cmd, Group::Shared)
        .get_envs()
        .map(|(k, v)| {
            let s = |o: &std::ffi::OsStr| o.to_string_lossy().into_owned();
            (s(k), v.map(s))
        })
        .collect()
}

#[test]
fn the_opt_out_is_passed_on_only_when_the_user_asked_for_it() {
    for why in [Bypass::Flag, Bypass::Env] {
        assert_eq!(
            env_of(&Session::bypassed(why)),
            [(NO_DETACH_VAR.to_string(), Some("1".to_string()))],
            "{why:?}"
        );
        let env = Session::bypassed(why).child_env();
        assert_eq!(env.vars(), [(NO_DETACH_VAR.into(), "1".into())]);
    }
    // Plain for another reason: a TUI it starts decides for itself.
    for why in [
        Bypass::Nested,
        Bypass::NotTerminal,
        Bypass::Multiplexer,
        Bypass::NotAsked,
        Bypass::NoHost,
        Bypass::Unsupported,
        Bypass::Tool,
    ] {
        assert!(env_of(&Session::bypassed(why)).is_empty(), "{why:?}");
    }
    // Opened plain by the tool: nothing passed on either.
    let tool = Session::plain(&Tool::new("tool"));
    assert!(env_of(&tool).is_empty());
    assert!(matches!(tool.bypass(), Some(Bypass::Tool | Bypass::Nested)));
}

/// The process group of process `pid`.
#[cfg(sessions)]
fn group_of(pid: u32) -> i32 {
    let pid = rustix::process::Pid::from_raw(pid as i32).unwrap();
    rustix::process::getpgid(Some(pid))
        .unwrap()
        .as_raw_nonzero()
        .get()
}

#[cfg(sessions)]
#[test]
fn a_child_has_a_group_of_its_own_as_asked() {
    let ours = group_of(std::process::id());
    let group = |env: &ChildEnv, group: Group| {
        let mut cmd = Command::new("sleep");
        let mut child = env.apply(cmd.arg("30"), group).spawn().unwrap();
        let g = group_of(child.id());
        let _ = (child.kill(), child.wait());
        (child.id() as i32, g)
    };
    let hosted = ChildEnv {
        vars: Vec::new(),
        hosted: true,
    };
    let (pid, g) = group(&hosted, Group::OwnWhenHosted);
    assert_eq!(g, pid, "hosted, it leads its own");
    assert_eq!(group(&hosted, Group::Shared).1, ours);
    // Plain, a hangup of the user's terminal reaches it with the program.
    let plain = Session::bypassed(Bypass::Flag).child_env();
    assert_eq!(group(&plain, Group::OwnWhenHosted).1, ours);
    // A runner's child leads its own either way.
    let (pid, g) = group(&plain, Group::Own);
    assert_eq!(g, pid, "plain, asked for its own");
    let (pid, g) = group(&hosted, Group::Own);
    assert_eq!(g, pid);
    // Owned, for a worker thread.
    let sent = std::thread::spawn(move || plain.vars().len())
        .join()
        .unwrap();
    assert_eq!(sent, 1);
}

#[test]
fn a_plain_session_offers_no_keys_and_hears_no_stop() {
    let s = Session::bypassed(Bypass::Flag);
    assert!(!s.offers_keys());
    assert!(!s.stop_requested());
    assert!(help_keys().is_empty());
    // Offered (as a hosted session would): each key in a word and a phrase.
    KEYS_OFFERED.store(true, SeqCst);
    let keys = help_keys();
    KEYS_OFFERED.store(false, SeqCst);
    let short: Vec<_> = keys.iter().map(|k| k.short.as_str()).collect();
    assert_eq!(short, ["detach", "switch"]);
    assert_eq!(keys[0].key, detach_key());
    assert!(keys[0].long.starts_with("detach, leaving it running"));
    assert!(keys[1].long.starts_with("switch,"));
}

#[test]
fn a_plain_session_repaints_once_at_the_start_and_when_asked() {
    let s = Session::bypassed(Bypass::Flag);
    assert!(s.take_repaint());
    assert!(!s.take_repaint());
    assert!(s.attached());
    assert!(!s.screen_up());
    s.request_repaint();
    assert!(s.take_repaint(), "asked for");
    assert!(!s.take_repaint());
}

#[test]
fn work_off_a_screen_never_up_just_runs_and_owes_nothing() {
    let s = Session::bypassed(Bypass::Flag);
    assert!(s.take_repaint(), "the first");
    assert_eq!(s.off_screen(|| 7).unwrap(), 7);
    assert_eq!(s.stoppable(|| 8), 8);
    assert!(!s.stop_requested(), "plain: no stop is taken");
    assert!(!s.take_repaint(), "no screen was taken back");
    assert_eq!(attaches(), 0, "plain: nothing attaches");
}

#[test]
fn a_round_gives_its_event() {
    let key = Event::FocusGained;
    assert_eq!(Round::Event(key.clone()).event(), Some(key));
    assert_eq!(Round::Timeout.event(), None);
    assert_eq!(Round::Repainted.event(), None);
    assert_eq!(Round::Stopped.event(), None);
}

#[test]
fn the_keyboard_is_enhanced_as_the_modes_and_the_terminal_say() {
    use ratatui::crossterm::event::KeyboardEnhancementFlags as Flags;
    let s = Session::bypassed(Bypass::Flag);
    assert!(!s.keyboard_enhanced(), "off by default");
    let flags = Flags::DISAMBIGUATE_ESCAPE_CODES;
    let with = |keyboard| Modes::new().keyboard_enhancement(keyboard);
    s.set_modes(with(Keyboard::On(flags))).unwrap();
    assert!(s.keyboard_enhanced());
    s.set_modes(with(Keyboard::IfSupported(flags))).unwrap();
    assert!(!s.keyboard_enhanced(), "not known to have it");
    s.heard_keyboard(true);
    assert!(s.keyboard_enhanced(), "the terminal said so");
    s.heard_keyboard(false);
    assert!(!s.keyboard_enhanced());
    assert_eq!(Keyboard::IfSupported(flags).flags(false), None);
    assert_eq!(Keyboard::IfSupported(flags).flags(true), Some(flags));
    assert_eq!(Keyboard::On(flags).flags(false), Some(flags));
    assert_eq!(Keyboard::Off.flags(true), None);
}

#[test]
fn tools_are_told_apart_by_their_whole_name() {
    let tool = Tool::new("tool");
    assert_eq!(tool.prefix(), "tool-");
    assert_eq!(tool.name(), "tool");
}

#[test]
fn the_session_tool_session_opened_is_current_and_shared() {
    let opts = Options {
        no_detach: true,
        ..Options::default()
    };
    let s = Tool::new("tool").session("view", &opts).unwrap();
    let current = current().expect("a session is open");
    assert!(!current.owner, "dropping it restores nothing");
    // The first one opened in this process, which is this one here.
    #[cfg(sessions)]
    {
        assert!(s.take_repaint());
        current.request_repaint();
        assert!(s.take_repaint(), "the same session");
    }
    drop(s);
}

#[cfg(sessions)]
#[test]
fn the_terminal_env_is_the_attached_terminals_once_one_attached() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::bypassed(Bypass::Flag);
    Arc::get_mut(&mut s.inner).unwrap().at = Some((dir.path().to_path_buf(), "tool-view-1".into()));
    let path_var = std::env::var("PATH").ok();
    // Before any attach: this process's own.
    assert_eq!(s.terminal_env("TERM"), std::env::var("TERM").ok());
    let vars = [("TERM".to_string(), "xterm-kitty".to_string())];
    dir::write_terminal_env(dir.path(), "tool-view-1", &vars).unwrap();
    assert_eq!(s.terminal_env("TERM").as_deref(), Some("xterm-kitty"));
    // Not in the attached terminal: not there, whatever this process has.
    assert_eq!(s.terminal_env("KITTY_WINDOW_ID"), None);
    // Not one a client passes on: this process's own.
    assert_eq!(s.terminal_env("PATH"), path_var);
}

#[cfg(sessions)]
#[test]
fn a_hosted_program_leaves_its_words_beside_the_socket_to_be_said_once() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::new(None, None);
    Arc::get_mut(&mut s.inner).unwrap().at = Some((dir.path().to_path_buf(), "tool-run-1".into()));
    assert!(s.is_hosted());
    s.leave("first");
    s.leave("fits finished: 2 of 2 done\nGENE1 \x1b[1mbold\x1b[0m");
    // Its socket is not there: it ended.
    let left = dir::take_left(dir.path(), "tool-");
    assert_eq!(
        left,
        [(
            "tool-run-1".to_string(),
            "fits finished: 2 of 2 done\nGENE1  [1mbold [0m".to_string()
        )]
    );
    assert!(dir::take_left(dir.path(), "tool-").is_empty(), "said once");
    // Another tool's are not ours.
    dir::write_leave(dir.path(), "other-run-1", "theirs").unwrap();
    assert!(dir::take_left(dir.path(), "tool-").is_empty());
    assert_eq!(
        dir::take_leave(dir.path(), "other-run-1").as_deref(),
        Some("theirs")
    );
}

#[cfg(sessions)]
#[test]
fn the_environment_goes_on_the_wire_and_comes_back_whole() {
    let vars = vec![
        ("TERM".to_string(), "xterm-256color".to_string()),
        ("TERM_PROGRAM".to_string(), "a=b c".to_string()),
        ("COLORTERM".to_string(), String::new()),
    ];
    let body = wire::hello_body(Size::default(), &vars);
    assert_eq!(wire::hello_env(&body), vars);
    assert_eq!(wire::hello_env(&body[..9]), Vec::new(), "none passed");
}

#[cfg(not(sessions))]
#[test]
fn without_sessions_every_tui_runs_plain_as_before() {
    let tool = Tool::new("tool");
    let s = tool.session("view", &Options::default()).unwrap();
    assert!(!s.is_hosted() && !s.offers_keys());
    assert!(matches!(
        s.bypass(),
        Some(Bypass::Unsupported | Bypass::Env)
    ));
    let opts = Options {
        no_detach: true,
        ..Options::default()
    };
    let s = tool.session("view", &opts).unwrap();
    assert_eq!(s.bypass(), Some(Bypass::Flag));
    assert!(s.take_repaint());
    assert!(!s.take_repaint());
    assert!(s.attached());
    assert!(!s.stop_requested());
    assert_eq!(s.name(), None);
    assert_eq!(s.terminal_env("TERM"), std::env::var("TERM").ok());
    assert_eq!(Session::plain(&tool).bypass(), Some(Bypass::Tool));
    // A child runs and is waited for: this test binary, listing its tests.
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.arg("--list").stdout(std::process::Stdio::null());
    assert!(s.run_child(&mut child).unwrap().success());
    assert!(!s.take_repaint(), "no screen was up to repaint");
}
