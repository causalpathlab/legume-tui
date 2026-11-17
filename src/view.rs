//! The screen loop the tools' full-screen views run in, and the chrome
//! they share. A view is a [`Screen`]: drawn, fed key by key, asked about
//! every [`TICK`] for work in the background. [`Tui`] runs a command's
//! views one after another on one screen in its [`Session`], with the
//! command's own work between them on the normal screen; [`run_screen`]
//! runs one in a plain session of its own. A `kill` takes a view's quit
//! path ([`Screen::quit`]).
//!
//! [`run_view`] runs a view in the process's session ([`crate::current`])
//! with the session keys at the right end of its title row ([`Framed`]),
//! keeping the screen up from view to view until [`end_views`]; a `kill`
//! ends it with [`Stopped`] once the view is done.
//!
//! The chrome: a title row ([`header`], [`title_bar`]), rounded panels
//! ([`panel`]), and footers of keys ([`help_line`], [`input_line`]), in a
//! [`Look`]'s styles.
//!
//! Ported from data-beans (`src/interactive/ui.rs`) and faba
//! (`src/tui.rs`).

use crate::look::Look;
use crate::{Keyboard, Modes, Mouse, Round, Session, SessionCommand, Tool};
use ratatui::crossterm::event::{
    self, Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers, KeyboardEnhancementFlags,
    MouseEvent,
};
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};
use ratatui::Frame;
use std::cell::{Cell, RefCell};
use std::io::{self, IsTerminal, Write};
use std::time::{Duration, Instant};

/// How often a view's loop asks [`Screen::tick`], keys or not.
pub const TICK: Duration = Duration::from_millis(200);

/// A full-screen view, driven by [`Tui::run`] or [`run_screen`].
pub trait Screen {
    /// Draw the view.
    fn render(&mut self, frame: &mut Frame);
    /// A key press (Ctrl-C goes to [`Screen::interrupt`] instead).
    fn handle_key(&mut self, key: KeyEvent);
    /// Ctrl-C.
    fn interrupt(&mut self);
    /// Whether the view has ended.
    fn done(&self) -> bool;
    /// A `kill` asked the view to stop: leave as its quit key does,
    /// without asking; work that must finish may run on, and is waited
    /// for ([`Tui::settle`]). By default as Ctrl-C.
    fn quit(&mut self) {
        self.interrupt();
    }
    /// Blocking work the last key asked for, as a line to print while it
    /// runs; [`Screen::do_work`] then does it.
    fn pending_work(&self) -> Option<String> {
        None
    }
    /// Do the work [`Screen::pending_work`] named.
    fn do_work(&mut self) {}
    /// Called about every [`TICK`], keys or not: true redraws, for a view
    /// waiting on work in the background.
    fn tick(&mut self) -> bool {
        false
    }
    /// Whether the view takes the mouse: only then is the terminal asked for
    /// it, since a view holding it keeps text from being selected.
    fn takes_mouse(&self) -> bool {
        false
    }
    /// A mouse event (a button pressed or let go, or a turn of the wheel)
    /// at a cell of the screen; whether the screen changed and is drawn
    /// again.
    fn mouse(&mut self, _event: MouseEvent) -> bool {
        false
    }
    /// Whether the terminal is asked to report keys held with Shift, Alt or
    /// Ctrl apart (the kitty keyboard protocol; others ignore the request),
    /// so that Shift+Enter, say, is not taken for Enter.
    fn reports_chords(&self) -> bool {
        false
    }
    /// Something to tell whoever is away, once: the loop rings the
    /// terminal's bell and asks it for a desktop notification saying it
    /// ([`notify`]).
    fn take_notice(&mut self) -> Option<String> {
        None
    }
    /// What the view is doing, in one short line, for the tool's session
    /// list and the session picker to show ([`Session::status`]).
    fn status(&self) -> Option<String> {
        None
    }
}

