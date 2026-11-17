//! The command a run made, as a shell script beside its outputs
//! (`{out}.cmd.sh`), so it can be read, edited and run again; and the same
//! command laid out for a preview, one flag to a line.
//!
//! faba's (`src/script.rs`) and senna's (`src/run/tui/script.rs`) scripts
//! in one: the program overridable from the shell (`$FABA`, `$SENNA`), an
//! output whose presence means the run finished (the script then refuses
//! to run), and the run started from the script's folder when asked, or
//! from a folder above it; senna's output prefix as `$out`, and its
//! guard on `${out}.<result>`.

use std::io::Write;
use std::path::{Component, Path};

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
    lay_out(program, argv, head, None)
}

/// [`command_lines`], the output prefix written as `"$out"` where it is
/// the value of its flag: `(flag, prefix)` as two words (`--out "$out"`) or
/// as one (`--out="$out"`). Any other value of the flag stays as it is.
fn lay_out(program: &str, argv: &[String], head: usize, out: Option<(&str, &str)>) -> Vec<String> {
    let mut lines = vec![program.to_string()];
    // A flag has started a line: the words that follow, up to the next
    // flag, are its values.
    let mut in_flag = false;
    for (k, w) in argv.iter().enumerate() {
        let flag = is_flag(w);
        let add_to_last = k < head || (!flag && in_flag);
        let word = match out {
            Some((flag, prefix)) if k > 0 && argv[k - 1] == flag && w == prefix => {
                "\"$out\"".to_string()
            }
            Some((flag, prefix)) if w.strip_prefix(flag) == Some(&format!("={prefix}")) => {
                format!("{flag}=\"$out\"")
            }
            _ => quote(w),
        };
        match lines.last_mut() {
            Some(last) if add_to_last => {
                last.push(' ');
                last.push_str(&word);
            }
            _ => {
                lines.push(word);
                in_flag |= flag;
            }
        }
    }
    lines
}

/// How many folders a relative output prefix goes down (`runs/r` and
/// `./runs/r`: 1; `r`, `./r` and an absolute prefix: 0): how far a script
/// written beside its outputs ([`Script::up`]) climbs to where the run
/// started. `.` is no folder and `..` climbs back one, by the words alone.
///
/// `None` when the prefix climbs above where the run started (`../r`): the
/// script, beside it, could only get back down by the start folder's name,
/// which the prefix does not say. Write such a prefix absolute, or leave
/// the script to run from where it is started.
#[must_use]
pub fn depth(out: &str) -> Option<usize> {
    let out = Path::new(out);
    let Some(parent) = out.parent().filter(|_| out.is_relative()) else {
        return Some(0);
    };
    parent.components().try_fold(0usize, |down, c| match c {
        Component::Normal(_) => Some(down + 1),
        Component::ParentDir => down.checked_sub(1),
        _ => Some(down),
    })
}

/// How a script is made: [`Script::new`], then what it adds.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
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
    /// How many folders above its own it runs from (implies `cd`).
    pub up: usize,
    /// Variables it exports unless the shell has them: `("RUST_LOG", "info")`.
    pub env: &'a [(&'a str, &'a str)],
    /// How many words of the command go on the program's line.
    pub head: usize,
    /// The output prefix, set as `out=...` and written `"$out"` where it
    /// is the value of [`out_flag`](Self::out_flag) (`--out r` or
    /// `--out=r`).
    pub out: Option<&'a str>,
    /// The flag whose value is the output prefix: `--out`.
    pub out_flag: &'a str,
    /// What a finished run leaves at `${out}.` (`senna.json`): a file
    /// whose presence makes the script refuse to run. Read only with
    /// [`out`](Self::out).
    pub result: Option<&'a str>,
}

impl<'a> Script<'a> {
    /// A script `name` running `program` at `version`, made by `made_by`:
    /// from wherever it is started, nothing exported, no guard, every
    /// word after the program on a line of its own.
    #[must_use]
    pub const fn new(program: &'a str, version: &'a str, made_by: &'a str, name: &'a str) -> Self {
        Self {
            program,
            version,
            made_by,
            name,
            guard: None,
            cd: false,
            up: 0,
            env: &[],
            head: 0,
            out: None,
            out_flag: "--out",
            result: None,
        }
    }

