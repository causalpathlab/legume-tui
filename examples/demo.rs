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
//! misbehave on purpose: `demo fail` says an error (with a long trace
//! after it when `RUST_BACKTRACE` is set) and exits at once,
//! `demo slow` loads for a long time before its screen, and `demo stall`
//! stops reading its input.
//!
//! Others show what a program leaves and holds: `demo bye` leaves a line
//! when it ends (by q, a `kill`, or a file named `end` appearing in its
//! folder), `demo error` holds an error there instead and then says it as
//! a program ending with one does, `demo work` works a while before any
//! screen with a stop waiting for it, `demo task` runs the nested screen
//! as such work, `demo jobs` holds a fit and a save in process groups of
//! their own (the save writes once a file named `go` appears in its
//! folder), and `demo probe` shows what the attached terminal answered.
//!
//! `demo start LABEL ARGS...` starts `demo ARGS...` in a new detached
//! session from code (an empty LABEL for the next number) and prints its
//! id; `demo find ID` prints its state, or `gone`; `demo stop ID
//! [--force]` stops it from code.

use legume_tui::child::Stopper;
use legume_tui::cli::{AttachArgs, KillArgs, RenameArgs};
use legume_tui::graphics::{self, Fallback, Graphics};
use legume_tui::{Group, Keyboard, Modes, Options, Round, Tool};
use ratatui::crossterm::event::{Event, KeyCode, KeyEventKind, KeyboardEnhancementFlags};
use ratatui::widgets::Paragraph;
use std::path::Path;
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
        Some("start") => {
            let label = name.filter(|n| !n.is_empty());
            let mut program = Command::new(std::env::current_exe()?);
            program.args(args.get(2..).unwrap_or_default());
            println!("{}", TOOL.start_detached("job", label.as_deref(), program)?);
            return Ok(());
        }
        Some("find") => {
            let state = match TOOL.find(&name.unwrap_or_default())? {
                Some(legume_tui::Attached(true)) => "attached",
                Some(legume_tui::Attached(false)) => "detached",
                None => "gone",
            };
            println!("{state}");
            return Ok(());
        }
        Some("stop") => {
            let force = args.get(2).is_some_and(|a| a == "--force");
            let stopped = TOOL.stop(&name.unwrap_or_default(), force, Duration::from_secs(10))?;
            println!("{}", if stopped { "stopped" } else { "running" });
            return Ok(());
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
            // With a backtrace asked for, a long trace after it, as a
            // `main` returning an error says one: several KB, more lines
            // than a screen.
            if std::env::var_os("RUST_BACKTRACE").is_some_and(|v| v != "0") {
                eprintln!("\nStack backtrace:");
                for i in 0..200 {
                    eprintln!("  {i:>3}: demo::frame_{i}\n             at ./src/frame.rs:{i}:5");
                }
                eprintln!("demo-fail: end of trace");
            }
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
            // Drawn once, when a terminal is there to draw on: a draw
            // before the client is ready is dropped.
            while !session.attached() && !session.stop_requested() {
                std::thread::sleep(Duration::from_millis(50));
            }
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
        Some("bye") => return bye(false),
        Some("error") => return bye(true),
        Some("work") => {
            let session = TOOL.session("work", &Options::default())?;
            // A stop waits for the work: it is not one to cut short.
            session.stoppable(|| {
                println!("demo-work working");
                std::thread::sleep(Duration::from_secs(1));
                println!("demo-work done");
            });
            if session.stop_requested() {
                println!("demo-work stopped after its work");
            }
            return Ok(());
        }
        Some("task") => {
            let session = TOOL.session("task", &Options::default())?;
            // A stop passed on to the child, and waited for with the rest.
            session.stoppable(|| -> std::io::Result<()> {
                let mut nested = Command::new(std::env::current_exe()?);
                session.run_child(nested.arg("nested"))?;
                println!("demo-task done");
                Ok(())
            })?;
            if session.stop_requested() {
                println!("demo-task stopped after its work");
            }
            return Ok(());
        }
        Some("jobs") => return jobs(),
        Some("probe") => return probe(),
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
        let round = session.poll_with(&mut terminal, wait, || {
            repaints += 1;
            Ok(())
        })?;
        if round == Round::Stopped {
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
        let Some(Event::Key(k)) = round.event() else {
            continue;
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        match k.code {
            KeyCode::Char('q') => break,
            KeyCode::Char('n') => {
                let mut which = Command::new(std::env::current_exe()?);
                which.arg("which");
                session.command(&mut which, Group::Shared).status()?;
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

/// Whether the user ended `round`: q, or a `kill`.
fn quits(round: &Round) -> bool {
    match round {
        Round::Stopped => true,
        Round::Event(Event::Key(k)) => {
            k.kind != KeyEventKind::Release && k.code == KeyCode::Char('q')
        }
        _ => false,
    }
}

/// A screen that ends on q, a `kill`, or a file named `end` appearing in
/// its folder, and leaves a line saying so; with `error`, the file ends it
/// with an error, held where it will be seen.
fn bye(error: bool) -> std::io::Result<()> {
    let screen = if error { "error" } else { "bye" };
    let session = TOOL.session(screen, &Options::default())?;
    let mut terminal = session.terminal()?;
    loop {
        session.draw(&mut terminal, |f| {
            f.render_widget(Paragraph::new(format!("demo-{screen} waiting")), f.area())
        })?;
        let round = session.poll(&mut terminal, TICK)?;
        if quits(&round) {
            session.leave(&format!("demo-{screen}: left by the quit path"));
            break;
        }
        if Path::new("end").exists() {
            if error {
                let error = "demo-error: the input went away";
                session.hold_error(&mut terminal, error)?;
                session.restore();
                // As `main` returning it says it.
                eprintln!("Error: {error}");
                std::process::exit(2);
            }
            session.leave("demo-bye: left on its own");
            break;
        }
    }
    session.restore();
    Ok(())
}

/// A screen holding two children in process groups of their own, a fit
/// and a save, their pids its status; on q or a `kill`, the fit is
/// stopped and the save let finish. The save waits for a file named `go`,
/// then says so on the program's stderr, which may be gone by then, and
/// writes `saved`.
fn jobs() -> std::io::Result<()> {
    let session = TOOL.session("jobs", &Options::default())?;
    let env = session.child_env();
    let start = |stopper: &Stopper, script: &str| -> std::io::Result<u32> {
        let mut sh = Command::new("sh");
        let child = env.apply(sh.args(["-c", script]), Group::Own).spawn()?;
        let pid = child.id();
        stopper.hold(child);
        Ok(pid)
    };
    let (fit, save) = (Stopper::new(), Stopper::saving());
    let saving = "while [ ! -e go ]; do sleep 0.1; done; echo saving >&2; echo saved > saved";
    let status = format!(
        "jobs fit={} save={}",
        start(&fit, "exec sleep 300")?,
        start(&save, saving)?
    );
    let mut terminal = session.terminal()?;
    loop {
        session.status(&status);
        session.draw(&mut terminal, |f| {
            f.render_widget(Paragraph::new(format!("demo-{status}")), f.area())
        })?;
        if quits(&session.poll(&mut terminal, TICK)?) {
            break;
        }
    }
    session.restore();
    fit.quit();
    save.quit();
    for stopper in [fit, save] {
        if let Some(mut child) = stopper.release() {
            if stopper.saves() {
                // Not waited for here: the test ends it.
                continue;
            }
            child.wait()?;
        }
    }
    Ok(())
}

/// A screen showing what the attached terminal answered: its picture
/// protocol (asked, else guessed), its background, whether it has kitty's
/// keyboard, and how kitty's pictures would reach it.
fn probe() -> std::io::Result<()> {
    let session = TOOL.session("probe", &Options::default())?;
    let modes = Modes::new().keyboard_enhancement(Keyboard::IfSupported(
        KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES,
    ));
    let mut terminal = session.terminal_with(modes)?;
    let mut said = String::from("demo-probe asking");
    loop {
        session.draw(&mut terminal, |f| {
            f.render_widget(Paragraph::new(said.as_str()), f.area())
        })?;
        let mut asked = None;
        let round = session.poll_with(&mut terminal, TICK, || {
            asked = Some(graphics::pick_with(
                &session,
                Graphics::Auto,
                Fallback::ProbeThenEnv,
            ));
            Ok(())
        })?;
        if let Some((picker, answer)) = asked {
            said = format!(
                "protocol={:?}\nbackground={:?}\nkeyboard={}\ntransport={:?}",
                picker.protocol_type(),
                answer.and_then(|a| a.background),
                session.keyboard_enhanced(),
                graphics::kitty_transport(&session),
            );
        }
        if quits(&round) {
            break;
        }
    }
    session.restore();
    Ok(())
}