/// The terminal modes `screen` asks for, as its session turns them on (and
/// on again after every attach): the mouse as buttons and the wheel in SGR
/// form, not the pointer's every move, which would wake the loop for
/// nothing; keys held with modifiers told apart.
pub fn modes_of(screen: &impl Screen) -> Modes {
    Modes::new()
        .mouse(if screen.takes_mouse() {
            Mouse::Buttons
        } else {
            Mouse::Off
        })
        .keyboard_enhancement(if screen.reports_chords() {
            Keyboard::On(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)
        } else {
            Keyboard::Off
        })
}

/// Ring the terminal's bell and ask it for a desktop notification saying
/// `text` (OSC 9); terminals without either ignore them.
pub fn notify(text: &str) {
    let text: String = text.chars().filter(|c| !c.is_control()).collect();
    let mut out = io::stdout();
    // Whether it took cannot be known; a write that fails is as one
    // ignored.
    let _ = write!(out, "\x07\x1b]9;{text}\x07").and_then(|()| out.flush());
}

/// Whether a full-screen view can be drawn here: stdin and stdout are a
/// terminal, and one that can draw it (`TERM` is not `dumb`).
pub fn terminal_available() -> bool {
    io::stdin().is_terminal()
        && io::stdout().is_terminal()
        && std::env::var("TERM").ok().as_deref() != Some("dumb")
}

/// Refuse to open a screen with no terminal to draw it on (input or output
/// piped, as under an agent), saying what works `instead`. A session's own
/// program draws on its host's terminal.
pub fn need_terminal(instead: &str) -> io::Result<()> {
    if crate::in_session() || (io::stdin().is_terminal() && io::stdout().is_terminal()) {
        return Ok(());
    }
    Err(io::Error::other(format!(
        "no terminal to draw the screen on: {instead}"
    )))
}

/// The terminal a command's views draw on, in the command's session: up
/// from the first view ([`Tui::run`]) until dropped, so every view of the
/// command and the work between them ([`Tui::work`]) share one screen.
/// While it is up, a `kill` asks the views to stop ([`Screen::quit`],
/// [`Tui::stop_requested`]), and work under way finishes first. Log
/// records raised while it is up are held back and written once the
/// normal screen is back, or as work between views runs.
///
/// A `Tui` started while another has the screen up (a view run from
/// inside another's key handler or work) joins that screen rather than
/// bringing up one of its own: it draws on it, gives back the modes and
/// the normal screen as it found them when it ends, and never restores
/// the terminal, which the `Tui` that brought it up does, once.
pub struct Tui<'s> {
    session: &'s Session,
    /// What it draws on, while it is up ([`Role`]).
    terminal: Option<ratatui::DefaultTerminal>,
    role: Role,
    /// The modes the screen has on now.
    modes: Modes,
    _held: crate::logs::Hold,
}

/// A [`Tui`]'s part in the screen it draws on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    /// No screen yet, or one gone since.
    Down,
    /// It brought the screen up and restores it as it is dropped.
    Owner,
    /// It joined a screen another `Tui` brought up: the modes it had then,
    /// and whether it was shown or handed off for work.
    Guest { modes: Modes, shown: bool },
}

/// How a [`Tui`] comes up for a view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Rise {
    /// The session's screen is down: bring it up, as its owner.
    Bring,
    /// Another `Tui` has it up: join it, as a guest.
    Join,
    /// It is up already and this `Tui`'s: switch the modes only.
    Switch,
}

impl Role {
    /// How to come up with the session's screen up or not. A screen gone
    /// down under this `Tui` (restored by another) is brought up anew.
    fn rise(self, screen_up: bool) -> Rise {
        match (self, screen_up) {
            (_, false) => Rise::Bring,
            (Role::Down, true) => Rise::Join,
            (Role::Owner | Role::Guest { .. }, true) => Rise::Switch,
        }
    }

    /// Whether a `Tui` in this role is kept for the next view
    /// ([`run_view`]): only the screen's owner, which must restore it.
    fn kept(self) -> bool {
        self == Role::Owner
    }
}