    /// Refuses to run while `guard` exists.
    #[must_use]
    pub const fn guard(self, guard: &'a str) -> Self {
        Self {
            guard: Some(guard),
            ..self
        }
    }

    /// Runs from its own folder.
    #[must_use]
    pub const fn cd(self, cd: bool) -> Self {
        Self { cd, ..self }
    }

    /// Runs from `levels` folders above its own: from where the run
    /// started, for a script beside an output `levels` deep ([`depth`],
    /// which is `None` for an output above that folder: no `up` reaches it).
    #[must_use]
    pub const fn up(self, levels: usize) -> Self {
        Self {
            up: levels,
            cd: true,
            ..self
        }
    }

    /// Exports `env` unless the shell has them.
    #[must_use]
    pub const fn env(self, env: &'a [(&'a str, &'a str)]) -> Self {
        Self { env, ..self }
    }

    /// `head` words of the command on the program's line.
    #[must_use]
    pub const fn head(self, head: usize) -> Self {
        Self { head, ..self }
    }

    /// The output prefix as the shell variable `out`, the value of
    /// [`out_flag`](Self::out_flag) written `"$out"`.
    #[must_use]
    pub const fn out(self, prefix: &'a str) -> Self {
        Self {
            out: Some(prefix),
            ..self
        }
    }

    /// The flag whose value is the output prefix, in place of `--out`.
    #[must_use]
    pub const fn out_flag(self, flag: &'a str) -> Self {
        Self {
            out_flag: flag,
            ..self
        }
    }

    /// Refuses to run while `${out}.{ending}` exists ([`out`](Self::out)).
    #[must_use]
    pub const fn result(self, ending: &'a str) -> Self {
        Self {
            result: Some(ending),
            ..self
        }
    }

    /// The command as the script writes it, for a preview: the program
    /// word, then [`command_lines`]' layout, the output prefix `"$out"`.
    #[must_use]
    pub fn command_lines(&self, argv: &[String]) -> Vec<String> {
        let out = self.out.map(|prefix| (self.out_flag, prefix));
        lay_out(&program_word(self.program), argv, self.head, out)
    }
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
    if script.cd || script.up > 0 {
        s.push_str(&format!(
            "cd \"$(dirname \"$0\")\"{}\n",
            "/..".repeat(script.up)
        ));
    }
    for (var, value) in script.env {
        s.push_str(&format!("export {var}=\"${{{var}:-{value}}}\"\n"));
    }
    if let Some(out) = script.out {
        s.push_str(&format!("out={}\n", quote(out)));
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
    if let (Some(_), Some(ending)) = (script.out, script.result) {
        let result = format!("${{out}}.{ending}");
        s.push_str(&format!("if [ -f \"{result}\" ]; then\n"));
        s.push_str(&format!(
            "  echo \"{result} exists; move it away to run again\" >&2\n"
        ));
        s.push_str("  exit 1\nfi\n");
    }
    s.push_str(&script.command_lines(argv).join(" \\\n  "));
    s.push('\n');
    s
}

/// Write the script for `argv` to `path`, executable, never over an
/// existing file.
pub fn write(path: &Path, script: &Script, argv: &[String]) -> std::io::Result<()> {
    put(path, script, argv, false)
}

/// Write the script for `argv` to `path`, executable, over the one there
/// (a script rewritten each run).
pub fn write_over(path: &Path, script: &Script, argv: &[String]) -> std::io::Result<()> {
    put(path, script, argv, true)
}

fn put(path: &Path, script: &Script, argv: &[String], over: bool) -> std::io::Result<()> {
    let named =
        |e: std::io::Error| std::io::Error::new(e.kind(), format!("{}: {e}", path.display()));
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true);
    if over {
        opts.create(true).truncate(true);
    } else {
        opts.create_new(true);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o755);
    }
    let mut f = opts.open(path).map_err(named)?;
    f.write_all(text(script, argv).as_bytes()).map_err(named)?;
    #[cfg(unix)]
    if over {
        // The mode above is only a new file's.
        use std::os::unix::fs::PermissionsExt;
        f.set_permissions(std::fs::Permissions::from_mode(0o755))
            .map_err(named)?;
    }
    Ok(())
}

#[cfg(test)]
#[path = "../tests/script.rs"]
mod tests;
