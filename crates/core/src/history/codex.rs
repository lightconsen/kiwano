//! The Codex reader: `~/.codex/sessions/YYYY/MM/DD/rollout-*.jsonl`.
//!
//! One JSON object per line, each with `timestamp` and `type`. The first record
//! is `session_meta` and carries `payload.cwd` and `payload.session_id`;
//! `turn_context` records carry a possibly-updated `payload.cwd` and the
//! `payload.model`; tool calls are `response_item` records.
//!
//! The token records are the interesting ones. An `event_msg` whose
//! `payload.type` is `token_count` carries `payload.info.total_token_usage` and
//! `last_token_usage`, and the totals are **cumulative within the session**. The
//! reader therefore emits one usage row per *delta* between consecutive totals —
//! not one per record — because a row carrying the running total would make the
//! dashboard's spend a staircase of ever-larger numbers. It differences rather
//! than trusting `last_token_usage` because that field can arrive zeroed (it was,
//! across a whole observed 219 MB rollout).
//!
//! Token semantics are OpenAI-shaped: `input_tokens` **includes** the cached
//! bucket, so [`crate::history::HistoryUsageRow::cache_inclusive`] is `true`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

use serde_json::Value;

use super::{
    config_root, counts, i64_at, jsonl_files, project_label, str_at, HistoryRead,
    HistorySessionRow, HistoryUsageRow, ParsedFile,
};
use crate::detect::ShellVars;

/// Read every rollout under the Codex root into `read`.
pub(super) fn read(home: &Path, vars: &ShellVars, read: &mut HistoryRead) {
    let root = match config_root(home, vars, "CODEX_HOME", ".codex") {
        Ok(root) => root,
        Err(e) => {
            read.detail.push(format!("codex: {e}"));
            return;
        }
    };
    let sessions = root.join("sessions");
    if !sessions.is_dir() {
        read.detail.push(format!(
            "codex: no session directory at {}",
            sessions.display()
        ));
        return;
    }

    // `sessions/YYYY/MM/DD/rollout-*.jsonl` — four levels below `sessions`,
    // which the depth cap covers with room to spare.
    let files = jsonl_files(&sessions, 5);
    let (mut session_count, mut rows) = (0usize, 0usize);
    for file in &files {
        let stem = file
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty());
        match read_file(file, stem.as_deref()) {
            Ok(parsed) => {
                if let Some(session) = parsed.session {
                    session_count += 1;
                    rows += session.turns as usize;
                    read.batch.sessions.push(session);
                }
                read.batch.usage.extend(parsed.usage);
                if parsed.malformed > 0 {
                    read.detail.push(format!(
                        "codex: {}: {} malformed line(s) skipped",
                        file.display(),
                        parsed.malformed
                    ));
                }
            }
            Err(e) => read.skips.push(format!("codex: {}: {e}", file.display())),
        }
    }
    read.detail.push(format!(
        "codex: {} files under {} · {} sessions, {} usage rows",
        files.len(),
        sessions.display(),
        session_count,
        rows
    ));
}

/// One `total_token_usage` reading, as a difference-friendly value.
#[derive(Default, Clone, Copy)]
struct TokenTotals {
    input: i64,
    cached: i64,
    cache_write: i64,
    output: i64,
    reasoning: i64,
    total: i64,
}

impl TokenTotals {
    fn from_value(v: &Value) -> Self {
        Self {
            input: i64_at(v, "input_tokens"),
            cached: i64_at(v, "cached_input_tokens"),
            cache_write: i64_at(v, "cache_write_input_tokens"),
            output: i64_at(v, "output_tokens"),
            reasoning: i64_at(v, "reasoning_output_tokens"),
            total: i64_at(v, "total_tokens"),
        }
    }

