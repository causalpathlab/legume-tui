# legume-tui

Detachable terminal sessions for the legume tools. A TUI started through
this crate can be left (Ctrl+Alt+D, a dropped SSH connection, a closed
laptop) and come back to by starting the tool again or with `<tool>
attach`; the program, and whatever it started, keeps running in between.
Ctrl+Alt+S moves the terminal to another of the tool's sessions.

Unix only (Linux and macOS). Not published; used as a path dependency while
piloting.

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

Order on attach: the client says hello with its size, goes raw and says
ready; only then does the host size the pty, make it the client and tell
the program to repaint and probe the terminal (so the probe's reply has a
terminal to come from).

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
use legume_tui::graphics::Repick;
const TOOL: legume_tui::Tool = legume_tui::Tool::new("tool");

fn main() -> anyhow::Result<()> {
    legume_tui::serve_if_host(); // first thing: a host does no tool work
    // ...
}

// In place of ratatui::init() / ratatui::restore():
let session = TOOL.session("view", &args.session.options(has_target))?;
let mut terminal = session.terminal_with(legume_tui::Modes {
    mouse: legume_tui::Mouse::Buttons,
    ..Default::default()
})?;
// Pictures start in block characters; the first repaint asks the terminal.
let mut picker = ratatui_image::picker::Picker::halfblocks();
while !quit && !session.stop_requested() {
    session.draw(&mut terminal, |f| app.draw(f))?; // nothing while detached
    session.status(&app.one_line_status()); // the same line is not written again
    let event = session.poll_with(&mut terminal, app.wait(), || {
        // After an attach, before the mouse is on again.
        match legume_tui::graphics::reprobe(&session, &picker, forced) {
            Repick::Same => {}
            Repick::Resend => app.encode_images_again(),
            Repick::Changed(p) => app.remake_images(&p),
        }
        Ok(())
    })?;
    if let Some(event) = event {
        app.handle(event);
    }
}
session.restore();
```

`poll` waits for the next event up to the time given (at least a second
while detached). After an attach it repaints first: back into the
alternate screen, cleared, the hook called (to ask the terminal anything,
before mouse reports could land amid the answer), the session's modes on
again; then it returns at once, for the screen to be drawn whole. A
`kill` ends the wait at once. `Modes` (mouse off, buttons or full,
keyboard enhancement, bracketed paste, focus events) are the session's:
on with the screen and after every attach, off when it is handed back
and on a panic.

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
them: `cli::SESSION_HELP` for a TUI subcommand's long help, and
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
`session.restore()`) and while the program waits for a child
(`session.run_child()`); elsewhere (loading, say) SIGTERM ends it at once,
as by default. A stop taken during the screen stays reported after
`restore()`. `kill` waits for the program to end, saying so after 3 s
and every 10 s, for up to a minute before it suggests `--force`; the host
holds the connection it was asked on until it ends, so the wait is a read
that ends with it.
`kill --force` sends SIGKILL to the program only, so processes it started
in their own process groups may keep running. `sessions` removes sockets
nobody listens on, asking every host at once.

A help screen lists `legume_tui::help_keys()`: the detach and switch keys
with what they do, none unless a session of the process offers them.

`session.run_child(&mut command)` runs a program on this terminal and
waits for it, at any point. It passes a `kill` on to the child as
SIGTERM, which a TUI child takes through its own stop path. With the
screen up, the child is handed the terminal, which is taken back after
and repainted whole; a stop that came meanwhile stays reported. Before
the screen or after it, a stop that came ends the program once the child
is gone, as `kill` does there. The wait blocks (a thread waits for the
child) and wakes only when the child ends or a stop comes. A child the
tool starts and waits for itself gets the same environment from
`session.command(&mut command)`: the user's opt-out
(`LEGUME_TUI_NO_DETACH=1` when this one runs plain by `--no-detach` or
that variable). `run_child` sets it too.

## Features

- `clap` (default): the `attach`, `sessions`, `kill` and `rename` handlers
  and the `SessionArgs` flags.
- `graphics` (default): `graphics::probe`, which asks the attached terminal
  for its image protocol with a 500 ms timeout and falls back to block
  characters (also while detached); `graphics::probe_as`, with a protocol
  the user forced; and `graphics::reprobe`, which says what pictures made
  for the last terminal need: nothing, sent again (kitty's images, which
  the client's reset on attach deleted), or made again with a new picker.
- `testing`: `testing::Private` and `testing::Term`, a harness that drives
  a tool's TUI through pseudo-terminals in its tests, its sessions in a
  private directory killed with it.

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
takeovers, watchers, lookup by label or id, SIGPIPE at its default), and
an end-to-end run of `examples/demo.rs` through pseudo-terminals (launch,
detach, attach and repaint, a lost client, a client stopped by a signal,
nested launches, the resume picker, naming on detach, rename, a bare
attach picking, the switch key in both encodings, other keys, kill, kill
waking a screen that waits long for a key, kill passed on to a child run
before the screen, a program that fails at once, one slow to start its
screen, one that stops reading its input, a signal in the switch picker),
through the `testing` harness.
Graphics on real terminals are checked by hand.
