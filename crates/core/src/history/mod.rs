//! Reading coding-agent session files into rows — the client half of the
//! history import.
//!
//! The gateway only knows the traffic that goes *through* it, so a fresh
//! install's dashboard is empty. The agents' own session files hold what they
//! spent before Kiwano existed, and a project dimension the gateway can never
//! observe. The daemon must never touch a user path (`migrate.local.md` §5
//! #1/#2), so this side — which is already allowed to read this machine's files,
//! the same way [`crate::import`] reads cc-switch's — parses them and hands the
//! daemon rows. There is no store, no HTTP and no watermarking here: the daemon
//! owns all three.
//!
//! The shape mirrors [`crate::import::read_cc_switch`]: rows out, a `detail` line
//! per thing that answered, and a `skip` line per file that could not be read at
//! all — so the CLI can print detail/skips and `--json` the batch.
//!
//! Two rules hold for every reader here:
//!
//! 1. **No paths cross the interface.** A record's `cwd` is read locally to
//!    derive a project *label* (see [`project_label`]); the row carries the
//!    label only, never a path. The daemon is not told this machine's layout,
//!    and a label is what the user reads anyway.
//! 2. **Read line by line.** These files reach hundreds of MB, so nothing here
//!    `read_to_string`s a transcript. A malformed line is skipped rather than
//!    failing the file; only a file that cannot be opened at all becomes a skip.
//!
//! The readers are split by agent ([`claude_code`], [`codex`]); this module fans
//! out, owns the path table, and holds the two shared helpers the split would
//! otherwise duplicate.

use std::collections::BTreeMap;
use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

use kiwano_adapters::config::EnvDir;
use serde_json::Value;

use crate::detect::ShellVars;

pub mod claude_code;
pub mod codex;

// The row types are the daemon's wire types and live in `kiwano-api`; re-exported
// so a caller reaches them through this module (the [`crate::import`] shape).
pub use kiwano_api::history::{HistoryBatch, HistoryCount, HistorySessionRow, HistoryUsageRow};

/// How far up from a record's `cwd` [`project_label`] will look for a `.git`.
/// Bounded because the walk ends in a stat per level and a `cwd` is normally at
/// or one below the repo root — deep enough for a nested checkout, shallow
/// enough that a stray `cwd` cannot walk to `/`.
const GIT_WALK_LIMIT: usize = 8;

/// The most rows one import request carries.
///
/// The admin plane has no body limit of its own, so axum's default applies: **2
/// MiB per JSON body**. A first scan of a busy machine is far past that — a year
/// of transcripts is thousands of sessions and hundreds of thousands of messages
/// — so the batch is split here, and the sender loops. The number is a judgement
/// rather than a measurement of the wire format: a usage row serializes to a few
/// hundred bytes, and 500 of them leaves an order of magnitude of headroom for
/// the session rows that ride in the same batch.
pub const MAX_ROWS_PER_REQUEST: usize = 500;

/// Split one read into request-sized batches.
///
/// Sessions ride with the first chunk rather than in a batch of their own: they
/// are one row per session against hundreds per session, and a batch that carried
/// only sessions would be a request for the sake of a few hundred bytes.
pub fn chunks(batch: &HistoryBatch, max_rows: usize) -> Vec<HistoryBatch> {
    if batch.usage.is_empty() {
        return vec![HistoryBatch {
            usage: Vec::new(),
            sessions: batch.sessions.clone(),
            scanned_agents: batch.scanned_agents.clone(),
            source_machine: batch.source_machine.clone(),
        }];
    }
    batch
        .usage
        .chunks(max_rows.max(1))
        .enumerate()
        .map(|(i, usage)| HistoryBatch {
            usage: usage.to_vec(),
            sessions: if i == 0 {
                batch.sessions.clone()
            } else {
                Vec::new()
            },
            // Every chunk carries the same set: the daemon stamps per agent on the
            // first one and re-stamping is idempotent, and a chunk without it
            // would leave the scan looking partial.
            scanned_agents: batch.scanned_agents.clone(),
            // The machine travels with every chunk for the same reason, and it is
            // the one fact that is the *same* on all of them: one scan is one
            // machine, and a chunk that dropped it would import its rows as if
            // they came from nowhere.
            source_machine: batch.source_machine.clone(),
        })
        .collect()
}