impl<'s> Tui<'s> {
    /// Views of `session`, the screen not up yet.
    pub fn new(session: &'s Session) -> Self {
        Self {
            session,
            terminal: None,
            role: Role::Down,
            modes: Modes::new(),
            _held: crate::logs::hold(),
        }
    }

    /// The session the views run in.
    pub fn session(&self) -> &'s Session {
        self.session
    }

    /// Whether a `kill` asked the command to stop: no more views and no
    /// more work after the one running.
    pub fn stop_requested(&self) -> bool {
        self.session.stop_requested()
    }

    /// Run `screen` until it is done, its keys to [`Screen::handle_key`].
    pub fn run<S: Screen>(&mut self, screen: &mut S) -> io::Result<()> {
        self.run_with(screen, S::handle_key)
    }

    /// Run `screen` until it is done, each key (but Ctrl-C) handed to
    /// `on_key`. The screen comes up with the first view and stays up
    /// after. A stop takes the view's quit path ([`Screen::quit`]) and
    /// ends the loop; [`Tui::settle`] waits for what it must finish.
    /// Blocking work ([`Screen::pending_work`]) runs on the normal screen,
    /// where its own progress output belongs, and the view comes back
    /// after it. The mouse and keys held with modifiers are reported only
    /// to a screen that asks for them, and events already queued (a turn
    /// of the wheel is several) are taken before the screen is drawn
    /// again. After an attach the modes are on again and the view is drawn
    /// whole.
    pub fn run_with<S: Screen>(
        &mut self,
        screen: &mut S,
        mut on_key: impl FnMut(&mut S, KeyEvent),
    ) -> io::Result<()> {
        let session = self.session;
        if session.stop_requested() {
            screen.quit();
            return Ok(());
        }
        self.up(modes_of(screen))?;
        let Some(terminal) = self.terminal.as_mut() else {
            unreachable!("the screen is up");
        };
        let mut redraw = true;
        let mut joins = JOINS.get();
        // Ticks keep time while events come: a stream of them must not
        // keep a view from its background work.
        let mut next_tick = Instant::now() + TICK;
        while !screen.done() {
            if session.stop_requested() {
                screen.quit();
                break;
            }
            if JOINS.get() != joins {
                // A view joined this screen and drew over it: drawn whole.
                joins = JOINS.get();
                forget(terminal)?;
                redraw = true;
            }
            if redraw {
                session.draw(terminal, |f| screen.render(f))?;
                let wanted = modes_of(screen);
                if wanted != self.modes {
                    session.set_modes(wanted)?;
                    self.modes = wanted;
                }
            }
            if let Some(status) = screen.status() {
                session.status(&status);
            }
            if let Some(notice) = screen.take_notice() {
                notify(&notice);
            }
            redraw = false;
            let wait = next_tick.saturating_duration_since(Instant::now());
            match session.poll(terminal, wait)? {
                Round::Event(mut event) => loop {
                    redraw |= feed(screen, &mut on_key, event);
                    if screen.done()
                        || screen.pending_work().is_some()
                        || session.stop_requested()
                        || !event::poll(Duration::ZERO)?
                    {
                        break;
                    }
                    event = event::read()?;
                },
                // An attach cleared the screen: drawn whole again.
                Round::Repainted => redraw = true,
                _ => {}
            }
            if Instant::now() >= next_tick {
                redraw |= screen.tick();
                next_tick = Instant::now() + TICK;
            }
            if let Some(message) = screen.pending_work() {
                session.off_screen(|| {
                    eprintln!("{message}");
                    screen.do_work();
                })?;
                redraw = true;
            }
        }
        Ok(())
    }

    /// After a stop ended [`Tui::run_with`]: wait, off the screen, for
    /// `screen` to be done, ticking it, so work its quit path let finish
    /// (a write under way) does. At once when it is done already.
    pub fn settle<S: Screen>(&mut self, screen: &mut S) -> io::Result<()> {
        if screen.done() {
            return Ok(());
        }
        self.work(|| {
            while !screen.done() {
                screen.tick();
                std::thread::sleep(TICK);
            }
        })
    }

    /// The screen, brought up with `modes` if it is not up yet, joined if
    /// another `Tui` has it up, else switched to them.
    fn up(&mut self, modes: Modes) -> io::Result<()> {
        match self.role.rise(self.session.screen_up()) {
            Rise::Bring => {
                (self.terminal, self.role) = (None, Role::Down);
                self.terminal = Some(self.session.terminal_with(modes)?);
                self.role = Role::Owner;
            }
            Rise::Join => {
                let (then, shown) = (self.session.modes(), crate::screen_shown());
                if !shown {
                    // Joined from work off the screen: taken back for the
                    // view, handed off again as it ends.
                    self.session.take_back()?;
                }
                self.role = Role::Guest { modes: then, shown };
                let backend = ratatui::backend::CrosstermBackend::new(io::stdout());
                let mut terminal = ratatui::Terminal::new(backend)?;
                forget(&mut terminal)?;
                self.terminal = Some(terminal);
                self.session.set_modes(modes)?;
            }
            Rise::Switch => {
                if modes != self.modes {
                    self.session.set_modes(modes)?;
                }
            }
        }
        self.modes = modes;
        Ok(())
    }

    /// Run `f`, the command's own work before, between or after its
    /// views, on the normal screen, its log records written as they come;
    /// the screen is taken back after it, to be drawn whole by the next
    /// view. A `kill` meanwhile lets `f` finish and is then
    /// [`Tui::stop_requested`]. Before the first view the screen stays
    /// down: `f` runs where it is, a `kill` waiting for it all the same.
    pub fn work<T>(&mut self, f: impl FnOnce() -> T) -> io::Result<T> {
        if !self.session.screen_up() {
            return Ok(self.session.stoppable(f));
        }
        self.session.off_screen(f)
    }
}

