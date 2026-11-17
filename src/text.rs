//! Text broken to fit a width: key hints that never part a key from what
//! it does, words wrapped under a key, and lines capped with a `…`.
//!
//! senna's `wrap_keys`, `wrap_words` and `key_lines` (`src/tui/mod.rs`),
//! faba's `wrap_help` (`src/tui.rs`), and lupin's capped `wrap` and
//! `cut_words` (`src/tui/ui.rs`). Names cut by characters are
//! `chrome::short`, `middle` and `tail`; help text wrapped around quoted
//! commands is `cli::wrap`.

use ratatui::style::Style;
use ratatui::text::{Line, Span};

/// Key hints `text`, its entries three spaces apart, broken into lines of
/// at most `width` (less one) columns between entries, so a narrow
/// terminal cuts none in half.
#[must_use]
pub fn wrap_keys(text: &str, width: usize) -> Vec<String> {
    let room = width.saturating_sub(1);
    let mut lines = vec![String::new()];
    for entry in text.split("   ").filter(|e| !e.is_empty()) {
        let line = lines.last_mut().expect("a line");
        if !line.is_empty() && line.chars().count() + 3 + entry.chars().count() > room {
            lines.push(entry.to_string());
        } else {
            if !line.is_empty() {
                line.push_str("   ");
            }
            line.push_str(entry);
        }
    }
    lines
}

/// `text` broken between words (at each space) into lines of at most
/// `width` columns; a word longer than that has a line of its own.
#[must_use]
pub fn wrap_words(text: &str, width: usize) -> Vec<String> {
    let mut lines = vec![String::new()];
    for word in text.split(' ') {
        let line = lines.last_mut().expect("a line");
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width.max(1) {
            lines.push(word.to_string());
        } else {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(word);
        }
    }
    lines
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
    let mut lines: Vec<String> = Vec::new();
    for word in text.split_whitespace() {
        match lines.last_mut() {
            Some(l) if l.chars().count() + 1 + word.chars().count() <= width => {
                l.push(' ');
                l.push_str(word);
            }
            _ => lines.push(word.chars().take(width).collect()),
        }
    }
    if lines.len() > max {
        lines.truncate(max);
        let last = &mut lines[max - 1];
        let keep: String = last.chars().take(width.saturating_sub(1)).collect();
        *last = format!("{keep}…");
    }
    if lines.is_empty() {
        lines.push(String::new());
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

#[cfg(test)]
#[path = "tests/wrap.rs"]
mod tests;