/// What one [`read_history`] pass produced.
pub struct HistoryRead {
    pub batch: HistoryBatch,
    pub detail: Vec<String>,
    pub skips: Vec<String>,
}

impl HistoryRead {
    fn new(agents: &[String]) -> Self {
        Self {
            batch: HistoryBatch {
                usage: Vec::new(),
                sessions: Vec::new(),
                // Named at the start, not derived at the end: the point is that an
                // agent which yielded nothing is still recorded as read.
                scanned_agents: agents.to_vec(),
                // Left `None` on purpose: `read_history` reads files, and which
                // machine it did that on is the *sender's* fact to attach before
                // chunking. A test can therefore build a batch without this
                // machine's name in it.
                source_machine: None,
            },
            detail: Vec::new(),
            skips: Vec::new(),
        }
    }
}

/// Read every requested agent's session files into one batch.
///
/// `home` roots the search and `vars` answers the relocation variables
/// (`CLAUDE_CONFIG_DIR`, `CODEX_HOME`), exactly as [`crate::takeover::takeover_paths`]
/// threads them — passed in rather than read from this process's environment so
/// a test never touches the real `~/.claude` or `~/.codex`. `agents` filters
/// which readers run; an id with no reader here (every agent but these two) is
/// ignored rather than an error, because the caller passes the enabled set.
///
/// Ordering is deterministic: sessions sort by `started_at`, usage rows by
/// `(ts, import_key)`.
pub fn read_history(home: &Path, vars: &ShellVars, agents: &[String]) -> HistoryRead {
    // Only the agents a reader exists for are *read*, and only those are named in
    // the batch: the daemon stamps what it is told, so passing through a name like
    // `gemini` would record that agent as scanned when nothing looked at it.
    let read_agents: Vec<String> = agents
        .iter()
        .filter(|a| matches!(a.as_str(), "claude" | "codex"))
        .cloned()
        .collect();
    let mut read = HistoryRead::new(&read_agents);
    if agents.iter().any(|a| a == "claude") {
        claude_code::read(home, vars, &mut read);
    }
    if agents.iter().any(|a| a == "codex") {
        codex::read(home, vars, &mut read);
    }
    read.batch.sessions.sort_by(|a, b| {
        a.started_at
            .cmp(&b.started_at)
            .then_with(|| a.import_key.cmp(&b.import_key))
    });
    read.batch.usage.sort_by(|a, b| {
        a.ts.cmp(&b.ts)
            .then_with(|| a.import_key.cmp(&b.import_key))
    });
    read
}

/// One file's worth of parsing, the shape both readers return.
pub(super) struct ParsedFile {
    /// Present only when the file carried at least one usage row: a session with
    /// no tokens is not a session anyone spent anything on, and importing it
    /// would grow the dashboard's session list with rows that mean nothing.
    pub session: Option<HistorySessionRow>,
    pub usage: Vec<HistoryUsageRow>,
    /// Lines that were not JSON objects. Reported as a detail line, not a skip —
    /// the file *was* read.
    pub malformed: usize,
}

/// A named-count map → the sorted vector the wire type carries. A `BTreeMap`
/// because the wire order must not depend on hash iteration.
pub(super) fn counts(map: BTreeMap<String, i64>) -> Vec<HistoryCount> {
    map.into_iter()
        .map(|(name, count)| HistoryCount { name, count })
        .collect()
}

/// The root an agent keeps its session files under: the variable that relocates
/// it (honored only when absolute), else the default directory under `home`.
///
/// `takeover_paths` is `pub(crate)` and answers only for the configs a takeover
/// writes, so session files get their own table here. A *relative* value is
/// refused rather than fallen through: the tool resolves it against whatever
/// directory it happened to run in, so there is no root Kiwano can name — the
/// same stance [`crate::takeover::paths`] takes.
pub(super) fn config_root(
    home: &Path,
    vars: &ShellVars,
    env_var: &str,
    default_dir: &str,
) -> Result<PathBuf, String> {
    match kiwano_adapters::config::classify_dir(vars.get(env_var).map(OsStr::new)) {
        EnvDir::Unset => Ok(home.join(default_dir)),
        EnvDir::Absolute(path) => Ok(path),
        EnvDir::Relative(raw) => Err(format!(
            "{env_var} is set to `{raw}`, which is not an absolute path; the session files cannot \
             be located"
        )),
    }
}

