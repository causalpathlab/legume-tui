//! A small TUI on a session, and the subcommands that go with it; the
//! end-to-end test drives it through a pseudo-terminal.
//!
//! `demo` shows a screen that counts its repaints and ticks (q quits, n
//! starts a one-shot nested `demo which`, v a nested screen of its own),
//! `demo open` does the same for a named target, `demo nested` is that
//! nested screen, `demo which` says how a session would run here, and
//! `demo attach|sessions|kill [name]` and `demo rename name label` are the
//! handlers. `demo idle` waits a minute for a key each round, and `demo
//! first` runs the nested screen before a screen of its own. Three
//! misbehave on purpose: `demo fail` says an error and exits at once,
//! `demo slow` loads for a long time before its screen, and `demo stall`
//! stops reading its input.

use legume_tui::cli::{AttachArgs, KillArgs, RenameArgs};
use legume_tui::{Options, Tool};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind};
use ratatui::widgets::Paragraph;
use std::process::Command;
use std::time::Duration;

const TOOL: Tool = Tool::new("demo");

fn main() -> std::io::Result<()> {
    // As some tools do, before anything else: the host must survive it.
    #[cfg(unix)]
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    legume_tui::serve_if_host();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let name = args.get(1).cloned();
    match args.first().map(String::as_str) {
        Some("attach") => return legume_tui::cli::attach(&TOOL, &AttachArgs { name }),
        Some("sessions") => return legume_tui::cli::sessions(&TOOL),
        Some("kill") => {
            let force = args.iter().any(|a| a == "--force");
            let name = name.filter(|n| n != "--force");
            return legume_tui::cli::kill(&TOOL, &KillArgs { name, force });
        }
        Some("rename") => {
            let label = args.get(2).cloned().unwrap_or_default();
            let session = name.unwrap_or_default();
            return legume_tui::cli::rename(&TOOL, &RenameArgs { session, label });
        }
        Some("which") => {
            let s = TOOL.session("which", &Options::default())?;
            println!("bypass={:?} hosted={}\r", s.bypass(), s.is_hosted());
            return Ok(());
        }
        Some("nested") => return screen("nested", "nested", &Options::default(), TICK),
        Some("idle") => {
            return screen("idle", "idle", &Options::default(), Duration::from_secs(60));
        }
        Some("first") => {
            let session = TOOL.session("first", &Options::default())?;
            let mut nested = Command::new(std::env::current_exe()?);
            session.run_child(nested.arg("nested"))?;
            println!("demo-first after");
            return Ok(());
        }
        Some("fail") => {
            let _session = TOOL.session("fail", &Options::default())?;
            eprintln!("demo-fail: no such input");
            std::process::exit(1);
        }
        Some("slow") => {
            let session = TOOL.session("slow", &Options::default())?;
            println!("demo-slow loading");
            std::thread::sleep(Duration::from_secs(60));
            drop(session);
            return Ok(());
        }
        Some("stall") => {
            let session = TOOL.session("stall", &Options::default())?;
            let mut terminal = session.terminal()?;
            session.draw(&mut terminal, |f| {
                f.render_widget(Paragraph::new("demo-stall-stalled"), f.area())
            })?;
            // Reads nothing more until told to stop.
            while !session.stop_requested() {
                std::thread::sleep(Duration::from_millis(50));
            }
            session.restore();
            return Ok(());
        }
        Some("open") => {
            let opts = Options {
                target: true,
                ..Options::default()
            };
            return screen("view", "demo-screen", &opts, TICK);
        }
        _ => {}
    }
    screen("view", "demo-screen", &Options::default(), TICK)
}

/// How long a screen waits for a key, as a rule.
const TICK: Duration = Duration::from_millis(100);

/// A screen showing `label-repaint-N`, a tick count, the keys and the
/// attached terminal's `TERM_PROGRAM`, until q; it waits `wait` for a key
/// each round.
fn screen(screen: &str, label: &str, opts: &Options, wait: Duration) -> std::io::Result<()> {
    let session = TOOL.session(screen, opts)?;
    let mut terminal = session.terminal()?;
    let mut repaints = 0;
    let mut ticks = 0u64;
    loop {
        let event = session.poll_with(&mut terminal, wait, || {
            repaints += 1;
            Ok(())
        })?;
        if session.stop_requested() {
            break;
        }
        session.status(&format!("{label} after {repaints} repaints"));
        if session.attached() {
            ticks += 1;
        }
        let text = format!(
            "{label}-repaint-{repaints}\ntick {ticks}\ndetach-key={}\nswitch-key={}\nterminal={}",
            legume_tui::detach_key(),
            legume_tui::switch_key(),
            session.terminal_env("TERM_PROGRAM").unwrap_or_default()
        );
        session.draw(&mut terminal, |f| {
            f.render_widget(Paragraph::new(text), f.area())
        })?;
        let Some(Event::Key(k)) = event else { continue };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match k.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('n') => {
                let mut which = Command::new(std::env::current_exe()?);
                which.arg("which");
                session.command(&mut which).status()?;
            }
            // The screen hands the terminal over, and takes it back to
            // repaint; `kill` reaches the nested one through us.
            KeyCode::Char('v') => {
                let mut nested = Command::new(std::env::current_exe()?);
                session.run_child(nested.arg("nested"))?;
            }
            _ => {}
        }
    }
    session.restore();
    println!("{label} done after {repaints} repaints");
    Ok(())
}
