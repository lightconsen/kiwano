//! Where command output goes.
//!
//! Two rules, and they are the whole design:
//!
//! - **stdout carries the payload and nothing else.** Under `--json` that means
//!   exactly one JSON document, so `kiwano --json providers add … | jq` works.
//!   The previous client printed its diagnostics to stdout, which put
//!   `gateway route table reloaded (1 agents)` after the JSON and broke every
//!   pipe that read it.
//! - **Diagnostics go to stderr, always.** Including under `--json`: putting an
//!   `{"error": …}` object on stdout would make a partial-success pipeline
//!   ambiguous about which document it was reading. The exit code is the
//!   machine-readable failure channel.
//!
//! There is deliberately no formatter registry and no `Display` impl per view
//! model. A command either emits a serializable value — with a closure for the
//! text rendering — or prints lines.

use std::fmt::Display;
use std::io::Write;

use serde::Serialize;

pub struct Out<'a> {
    stdout: &'a mut dyn Write,
    stderr: &'a mut dyn Write,
    json: bool,
    quiet: bool,
}

impl<'a> Out<'a> {
    pub fn new(
        stdout: &'a mut dyn Write,
        stderr: &'a mut dyn Write,
        json: bool,
        quiet: bool,
    ) -> Self {
        Self {
            stdout,
            stderr,
            json,
            quiet,
        }
    }

    pub fn json(&self) -> bool {
        self.json
    }

    /// The payload: the value as JSON under `--json`, the closure's text
    /// otherwise.
    ///
    /// Only the text branch runs the closure, so a command can keep its table
    /// rendering next to itself and never pay for it in JSON mode.
    pub fn emit<T: Serialize>(&mut self, value: &T, text: impl FnOnce() -> String) {
        if self.json {
            match serde_json::to_string_pretty(value) {
                Ok(json) => {
                    let _ = writeln!(self.stdout, "{json}");
                }
                // A serialize failure is a bug in a view model, not user input.
                // Saying so beats printing nothing.
                Err(e) => {
                    let _ = writeln!(self.stderr, "error: cannot serialize output: {e}");
                }
            }
        } else {
            let _ = writeln!(self.stdout, "{}", text());
        }
    }

    /// A payload line. stdout.
    pub fn line(&mut self, msg: impl Display) {
        let _ = writeln!(self.stdout, "{msg}");
    }

    /// A diagnostic. stderr, and suppressed by `--quiet`.
    pub fn note(&mut self, msg: impl Display) {
        if !self.quiet {
            let _ = writeln!(self.stderr, "{msg}");
        }
    }

    /// A failure report. stderr, never suppressed.
    pub fn error(&mut self, msg: impl Display) {
        let _ = writeln!(self.stderr, "error: {msg}");
    }
}

/// A table whose columns are sized to their content.
///
/// A free function rather than a method because the JSON branch of
/// [`Out::emit`] takes a closure that builds this string — the rendering has to
/// be available without the sink.
///
/// Content-derived widths rather than the fixed `{:<24}` fields the old client
/// used, which truncated long provider ids and padded short ones.
///
/// Widths are display columns, not `char`s: a CJK provider name is twice as
/// wide as its character count suggests, and counting characters shifted every
/// column after the first such name.
pub fn render_table(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = head.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
    }
    fn pad(cells: &[String], widths: &[usize]) -> String {
        let mut line = String::new();
        for (i, cell) in cells.iter().enumerate() {
            if i > 0 {
                line.push(' ');
            }
            match widths.get(i) {
                Some(w) => line.push_str(&pad_to(cell, *w)),
                None => line.push_str(cell),
            }
        }
        line.trim_end().to_string()
    }

    let head_cells: Vec<String> = head.iter().map(|h| (*h).to_string()).collect();
    let mut out = pad(&head_cells, &widths);
    for row in rows {
        out.push('\n');
        out.push_str(&pad(row, &widths));
    }
    out
}

/// Shorten to `max` display columns, ellipsis included.
///
/// A wide character that would straddle the budget is dropped whole rather than
/// split: half a CJK glyph is a mojibake column, not a truncation.
pub fn ellipsize(s: &str, max: usize) -> String {
    if display_width(s) <= max {
        return s.to_string();
    }
    let budget = max.saturating_sub(1);
    let mut out = String::new();
    let mut used = 0;
    for c in s.chars() {
        let w = char_width(c);
        if used + w > budget {
            break;
        }
        used += w;
        out.push(c);
    }
    out.push('…');
    out
}

/// `s` padded with spaces to `width` display columns. Never truncates: a value
/// wider than the column keeps its width and pushes the rest of the line, which
/// is what makes an over-long name visible instead of silently cut.
pub fn pad_to(s: &str, width: usize) -> String {
    let mut out = String::with_capacity(s.len() + width.saturating_sub(display_width(s)));
    out.push_str(s);
    for _ in display_width(s)..width {
        out.push(' ');
    }
    out
}