/// Every `*.jsonl` file within `max_depth` directory levels below `dir`, sorted
/// so two runs read the same files in the same order. `max_depth == 1` means the
/// files directly in `dir`'s immediate subdirectories (Claude Code's
/// `projects/<slug>/*.jsonl`).
pub(super) fn jsonl_files(dir: &Path, max_depth: usize) -> Vec<PathBuf> {
    let mut out = Vec::new();
    collect_jsonl(dir, max_depth, &mut out);
    out.sort();
    out
}

fn collect_jsonl(dir: &Path, depth: usize, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if depth > 0 {
                collect_jsonl(&path, depth - 1, out);
            }
        } else if path.extension().and_then(OsStr::to_str) == Some("jsonl") {
            out.push(path);
        }
    }
}

/// The project a record belongs to, as a **label** and never a path.
///
/// Walk up from `cwd` (at most [`GIT_WALK_LIMIT`] levels) looking for a `.git`
/// entry — a directory in a checkout, a file in a worktree or submodule, so the
/// test is `exists`, not `is_dir`. The label is that directory's file name, so
/// the same repo yields the same label whether the record's `cwd` is the root or
/// a subdirectory of it (a `cwd` that changes mid-session does not move the
/// project). Falling through, the `cwd`'s own file name is the label; with no
/// `cwd` at all, the caller's `slug` (Claude Code's lossy project-directory
/// name) is the last resort.
///
/// A label is a file name, so it carries no path separator by construction —
/// and [`clean_label`] refuses one anyway, which is the invariant the daemon's
/// "no paths cross the interface" rule rests on.
pub(super) fn project_label(cwd: Option<&str>, slug: Option<&str>) -> Option<String> {
    if let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty()) {
        let path = Path::new(cwd);
        if path.is_absolute() {
            let mut dir = Some(path);
            for _ in 0..GIT_WALK_LIMIT {
                let Some(d) = dir else { break };
                if d.join(".git").exists() {
                    if let Some(name) = d
                        .file_name()
                        .and_then(|n| clean_label(n.to_string_lossy().as_ref()))
                    {
                        return Some(name);
                    }
                }
                dir = d.parent();
            }
            if let Some(name) = path
                .file_name()
                .and_then(|n| clean_label(n.to_string_lossy().as_ref()))
            {
                return Some(name);
            }
        }
    }
    slug.and_then(clean_label)
}

/// Trim a candidate label, or reject it. Blank is nothing to show; anything with
/// a path separator is not a label — it is the machine's layout, which rule 1
/// says does not leave this function.
fn clean_label(raw: &str) -> Option<String> {
    let label = raw.trim();
    if label.is_empty() || label.contains('/') || label.contains('\\') {
        return None;
    }
    Some(label.to_string())
}