impl Drop for Tui<'_> {
    /// The owner restores the terminal; a guest gives the screen back to
    /// its host as it found it, to be drawn whole.
    fn drop(&mut self) {
        drop(self.terminal.take());
        match std::mem::replace(&mut self.role, Role::Down) {
            Role::Down => {}
            Role::Owner => self.session.restore(),
            Role::Guest { modes, shown } => {
                if !self.session.screen_up() {
                    return;
                }
                if modes != self.modes {
                    let _ = self.session.set_modes(modes);
                }
                if shown {
                    JOINS.set(JOINS.get().wrapping_add(1));
                } else {
                    self.session.hand_off();
                }
            }
        }
    }
}

/// Forget what `terminal` drew, clearing the screen, so the next draw
/// paints all of it; without the cursor query `Terminal::clear` makes.
fn forget(terminal: &mut ratatui::DefaultTerminal) -> io::Result<()> {
    let size = terminal.size()?;
    terminal.resize(Rect::new(0, 0, size.width, size.height))
}

/// Hand `event` to `screen`: Ctrl-C to [`Screen::interrupt`], other key
/// presses to `on_key`. Whether the screen is drawn again.
fn feed<S: Screen>(
    screen: &mut S,
    on_key: &mut impl FnMut(&mut S, KeyEvent),
    event: Event,
) -> bool {
    match event {
        Event::Key(key) if key.kind != KeyEventKind::Press => false,
        Event::Key(key)
            if key.modifiers.contains(KeyModifiers::CONTROL) && key.code == KeyCode::Char('c') =>
        {
            screen.interrupt();
            true
        }
        Event::Key(key) => {
            on_key(screen, key);
            true
        }
        Event::Mouse(m) => screen.mouse(m),
        Event::Resize(..) => true,
        _ => false,
    }
}

