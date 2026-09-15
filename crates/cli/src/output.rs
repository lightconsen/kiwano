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
/// `right` names the columns whose cells are numbers, which hug the right edge
/// of their column; everything else is left-aligned. Indices rather than a
/// per-cell flag because it is a property of the *column* — a `TOKENS` column
/// reads as a column of numbers or not at all — and because the header it
/// indexes is passed in beside it.
///
/// The table is a MySQL-shaped one: a `+---+` rule under the header and after
/// the last row, `|` between the cells. That rule is what makes a wide table
/// readable when the columns are ragged, which they are: a provider id is 20
/// columns and its billing tag is 5.
///
/// It is laid out inside the terminal when it can tell how wide that is
/// (`table_width`): the widest columns give up space, down to a floor, and cells
/// are ellipsized to fit. A pipe or a file has no width to fit, so nothing is
/// narrowed and the table comes out whole.
pub fn render_table(head: &[&str], rows: &[Vec<String>], right: &[usize]) -> String {
    render_table_within(head, rows, right, table_width())
}

/// `render_table` with the width spelled out — what the tests drive, and what
/// `COLUMNS` reaches when a shell exports it.
pub fn render_table_within(
    head: &[&str],
    rows: &[Vec<String>],
    right: &[usize],
    within: Option<usize>,
) -> String {
    let mut widths: Vec<usize> = head.iter().map(|h| display_width(h)).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(display_width(cell));
            }
        }
    }
    fit(&mut widths, within);

    // `| a | bb |` — two spaces of gutter per column plus the closing bar.
    let rule = |widths: &[usize]| {
        let mut line = String::from("+");
        for w in widths {
            line.push_str(&"-".repeat(w + 2));
            line.push('+');
        }
        line
    };
    let cells = |cells: &[String], widths: &[usize]| {
        let mut line = String::from("|");
        for (i, w) in widths.iter().enumerate() {
            let cell = cells.get(i).map(String::as_str).unwrap_or("");
            // The ellipsis is the last thing to go: a cell too wide for its
            // column (which only happens on a narrow terminal) is shortened the
            // same way the callers shorten theirs.
            let cell = ellipsize(cell, *w);
            let gap = w.saturating_sub(display_width(&cell));
            line.push(' ');
            if right.contains(&i) {
                line.push_str(&" ".repeat(gap));
                line.push_str(&cell);
            } else {
                line.push_str(&cell);
                line.push_str(&" ".repeat(gap));
            }
            line.push_str(" |");
        }
        line
    };

    let head_cells: Vec<String> = head.iter().map(|h| (*h).to_string()).collect();
    let mut out = rule(&widths);
    out.push('\n');
    out.push_str(&cells(&head_cells, &widths));
    out.push('\n');
    out.push_str(&rule(&widths));
    for row in rows {
        out.push('\n');
        out.push_str(&cells(row, &widths));
    }
    out.push('\n');
    out.push_str(&rule(&widths));
    out
}

/// How many columns the table has to fit in, when that is knowable.
///
/// `COLUMNS` first: it is what a shell, a test or a CI job can set to say "this
/// is the width", and it is the only one of the two answers that works when
/// output is not a terminal. Then the terminal itself (unix). `None` means the
/// output is a pipe or a file — which has no width, so the table is not narrowed
/// to one.
pub fn table_width() -> Option<usize> {
    if let Ok(cols) = std::env::var("COLUMNS") {
        if let Ok(n) = cols.trim().parse::<usize>() {
            if n > 0 {
                return Some(n);
            }
        }
    }
    terminal_width()
}

/// The tty's width, or `None` when stdout is not one.
#[cfg(unix)]
fn terminal_width() -> Option<usize> {
    // `ioctl(TIOCGWINSZ)` on stdout. Not in std, and the request number differs
    // per platform — which is the whole reason this goes through libc rather
    // than being spelled out by hand.
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let rc = unsafe { libc::ioctl(libc::STDOUT_FILENO, libc::TIOCGWINSZ, &mut ws) };
    (rc == 0 && ws.ws_col > 0).then_some(ws.ws_col as usize)
}

#[cfg(not(unix))]
fn terminal_width() -> Option<usize> {
    // Windows has no `COLUMNS` in practice and this build does not link the
    // console API for it, so a table there is laid out whole. `COLUMNS` still
    // works when something sets it.
    None
}

