//! Text broken to fit a width: key hints that never part a key from what
//! it does, words wrapped under a key, and lines capped with a `…`.
//!
//! senna's `wrap_keys`, `wrap_words` and `key_lines` (`src/tui/mod.rs`),
//! faba's `wrap_help` (`src/tui.rs`), and lupin's capped `wrap` and
//! `cut_words` (`src/tui/ui.rs`); pinto's names cut by characters,
//! [`short`], [`middle`], [`tail`] and [`wrap`], senna's [`wrapped_rows`],
//! and faba's text progress [`bar`]. Help text wrapped around quoted
//! commands is `cli::wrap`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Paragraph, Wrap};

/// Key hints `text`, its entries three spaces apart, broken into lines of
/// at most `width` (less one) columns between entries, so a narrow
/// terminal cuts none in half.
#[must_use]
pub fn wrap_keys(text: &str, width: usize) -> Vec<String> {
    let entries = text.split("   ").filter(|e| !e.is_empty());
    wrap_on(entries, "   ", width.saturating_sub(1))
}

/// `words` joined by `sep` into lines of at most `width` columns, greedily:
/// a word goes on the line before it unless that would make it too wide;
/// a word longer than a line has a line of its own. One empty line for no
/// words.
fn wrap_on<'a>(words: impl IntoIterator<Item = &'a str>, sep: &str, width: usize) -> Vec<String> {
    let sep_w = sep.chars().count();
    let mut lines = vec![String::new()];
    for word in words {
        let line = lines.last_mut().expect("a line");
        if !line.is_empty() && line.chars().count() + sep_w + word.chars().count() > width {
            lines.push(word.to_string());
        } else {
            if !line.is_empty() {
                line.push_str(sep);
            }
            line.push_str(word);
        }
    }
    lines
}

/// `text` broken between words (at each space) into lines of at most
/// `width` columns; a word longer than that has a line of its own.
#[must_use]
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    wrap_on(text.split(' '), " ", width.max(1))
}

/// A key and what it does, as lines of at most `width` columns: the key in
/// a column `key_w` wide after `indent`, the words after it wrapped under
/// themselves (in 8 columns at least).
#[must_use]
pub fn key_lines(indent: usize, key: &str, key_w: usize, what: &str, width: usize) -> Vec<String> {
    let lead = indent + key_w + 1;
    let pad = key_w.saturating_sub(key.chars().count());
    wrap_words(what, width.saturating_sub(lead).max(8))
        .into_iter()
        .enumerate()
        .map(|(i, l)| {
            if i == 0 {
                format!("{}{key}{} {l}", " ".repeat(indent), " ".repeat(pad))
            } else {
                format!("{}{l}", " ".repeat(lead))
            }
        })
        .collect()
}