/// Run `screen` full screen until it is done, in a plain session of
/// `tool`'s own ([`Session::plain`]). The terminal is restored on return
/// and on panic. See [`Tui::run_with`] for the rest.
pub fn run_screen<S: Screen>(tool: &Tool, screen: &mut S) -> io::Result<()> {
    run_screen_with(tool, screen, S::handle_key)
}

/// [`run_screen`], with each key (but Ctrl-C) handed to `on_key` rather
/// than [`Screen::handle_key`]: for a caller with rules of its own about
/// which keys a screen sees.
pub fn run_screen_with<S: Screen>(
    tool: &Tool,
    screen: &mut S,
    on_key: impl FnMut(&mut S, KeyEvent),
) -> io::Result<()> {
    let session = Session::plain(tool);
    run_screen_in_with(&session, screen, on_key)
}

/// [`run_screen`] in `session`: hosted, it keeps running when the
/// connection drops and a `kill` stops it ([`Screen::quit`]). The screen
/// is handed back on return; [`Tui`] keeps it up across several views.
pub fn run_screen_in<S: Screen>(session: &Session, screen: &mut S) -> io::Result<()> {
    run_screen_in_with(session, screen, S::handle_key)
}

/// [`run_screen_in`], with each key (but Ctrl-C) handed to `on_key`.
pub fn run_screen_in_with<S: Screen>(
    session: &Session,
    screen: &mut S,
    on_key: impl FnMut(&mut S, KeyEvent),
) -> io::Result<()> {
    Tui::new(session).run_with(screen, on_key)
}

/// Title row: a reverse-video badge naming the view, then `title`, then
/// `extra` dim.
pub fn header(badge: &str, title: &str, extra: &str, look: &Look) -> Line<'static> {
    Line::from(vec![
        Span::styled(
            format!(" {badge} "),
            look.highlight().add_modifier(Modifier::REVERSED),
        ),
        Span::styled(format!(" {title}"), look.page),
        Span::styled(format!("   {extra}"), look.hint),
    ])
}

/// Title row: `title` (a [`header`]) with the session keys, `keys`
/// ([`session_keys`]), at its right end. The keys stay whole however
/// narrow the terminal, the title giving way, so the detach key is always
/// on screen.
pub fn title_bar(
    frame: &mut Frame,
    area: Rect,
    title: Line<'static>,
    keys: &[(String, String)],
    look: &Look,
) {
    let Some(keys) = keys_line(keys, look) else {
        frame.render_widget(title, area);
        return;
    };
    let width = (keys.width() as u16).min(area.width);
    let [left, right] =
        Layout::horizontal([Constraint::Fill(1), Constraint::Length(width)]).areas(area);
    frame.render_widget(title, left);
    frame.render_widget(keys, right);
}

/// Rounded panel titled `title`: the page's border and the accent title
/// when `focused`, the look's border otherwise.
pub fn panel(title: String, focused: bool, look: &Look) -> Block<'static> {
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(if focused { look.page } else { look.border })
        // Titles inherit the border style; start clear of it.
        .title(Line::from(title).style(Style::reset().patch(look.highlight())))
}

/// Help line from `(key, what it does)` pairs: each key in the accent in
/// bold, what it does dim ([`crate::widgets::key_line`]).
pub fn help_line(pairs: &[(&str, &str)], look: &Look) -> Line<'static> {
    crate::widgets::key_line(pairs, look)
}

/// Footer while typing: `prompt`, the text so far with a cursor, then keys.
pub fn input_line(prompt: &str, text: &str, keys: &[(&str, &str)], look: &Look) -> Line<'static> {
    let mut spans = vec![
        Span::styled(format!(" {prompt}"), look.page),
        Span::styled(format!("{text}▏"), look.highlight()),
        Span::raw("  "),
    ];
    spans.extend(help_line(keys, look).spans);
    Line::from(spans)
}

/// The detach and switch keys for a title row ([`title_bar`],
/// [`Framed`]), each with what it does in a word (`detach`, `switch`);
/// none when the views run plain.
pub fn session_keys() -> Vec<(String, String)> {
    crate::help_keys()
        .into_iter()
        .map(|k| (k.key.to_lowercase(), k.short))
        .collect()
}