/// Narrow the columns until the table fits, taking from the widest one each
/// time (ties to the left).
///
/// From the widest rather than proportionally, because the widest column is the
/// one with room to give: a table of `ID | NAME` at 60 columns should not halve
/// a four-letter name to make room for an id nobody can read either way.
fn fit(widths: &mut [usize], within: Option<usize>) {
    /// Below this a column is not a column — an id elided to three characters
    /// tells the reader nothing, and an over-long line says more.
    const FLOOR: usize = 6;
    let Some(max) = within else { return };
    loop {
        // `| a |` per column, plus the closing `|` and two spaces of gutter.
        let total: usize = widths.iter().map(|w| w + 3).sum::<usize>() + 1;
        if total <= max {
            return;
        }
        let Some(widest) = widths
            .iter()
            .enumerate()
            .filter(|(_, w)| **w > FLOOR)
            .max_by_key(|(i, w)| (**w, std::cmp::Reverse(*i)))
            .map(|(i, _)| i)
        else {
            // Every column is at the floor: the line will be wider than the
            // terminal, which is the honest end of "make it fit".
            return;
        };
        widths[widest] -= 1;
    }
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
/// and rounding it away would be the quiet lie. The source of the `-0.0` this
/// was written for — an empty bucket list summing negatively in
/// `pricing::convert_cost_buckets` — is fixed there too; this is the edge that
/// keeps a future one from reaching a screen.
pub fn fmt_amount(v: f64, places: usize) -> String {
    let scale = 10f64.powi(places as i32);
    let v = if (v * scale).round() == 0.0 { 0.0 } else { v };
    format!("{:.*}", places, v)
}

#[cfg(test)]
mod tests {
    use super::{display_width, ellipsize, fmt_amount, render_table_within};

    /// A CJK name is twice as wide as its character count, so a table that
    /// measured in characters put everything after it one column out per
    /// character. The borders make the invariant exact: every line is the same
    /// number of display columns, wide names or not.
    #[test]
    fn wide_names_do_not_shift_the_columns_after_them() {
        let table = render_table_within(
            &["NAME", "PROTO"],
            &[
                vec!["alpha".to_string(), "openai".to_string()],
                vec!["深度求索".to_string(), "openai".to_string()],
                vec!["a".to_string(), "openai".to_string()],
            ],
            &[],
            None,
        );
        let widths: Vec<usize> = table.lines().map(display_width).collect();
        assert!(
            widths.windows(2).all(|w| w[0] == w[1]),
            "ragged lines: {widths:?}\n{table}"
        );
        // …and the wide name is what the name column was sized to: 8 display
        // columns, so its rule segment is 10 dashes.
        assert!(
            table.starts_with("+----------+"),
            "{}",
            table.lines().next().unwrap()
        );
    }

    /// A numeric column is read from its right edge: its cells and its header
    /// end at the same column. The one space before each closing bar is the
    /// gutter, so it is dropped before asking whether the cell is flush.
    #[test]
    fn numbers_hug_the_right_edge_of_their_column() {
        let table = render_table_within(
            &["PROVIDER", "REQUESTS"],
            &[
                vec!["DeepSeek".to_string(), "4".to_string()],
                vec!["Kimi".to_string(), "1024".to_string()],
            ],
            &[1],
            None,
        );
        for line in table.lines().filter(|l| l.starts_with('|')) {
            let cell = line.rsplit_once('|').unwrap().0.rsplit_once('|').unwrap().1;
            let content = &cell[..cell.len() - 1];
            assert!(
                !content.ends_with(' '),
                "a number column must be flush right: {line:?}"
            );
            // The header goes with them.
            assert!(
                content.ends_with("REQUESTS")
                    || content
                        .trim_start()
                        .starts_with(|c: char| c.is_ascii_digit() || c == 'R'),
                "{line:?}"
            );
        }
    }

    /// A narrow terminal takes from the widest column, down to a floor, and
    /// elides the cells that no longer fit — the table still fits.
    #[test]
    fn a_narrow_terminal_narrows_the_widest_columns() {
        let heads = ["ID", "ENDPOINT"];
        let rows = vec![vec![
            "api-deepseek-com-271eb4".to_string(),
            "api.deepseek.com".to_string(),
        ]];

        // Natural layout, no terminal to fit: nothing is shortened.
        let whole = render_table_within(&heads, &rows, &[], None);
        assert!(whole.contains("api-deepseek-com-271eb4"), "{whole}");

        let narrow = render_table_within(&heads, &rows, &[], Some(30));
        for line in narrow.lines() {
            assert!(
                display_width(line) <= 30,
                "{} columns: {line}",
                display_width(line)
            );
        }
        // Both columns started wide, so both gave ground; the cells that no
        // longer fit are elided rather than allowed to wrap the line.
        assert!(
            narrow.contains('…'),
            "cells past the budget are elided: {narrow}"
        );
        // Elided, not mangled: what fits of the id is still its start.
        assert!(narrow.contains("api-deep"), "{narrow}");
    }

    /// Nothing to infer a width from (a pipe, a file) means no narrowing. The
    /// table is written whole, which is what a reader of the file wants.
    #[test]
    fn a_pipe_is_not_narrowed() {
        let table = render_table_within(
            &["ID"],
            &[vec!["a-very-long-provider-id".to_string()]],
            &[],
            None,
        );
        assert!(table.contains("a-very-long-provider-id"), "{table}");
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
