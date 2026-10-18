//! What a session tells a tool about itself: the keys it offers and the
//! environment for a TUI it starts.

use super::*;

/// The environment `session` sets on a command.
fn env_of(session: &Session) -> Vec<(String, Option<String>)> {
    let mut cmd = Command::new("true");
    session
        .command(&mut cmd)
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
            env_of(&Session::plain(why)),
            [(NO_DETACH_VAR.to_string(), Some("1".to_string()))],
            "{why:?}"
        );
    }
    // Plain for another reason: a TUI it starts decides for itself.
    for why in [
        Bypass::Nested,
        Bypass::NotTerminal,
        Bypass::Multiplexer,
        Bypass::NotAsked,
        Bypass::NoHost,
        Bypass::Unsupported,
    ] {
        assert!(env_of(&Session::plain(why)).is_empty(), "{why:?}");
    }
}

#[test]
fn a_plain_session_offers_no_keys_and_hears_no_stop() {
    let s = Session::plain(Bypass::Flag);
    assert!(!s.offers_keys());
    assert!(!s.stop_requested());
    assert!(help_keys().is_empty());
}

#[test]
fn a_plain_session_repaints_once_at_the_start() {
    let s = Session::plain(Bypass::Flag);
    assert!(s.take_repaint());
    assert!(!s.take_repaint());
    assert!(s.attached());
}

#[test]
fn work_off_a_screen_never_up_just_runs_and_owes_nothing() {
    let s = Session::plain(Bypass::Flag);
    assert!(s.take_repaint(), "the first");
    assert_eq!(s.off_screen(|| 7).unwrap(), 7);
    assert!(!s.take_repaint(), "no screen was taken back");
    assert_eq!(attaches(), 0, "plain: nothing attaches");
}

#[test]
fn tools_are_told_apart_by_their_whole_name() {
    let tool = Tool::new("tool");
    assert_eq!(tool.prefix(), "tool-");
    assert_eq!(tool.name(), "tool");
}

#[cfg(sessions)]
#[test]
fn the_terminal_env_is_the_attached_terminals_once_one_attached() {
    let dir = tempfile::tempdir().unwrap();
    let mut s = Session::plain(Bypass::Flag);
    s.at = Some((dir.path().to_path_buf(), "tool-view-1".into()));
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
    // A child runs and is waited for: this test binary, listing its tests.
    let mut child = Command::new(std::env::current_exe().unwrap());
    child.arg("--list").stdout(std::process::Stdio::null());
    assert!(s.run_child(&mut child).unwrap().success());
    assert!(!s.take_repaint(), "no screen was up to repaint");
}
