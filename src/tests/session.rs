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
fn tools_are_told_apart_by_their_whole_name() {
    let tool = Tool::new("tool");
    assert_eq!(tool.prefix(), "tool-");
    assert_eq!(tool.name(), "tool");
}