/// A JSON string field, or `None` when absent or not a string.
pub(super) fn str_at(v: &Value, key: &str) -> Option<String> {
    v.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// A JSON integer field, or `0` when absent or not an integer. Token fields that
/// are missing are zero, which is what a file that omits them means.
pub(super) fn i64_at(v: &Value, key: &str) -> i64 {
    v.get(key).and_then(Value::as_i64).unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn home() -> tempfile::TempDir {
        tempfile::tempdir().unwrap()
    }

    /// A root that is not there is not an error: there is nothing to import, and
    /// a fresh machine has no `~/.claude` at all. The reader says so in a detail
    /// line and returns an empty batch with no skips — the distinction a caller
    /// needs to tell "nothing installed" from "something went wrong".
    #[test]
    fn a_missing_root_returns_empty_with_a_detail_line_not_an_error() {
        let dir = home();
        let read = read_history(
            dir.path(),
            &ShellVars::new(),
            &["claude".into(), "codex".into()],
        );
        assert!(read.batch.usage.is_empty());
        assert!(read.batch.sessions.is_empty());
        assert!(read.skips.is_empty(), "a missing root is not a skip");
        assert_eq!(read.detail.len(), 2, "detail={:?}", read.detail);
        assert!(read.detail.iter().any(|d| d.starts_with("claude:")));
        assert!(read.detail.iter().any(|d| d.starts_with("codex:")));
    }

    /// The variable that relocates a root is honored, and only when it is
    /// absolute: a relative value names no one place, so the root cannot be found
    /// and the reader says so rather than guessing at a path.
    #[test]
    fn a_relocated_root_is_honored_and_a_relative_one_is_refused() {
        let dir = home();
        let elsewhere = dir.path().join("elsewhere");
        fs::create_dir_all(elsewhere.join("projects").join("slug")).unwrap();
        fs::write(elsewhere.join("projects").join("slug").join("s.jsonl"), "").unwrap();

        let vars = ShellVars::from([(
            "CLAUDE_CONFIG_DIR".to_string(),
            elsewhere.to_string_lossy().into_owned(),
        )]);
        let read = read_history(dir.path(), &vars, &["claude".into()]);
        assert!(
            read.detail
                .iter()
                .any(|d| d.contains("1 files under") && d.contains("elsewhere")),
            "detail={:?}",
            read.detail
        );

        let vars = ShellVars::from([("CLAUDE_CONFIG_DIR".to_string(), "relative/dir".to_string())]);
        let read = read_history(dir.path(), &vars, &["claude".into()]);
        assert!(read.batch.usage.is_empty());
        assert!(
            read.detail
                .iter()
                .any(|d| d.contains("CLAUDE_CONFIG_DIR") && d.contains("absolute")),
            "detail={:?}",
            read.detail
        );
    }

    /// The invariant rule 1 exists for: no emitted `project` is a path. Checked
    /// over every row of a batch whose records carried real absolute `cwd`s — a
    /// repo (so the label is the repo directory), a subdirectory (same label) and
    /// a directory with no repo above it.
    #[test]
    fn no_emitted_project_is_a_path() {
        let dir = home();
        let home = dir.path();
        let repo = home.join("my-repo");
        fs::create_dir_all(repo.join(".git")).unwrap();
        let loose = home.join("loose-dir");
        fs::create_dir_all(&loose).unwrap();

        let slug = home
            .join(".claude")
            .join("projects")
            .join("-home-me-my-repo");
        fs::create_dir_all(&slug).unwrap();
        let lines = [
            serde_json::json!({
                "type": "assistant", "uuid": "u1", "timestamp": "2024-01-01T00:00:00Z",
                "sessionId": "s1", "cwd": repo.to_string_lossy(),
                "message": { "model": "m", "usage": { "input_tokens": 1, "output_tokens": 1 } }
            }),
            serde_json::json!({
                "type": "assistant", "uuid": "u2", "timestamp": "2024-01-01T00:01:00Z",
                "sessionId": "s1", "cwd": loose.to_string_lossy(),
                "message": { "model": "m", "usage": { "input_tokens": 1, "output_tokens": 1 } }
            }),
        ];
        let mut body = String::new();
        for l in &lines {
            body.push_str(&serde_json::to_string(l).unwrap());
            body.push('\n');
        }
        fs::write(slug.join("s1.jsonl"), body).unwrap();

        let read = read_history(home, &ShellVars::new(), &["claude".into()]);
        assert_eq!(read.batch.usage.len(), 2, "detail={:?}", read.detail);
        for row in &read.batch.usage {
            let project = row.project.as_deref().expect("a cwd was present");
            assert!(
                !project.contains('/') && !project.contains('\\'),
                "project leaked a path: {project}"
            );
        }
        for session in &read.batch.sessions {
            let project = session.project.as_deref().unwrap();
            assert!(
                !project.contains('/') && !project.contains('\\'),
                "session project leaked a path: {project}"
            );
        }
        // The repo's label is the repo directory, the loose cwd's its own name.
        assert_eq!(read.batch.usage[0].project.as_deref(), Some("my-repo"));
        assert_eq!(read.batch.usage[1].project.as_deref(), Some("loose-dir"));
    }
}
