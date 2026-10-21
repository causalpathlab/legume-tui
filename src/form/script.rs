//! The command a run made, as a shell script beside its outputs
//! (`{out}.cmd.sh`), so it can be read, edited and run again; and the same
//! command laid out for a preview, one flag to a line.
//!
//! faba's (`src/script.rs`) and senna's (`src/run/tui/script.rs`) scripts
//! in one: the program overridable from the shell (`$FABA`, `$SENNA`), an
//! output whose presence means the run finished (the script then refuses
//! to run), and the run started from the script's folder when asked.

use std::io::Write;
use std::path::Path;

/// `word` as one shell word: as it is when nothing in it is special, else
/// in single quotes.
#[must_use]
pub fn quote(word: &str) -> String {
    let plain = !word.is_empty()
        && word
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c));
    if plain {
        word.to_string()
    } else {
        format!("'{}'", word.replace('\'', r"'\''"))
    }
}

/// A command line as one shell line.
#[must_use]
pub fn line(argv: &[String]) -> String {
    argv.iter().map(|w| quote(w)).collect::<Vec<_>>().join(" ")
}

/// The program `name` as a script calls it, overridable from the shell:
/// `"${COCOA:-cocoa}"`.
#[must_use]
pub fn program_word(name: &str) -> String {
    let var: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    format!("\"${{{var}:-{name}}}\"")
}

/// Whether a word is a flag: `-[A-Za-z]` or `--[A-Za-z]...`.
fn is_flag(w: &str) -> bool {
    let mut c = w.chars();
    match (c.next(), c.next(), c.next()) {
        (Some('-'), Some(x), None) => x.is_ascii_alphabetic(),
        (Some('-'), Some('-'), Some(x)) => x.is_ascii_alphabetic(),
        _ => false,
    }
}

/// `program` and the first `head` words of `argv` on the first line, each
/// input after them on its own line, then one flag and all its values per
/// line.
#[must_use]
pub fn command_lines(program: &str, argv: &[String], head: usize) -> Vec<String> {
    let mut lines = vec![program.to_string()];
    // A flag has started a line: the words that follow, up to the next
    // flag, are its values.
    let mut in_flag = false;
    for (k, w) in argv.iter().enumerate() {
        let flag = is_flag(w);
        let add_to_last = k < head || (!flag && in_flag);
        match lines.last_mut() {
            Some(last) if add_to_last => {
                last.push(' ');
                last.push_str(&quote(w));
            }
            _ => {
                lines.push(quote(w));
                in_flag |= flag;
            }
        }
    }
    lines
}

/// How a script is made.
#[derive(Clone, Copy, Debug)]
pub struct Script<'a> {
    /// The program's name: the script calls it as [`program_word`] does.
    pub program: &'a str,
    /// Its version, said in the script's head.
    pub version: &'a str,
    /// The command that made the script: `cocoa diff`, `senna run`.
    pub made_by: &'a str,
    /// The script's file name, for its "run it again" line.
    pub name: &'a str,
    /// An output whose presence means the run finished: the script then
    /// refuses to run.
    pub guard: Option<&'a str>,
    /// Whether it runs from its own folder.
    pub cd: bool,
    /// Variables it exports unless the shell has them: `("RUST_LOG", "info")`.
    pub env: &'a [(&'a str, &'a str)],
    /// How many words of the command go on the program's line.
    pub head: usize,
}

/// The script running `argv` (without the program).
#[must_use]
pub fn text(script: &Script, argv: &[String]) -> String {
    let mut s = String::new();
    s.push_str("#!/usr/bin/env bash\n");
    s.push_str(&format!(
        "# Made by `{}` ({} {}). Run it again with: bash {}\n",
        script.made_by,
        script.program,
        script.version,
        quote(script.name)
    ));
    s.push_str("set -euo pipefail\n");
    if script.cd {
        s.push_str("cd \"$(dirname \"$0\")\"\n");
    }
    for (var, value) in script.env {
        s.push_str(&format!("export {var}=\"${{{var}:-{value}}}\"\n"));
    }
    if let Some(guard) = script.guard {
        let guard = quote(guard);
        s.push_str(&format!("if [ -e {guard} ]; then\n"));
        s.push_str(&format!(
            "  echo {} >&2\n",
            quote(&format!("{guard} exists; move it away to run again"))
        ));
        s.push_str("  exit 1\nfi\n");
    }
    let program = program_word(script.program);
    s.push_str(&command_lines(&program, argv, script.head).join(" \\\n  "));
    s.push('\n');
    s
}

/// Write the script for `argv` to `path`, executable, never over an
/// existing file.
pub fn write(path: &Path, script: &Script, argv: &[String]) -> std::io::Result<()> {
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o755);
    }
    let mut f = opts
        .open(path)
        .map_err(|e| std::io::Error::new(e.kind(), format!("{}: {e}", path.display())))?;
    f.write_all(text(script, argv).as_bytes())
}

#[cfg(test)]
#[path = "../tests/script.rs"]
mod tests;