    /// A record whose usage is all zeroes is not a turn. The totals are
    /// cumulative and start at zero, so the first record can legitimately be a
    /// zeroed one — and emitting a row of zeroes for it would invent a turn that
    /// spent nothing.
    fn is_zero(&self) -> bool {
        self.input == 0
            && self.cached == 0
            && self.cache_write == 0
            && self.output == 0
            && self.reasoning == 0
            && self.total == 0
    }

    /// Component-wise difference against the previous cumulative reading,
    /// saturating at zero: a total that went *down* is a reset, not a negative
    /// turn.
    fn delta_since(&self, prev: &Self) -> Self {
        Self {
            input: self.input.saturating_sub(prev.input),
            cached: self.cached.saturating_sub(prev.cached),
            cache_write: self.cache_write.saturating_sub(prev.cache_write),
            output: self.output.saturating_sub(prev.output),
            reasoning: self.reasoning.saturating_sub(prev.reasoning),
            total: self.total.saturating_sub(prev.total),
        }
    }

    /// Whether the delta carries a token the row reports. The row has fields for
    /// input, output (reasoning folded in) and the two cache buckets only, so a
    /// delta that is zero across all of them is not a row worth emitting — the
    /// shape a repeat reading or a components-zeroed record takes.
    fn has_reported_tokens(&self) -> bool {
        self.input != 0
            || self.output != 0
            || self.reasoning != 0
            || self.cached != 0
            || self.cache_write != 0
    }
}

/// Record a `cwd` a record carried: it is the latest, and the first one is what
/// the session's label is derived from (a `cwd` that changes mid-session rarely
/// moves the project, and the first is where the session started).
fn note_cwd(cwd: &mut Option<String>, first_cwd: &mut Option<String>, c: String) {
    if first_cwd.is_none() {
        *first_cwd = Some(c.clone());
    }
    *cwd = Some(c);
}