/// A footer `line` as lines no wider than `width`: a lead, then each key
/// (a span drawn in `key`) with the spans after it, never parted from
/// them. Lines after the first start with a space. A line that fits is
/// left as it is.
#[must_use]
pub fn wrap_help(line: Line<'static>, width: u16, key: Style) -> Vec<Line<'static>> {
    let width = usize::from(width);
    if line.width() <= width {
        return vec![line];
    }
    let mut groups: Vec<Vec<Span<'static>>> = vec![Vec::new()];
    for span in line.spans {
        if span.style == key {
            groups.push(Vec::new());
        }
        groups.last_mut().expect("a group").push(span);
    }
    let mut out = Vec::new();
    let mut row: Vec<Span<'static>> = Vec::new();
    let mut used = 0;
    for group in groups {
        let w: usize = group.iter().map(Span::width).sum();
        if used + w > width && used > 1 {
            out.push(Line::from(std::mem::take(&mut row)));
            row.push(Span::raw(" "));
            used = 1;
        }
        used += w;
        row.extend(group);
    }
    out.push(Line::from(row));
    out
}

/// `text` in lines of at most `width` characters, broken between words
/// (any whitespace), at most `max` of them: the last ends in `…` when some
/// are cut, and a word longer than a line is cut to it. One empty line for
/// no words; none for `max` 0.
#[must_use]
pub fn wrap_at_most(text: &str, width: usize, max: usize) -> Vec<String> {
    if max == 0 {
        return Vec::new();
    }
    let mut lines: Vec<String> = wrap_on(text.split_whitespace(), " ", width)
        .into_iter()
        .map(|l| l.chars().take(width).collect())
        .collect();
    if lines.len() > max {
        lines.truncate(max);
        let last = &mut lines[max - 1];
        let keep: String = last.chars().take(width.saturating_sub(1)).collect();
        *last = format!("{keep}…");
    }
    lines
}

/// `text` in `width` columns: as it is when it fits, else the words that
/// fit before a `…`, or its first word cut when not even that one does.
#[must_use]
pub fn cut_words(text: &str, width: usize) -> String {
    if text.chars().count() <= width {
        return text.to_string();
    }
    let mut kept = String::new();
    for word in text.split_whitespace() {
        let with = if kept.is_empty() {
            word.to_string()
        } else {
            format!("{kept} {word}")
        };
        if with.chars().count() + 1 > width {
            break;
        }
        kept = with;
    }
    if kept.is_empty() {
        kept = text.chars().take(width.saturating_sub(1)).collect();
    }
    format!("{kept}…")
}

/// How many rows `text` takes wrapped at word boundaries to `width`
/// columns, as a wrapping paragraph lays it out; a word longer than a row
/// is broken across rows of its own.
#[must_use]
pub fn wrapped_rows(text: &str, width: usize) -> usize {
    // ratatui's own count, so a leading space and wide characters take
    // the columns the paragraph gives them.
    Paragraph::new(text)
        .wrap(Wrap { trim: false })
        .line_count(u16::try_from(width.max(1)).unwrap_or(u16::MAX))
        .max(1)
}

/// `s` in at most `n` characters, `…` marking a cut at the end.
#[must_use]
pub fn short(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(n.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

/// `s` in at most `n` characters, `…` marking a cut in the middle: the
/// start and the end kept, so names sharing a prefix still differ.
#[must_use]
pub fn middle(s: &str, n: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    if chars.len() <= n {
        return s.to_string();
    }
    let end = n.saturating_sub(1) / 2;
    let start = n.saturating_sub(1) - end;
    let mut out: String = chars[..start].iter().collect();
    out.push('…');
    out.extend(&chars[chars.len() - end..]);
    out
}

/// The last `n` characters of `s`, `…` marking a cut at the start.
#[must_use]
pub fn tail(s: &str, n: usize) -> String {
    let len = s.chars().count();
    if len <= n {
        s.to_string()
    } else {
        let mut out = String::from("…");
        out.extend(s.chars().skip(len + 1 - n.max(1)));
        out
    }
}

/// `s` in pieces of at most `first` characters, then `rest` for each
/// piece after the first; none for an empty `s`.
#[must_use]
pub fn wrap(s: &str, first: usize, rest: usize) -> Vec<String> {
    let chars: Vec<char> = s.chars().collect();
    let (head, tail) = chars.split_at(first.max(1).min(chars.len()));
    std::iter::once(head)
        .filter(|h| !h.is_empty())
        .chain(tail.chunks(rest.max(1)))
        .map(|c| c.iter().collect())
        .collect()
}

/// How many of `width` cells a bar shows filled for `done` of `total`:
/// whole cells, so a bar is full only when it is done.
#[must_use]
pub fn filled(done: u64, total: u64, width: usize) -> usize {
    match total {
        0 => 0,
        _ => (width as u128 * u128::from(done.min(total)) / u128::from(total)) as usize,
    }
}

/// A `width`-cell bar for `done` of `total` as text: `█` filled, `░` not.
#[must_use]
pub fn bar(done: u64, total: u64, width: usize) -> String {
    let full = filled(done, total, width);
    format!("{}{}", "█".repeat(full), "░".repeat(width - full))
}

#[cfg(test)]
#[path = "tests/wrap.rs"]
mod tests;
