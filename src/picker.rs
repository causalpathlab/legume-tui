//! The session picker: what [`crate::Tool::session`] shows when a tool
//! starts hosted, with nothing named to open, while it has live sessions;
//! what a bare `<tool> attach` shows with several; and what the switch key
//! shows from inside one. Each session's label and id, state, age, command line
//! and status, and "start new" where a new one can start. `r` gives the
//! highlighted session a label.

use crate::dir::{self, Listed};
use crate::Tool;
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Modifier, Style};
use ratatui::widgets::{List, ListItem, ListState, Paragraph};
use std::io;
use std::path::Path;

/// What the user picked.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Choice {
    /// Attach to this session (taking it over if attached elsewhere).
    Attach(String),
    /// Start a new session.
    New,
    /// Neither.
    Quit,
}

/// What the picker offers, beside the sessions.
struct Offer<'a> {
    /// Where the sessions live, for a rename.
    dir: &'a Path,
    tool: &'a Tool,
    /// Whether "start new" is a choice.
    new: bool,
    /// The session the user is switching from, marked.
    current: Option<&'a str>,
}

/// Ask which of `live`, `tool`'s sessions in `dir`, to go to, or whether to
/// start anew where `new` offers it. The first detached session (other
/// than `current`, the one the user is switching from, marked) is offered
/// first, "start new" when there is none and it is offered.
pub(crate) fn choose(
    dir: &Path,
    tool: &Tool,
    new: bool,
    current: Option<&str>,
    live: Vec<Listed>,
) -> io::Result<Choice> {
    let offer = Offer {
        dir,
        tool,
        new,
        current,
    };
    let mut terminal = ratatui::try_init()?;
    let chosen = pick(&mut terminal, &offer, live);
    ratatui::restore();
    chosen
}

/// [`choose`]; None, said, when the picker cannot open.
pub(crate) fn choose_or_say(
    dir: &Path,
    tool: &Tool,
    new: bool,
    current: Option<&str>,
    live: Vec<Listed>,
) -> Option<Choice> {
    choose(dir, tool, new, current, live)
        .inspect_err(|e| eprintln!("legume-tui: no session picker ({e})"))
        .ok()
}

/// One session, as a line of the list.
fn row(l: &Listed, current: Option<&str>) -> String {
    let status = l
        .status
        .as_deref()
        .map_or(String::new(), |s| format!("  {s}"));
    let mark = if current == Some(l.name.as_str()) {
        " (current)"
    } else {
        ""
    };
    format!(
        "{}{mark}  {}  {}  {}{status}",
        l.shown(),
        l.state(),
        l.age(),
        l.command
    )
}

fn pick(
    terminal: &mut ratatui::DefaultTerminal,
    offer: &Offer,
    mut live: Vec<Listed>,
) -> io::Result<Choice> {
    let prefix = offer.tool.prefix();
    // "start new", where offered, is the line after the sessions.
    let rows = live.len() + usize::from(offer.new);
    let mut at = live
        .iter()
        .position(|l| !l.attached && Some(l.name.as_str()) != offer.current)
        .or_else(|| {
            live.iter()
                .position(|l| Some(l.name.as_str()) == offer.current)
        })
        .unwrap_or(if offer.new { live.len() } else { 0 });
    // The label being typed for the highlighted session, and what became
    // of the last try.
    let mut editing: Option<String> = None;
    let mut said: Option<String> = None;
    loop {
        terminal.draw(|f| {
            let [head, body, foot] = Layout::vertical([
                Constraint::Length(2),
                Constraint::Min(1),
                Constraint::Length(1),
            ])
            .areas(f.area());
            f.render_widget(
                Paragraph::new(format!("{}: live sessions", offer.tool.name())),
                head,
            );
            let items = live
                .iter()
                .enumerate()
                .map(|(i, l)| match &editing {
                    Some(text) if i == at => ListItem::new(format!("{}  label: {text}▏", l.name)),
                    _ => ListItem::new(row(l, offer.current)),
                })
                .chain(offer.new.then(|| ListItem::new("start new")));
            let list = List::new(items)
                .highlight_symbol("> ")
                .highlight_style(Style::default().add_modifier(Modifier::REVERSED));
            let mut state = ListState::default().with_selected(Some(at));
            f.render_stateful_widget(list, body, &mut state);
            let keys = match (&editing, offer.new) {
                (Some(_), _) => "Enter: set label   Esc: cancel",
                (None, true) => "Enter: open   r: rename   n: start new   q: quit",
                (None, false) => "Enter: open   r: rename   q: quit",
            };
            let foot_text = said
                .as_deref()
                .map_or(keys.to_string(), |s| format!("{s}   {keys}"));
            f.render_widget(Paragraph::new(foot_text), foot);
        })?;
        let Event::Key(k) = event::read()? else {
            continue;
        };
        if k.kind == KeyEventKind::Release {
            continue;
        }
        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
            return Ok(Choice::Quit);
        }
        if let Some(text) = editing.as_mut() {
            match k.code {
                KeyCode::Char(c) => text.push(c),
                KeyCode::Backspace => {
                    text.pop();
                }
                KeyCode::Enter if text.is_empty() => editing = None,
                KeyCode::Enter => match dir::set_label(offer.dir, &prefix, &live[at].name, text) {
                    Ok(()) => {
                        live[at].label = editing.take();
                        said = None;
                    }
                    Err(e) => said = Some(e.to_string()),
                },
                KeyCode::Esc => {
                    editing = None;
                    said = None;
                }
                _ => {}
            }
            continue;
        }
        match k.code {
            KeyCode::Up | KeyCode::Char('k') => at = at.saturating_sub(1),
            KeyCode::Down | KeyCode::Char('j') => at = (at + 1).min(rows.saturating_sub(1)),
            KeyCode::Enter if at < live.len() => return Ok(Choice::Attach(live[at].name.clone())),
            KeyCode::Enter | KeyCode::Char('n') if offer.new => return Ok(Choice::New),
            KeyCode::Char('r') if at < live.len() => {
                editing = Some(live[at].label.clone().unwrap_or_default());
                said = None;
            }
            KeyCode::Char('q') | KeyCode::Esc => return Ok(Choice::Quit),
            _ => {}
        }
    }
}
