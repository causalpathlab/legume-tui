# legume-tui

Detachable terminal sessions for the legume tools. A TUI started through
this crate can be left (Ctrl+Alt+D, a dropped SSH connection, a closed
laptop) and come back to by starting the tool again or with `<tool>
attach`; the program, and whatever it started, keeps running in between.
Ctrl+Alt+S moves the terminal to another of the tool's sessions.

It also holds what the tools' terminal UIs share: picking the image
protocol of the attached terminal, the session subcommands and their help,
keeping log lines off the screen, running and following the child
processes a screen starts, and a test harness that drives a TUI through
pseudo-terminals.

Sessions need Unix (Linux and macOS). Elsewhere the same calls run every
TUI plain, in the terminal it started in, and the session subcommands say
that sessions are not supported there; a tool needs no `cfg` of its own.

```toml
[dependencies]
legume-tui = "0.3"
```

## Adopting it in a tool

1. `legume_tui::serve_if_host()` first thing in `main`, before any of the
   tool's own work: a session's host runs the same executable.
2. `TOOL.session("screen", &opts)` at each TUI, in place of
   `ratatui::init()`, before work the program would otherwise repeat
   (`const TOOL: legume_tui::Tool = legume_tui::Tool::new("tool")`).
   Code far from there (a worker thread) reaches the same session with
   `legume_tui::current()`. A screen the tool opens by itself, never
   hosted, takes `Session::plain(&TOOL)`.
3. Each round of the loop: `session.draw` (nothing while detached),
   `session.status` (one line for the picker and `sessions`), and
   `session.poll` or `poll_with` in place of crossterm's poll; the
   `Round` it returns says whether an event came, the time ran out, the
   screen was repainted (draw it whole) or a `kill` asked to stop.
4. `session.terminal_with(Modes::new().mouse(..))` for the screen and its modes
   (mouse, keyboard enhancement, paste, focus), and `session.restore()`
   at the end, in place of `ratatui::restore()`. Ask for the keyboard
   enhancement with `Keyboard::IfSupported(flags)` and read
   `session.keyboard_enhanced()` (Shift+Enter told from Enter) rather
   than asking the terminal again.
5. Pictures: `graphics::pick` (or `pick_protocol` for a tool's own
   choices) once the screen is up, and `graphics::reprobe` in the
   `poll_with` hook after every attach. A screen that follows another on
   the same terminal (a picker) calls `session.request_repaint()` to get
   that first probe from its own loop.
6. Children: `session.run_child(&mut command)` for a program the TUI
   hands the terminal to; `session.command(&mut command, group)` for one
   the tool waits for itself; `session.child_env()` for one started on a
   worker thread; `child::run_one` with a `child::Stopper` to run one to
   its end and follow its log and progress bars. `Group::Own` puts a
   runner's child in a process group of its own, so a stop reaches all it
   started; `Group::OwnWhenHosted` does so only while hosted. A child the
   tool polls itself goes in a `Stopper` too (`hold`, then `try_wait`,
   `id`, `stop`), so a hangup reaches it. `child::Bar::track`, fed each
   word the child says, follows its progress bar: where it stands, the
   trend of the scores it reports (`key=value`, one point a message), and
   whether it is still on the child's screen (a clear, `Said::Cleared`, or
   a log line hides it until its next frame); `child::gauge` draws it as
   text, the trend as a sparkline.
7. Work outside the screen: `session.off_screen(|| ..)` for work that
   writes to the terminal as a plain program does (the screen handed over
   while it runs, if up), and `session.stoppable(|| ..)` for work that
   does not; a `kill` waits for either.
8. Closing words: `session.leave(text)` for a run's summary, in place of
   printing it; `session.hold_error` to keep an error that ends the
   program on a detached screen until it is seen, after which the program
   says it as usual (`main` returning it).
9. The `attach`, `sessions`, `kill` and `rename` subcommands from
   `legume_tui::cli`, `cli::SessionArgs` flattened into each TUI
   subcommand, and `cli::session_help` at the end of its long help.
10. `kill` is the quit path: when `session.stop_requested()` is true (or a
   round is `Round::Stopped`), stop as the quit key does, wrapping up
   running work; a save under way finishes (`Stopper::saving`,
   `Stopper::quit`) and unsaved edits are dropped.
