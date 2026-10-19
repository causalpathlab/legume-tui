//! The session paragraph of a TUI subcommand's help, and its wrapping.

use super::*;

const TOOL: Tool = Tool::new("tool");

/// `text` on one line, its words a space apart.
fn flat(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[test]
fn the_session_paragraph_says_it_all_by_default() {
    let help = session_help(&TOOL, SessionHelp::default());
    assert_eq!(
        flat(&help),
        "In an interactive terminal it runs as a detachable session, so its \
         work goes on when the connection drops. ctrl+alt+d detaches (the \
         first time, it asks for a name; LEGUME_TUI_DETACH_KEY sets another \
         key); ctrl+alt+s switches to another session (LEGUME_TUI_SWITCH_KEY). \
         Started again with nothing named, it offers the live sessions to \
         return to; `tool attach`, `sessions`, `kill` and `rename` manage \
         them. --no-detach (or LEGUME_TUI_NO_DETACH=1) runs it in this \
         terminal only."
    );
    assert!(
        help.lines().all(|l| l.chars().count() <= HELP_WIDTH),
        "{help}"
    );
}

#[test]
fn a_tool_says_it_in_its_own_words() {
    let help = session_help(
        &TOOL,
        SessionHelp::new()
            .what_goes_on("the fits go on")
            .resume_hint(Some("`tool run` with no folder"))
            .more("`?` shows the keys in effect."),
    );
    let said = flat(&help);
    for part in [
        "so the fits go on when the connection drops",
        "`tool run` with no folder offers the live sessions",
        "terminal only. `?` shows the keys in effect.",
    ] {
        assert!(said.contains(part), "{part:?} in {said}");
    }
}

#[test]
fn a_tui_on_something_named_or_behind_a_flag_says_so() {
    let help = session_help(
        &TOOL,
        SessionHelp::new()
            .lead("With --interactive, a view opens first.")
            .resume_hint(None),
    );
    let said = flat(&help);
    assert!(
        said.starts_with("With --interactive, a view opens first. In an interactive"),
        "{said}"
    );
    assert!(
        !said.contains("offers the live sessions to return to"),
        "{said}"
    );
    assert!(
        said.contains(
            "Starting it again starts a new session, naming the detached ones; \
             `tool attach` offers them to return to"
        ),
        "{said}"
    );
    assert!(help.lines().all(|l| l.chars().count() <= HELP_WIDTH));
}

#[test]
fn kill_stops_a_session_as_the_tool_says() {
    let quit_key = about(&TOOL, QUIT_KEY);
    assert!(flat(&quit_key.kill).starts_with("Stop a `tool` session as its quit key does"));
    let ctrl_c = about(&TOOL, "ctrl+c in it");
    assert!(flat(&ctrl_c.kill).starts_with("Stop a `tool` session as ctrl+c in it does"));
    assert!(ctrl_c.kill.lines().all(|l| l.chars().count() <= HELP_WIDTH));
    assert!(ctrl_c.attach.contains("`tool attach`") || ctrl_c.attach.contains("`tool`"));
}

#[test]
fn lines_break_between_words_and_never_inside_backticks() {
    let text = "aaa bbb `ccc ddd` eee\n\nfff `ggg hhh iii` jjj";
    assert_eq!(
        wrap(text, 9),
        "aaa bbb\n`ccc ddd`\neee\n\nfff\n`ggg hhh iii`\njjj"
    );
    // A width every unit fits in: whole lines, paragraphs kept apart.
    assert_eq!(
        wrap(text, 80),
        "aaa bbb `ccc ddd` eee\n\nfff `ggg hhh iii` jjj"
    );
    // Line breaks in a paragraph are only spaces.
    assert_eq!(wrap("aaa\nbbb", 80), "aaa bbb");
}

#[cfg(not(sessions))]
#[test]
fn without_sessions_the_subcommands_say_so() {
    let unsupported = |r: io::Result<()>| r.unwrap_err().kind() == io::ErrorKind::Unsupported;
    assert!(unsupported(attach(&TOOL, &AttachArgs::default())));
    assert!(unsupported(sessions(&TOOL)));
    assert!(unsupported(kill(&TOOL, &KillArgs::default())));
    assert!(unsupported(rename(&TOOL, &RenameArgs::default())));
}