/// Parse one rollout. `stem` is the file name, the session-id fallback for a
/// file whose `session_meta` did not supply one.
fn read_file(file: &Path, stem: Option<&str>) -> Result<ParsedFile, String> {
    let handle = File::open(file).map_err(|e| e.to_string())?;

    let mut usage: Vec<HistoryUsageRow> = Vec::new();
    let mut tool_calls: BTreeMap<String, i64> = BTreeMap::new();
    let mut session_id = stem.map(str::to_string);
    let mut cwd: Option<String> = None;
    let mut first_cwd: Option<String> = None;
    let mut model: Option<String> = None;
    let mut prev = TokenTotals::default();
    let mut turns = 0i64;
    let (mut first_ts, mut last_ts): (Option<String>, Option<String>) = (None, None);
    let mut malformed = 0usize;

    for line in BufReader::new(handle).lines() {
        let Ok(line) = line else {
            malformed += 1;
            continue;
        };
        if line.trim().is_empty() {
            continue;
        }
        let Ok(record) = serde_json::from_str::<Value>(&line) else {
            malformed += 1;
            continue;
        };
        if !record.is_object() {
            malformed += 1;
            continue;
        }

        let ts = str_at(&record, "timestamp");
        if let Some(ts) = &ts {
            if first_ts.is_none() {
                first_ts = Some(ts.clone());
            }
            last_ts = Some(ts.clone());
        }

        let payload = record.get("payload");

        match record.get("type").and_then(Value::as_str).unwrap_or("") {
            "session_meta" => {
                if let Some(p) = payload {
                    if let Some(c) = str_at(p, "cwd") {
                        note_cwd(&mut cwd, &mut first_cwd, c);
                    }
                    if let Some(s) = str_at(p, "session_id") {
                        session_id = Some(s);
                    }
                }
            }
            "turn_context" => {
                if let Some(p) = payload {
                    if let Some(c) = str_at(p, "cwd") {
                        note_cwd(&mut cwd, &mut first_cwd, c);
                    }
                    if let Some(m) = str_at(p, "model") {
                        model = Some(m);
                    }
                }
            }
            "response_item" => {
                if let Some(p) = payload {
                    // Tool calls are function calls; `function_call_output` and
                    // message items are not calls and carry no name to count.
                    let item = p.get("type").and_then(Value::as_str).unwrap_or("");
                    if matches!(item, "function_call" | "custom_tool_call") {
                        if let Some(name) = str_at(p, "name") {
                            *tool_calls.entry(name).or_default() += 1;
                        }
                    }
                }
            }
            _ => {}
        }

        // A cumulative reading: turn it into a per-turn delta.
        let Some(total) = payload
            .and_then(|p| p.get("info"))
            .and_then(|i| i.get("total_token_usage"))
        else {
            continue;
        };
        let cur = TokenTotals::from_value(total);
        if cur.is_zero() {
            // A zeroed record: no turn, and the baseline is left where it was —
            // advancing it here would make the next real reading look smaller.
            continue;
        }
        let delta = cur.delta_since(&prev);
        prev = cur;
        if !delta.has_reported_tokens() {
            continue;
        }

        turns += 1;
        let sid = session_id.clone().unwrap_or_default();
        usage.push(HistoryUsageRow {
            agent: "codex".into(),
            ts: ts.unwrap_or_default(),
            model: model.clone(),
            // Rule 1: a label, never the `cwd` path the record carries.
            project: project_label(cwd.as_deref(), None),
            session_id: (!sid.is_empty()).then(|| sid.clone()),
            import_key: format!("codex:{sid}:turn-{turns}"),
            input_tokens: delta.input,
            // `reasoning_output_tokens` is output the model produced; the row has
            // one output field, so the two are folded rather than one dropped.
            output_tokens: delta.output + delta.reasoning,
            cache_read_tokens: delta.cached,
            cache_creation_tokens: delta.cache_write,
            cache_inclusive: true,
        });
    }

    let session = if usage.is_empty() {
        None
    } else {
        let sid = session_id.clone().unwrap_or_default();
        Some(HistorySessionRow {
            import_key: format!("codex:{sid}"),
            agent: "codex".into(),
            project: first_cwd
                .as_deref()
                .and_then(|c| project_label(Some(c), None)),
            session_id: sid,
            started_at: first_ts.unwrap_or_default(),
            ended_at: last_ts.unwrap_or_default(),
            turns,
            input_tokens: usage.iter().map(|r| r.input_tokens).sum(),
            output_tokens: usage.iter().map(|r| r.output_tokens).sum(),
            cache_read_tokens: usage.iter().map(|r| r.cache_read_tokens).sum(),
            cache_creation_tokens: usage.iter().map(|r| r.cache_creation_tokens).sum(),
            tool_calls: counts(tool_calls),
            // Codex has no `Skill` tool; the field is empty, not omitted.
            skills: Vec::new(),
        })
    };

    Ok(ParsedFile {
        session,
        usage,
        malformed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::read_history;

    /// Write `lines` as a rollout under `<home>/.codex/sessions/2024/01/02/`.
    fn write_rollout(home: &Path, file: &str, lines: &[Value]) {
        let dir = home.join(".codex").join("sessions").join("2024/01/02");
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::new();
        for line in lines {
            body.push_str(&serde_json::to_string(line).unwrap());
            body.push('\n');
        }
        std::fs::write(dir.join(file), body).unwrap();
    }

    fn session_meta(cwd: &str, sid: &str) -> Value {
        serde_json::json!({
            "timestamp": "2024-01-02T00:00:00Z",
            "type": "session_meta",
            "payload": { "cwd": cwd, "session_id": sid, "model_provider": "openai", "cli_version": "1.0" }
        })
    }

    fn turn_context(cwd: &str, model: &str) -> Value {
        serde_json::json!({
            "timestamp": "2024-01-02T00:00:01Z",
            "type": "turn_context",
            "payload": { "cwd": cwd, "model": model }
        })
    }

    fn token_count(ts: &str, total: Value) -> Value {
        serde_json::json!({
            "timestamp": ts,
            "type": "event_msg",
            "payload": { "type": "token_count", "info": { "total_token_usage": total } }
        })
    }

    fn totals(input: i64, cached: i64, cache_write: i64, output: i64, reasoning: i64) -> Value {
        let total = input + output + reasoning + cache_write;
        serde_json::json!({
            "input_tokens": input,
            "cached_input_tokens": cached,
            "cache_write_input_tokens": cache_write,
            "output_tokens": output,
            "reasoning_output_tokens": reasoning,
            "total_tokens": total
        })
    }

    fn codex_read(home: &Path) -> crate::history::HistoryRead {
        read_history(home, &ShellVars::new(), &["codex".into()])
    }

    /// Two cumulative `token_count` records produce one delta row each: the first
    /// is a delta from zero, the second the difference against it.
    #[test]
    fn two_token_counts_produce_one_delta_row_each() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_rollout(
            home,
            "rollout-a.jsonl",
            &[
                session_meta("/home/me/code", "sess-1"),
                turn_context("/tmp", "gpt-5"),
                token_count("2024-01-02T00:01:00Z", totals(100, 40, 0, 20, 5)),
                token_count("2024-01-02T00:02:00Z", totals(160, 70, 0, 50, 8)),
            ],
        );

        let read = codex_read(home);
        assert_eq!(read.batch.usage.len(), 2, "detail={:?}", read.detail);
        assert_eq!(read.batch.sessions.len(), 1);

        // Turn 1: input 100, output 20 + reasoning 5 = 25, cache read 40.
        let first = &read.batch.usage[0];
        assert_eq!(first.input_tokens, 100);
        assert_eq!(first.output_tokens, 25);
        assert_eq!(first.cache_read_tokens, 40);
        assert_eq!(first.import_key, "codex:sess-1:turn-1");
        assert!(first.cache_inclusive, "OpenAI-shaped tokens");

        // Turn 2: the difference, not the running total.
        let second = &read.batch.usage[1];
        assert_eq!(second.input_tokens, 60);
        assert_eq!(second.output_tokens, 30 + 3);
        assert_eq!(second.cache_read_tokens, 30);
        assert_eq!(second.import_key, "codex:sess-1:turn-2");

        let session = &read.batch.sessions[0];
        assert_eq!(session.turns, 2);
        assert_eq!(session.input_tokens, 160);
        assert_eq!(session.output_tokens, 25 + 33);
    }

    /// A zeroed `token_count` is not a turn: it produces no row and does not move
    /// the baseline, so the next real reading still differences against the last
    /// real total rather than double-counting.
    #[test]
    fn a_zeroed_token_count_produces_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_rollout(
            home,
            "rollout-b.jsonl",
            &[
                session_meta("/home/me/code", "sess-2"),
                turn_context("/tmp", "gpt-5"),
                token_count("2024-01-02T00:01:00Z", totals(100, 0, 0, 20, 0)),
                token_count(
                    "2024-01-02T00:02:00Z",
                    serde_json::json!({
                        "input_tokens": 0, "cached_input_tokens": 0,
                        "cache_write_input_tokens": 0, "output_tokens": 0,
                        "reasoning_output_tokens": 0, "total_tokens": 0
                    }),
                ),
                token_count("2024-01-02T00:03:00Z", totals(150, 0, 0, 40, 0)),
            ],
        );

        let read = codex_read(home);
        assert_eq!(read.batch.usage.len(), 2, "detail={:?}", read.detail);
        assert_eq!(read.batch.usage[0].input_tokens, 100);
        // The second row differences against the last *real* total (100), not the
        // zeroed record — so it is 50, not 150.
        assert_eq!(read.batch.usage[1].input_tokens, 50);
        assert_eq!(read.batch.sessions[0].turns, 2);
    }

    /// `session_meta` supplies the `cwd` and the `session_id`; the project label
    /// comes from the `cwd` (a repo directory), and the session id is the one the
    /// file named, not the file's own stem.
    #[test]
    fn session_meta_supplies_the_cwd_session_id_and_project_label() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let repo = home.join("checkout");
        std::fs::create_dir_all(repo.join(".git")).unwrap();

        write_rollout(
            home,
            "rollout-c.jsonl",
            &[
                session_meta(&repo.to_string_lossy(), "sess-real"),
                turn_context(&repo.to_string_lossy(), "gpt-5"),
                token_count("2024-01-02T00:01:00Z", totals(10, 0, 0, 5, 0)),
            ],
        );

        let read = codex_read(home);
        let session = &read.batch.sessions[0];
        assert_eq!(session.session_id, "sess-real");
        assert_eq!(session.project.as_deref(), Some("checkout"));
        assert_eq!(read.batch.usage[0].session_id.as_deref(), Some("sess-real"));
        assert_eq!(read.batch.usage[0].project.as_deref(), Some("checkout"));
    }

    /// A `response_item` function call is a tool call, counted by name; Codex has
    /// no skill concept, so `skills` stays empty.
    #[test]
    fn response_item_function_calls_are_counted_as_tool_calls() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_rollout(
            home,
            "rollout-d.jsonl",
            &[
                session_meta("/home/me/code", "sess-4"),
                turn_context("/tmp", "gpt-5"),
                serde_json::json!({
                    "timestamp": "2024-01-02T00:00:30Z", "type": "response_item",
                    "payload": { "type": "function_call", "name": "shell" }
                }),
                serde_json::json!({
                    "timestamp": "2024-01-02T00:00:31Z", "type": "response_item",
                    "payload": { "type": "function_call", "name": "shell" }
                }),
                token_count("2024-01-02T00:01:00Z", totals(10, 0, 0, 5, 0)),
            ],
        );

        let read = codex_read(home);
        let session = &read.batch.sessions[0];
        assert_eq!(
            session.tool_calls,
            vec![crate::history::HistoryCount {
                name: "shell".into(),
                count: 2
            }]
        );
        assert!(session.skills.is_empty());
    }

    /// The idempotency keys are deterministic from the file, so a re-import
    /// refreshes rather than duplicates.
    #[test]
    fn an_import_key_is_stable_across_two_runs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_rollout(
            home,
            "rollout-e.jsonl",
            &[
                session_meta("/home/me/code", "sess-5"),
                turn_context("/tmp", "gpt-5"),
                token_count("2024-01-02T00:01:00Z", totals(10, 0, 0, 5, 0)),
            ],
        );

        let first = codex_read(home);
        let second = codex_read(home);
        assert_eq!(first.batch.usage[0].import_key, "codex:sess-5:turn-1");
        assert_eq!(
            first.batch.usage[0].import_key,
            second.batch.usage[0].import_key
        );
        assert_eq!(first.batch.sessions[0].import_key, "codex:sess-5");
    }

    /// A file that cannot be read at all is a skip line, and the rest of the run
    /// continues — the shape `read_cc_switch` reports too.
    #[cfg(unix)]
    #[test]
    fn an_unreadable_file_is_a_skip_line() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let folder = home.join(".codex/sessions/2024/01/02");
        std::fs::create_dir_all(&folder).unwrap();
        // A rollout that names a file which is not there: `File::open` fails, so
        // the file is skipped rather than failing the run.
        std::os::unix::fs::symlink(
            folder.join("missing.jsonl"),
            folder.join("rollout-zzz.jsonl"),
        )
        .unwrap();

        let read = codex_read(home);
        assert_eq!(read.skips.len(), 1, "detail={:?}", read.detail);
        assert!(read.skips[0].contains("rollout-zzz.jsonl"));
        assert!(read.batch.usage.is_empty());
    }
}