11. Logging: `let _quiet = legume_tui::logs::quiet();` while the screen
   is up, or the tool's stderr logger behind `logs::gate` with
   `logs::hold()` (kept and written after) or `logs::quiet_stderr()`
   (dropped there, while a pane of the tool's own still gets them).
12. Tests: `testing::Private` and `testing::Term`, with `Setup` for the
   terminal's size, the tool's working directory and its canned
   `Replies`, and `Term::screen` for what it shows.

## 0.3.5

0.3.4 was not published: its changes (below) ship in this release.

- A TUI started inside a session (a hosted program's own screen, on the
  same terminal) hears from the host whether a terminal is attached over
  a watch, which the host answers with the state now. Under load that
  word came after the TUI's first round had read nothing, so it skipped
  its first draw and then waited for a key as long as it asked (a minute,
  for some), the word unread: a blank screen until a key. The TUI now
  waits up to a second for that word as it starts, and until it comes,
  no round waits longer than a second. Later words need no wake of their
  own: an attach signals the terminal's foreground, which is that TUI.
- The demo's program that stops reading its input draws its one screen
  once a client is attached; under load its draw came first and was
  dropped, and its end-to-end test waited in vain.

## 0.3.4

- A session that cannot start (a socket path too long for the system, a
  sessions directory open to other users) said why on stderr, and the
  screen opening next covered it at once: the detach key then did
  nothing, unexplained. The reason now names the key that does not
  detach and how to run without trying (`LEGUME_TUI_NO_DETACH=1`), and
  is said again as each screen closes, after anything the program left
  (not after a panic).
- `kill` also names a labelled session by its label, after its id:
  `tool-view-1 stopped (labelled work)`, and likewise in its waiting and
  `--force` lines. The text `<id> stopped` that tools and their tests
  look for is unchanged (see "What tools may read").

## 0.3.3

- On macOS a socket whose peer has closed refuses socket options
  (EINVAL). `SO_NOSIGPIPE` was set before every send, so a send to a
  client or host that was gone failed as an invalid argument instead of a
  broken pipe; it is now set once, as each socket is connected, accepted
  or paired. A read or write timeout on such a socket is let be: a read
  there ends at once and a write fails at once. `kill` waiting for a
  session to end no longer fails when the host closes between its reads.
- On macOS `send(MSG_DONTWAIT)` on a blocking socket still waits for
  room, so a client whose host stopped reading stopped reading the keys,
  the detach key with them. The client's relay now makes its socket
  non-blocking, and waits for room (up to its timeout) only for its last
  word to the host.
- CI on Linux and macOS: rustfmt, clippy for each feature set and the
  plain build, tests, and the MSRV.

## 0.3.2

- `testing::Private::command` (and every terminal and run made from it)
  no longer passes the test runner's `RUST_BACKTRACE` and
  `RUST_LIB_BACKTRACE` on to the tool. With CI's `RUST_BACKTRACE=1`, an
  error a tool ends with was followed by a trace longer than the test
  terminal, scrolling the error off the modelled screen, so a test that
  looked for it there failed on CI only. A test that wants a backtrace
  passes it in `env`. The sessions themselves were right: the whole error
  and the end of the session reach the terminal, in order.
- `testing::Term::cooked` reads the terminal's mode through the test's
  own end of the pty. On macOS the tool's end is revoked once the tool
  (its session leader) has ended, and reading it there failed (ENOTTY).

## 0.3.1

- `help_keys()` says what each key does in 0.2's words again
  (`detach: leave it running (named on the first detach); ...` and
  `switch: leave it running and pick another session`), so a tool that
  reworded 0.3.0's phrases can show `long` as it is.
- What a program leaves (`session.leave`) when it ends with a terminal
  attached is handed to that terminal by the host, with the program's
  status, and no `<id>.leave` stays behind. A `kill` from another
  terminal no longer says words meant for the attached one, however the
  two race; ended with none attached, or with one that leaves (a key, a
  signal) before it takes them, they are kept and said once, as before.

## From 0.2

- `Session::poll` and `poll_with` return a `Round` in place of
  `Option<Event>`: `Round::Event(e)`, `Timeout`, `Repainted` (no more
  flag set in the hook to tell a repaint from a timeout) or `Stopped`.
  `round.event()` gives the `Option<Event>` back.
- `Session::command(&mut cmd, group)` takes a `Group`; `Group::Shared` is
  0.2's behaviour, and a runner's child (a tool's own `process_group(0)`)
  takes `Group::Own`.
- `Modes` and `cli::SessionHelp` may grow, so they are built in steps:
  `Modes::new().mouse(Mouse::Buttons)`, `SessionHelp::new().resume_hint(None)`
  in place of struct literals. `Modes::keyboard_enhancement` is a
  `Keyboard`: `Off`, `On(flags)` (0.2's `Some(flags)`) or
  `IfSupported(flags)`.
- `Round::Stopped` comes once a screen; `session.stop_requested()` stays
  true for a loop or a later screen to check.
- `testing::Private::term` answers the terminal's queries by default (the
  device attributes and the status report, `Replies::plain`), so a probe
  ends at once where 0.2's terminal let it time out;
  `Setup::new().replies(Replies::silent())` gives 0.2's silent terminal
  back.
- `help_keys()` gives `HelpKey { key, short, long }` in place of pairs
  split on `:`.
- `cli::SessionHelp::resume_hint` is an `Option` (None for a TUI that
  always opens on something named), beside a new `lead`; `cli::about`
  takes the `Tool` and what `kill` stops a session as (`cli::QUIT_KEY`,
  or `"ctrl+c in it"`).
- `graphics::Answer` has `keyboard_enhancement`; `Fallback` has
  `ProbeThenEnv`.
- A tool's own copies of these go: a process-wide session (`current`), a
  plain session that passes no opt-out on (`Session::plain`), the child
  runner (`child`), kitty's file or inline transport
  (`graphics::kitty_transport`), a log gate that holds records
  (`logs::gate`, `logs::hold`).

## How it works

A **host** process, off the user's login session so a hangup cannot reach
it, owns a pseudo-terminal. The TUI runs on it in a session of its own,
with `LEGUME_TUI_SESSION=<name>` in its environment. The host relays bytes
between the pty and at most one **client** over a Unix socket in a
per-user directory (mode 0700). It never changes the bytes, so kitty,
sixel and iTerm2 images reach the terminal as the program wrote them, and
it keeps no picture of the screen: on attach the program repaints. It
does follow where the output stands between escape sequences, so a new
client starts at the start of one (and of a character), told whether the
program's screen there is the alternate one. What the program writes
before its first client is ready (up to 64 KiB) is kept for that client,
so an error it ends with at the start is seen.

No lock is held across a write to a client or to the pty: each client has
a writer thread with a bounded queue, and one that stops taking output for
10 s is let go. Input a program is not reading is dropped once its queue
is full, so the host always reads the socket; the client never waits on
the host either (it holds up to 1 MiB of typed input, dropping the oldest),
so the detach key works whatever the program does. Frames are sent with `MSG_NOSIGNAL` (`SO_NOSIGPIPE` on
macOS), and the host catches SIGPIPE, so neither side dies of a peer that
vanished, whatever the tool set SIGPIPE to.

The launching process binds the session's socket itself, under a lock on
the directory, and hands it to the host it starts: two launches at once
never share a name, and a client always reaches its own host.

The client puts the user's terminal in raw mode, forwards its size (cells
and pixels) on attach and on SIGWINCH, and keeps the detach key for
itself: Ctrl+Alt+D, in the plain encoding (ESC then 0x04) and in the
extended ones (CSI-u, modifyOtherKeys). `LEGUME_TUI_DETACH_KEY` names
another (`ctrl+alt+k`, `ctrl+\\`) for a terminal or desktop that takes it;
on macOS, Alt is Option set to send Meta. `legume_tui::detach_key()` gives
the active key for a help screen. The switch key, Ctrl+Alt+S
(`LEGUME_TUI_SWITCH_KEY`, `legume_tui::switch_key()`), is found the same
way: the client leaves the session as on a detach, opens the picker on the
same terminal (the current session marked, no "start new") and attaches
to the one chosen; quitting the picker, or choosing the same session,
returns to it. On attach and on detach the client
writes a fixed reset (any half-sent sequence ended with ST and CAN, kitty
images deleted, keyboard, mouse, paste and focus modes off). It does not
enter the alternate screen itself: the program does, or the host says a
program it joins is there. It follows the screen through the output it
relays and, at the end, leaves the alternate screen only if it is in it,
so a program that fails before its screen leaves its error in sight.
SIGTERM, SIGINT, SIGQUIT and SIGHUP end it as a detach, with the terminal
restored; once it is done (in the picker, or after `attach` returns) they
end the process as by default.

Order on attach: the client says hello with its size and what of its
environment says which terminal it is, whether it is inside tmux and
whether it is reached over ssh (`legume_tui::TERMINAL_VARS`: `TERM`,
`TERM_PROGRAM`, `TERM_PROGRAM_VERSION`, `KITTY_WINDOW_ID`, `LC_TERMINAL`,
`COLORTERM`, `TMUX`, `WEZTERM_EXECUTABLE`, `KONSOLE_VERSION`,
`ITERM_SESSION_ID`, `SSH_CONNECTION`, `SSH_CLIENT`, `SSH_TTY`), goes raw and says ready; only then does the host size the pty,
make it the client, write that environment beside the socket
(`session.terminal_env(name)` reads it, so a guess made after attaching
from another terminal is that terminal's) and tell the program to repaint
and probe the terminal (so the probe's reply has a terminal to come
from). `legume_tui::attaches()` counts the attaches the program has heard
of, for a cache of what suits the terminal that is not where the loop
takes the repaint.

## When it hosts

By default, in an interactive terminal. It runs plain, as before, with:

- `--no-detach`, or `LEGUME_TUI_NO_DETACH=1`;
- `LEGUME_TUI_SESSION` set (a TUI started from a hosted one: no nested
  hosts). Such a TUI still repaints after each attach: it follows the
  host's attach state over the socket, and the host wakes the terminal's
  foreground process group, not only its own program;
- stdin or stdout not a terminal;
- inside tmux or screen.

The default lives in one function, `legume_tui::hosts_by_default()`; make it
return `false` to make hosting opt-in (`Options::detach` then asks for it).

## Using it in a tool

```rust
use legume_tui::graphics::{Fallback, Graphics, Repick};
use legume_tui::{Keyboard, Round};
const TOOL: legume_tui::Tool = legume_tui::Tool::new("tool");

fn main() -> anyhow::Result<()> {
    legume_tui::serve_if_host(); // first thing: a host does no tool work
    // ...
}

// In place of ratatui::init() / ratatui::restore():
let session = TOOL.session("view", &args.session.options(has_target))?;
let _quiet = legume_tui::logs::quiet(); // no log lines over the screen
let graphics: Graphics = args.graphics; // `--graphics`, a clap ValueEnum
let mut terminal = session.terminal_with(
    legume_tui::Modes::new()
        .mouse(legume_tui::Mouse::Buttons)
        .keyboard_enhancement(Keyboard::IfSupported(KeyboardEnhancementFlags::DISAMBIGUATE_ESCAPE_CODES)),
)?;
// Pictures start in block characters; the first repaint asks the terminal.
let mut picker = ratatui_image::picker::Picker::halfblocks();
while !quit {
    session.draw(&mut terminal, |f| app.draw(f))?; // nothing while detached
    session.status(&app.one_line_status()); // the same line is not written again
    let round = session.poll_with(&mut terminal, app.wait(), || {
        // After an attach, before the mouse is on again; the keyboard is
        // asked in the same round trip.
        match legume_tui::graphics::reprobe(&session, &picker, graphics, Fallback::Probe) {
            Repick::Same => {}
            Repick::Resend => app.encode_images_again(),
            Repick::Changed(p) => app.remake_images(&p),
        }
        Ok(())
    })?;
    match round {
        Round::Event(event) => app.handle(event),
        Round::Repainted => app.shift_enter = session.keyboard_enhanced(),
        Round::Stopped => quit = true, // `kill`: the quit path
        _ => {}
    }
}
session.restore();
// Said after the screen, or, detached, by whoever looks next.
session.leave(&app.summary());
```

`poll` waits for the next event up to the time given (at least a second
while detached). After an attach it repaints first: back into the
alternate screen, cleared, the hook called (to ask the terminal anything,
before mouse reports could land amid the answer), the session's modes on
again; then it returns `Round::Repainted` at once, for the screen to be
drawn whole. An event read as the attach came is kept for the next round.
A `kill` ends the wait at once (`Round::Stopped`, said once a screen:
later rounds wait as before, and `session.stop_requested()` stays true).
`Modes` (mouse off,
buttons or full, keyboard enhancement, bracketed paste, focus events) are
the session's: on with the screen and after every attach, off when it is
handed back and on a panic. `Keyboard::IfSupported` asks the attached
terminal (`CSI ? u`, kitty's keyboard query) as the screen comes up and
after every attach, in the graphics query's round trip when the hook
makes one (outside tmux), and turns the enhancement on only where it is
answered; `session.keyboard_enhanced()` says whether it is on.

`Session::plain(&TOOL)` opens a session that is never hosted, for a
screen the tool opens by itself: plain, or, inside a session
(`legume_tui::in_session()`), following it so it repaints after every
attach. Unlike `--no-detach`, it passes no opt-out on to the children it
starts. `legume_tui::current()` gives the first session `Tool::session`
opened in the process, to code far from where it was opened.

In the launching process `Tool::session` does not return when hosting: it
starts the host (this executable again, same arguments), relays the
terminal until the user detaches or the program ends, then exits with the
program's status. Call it before work the program would otherwise repeat.

Started again with nothing named to open (`Options::target` false, no
`--session`) while the tool has live sessions, `Tool::session` first opens a
picker: each session's name, state (detached, or attached elsewhere), age,
command line and status, and "start new". Picking one attached elsewhere
takes it over. With a target, a new session starts and a one-line hint
names the detached ones. The tool's sessions are those named
`<tool>-...`, the tool's name as `Tool::new` gives it. A session named
with `--session` that cannot be hosted (the name is taken, say) is an
error, not a plain run.

The subcommands, with the `clap` feature:

```rust
Commands::Attach(a) => legume_tui::cli::attach(&TOOL, &a)?,
Commands::Sessions => legume_tui::cli::sessions(&TOOL)?,
Commands::Kill(k) => legume_tui::cli::kill(&TOOL, &k)?,
Commands::Rename(r) => legume_tui::cli::rename(&TOOL, &r)?,
```

and `#[command(flatten)] session: legume_tui::cli::SessionArgs` on each TUI
subcommand for `--no-detach` and `--session NAME`. Their help comes with
them: `cli::session_help(&TOOL, cli::SessionHelp::new()..)` for the end of a
TUI subcommand's long help, in the tool's words (what goes on while
detached, who offers the sessions to return to, a sentence more), wrapped
to 70 columns and never inside a `` `quoted command` ``; and
`cli::about("tool")` for the long help of `attach`, `sessions`, `kill`
and `rename`.

## Ids and labels

A session's id is its socket's name, `<base>-N` (`tool-view-1`), fixed for
life. Its label, the name the user gives it, sits beside the socket in
`<id>.label`, written whole (beside it, then renamed over it) under the
directory's lock. A label is a valid name and unique among the tool's live
sessions: no other live session has it as its label or its id (in full or
after `<tool>-`). It goes with the socket: removed when the session ends,
and with a stale socket.

- `--session NAME` sets the label up front, and the id is `<base>-NAME`.
- After a detach by the key, a session with no label asks on the restored
  screen, in line mode: `Detached <id>. Name it (Enter to keep): `. Enter
  keeps it unnamed, an invalid or taken name is said and asked again, and
  Ctrl+C or the end of the input skips it. A labelled session prints
  `detached: <label>`. Nothing is asked after a lost connection, a
  takeover, a program exit, a signal, or when stdin is not a terminal.
- `r` in the picker labels the highlighted session in place, and `<tool>
  rename <session> <label>` does it from a shell.

`attach`, `kill` and `rename` find a session by its label, then by its id in
full or after `<tool>-`, then by the end of its id (`foo` for
`tool-view-foo`). `attach` and `kill` need none with one live session; a
bare `attach` with several opens the picker. `sessions` and the picker show
`label (id)`, or the id alone, then state, age, pid, command line and
status.
`kill` sends SIGTERM, which `session.stop_requested()` reports so the
program stops through its own quit path; a second SIGTERM ends it. It is
taken so while the screen is up (`session.terminal()` to
`session.restore()`), while the program waits for a child
(`session.run_child()`) and while work runs that a stop waits for
(`session.stoppable()`); elsewhere (loading, say) SIGTERM ends it at once,
as by default. A stop taken during the screen stays reported after
`restore()`. `kill` waits for the program to end, saying so after 3 s
and every 10 s, for up to a minute before it suggests `--force`; the host
holds the connection it was asked on until it ends, so the wait is a read
that ends with it.
`kill --force` sends SIGKILL to the program only, so processes it started
in their own process groups may keep running. `sessions` removes sockets
nobody listens on, asking every host at once.

A help screen lists `legume_tui::help_keys()`: the detach and switch keys
(`HelpKey { key, short, long }`: `detach` in a word and in a phrase),
none unless a session of the process offers them.

`session.leave(text)` keeps a program's closing words (a run's summary,
the error it ended with) where they will be read. Hosted, they are
written beside the socket, `<id>.leave`. The terminal attached when the
program ends says them after its screen, and only it: the host hands them
over with the program's status and the file goes once that terminal
says it took them, so a `kill` from another terminal does not say them.
With none attached, or one gone before it took them, they outlive the
session: the `kill` that ended it, the next `sessions`, `attach` or start
of the tool says `<id> ended: <text>`, once, and the file goes.
Plain, they are printed on stderr as the screen is restored.
`session.hold_error(&mut terminal, error)` makes an error that ends the
program the status (`stopped: ...`) and, on a screen nobody is attached
to, keeps it drawn until a terminal attaches and a key is pressed, or a
`kill` comes. It then returns, and the program says the error as it
would anyway (`main` returning it): a terminal attached then hears that,
and hosted with none attached it is left as by `leave`, to be said once
by whoever looks next. Either way it is said once.

`session.stoppable(|| work())` takes `kill` as a request to stop while
`work` runs, before or after the screen (a write that must finish, say):
the stop waits for it and stays asked after.

`session.run_child(&mut command)` runs a program on this terminal, in
the program's process group (it has the terminal), and waits for it, at
any point. It passes a `kill` on to the child as
SIGTERM, which a TUI child takes through its own stop path. With the
screen up, the child is handed the terminal, which is taken back after
and repainted whole; a stop that came meanwhile stays reported. Before
the screen or after it, a stop that came ends the program once the child
is gone, as `kill` does there, unless it runs inside `stoppable` or
`off_screen`, whose work the stop then waits for. The wait blocks (a thread waits for the
child) and wakes only when the child ends or a stop comes. A child the
tool starts and waits for itself gets the same environment from
`session.command(&mut command, group)`: the user's opt-out
(`LEGUME_TUI_NO_DETACH=1` when this one runs plain by `--no-detach` or
that variable). `run_child` sets it too. `session.child_env()` gives the
same as owned data (`Send`, `Clone`), for a child started on a worker
thread (`ChildEnv::apply`). `Group::Shared` keeps the child in the
program's group (it has the terminal, or should hear a ctrl+c typed while
the terminal is cooked). With `Group::Own` it always runs in a process
group of its own, which the program stops itself: a stop reaches all it
started, and nothing meant for the program (the wake after an attach, a
ctrl+c) does. `Group::OwnWhenHosted` does that only while hosted; plain,
the child stays in the program's group, so a hangup of the user's
terminal reaches it with the program. Own-group children do not hear the
hangup the program gets (its host lost, or the user's terminal): the
crate passes it on to those a `child::Stopper` holds, saves aside.

`session.off_screen(|| work())` does the same for the program's own work
that writes to the terminal as a plain program does: with the screen up,
its modes off, raw mode left and the normal screen shown while the work
runs, then the screen taken back and repainted whole. Up or not, `kill`
meanwhile asks the program to stop (`session.stop_requested()`) and waits
for the work, as `stoppable` does.

### What tools may read

The exit status of each subcommand is the first thing to go by. Beyond it,
these are kept as they are across releases, for tools, scripts and their
tests:

- `sessions`: one line per live session, tab-separated: `label (id)` or
  the id alone, then `attached` or `detached`, age, `pid N`, the command
  line and the last status; `no live <tool> sessions` when there are none.
- `kill`: a line beginning `<id> stopped` once the session has ended, its
  label, if any, after it (` (labelled <label>)`), then what the session
  left, if anything, as it left it.
- What an ended session left, said by `sessions`, `attach` or a start of
  the tool: `<id> ended: <text>`.

Other wording may change.

## Features

- `clap` (default): the `attach`, `sessions`, `kill` and `rename` handlers
  and the `SessionArgs` flags.
- `graphics` (default): `graphics::Graphics`, the `--graphics` value
  (auto, kitty, sixel, iterm2, blocks) as a clap `ValueEnum`, and
  `graphics::pick(&session, graphics, fallback)`: a protocol the user
  named is taken as it is; for auto, `Fallback::Probe` asks the terminal
  (500 ms timeout; kitty or sixel as it answers, but not from WezTerm or
  Konsole, else iTerm2 where the environment names a terminal that has
  it, as ratatui-image's own query decides) and `Fallback::Env` guesses
  from its environment (`graphics::env_guess`: `TERM`, `TERM_PROGRAM`,
  `KITTY_WINDOW_ID`, `LC_TERMINAL`, `TMUX`); block characters when
  neither finds one, while detached, or when the cell's pixel size is
  unknown. `graphics::reprobe` says what pictures made for the last
  terminal need: nothing, sent again (kitty's images, which the client's
  reset on attach deleted), or made again with a new picker.
  `graphics::pick_with` gives the terminal's answer beside the picker,
  with its background colour (OSC 11), and `graphics::query` the answer
  alone.

  Whether the attached terminal is inside tmux comes from what it passed
  on (`graphics::in_tmux(&session)`), and the query is wrapped for tmux
  by that. A picker wraps its own pictures as ratatui-image decided once,
  from this process's `TERM` and `TERM_PROGRAM`, and has no setter for
  it: when the two disagree after an attach (started outside tmux and
  attached from inside it, or the other way round), auto takes block
  characters.

  The query is the crate's own: it reads the answer on the calling thread,
  a byte at a time and only while one is waiting, so once it returns
  nothing is left reading stdin, even when the terminal never answered
  (ratatui-image's query leaves a reader thread behind that takes the next
  keys). A reply that comes after the timeout reaches the program as
  input.
  `graphics::pick_protocol(&session, wanted, fallback)` takes a tool's own
  choice of protocol (None for auto), so values of its own map onto it,
  and asks the terminal for its answer even for block characters.
  `Fallback::ProbeThenEnv` asks, and takes the environment's guess when
  no answer comes. `graphics::reprobe_with` gives the answer beside the
  repick. The query also asks for kitty's keyboard enhancement (outside
  tmux), which the session keeps (`Answer::keyboard_enhancement`).
  `graphics::kitty_transport(&session)` says how kitty's pictures reach
  the attached terminal: inline over ssh (`SSH_CONNECTION`, `SSH_CLIENT`
  or `SSH_TTY` in what it passed on), through a file otherwise.
- `logs` (default): `logs::quiet()`, a guard that turns logging off while
  the screen is up and back on when dropped (a panic unwinding past it
  drops it). `logs::gate(logger)` puts the tool's stderr logger behind a
  gate (`log::set_logger(gate)`, or a logger of the tool's own passing
  records on to it beside a pane): `logs::hold()` keeps what goes through
  it while a screen is up and writes it, in order, as the screen goes
  (handed over or restored) and when the hold ends, the latest 10,000 and
  a line saying how many older ones were dropped; `logs::quiet_stderr()`
  drops it while the guard lives, the log level untouched, so the pane
  still gets every record.
- `child` (default, Unix): `child::run_one(command, &stopper, each)` runs
  a child to its end, its stderr a pseudo-terminal so its progress bars
  draw: each frame a `Said::Progress` (`pos`, `len`, what it counts, and
  the `key=number` score in it), the rest `Said::Line`s of its log without
  colours or the logger's prefix. A `child::Stopper` interrupts it, then
  kills it, with the group it leads; `Stopper::hold` and `release` keep a
  child the tool started itself (its pipes its own) where a stop reaches
  it, and `Stopper::try_wait` and `id` let a loop poll it there (it is let
  go once it has ended). `Stopper::saving()` marks a save:
  `Stopper::quit` (the program's quit path) lets it finish, and a hangup
  is not passed on to it. Its stderr and stdout are the program's and
  close with it, so a save that must outlive a hangup writes what it has
  first, logs to a file, or bears a failed write (Rust's `eprintln!`
  panics on one).
- `map`: what the map viewers share. `map::colour`: linear light and
  sRGB by table (every byte reads back as it was), OKLCH and OKLab, label
  ink, `Categorical` palettes (`SENNA`, `PINTO_LIGHT`, `PINTO_DARK`, each
  the colours its tool drew), 256-entry `Ramp`s (`viridis`, `magma`) and
  a `diverging` colour. `map::theme::Theme`: the page, text, hint, ink and
  muted colours with the groups' palette; `PAPER` is senna's page,
  `LIGHT` and `DARK` pinto's, and `Theme::for_background` picks one for
  the terminal's background. `map::viewport::Viewport`: a centre and
  pixels per data unit, y up or down the screen (`YAxis`); `zoom_at`
  (a factor above one zooms in) and `pan_px` stay within `Limits` (a
  scale range, and `keep_inside` to stop panning past the data).
  `map::raster::Frame`: a page in linear light (`Frame::blank` in a page
  colour, or `Frame::from_srgba` over a picture in bytes, which keeps
  them), with `stamp`ed `Shape`s, `ring` and `edge`, and `keep_text` for
  a vector page. `map::points::Job` draws `PointLayer`s progressively, in
  chunks of doubling size, in the order `ranked` or `by_level` gives;
  `auto_radius`, `visible_count` and `group_medians` with it.
  `map::text`: noto bitmap `Font`s, `draw`, `draw_aligned`,
  `draw_vertical` and `TextRun`s, and `draw_tiny`, a 5×7 scale-bar font.
  `map::labels::place` sets labels centred on their anchors, dropping
  overlaps; one moved by `map::offsets::Offsets` (in line heights) goes
  first, stays inside the frame and is joined to its anchor past half a
  line, and `Hits` finds the label under a click. `OffsetBook` keeps
  offsets per layout, by label name, in one JSON file.
  `map::legend`: `draw_legend` (with `+k more` past a row limit) and
  `draw_ramp_key`. `map::pointer::Pointer` turns terminal mouse events
  over a `MapArea` into `MapAction`s in frame pixels: a drag pans (and
  goes on past the map's edge; any release ends it), the wheel zooms at
  the pointer, a press released in place clicks (adding, by
  `PointerOpts`, with the right button or held modifiers), motion hovers,
  and a press on a label drags the label (`MoveLabel`). `LabelEdit` moves
  labels from the keyboard, entered with `E`: Tab and BackTab pick a
  label, the arrows nudge it half a line (Shift: two), Backspace puts it
  back and Esc leaves. `nav_key` reads the zoom, pan and fit keys.
  `chrome::Chrome`: page, hint and menu styles from a theme, or the
  terminal's own with none, bordered popups, toasts and scrolled lists.
- `map-display`: `map` and `graphics`, for a map on the terminal:
  `map::display::cell` (frame pixels per cell), `stroke_floor` and
  `fit_protocol`, the frame fitted to an area through ratatui-image.
- `export`: a map, a chart or a grid of them as a PDF page.
  `export::write_pdf(&pages, dpi, path)` (and `pdf_bytes`) writes each
  `Page` as one raster image with its `TextRun`s set over it in Helvetica
  (WinAnsi Latin-1, AFM widths, `export::font`), centred or turned as the
  runs say, each in its own ink over a halo of its page colour.
  `export::grid_page` tiles ready frames, each titled at its bottom left,
  in the `shape` that keeps tiles largest. `PageSize` is a width in
  inches and a dpi (`WIDTHS_IN`, `DPIS` offered), its pixels for an
  aspect, the cell height text is sized for, and `check` against
  `MAX_PIXELS`. With `clap`, `PdfArgs` adds `--pdf`, `--width` (7 in)
  and `--dpi` (300).
- `testing`: `testing::Private` and `testing::Term`, a harness that drives
  a tool's TUI through pseudo-terminals in its tests, its sessions in a
  private directory killed with it. `Private::term_with(args, env, setup)`
  takes a `Setup`: the terminal's size, the tool's working directory, and
  the `Replies` the terminal gives to the program's queries (by default
  only the device attributes and the status report, so a probe ends at
  once; `Replies::plain().kitty().sixel().keyboard(flags).cell(w, h)
  .background(r, g, b)` for more, `Replies::silent()` for none).
  `Term::screen()` is what the terminal shows, modelled from the bytes
  (`vt100`), `Term::wait_screen(needle)` waits for it, and
  `Term::alternate()` says which screen it is on.

## Environment

| Variable | Meaning |
| --- | --- |
| `LEGUME_TUI_NO_DETACH` | set (not `0`): run every TUI plain |
| `LEGUME_TUI_SESSION` | set by a host in its program: the session's name |
| `LEGUME_TUI_DETACH_KEY` | the detach key in place of Ctrl+Alt+D, e.g. `ctrl+alt+k` |
| `LEGUME_TUI_SWITCH_KEY` | the switch key in place of Ctrl+Alt+S |
| `LEGUME_TUI_DIR` | socket directory in place of `$XDG_RUNTIME_DIR/legume-tui` or `$TMPDIR/legume-tui-<uid>` |

## Tests

`cargo test` runs the detach and switch key scanner, the output scanner,
the mode decision, the socket directory and labels, the client's relay
loop and label prompt (over a pty pair), the host driven over its socket
with `sh` as the program (stalled clients, a program not reading,
takeovers, watchers, lookup by label or id, SIGPIPE at its default), the
log gate (records held and capped), the child runner (bars read off a
pty, a stop and a kill reaching the group a child leads and what it
started, a save let finish, a held child polled), the test terminal's
canned replies (tmux passthroughs left unanswered), and an end-to-end run of `examples/demo.rs` through
pseudo-terminals (launch, detach, attach and repaint, a lost client, a
client stopped by a signal, nested launches, the resume picker, naming on
detach, rename, a bare attach picking, the switch key in both encodings,
other keys, kill, kill waking a screen that waits long for a key, a
nested screen that waits long for a key drawing soon after a host that
answers its watch late, kill passed on to a child run before the screen, a program that fails at once,
one slow to start its screen, one that stops reading its input, a signal
in the switch picker, what a program leaves said once by the attached
terminal (and not by a `kill` from another one), `kill`, `sessions` or
the next start, an error held on a
detached screen until it is seen and an error said once whether seen
then or later, plain or hosted, work a stop waits for, a child run inside
such work, a lost host ending the fits its program held but not a save
(which still writes what it has), and the probe reading a
terminal's canned answers off the modelled screen), through the `testing`
harness. `tests/child.rs` runs the child runner through the public API.
The terminal queries are tested over a pty pair: a terminal that never
answers leaves the next keys to the program's next read, an answer is
read to its end and no further, and the keyboard's reply is told from
the others. Graphics on real terminals are checked by hand. The plain path
(no sessions) is built and tested on Unix with
`RUSTFLAGS="--cfg legume_tui_plain"`.

Before a release:

```sh
cargo fmt --check
for f in "" --no-default-features --all-features; do
  cargo clippy --all-targets $f -- -D warnings
done
RUSTFLAGS="--cfg legume_tui_plain" cargo clippy --all-targets --all-features \
  --target-dir target/plain -- -D warnings
cargo test
RUSTFLAGS="--cfg legume_tui_plain" cargo test --all-features --target-dir target/plain
cargo publish --dry-run
```