/// The session keys as a help line; none for no keys.
fn keys_line(keys: &[(String, String)], look: &Look) -> Option<Line<'static>> {
    (!keys.is_empty()).then(|| crate::widgets::key_line(keys, look))
}

/// The session keys drawn over the right end of `area`'s top row, the
/// title there giving way, so the detach key is always on screen.
pub use crate::widgets::draw_session_keys;

thread_local! {
    /// Columns the session keys take at the right end of the title row of
    /// the view being drawn ([`Framed`]); 0 outside such a draw.
    static TITLE_KEYS: Cell<u16> = const { Cell::new(0) };

    /// The screen the command's views share while its session is up (see
    /// [`run_view`]), from the first view until [`end_views`]; out while a
    /// view runs on it.
    static VIEWS: RefCell<Option<Tui<'static>>> = const { RefCell::new(None) };

    /// How many [`Tui`]s have joined another's screen and left it, so the
    /// host draws whole what they drew over.
    static JOINS: Cell<u64> = const { Cell::new(0) };
}

/// Columns the session keys take at the right end of the title row of the
/// view being drawn in a [`Framed`], so a mark there can stand clear of
/// them; 0 when none, or outside such a draw.
pub fn title_keys_width() -> u16 {
    TITLE_KEYS.get()
}

/// A view with the session keys (`keys`, [`session_keys`]) drawn at the
/// right end of its title row, over whatever it drew there; while it
/// draws, [`title_keys_width`] says how much they take. Everything else is
/// the view's.
pub struct Framed<'a, S> {
    /// The view.
    pub screen: &'a mut S,
    /// The keys, as [`session_keys`] gives them.
    pub keys: Vec<(String, String)>,
    /// The keys' styles.
    pub look: Look,
}

impl<S: Screen> Screen for Framed<'_, S> {
    fn render(&mut self, frame: &mut Frame) {
        let keys = keys_line(&self.keys, &self.look).map_or(0, |l| l.width() as u16 + 1);
        TITLE_KEYS.set(keys);
        self.screen.render(frame);
        TITLE_KEYS.set(0);
        draw_session_keys(frame, frame.area(), &self.keys, &self.look);
    }

    fn handle_key(&mut self, key: KeyEvent) {
        self.screen.handle_key(key);
    }

    fn interrupt(&mut self) {
        self.screen.interrupt();
    }

    fn done(&self) -> bool {
        self.screen.done()
    }

    fn quit(&mut self) {
        self.screen.quit();
    }

    fn pending_work(&self) -> Option<String> {
        self.screen.pending_work()
    }

    fn do_work(&mut self) {
        self.screen.do_work();
    }

    fn tick(&mut self) -> bool {
        self.screen.tick()
    }

    fn takes_mouse(&self) -> bool {
        self.screen.takes_mouse()
    }

    fn mouse(&mut self, event: MouseEvent) -> bool {
        self.screen.mouse(event)
    }

    fn reports_chords(&self) -> bool {
        self.screen.reports_chords()
    }

    fn take_notice(&mut self) -> Option<String> {
        self.screen.take_notice()
    }

    fn status(&self) -> Option<String> {
        self.screen.status()
    }
}

/// A command's views were stopped by `kill`: the view's quit path was
/// taken (work under way finished) and nothing after the view runs. A
/// tool's `main` ends quietly on it ([`is_stopped`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stopped {
    /// The kill command, as typed: `tool kill`.
    kill: String,
}

impl Stopped {
    /// Stopped by `tool`'s kill command.
    pub fn new(tool: &Tool) -> Self {
        Self {
            kill: tool.typed(SessionCommand::Kill),
        }
    }
}

impl std::fmt::Display for Stopped {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "stopped by `{}`", self.kill)
    }
}

impl std::error::Error for Stopped {}