/// How many terminal columns `s` occupies.
///
/// The ranges below are the ones a provider name, model id or currency can
/// carry — East Asian wide/fullwidth forms, emoji, and the combining marks that
/// take no column — not a full UAX #11 table, which is what a dependency would
/// buy for the handful of user-authored strings the CLI prints.
pub fn display_width(s: &str) -> usize {
    s.chars().map(char_width).sum()
}

fn char_width(c: char) -> usize {
    match c as u32 {
        // Combining marks, zero-width spaces, joiners and variation selectors.
        0x0300..=0x036F
        | 0x1AB0..=0x1AFF
        | 0x200B..=0x200F
        | 0x20D0..=0x20FF
        | 0xFE00..=0xFE0F
        | 0xFE20..=0xFE2F => 0,
        // Wide and fullwidth forms: the CJK blocks, Hangul, Hiragana/Katakana,
        // fullwidth Latin, and the emoji planes.
        0x1100..=0x115F
        | 0x2E80..=0x303E
        | 0x3041..=0x33FF
        | 0x3400..=0x4DBF
        | 0x4E00..=0x9FFF
        | 0xA000..=0xA4CF
        | 0xAC00..=0xD7A3
        | 0xF900..=0xFAFF
        | 0xFE10..=0xFE19
        | 0xFE30..=0xFE6F
        | 0xFF00..=0xFF60
        | 0xFFE0..=0xFFE6
        | 0x1F300..=0x1F64F
        | 0x1F900..=0x1F9FF
        | 0x20000..=0x3FFFD => 2,
        _ => 1,
    }
}

/// A money amount at a fixed precision, printed without a sign once it is zero
/// at that precision.
///
/// `{:.4}` prints the sign of whatever `f64` it is handed, and a zero can
/// arrive negative — a sum of nothing is `-0.0` in Rust, and a rounded-down
/// remainder can land a hair below zero. `-0.0000` in a column of costs reads
/// as a bug because it looks like one, so a zero is printed as one.
/// A genuinely negative amount keeps its sign: that is a fact about the data,
/// and rounding it away would be the quiet lie.
pub fn fmt_amount(v: f64, places: usize) -> String {
    let scale = 10f64.powi(places as i32);
    let v = if (v * scale).round() == 0.0 { 0.0 } else { v };
    format!("{:.*}", places, v)
}

#[cfg(test)]
mod tests {
    use super::{display_width, ellipsize, fmt_amount, render_table};

    /// A CJK name is twice as wide as its character count, so a table that
    /// measured in characters put everything after it one column out per
    /// character.
    #[test]
    fn wide_names_do_not_shift_the_columns_after_them() {
        let table = render_table(
            &["NAME", "PROTO"],
            &[
                vec!["alpha".to_string(), "openai".to_string()],
                vec!["深度求索".to_string(), "openai".to_string()],
                vec!["a".to_string(), "openai".to_string()],
            ],
        );
        // Every line, wide name or not, starts its last column at column 9.
        for line in table.lines() {
            let last = line.rsplit(' ').next().unwrap();
            let start = display_width(line) - display_width(last);
            assert_eq!(start, 9, "misaligned: {line:?}");
        }
    }

    #[test]
    fn display_width_counts_columns_not_characters() {
        assert_eq!(display_width("alpha"), 5);
        assert_eq!(display_width("深度求索"), 8);
        assert_eq!(display_width("é"), 1);
        assert_eq!(display_width("e\u{0301}"), 1, "a combining mark takes none");
    }

    /// Truncation is to columns too, and never splits a wide glyph in half.
    #[test]
    fn ellipsize_measures_columns() {
        assert_eq!(ellipsize("alpha-beta-gamma", 8), "alpha-b…");
        assert_eq!(ellipsize("aaaaaaaa", 8), "aaaaaaaa");
        assert_eq!(ellipsize("深度求索深度求索", 5), "深度…");
        // Three glyphs plus the ellipsis fill the seven columns exactly; the
        // fourth would need eight.
        assert_eq!(ellipsize("深度求索", 7), "深度求…");
    }

    /// `-0.0` prints as `-0.0000`, and so does a cost a hair below zero —
    /// neither is a number anyone wants to read in a column of costs.
    #[test]
    fn a_zero_cost_is_printed_without_a_sign() {
        assert_eq!(fmt_amount(-0.0, 4), "0.0000");
        assert_eq!(fmt_amount(-1e-9, 4), "0.0000");
        assert_eq!(fmt_amount(0.0, 4), "0.0000");
        assert_eq!(fmt_amount(0.03029999, 4), "0.0303");
        // A real negative amount keeps its sign: rounding that away would be a
        // lie about the data, not a formatting choice.
        assert_eq!(fmt_amount(-0.5, 4), "-0.5000");
        assert_eq!(fmt_amount(-0.000051, 4), "-0.0001");
    }
}
