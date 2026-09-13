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
pub fn render_table(head: &[&str], rows: &[Vec<String>]) -> String {
    let mut widths: Vec<usize> = head.iter().map(|h| h.chars().count()).collect();
    for row in rows {
        for (i, cell) in row.iter().enumerate() {
            if i < widths.len() {
                widths[i] = widths[i].max(cell.chars().count());
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
                Some(w) => line.push_str(&format!("{cell:<w$}")),
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

/// Shorten to `max` characters, ellipsis included.
pub fn ellipsize(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}…")
    }
}