impl From<Stopped> for io::Error {
    fn from(s: Stopped) -> Self {
        io::Error::other(s)
    }
}

/// Whether `error`, or an error it wraps (an [`io::Error`]'s own among
/// them), is [`Stopped`]. With anyhow: `is_stopped(error.as_ref())`.
pub fn is_stopped(error: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(error);
    while let Some(e) = next {
        let inner = e
            .downcast_ref::<io::Error>()
            .and_then(io::Error::get_ref)
            .is_some_and(|i| i.is::<Stopped>());
        if e.is::<Stopped>() || inner {
            return true;
        }
        next = e.source();
    }
    false
}

/// Run `screen` full screen until it is done, in the process's session
/// ([`crate::current`]; a plain one of `tool`'s where there is none),
/// [`Framed`] with the session keys in `look`'s styles, each key (but
/// Ctrl-C) handed to `on_key`. In a session the screen stays up from the
/// first view to [`end_views`], so the command's views follow one another
/// on it. A `kill` takes the view's quit path ([`Screen::quit`]): work it
/// must finish is waited for, off the screen ([`Tui::settle`]), and then
/// the view ends with [`Stopped`].
pub fn run_view<S: Screen>(
    tool: &Tool,
    look: &Look,
    screen: &mut S,
    on_key: impl FnMut(&mut S, KeyEvent),
) -> io::Result<()> {
    let Some(session) = crate::current() else {
        let plain = Session::plain(tool);
        return run_framed(&mut Tui::new(&plain), tool, look, screen, on_key);
    };
    // Taken out while the view runs: a view started from inside another
    // finds none, and joins the screen the outer one has up as a guest
    // that never restores it.
    let mut tui = VIEWS
        .with_borrow_mut(Option::take)
        .unwrap_or_else(|| Tui::new(session));
    let shown = run_framed(&mut tui, tool, look, screen, on_key);
    keep(tui);
    shown
}

/// Keep `tui` for the next view if it owns the screen ([`Role::kept`]),
/// in place of any kept meanwhile, which goes; else let it go.
fn keep(tui: Tui<'static>) {
    let gone = if tui.role.kept() {
        VIEWS.with_borrow_mut(|views| views.replace(tui))
    } else {
        Some(tui)
    };
    drop(gone);
}

/// [`run_view`] on `tui`.
fn run_framed<S: Screen>(
    tui: &mut Tui<'_>,
    tool: &Tool,
    look: &Look,
    screen: &mut S,
    mut on_key: impl FnMut(&mut S, KeyEvent),
) -> io::Result<()> {
    let mut framed = Framed {
        screen,
        keys: session_keys(),
        look: *look,
    };
    tui.run_with(&mut framed, |f, key| on_key(f.screen, key))?;
    if !tui.stop_requested() {
        return Ok(());
    }
    tui.settle(framed.screen)?;
    Err(Stopped::new(tool).into())
}

/// Bring the views' screen ([`run_view`]) down, with the log records held
/// while it was up: before the command prints or writes on the normal
/// screen, and as it ends. A later view brings it up again.
pub fn end_views() {
    drop(VIEWS.with_borrow_mut(Option::take));
}

/// [`end_views`] as it is dropped: as a command ends, however it ends.
pub struct ViewsEnd;

impl Drop for ViewsEnd {
    fn drop(&mut self) {
        end_views();
    }
}

/// Run `f`, the command's own work between or after its views
/// ([`run_view`]; what it prints, say), on the normal screen, the views'
/// screen coming back for the next view; where no view has run, as it is.
/// A `kill` meanwhile lets `f` finish.
pub fn work<T>(f: impl FnOnce() -> T) -> io::Result<T> {
    let mut tui = VIEWS.with_borrow_mut(Option::take);
    let out = match tui.as_mut() {
        Some(t) => t.work(f),
        None => Ok(f()),
    };
    if let Some(t) = tui {
        keep(t);
    }
    out
}

#[cfg(test)]
#[path = "tests/view.rs"]
mod tests;
