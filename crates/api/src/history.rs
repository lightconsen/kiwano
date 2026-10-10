//! Session-history wire types: the rows a client's readers pull out of a coding
//! agent's own session files, for the daemon to import.
//!
//! Kiwano's gateway only ever sees the traffic that goes *through* it, so a
//! fresh install's dashboard is empty and a request row can name no project.
//! The agents themselves kept those sessions — months of them, before Kiwano
//! existed — under the user's own home, and the daemon must never touch a user
//! path (`migrate.local.md` §5 #1/#2). The *client* reads those files and sends
//! parsed rows; these are the rows.
//!
//! The alternative considered and rejected — paths in, the daemon reads the
//! files itself — fails twice: it puts the daemon on user paths, and it makes
//! each file format's parsing decisions (what a "turn" is, whether the cache
//! bucket sits inside `input_tokens`) the daemon's to own for every agent,
//! instead of the client's, where the file actually is.
//!
//! No field here is a path. `project` is a **label** a human reads, never the
//! directory a session ran in: the daemon is not told this machine's layout, and
//! a label is what the dashboard shows anyway.

use serde::{Deserialize, Serialize};

/// One metered record: a single Claude Code assistant message, or one Codex
/// turn — the unit the daemon stores against the agent's watermark.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryUsageRow {
    /// The Kiwano agent id: `"claude"` or `"codex"`.
    pub agent: String,
    /// RFC3339 UTC — the moment the message happened, not when it was read.
    pub ts: String,
    #[serde(default)]
    pub model: Option<String>,
    /// A project **label** — a directory name, never a path; see the module note.
    #[serde(default)]
    pub project: Option<String>,
    #[serde(default)]
    pub session_id: Option<String>,
    /// The idempotency key: deterministic from what the file says, so re-running
    /// an import refreshes a row instead of duplicating it.
    pub import_key: String,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// Whether the file's own `input_tokens` already counts the cache buckets.
    ///
    /// Anthropic-shaped records say `false` (their `input_tokens` excludes both
    /// cache buckets); the OpenAI family says `true` (its `input_tokens` includes
    /// the cached read). The daemon prices from this, so a row that guessed would
    /// either double-charge the cache or drop it.
    pub cache_inclusive: bool,
}

/// One named count — a tool that ran N times, or a skill invoked N times.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryCount {
    pub name: String,
    pub count: i64,
}

/// One session: what a single Claude Code transcript or Codex rollout spent, and
/// what it used.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistorySessionRow {
    /// `"claude-code:<session-uuid>"` or `"codex:<session-id>"` — the
    /// idempotency key for the session row.
    pub import_key: String,
    pub agent: String,
    #[serde(default)]
    pub project: Option<String>,
    pub session_id: String,
    /// RFC3339 UTC, the first record's timestamp.
    pub started_at: String,
    /// RFC3339 UTC, the last record's timestamp.
    pub ended_at: String,
    pub turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    #[serde(default)]
    pub tool_calls: Vec<HistoryCount>,
    #[serde(default)]
    pub skills: Vec<HistoryCount>,
}

/// Everything one `read_history` pass found. The daemon takes `usage` and
/// `sessions` wholesale and applies its own per-agent watermark — the reader
/// never does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HistoryBatch {
    #[serde(default)]
    pub usage: Vec<HistoryUsageRow>,
    #[serde(default)]
    pub sessions: Vec<HistorySessionRow>,
    /// The agents this batch is the result of scanning.
    ///
    /// Not derivable from the rows: an agent with no history produces none, and
    /// "I read it and there was nothing" is exactly the fact a client needs
    /// recorded — without it, that agent is re-read on every launch. The daemon
    /// stamps each name it is given (`Store::mark_history_scanned`).
    #[serde(default)]
    pub scanned_agents: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The batch survives a JSON round-trip, and a row with every optional field
    /// absent still parses — the daemon must not reject a row just because a
    /// reader left out a field the format had nothing for.
    #[test]
    fn a_batch_round_trips_and_a_row_tolerates_absent_optionals() {
        let row = HistoryUsageRow {
            agent: "claude".into(),
            ts: "2024-01-01T00:00:00Z".into(),
            model: Some("claude-sonnet-4-5".into()),
            project: Some("kiwano".into()),
            session_id: Some("s1".into()),
            import_key: "claude-code:s1:u1".into(),
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cache_inclusive: false,
        };
        let session = HistorySessionRow {
            import_key: "claude-code:s1".into(),
            agent: "claude".into(),
            project: Some("kiwano".into()),
            session_id: "s1".into(),
            started_at: "2024-01-01T00:00:00Z".into(),
            ended_at: "2024-01-01T00:01:00Z".into(),
            turns: 1,
            input_tokens: 10,
            output_tokens: 5,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: vec![HistoryCount {
                name: "Bash".into(),
                count: 1,
            }],
            skills: Vec::new(),
        };
        let batch = HistoryBatch {
            usage: vec![row],
            sessions: vec![session],
            scanned_agents: Vec::new(),
        };
        let json = serde_json::to_string(&batch).unwrap();
        let back: HistoryBatch = serde_json::from_str(&json).unwrap();
        assert_eq!(back, batch);

        let minimal = r#"{"usage":[{"agent":"codex","ts":"2024-01-01T00:00:00Z","import_key":"codex:s:t1","input_tokens":1,"output_tokens":2,"cache_read_tokens":0,"cache_creation_tokens":0,"cache_inclusive":true}]}"#;
        let parsed: HistoryBatch = serde_json::from_str(minimal).unwrap();
        assert_eq!(parsed.usage[0].model, None);
        assert_eq!(parsed.usage[0].project, None);
        assert_eq!(parsed.usage[0].session_id, None);
        assert!(parsed.sessions.is_empty(), "an absent list is an empty one");
    }
}
