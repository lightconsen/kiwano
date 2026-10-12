//! The Claude Code reader: `~/.claude/projects/<slug>/*.jsonl`.
//!
//! One JSON object per line. `type` names the record — `assistant` and `user`
//! carry content, but a transcript also holds `mode`, `summary`,
//! `file-history-snapshot` and others — and only an assistant record carries
//! `message.usage`. The `cwd` is a real absolute path and it **changes within a
//! session** (records appear for subdirectories of one repo), so the project
//! label is derived per record and must land on the same directory regardless
//! (see [`super::project_label`]). The directory name is a lossy slug of the
//! project path (`/` → `-`), used only as a fallback.
//!
//! Token semantics are Anthropic-shaped: `message.usage.input_tokens` **excludes**
//! the two cache buckets, so [`crate::history::HistoryUsageRow::cache_inclusive`]
//! is `false`.

use std::collections::BTreeMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};

use serde_json::Value;

use super::{
    config_root, counts, i64_at, jsonl_files, project_label, session_title, str_at, HistoryRead,
    HistorySessionRow, HistoryUsageRow, ParsedFile,
};
use crate::detect::ShellVars;

/// Read every transcript under the Claude Code root into `read`.
pub(super) fn read(home: &Path, vars: &ShellVars, read: &mut HistoryRead) {
    let root = match config_root(home, vars, "CLAUDE_CONFIG_DIR", ".claude") {
        Ok(root) => root,
        Err(e) => {
            read.detail.push(format!("claude: {e}"));
            return;
        }
    };
    let projects = root.join("projects");
    if !projects.is_dir() {
        read.detail.push(format!(
            "claude: no session directory at {}",
            projects.display()
        ));
        return;
    }

    // `projects/<slug>/*.jsonl`: one file per session, named by its uuid.
    let files = jsonl_files(&projects, 1);
    let (mut sessions, mut rows, mut spawned) = (0usize, 0usize, 0usize);
    for file in &files {
        let slug = file
            .parent()
            .and_then(Path::file_name)
            .map(|n| n.to_string_lossy().into_owned());
        let stem = file
            .file_stem()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|s| !s.is_empty());
        // The session's **subagents** live beside it: Claude Code gives the
        // session a directory named after itself and writes one transcript per
        // spawned agent under `subagents/`. Those are not sessions of their own —
        // they are turns of this one, and on a machine that uses agents they are
        // a fifth of the usage rows (measured: 30,988 of 146,566 here). Read them
        // into the same session, or the ledger is short by everything the
        // subagents spent.
        let subagents = match (file.parent(), stem.as_deref()) {
            (Some(dir), Some(stem)) => jsonl_files(&dir.join(stem).join("subagents"), 0),
            _ => Vec::new(),
        };
        spawned += subagents.len();
        match read_file(file, slug.as_deref(), stem.as_deref(), &subagents) {
            Ok(parsed) => {
                if let Some(session) = parsed.session {
                    sessions += 1;
                    rows += session.turns as usize;
                    read.batch.sessions.push(session);
                }
                read.batch.usage.extend(parsed.usage);
                if parsed.malformed > 0 {
                    read.detail.push(format!(
                        "claude: {}: {} malformed line(s) skipped",
                        file.display(),
                        parsed.malformed
                    ));
                }
            }
            Err(e) => read.skips.push(format!("claude: {}: {e}", file.display())),
        }
    }
    read.detail.push(format!(
        "claude: {} transcripts + {} subagent files under {} · {} sessions, {} usage rows",
        files.len(),
        spawned,
        projects.display(),
        sessions,
        rows
    ));
}

/// Parse one session: the transcript itself, and the subagent transcripts that
/// belong to it.
///
/// `slug` is the project directory's name, the label fallback for a record with
/// no `cwd`; `stem` is the file name, the session id fallback for a record with
/// no `sessionId` (the file is named by the session). `subagents` are that
/// session's spawned-agent transcripts — folded into the same accumulators, so
/// one `ParsedFile` is one session whatever number of files it took to record.
fn read_file(
    file: &Path,
    slug: Option<&str>,
    stem: Option<&str>,
    subagents: &[PathBuf],
) -> Result<ParsedFile, String> {
    // Opened here rather than inside the loop: a transcript that cannot be read
    // is a skip for the whole session, which is what this `?` reports.
    let handle = File::open(file).map_err(|e| e.to_string())?;
    // The session's own file first, then its subagents: everything below
    // accumulates across all of them, so one `ParsedFile` is one session.
    let sources = std::iter::once((file, BufReader::new(handle)))
        .chain(subagents.iter().filter_map(|f| {
            // A subagent file that cannot be read is not a reason to lose the
            // session: its transcript is still there, and the loss is bounded to
            // what that one spawned agent spent.
            File::open(f).ok().map(|h| (f.as_path(), BufReader::new(h)))
        }))
        .collect::<Vec<_>>();

    let mut usage: Vec<HistoryUsageRow> = Vec::new();
    // The tool-call vs skill rule: a `tool_use` block named `Skill` is a skill
    // invocation, and it is counted in `skills` **only** — `tool_calls` is the
    // real tools. A skill is not a tool the model reached for generically; it is
    // a named capability of its own, and listing it in both would double-count
    // it in any "tools used" total the dashboard draws.
    let mut tool_calls: BTreeMap<String, i64> = BTreeMap::new();
    let mut skills: BTreeMap<String, i64> = BTreeMap::new();
    // **The file's name is the session's identity, and the ids inside it are
    // not.** Claude Code mints a new `sessionId` when a conversation is compacted
    // or resumed, and keeps appending to the file it started — so a transcript
    // can hold records from several ids while its name never changes. Keying rows
    // by what the records say makes a session's identity depend on where the file
    // happened to end, which moves under the ledger between imports: this machine
    // had seven session rows left behind by exactly that, each a fragment of a
    // conversation that was already listed under another id.
    //
    // The record's own id is a fallback for a file with no usable name, which is
    // the only case where there is nothing better to use.
    let mut session_id = stem.map(str::to_string);
    // The two title candidates, and the agent's own name wins: Claude Code
    // rewrites `aiTitle` as the conversation develops (a long session carries
    // dozens of them), so the **last** one is the settled name and the earlier
    // ones are drafts. The opening user message is the fallback for a transcript
    // old enough to predate the record, or one whose title never landed.
    let mut ai_title: Option<String> = None;
    let mut opening_message: Option<String> = None;
    // The label from a real `cwd` wins over the slug fallback, so a transcript
    // whose first record has no `cwd` still names its project honestly.
    let mut cwd_label: Option<String> = None;
    let (mut first_ts, mut last_ts): (Option<String>, Option<String>) = (None, None);
    let mut malformed = 0usize;

    // `file_idx` is not decoration: a record without a `uuid` falls back to its
    // line number as an idempotency key, and the same line number in two files
    // would import as the same row — the second silently overwriting the first.
    for (file_idx, (_, reader)) in sources.into_iter().enumerate() {
        for (line_no, line) in reader.lines().enumerate() {
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

            // Across every file of the session, and by value rather than by
            // position: the subagent transcripts are read after the session's
            // own, so "the last record seen" is not "the latest moment". RFC3339
            // UTC strings order lexicographically — the same comparison every
            // `ts` filter in the store makes.
            if let Some(ts) = str_at(&record, "timestamp") {
                if first_ts.as_deref().is_none_or(|f| ts.as_str() < f) {
                    first_ts = Some(ts.clone());
                }
                if last_ts.as_deref().is_none_or(|l| ts.as_str() > l) {
                    last_ts = Some(ts);
                }
            }
            if session_id.is_none() {
                session_id = str_at(&record, "sessionId");
            }
            let cwd = str_at(&record, "cwd");
            if cwd_label.is_none() {
                cwd_label = project_label(cwd.as_deref(), None);
            }

            if let Some(t) = str_at(&record, "aiTitle").filter(|t| !t.trim().is_empty()) {
                ai_title = Some(t);
            }

            let message = record.get("message");
            // The opening message: the first `user` record that carries text. Its
            // content is either a string or the block array the API uses, and a
            // record whose content is an image or a tool result contributes nothing.
            if opening_message.is_none()
                && record.get("type").and_then(Value::as_str) == Some("user")
                && !is_scaffolding(&record)
            {
                opening_message = message
                    .and_then(|m| m.get("content"))
                    .and_then(first_text)
                    .map(str::to_string);
            }

            // A record with no `usage` is not a spend: it contributes no row. The
            // `user` records and every bookkeeping record land here.
            if let Some(usage_v) = message
                .and_then(|m| m.get("usage"))
                .filter(|u| u.is_object())
            {
                // The session's own file keeps the bare `line-N` spelling its
                // rows already carry; a subagent's fallback is scoped to its file,
                // because two files' line 5 are two different rows.
                let uuid = str_at(&record, "uuid").unwrap_or_else(|| match file_idx {
                    0 => format!("line-{}", line_no + 1),
                    i => format!("sub{i}-line-{}", line_no + 1),
                });
                let sid = session_id.clone().unwrap_or_default();
                usage.push(HistoryUsageRow {
                    agent: "claude".into(),
                    ts: str_at(&record, "timestamp").unwrap_or_default(),
                    model: message
                        .and_then(|m| m.get("model"))
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    // Rule 1: a label, never the `cwd` path the record carries.
                    project: project_label(cwd.as_deref(), slug),
                    session_id: (!sid.is_empty()).then(|| sid.clone()),
                    import_key: format!("claude-code:{sid}:{uuid}"),
                    input_tokens: i64_at(usage_v, "input_tokens"),
                    output_tokens: i64_at(usage_v, "output_tokens"),
                    cache_read_tokens: i64_at(usage_v, "cache_read_input_tokens"),
                    cache_creation_tokens: i64_at(usage_v, "cache_creation_input_tokens"),
                    cache_inclusive: false,
                });
            }

            if let Some(blocks) = message
                .and_then(|m| m.get("content"))
                .and_then(Value::as_array)
            {
                for block in blocks {
                    if block.get("type").and_then(Value::as_str) != Some("tool_use") {
                        continue;
                    }
                    let Some(name) = str_at(block, "name") else {
                        continue;
                    };
                    let bucket = if name == "Skill" {
                        &mut skills
                    } else {
                        &mut tool_calls
                    };
                    *bucket.entry(name).or_default() += 1;
                }
            }
        }
    }

    let session = if usage.is_empty() {
        None
    } else {
        let sid = session_id.clone().unwrap_or_default();
        Some(HistorySessionRow {
            import_key: format!("claude-code:{sid}"),
            agent: "claude".into(),
            project: cwd_label.or_else(|| slug.and_then(super::clean_label)),
            title: session_title(ai_title.as_deref().or(opening_message.as_deref())),
            session_id: sid,
            started_at: first_ts.unwrap_or_default(),
            ended_at: last_ts.unwrap_or_default(),
            turns: usage.len() as i64,
            input_tokens: usage.iter().map(|r| r.input_tokens).sum(),
            output_tokens: usage.iter().map(|r| r.output_tokens).sum(),
            cache_read_tokens: usage.iter().map(|r| r.cache_read_tokens).sum(),
            cache_creation_tokens: usage.iter().map(|r| r.cache_creation_tokens).sum(),
            tool_calls: counts(tool_calls),
            skills: counts(skills),
        })
    };

    Ok(ParsedFile {
        session,
        usage,
        malformed,
    })
}

/// Whether a `user` record is Claude Code talking to itself rather than the human
/// talking to the model.
///
/// Real transcripts are full of these — a `/clear` echo, a compaction caveat, a
/// local command's output — and they are written as `user` records whose content
/// reads like a message. Taking one as a session's opening message produces
/// titles like `<local-command-caveat>Caveat: The messages below were…`, which is
/// how a real run found this: no synthetic fixture had them.
///
/// Two signals, and either is enough. `isMeta` is Claude Code's own marker for
/// "not a real user turn". The tags are the wrappers it writes around the ones it
/// marks nothing on — a closed set of its own vocabulary, matched on the prefix,
/// so that a genuine message which merely *starts* with `<` (pasted markup) is
/// still kept.
fn is_scaffolding(record: &Value) -> bool {
    if record.get("isMeta").and_then(Value::as_bool) == Some(true) {
        return true;
    }
    let text = record
        .get("message")
        .and_then(|m| m.get("content"))
        .and_then(first_text)
        .unwrap_or_default()
        .trim_start();
    [
        "<command-name>",
        "<command-message>",
        "<local-command-caveat>",
        "<local-command-stdout>",
        "<local-command-stderr>",
    ]
    .iter()
    .any(|tag| text.starts_with(tag))
}

/// The text of a `user` message's content: a plain string, or the first text
/// block of the array form. Anything else (an image, a tool result) is not what
/// the user said and is not a title.
fn first_text(content: &Value) -> Option<&str> {
    if let Some(s) = content.as_str() {
        return Some(s).filter(|s| !s.trim().is_empty());
    }
    content
        .as_array()?
        .iter()
        .find(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .and_then(|b| b.get("text"))
        .and_then(Value::as_str)
        .filter(|s| !s.trim().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::history::read_history;

    /// Write `lines` as a JSONL transcript under `<home>/.claude/projects/<slug>/`.
    fn write_transcript(home: &Path, slug: &str, file: &str, lines: &[Value]) {
        let dir = home.join(".claude").join("projects").join(slug);
        std::fs::create_dir_all(&dir).unwrap();
        let mut body = String::new();
        for line in lines {
            body.push_str(&serde_json::to_string(line).unwrap());
            body.push('\n');
        }
        std::fs::write(dir.join(file), body).unwrap();
    }

    fn assistant(
        uuid: &str,
        ts: &str,
        sid: &str,
        cwd: &str,
        usage: Value,
        content: Value,
    ) -> Value {
        serde_json::json!({
            "type": "assistant",
            "uuid": uuid,
            "timestamp": ts,
            "sessionId": sid,
            "cwd": cwd,
            "message": { "model": "claude-sonnet-4-5", "usage": usage, "content": content }
        })
    }

    fn claude_read(home: &Path) -> crate::history::HistoryRead {
        read_history(home, &ShellVars::new(), &["claude".into()])
    }

    /// The session is the **file**, not the ids inside it.
    ///
    /// A transcript carries records from several `sessionId`s once its
    /// conversation has been compacted or resumed — the file keeps the name it
    /// was created with — and this reader used to take its identity from
    /// whatever id the last record happened to carry. On a real machine that left
    /// seven session rows behind, each a fragment of a conversation already
    /// listed under another id, and each looking like a session that had lost its
    /// numbers.
    #[test]
    fn a_session_is_its_file_not_the_last_id_inside_it() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let started_as = "11111111-2222-3333-4444-555555555555";
        let after_compaction = "99999999-8888-7777-6666-555555555555";

        write_transcript(
            home,
            "-home-me-work-acme",
            &format!("{started_as}.jsonl"),
            &[
                assistant(
                    "r1",
                    "2026-01-02T10:00:00Z",
                    started_as,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 10, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
                // The compaction happens and the ids after it are new…
                assistant(
                    "r2",
                    "2026-01-02T11:00:00Z",
                    after_compaction,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 20, "output_tokens": 2}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
            ],
        );

        let read = claude_read(home);
        assert_eq!(read.batch.sessions.len(), 1, "one file is one session");
        assert_eq!(
            read.batch.sessions[0].session_id, started_as,
            "the file's name is the identity, not the id the file ends on"
        );
        // And every row belongs to it — including the ones recorded after the id
        // changed, which is what used to split a session's numbers in two.
        assert_eq!(read.batch.usage.len(), 2);
        assert!(
            read.batch
                .usage
                .iter()
                .all(|r| r.session_id.as_deref() == Some(started_as)),
            "{:?}",
            read.batch
                .usage
                .iter()
                .map(|r| r.session_id.clone())
                .collect::<Vec<_>>()
        );
        assert_eq!(read.batch.sessions[0].turns, 2);
        assert_eq!(read.batch.sessions[0].input_tokens, 30);
    }

    /// A session's subagent transcripts are **the same session**: their usage
    /// belongs to it, they do not become sessions of their own, and they do not
    /// each contribute a row for the same turns.
    ///
    /// This is the shape Claude Code writes — `projects/<slug>/<session>.jsonl`
    /// beside `projects/<slug>/<session>/subagents/agent-*.jsonl` — and on this
    /// machine those files hold a fifth of all usage rows. Reading only the first
    /// left the ledger short by everything the spawned agents spent.
    #[test]
    fn subagent_transcripts_fold_into_their_session() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let sid = "11111111-2222-3333-4444-555555555555";

        write_transcript(
            home,
            "-home-me-work-acme",
            &format!("{sid}.jsonl"),
            &[assistant(
                "r1",
                "2026-01-02T10:00:00Z",
                sid,
                "/home/me/work/acme",
                serde_json::json!({"input_tokens": 10, "output_tokens": 1}),
                serde_json::json!([{"type": "tool_use", "name": "Task"}]),
            )],
        );
        // Two agents it spawned, each with its own usage — one of them later in
        // time than anything in the session's own file.
        write_transcript(
            home,
            &format!("-home-me-work-acme/{sid}/subagents"),
            "agent-aaa.jsonl",
            &[assistant(
                "a1",
                "2026-01-02T10:05:00Z",
                sid,
                "/home/me/work/acme",
                serde_json::json!({"input_tokens": 100, "output_tokens": 20}),
                serde_json::json!([{"type": "tool_use", "name": "Bash"}]),
            )],
        );
        write_transcript(
            home,
            &format!("-home-me-work-acme/{sid}/subagents"),
            "agent-bbb.jsonl",
            &[assistant(
                "b1",
                "2026-01-02T09:59:00Z",
                sid,
                "/home/me/work/acme",
                serde_json::json!({"input_tokens": 300, "output_tokens": 40}),
                serde_json::json!([{"type": "tool_use", "name": "Bash"}]),
            )],
        );

        let read = claude_read(home);
        assert_eq!(
            read.batch.sessions.len(),
            1,
            "one session, not one per spawned agent"
        );
        let session = &read.batch.sessions[0];
        assert_eq!(session.turns, 3, "every turn of the session counts");
        assert_eq!(
            session.input_tokens, 410,
            "the agents' tokens are the session's"
        );
        assert_eq!(session.output_tokens, 61);
        // Tool calls come from both, and the session's own `Task` is a tool too.
        let tools: std::collections::BTreeMap<&str, i64> = session
            .tool_calls
            .iter()
            .map(|c| (c.name.as_str(), c.count))
            .collect();
        assert_eq!(tools.get("Bash"), Some(&2));
        assert_eq!(tools.get("Task"), Some(&1));
        // And the span takes the earliest and the latest across the files: the
        // subagent that ran "before" the first record still belongs inside it.
        assert_eq!(session.started_at, "2026-01-02T09:59:00Z");
        assert_eq!(session.ended_at, "2026-01-02T10:05:00Z");
        // Every row is priced and attributable, so the ledger sums them.
        assert_eq!(read.batch.usage.len(), 3);
        assert!(read
            .batch
            .usage
            .iter()
            .all(|r| r.session_id.as_deref() == Some(sid)));
    }

    /// The title: the agent's own name for the session when it wrote one, the
    /// opening message when it did not, and neither when the file says neither.
    ///
    /// The `ai-title` records are rewritten as the conversation develops (a real
    /// transcript carries one per turn, with a handful of distinct values), so
    /// the **last** one is the settled name — this pins that the drafts lose.
    #[test]
    fn a_session_takes_the_agents_own_title_and_falls_back_to_its_first_message() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let sid = "11111111-2222-3333-4444-555555555555";

        write_transcript(
            home,
            "-home-me-work-acme",
            "a.jsonl",
            &[
                // A draft title, then the user's opening message…
                serde_json::json!({"type": "ai-title", "aiTitle": "draft", "sessionId": sid}),
                serde_json::json!({
                    "type": "user",
                    "sessionId": sid,
                    "message": { "role": "user", "content": "make the tests pass" }
                }),
                assistant(
                    "r1",
                    "2026-01-02T10:00:00Z",
                    sid,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 1, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
                // …and the title it settled on.
                serde_json::json!({"type": "ai-title", "aiTitle": "fixing the flaky test", "sessionId": sid}),
            ],
        );
        // A second transcript with no title record at all — and with the
        // scaffolding records a real one opens with, which must not become the
        // title. Both shapes here were taken from a live transcript after a real
        // run produced `<local-command-caveat>Caveat: …` as a session's name.
        write_transcript(
            home,
            "-home-me-work-acme",
            "b.jsonl",
            &[
                serde_json::json!({
                    "type": "user",
                    "isMeta": true,
                    "sessionId": sid,
                    "message": {"role": "user", "content": "<local-command-caveat>Caveat: The messages below were generated"}
                }),
                serde_json::json!({
                    "type": "user",
                    "sessionId": sid,
                    "message": {"role": "user", "content": "<command-name>/clear</command-name>"}
                }),
                serde_json::json!({
                    "type": "user",
                    "sessionId": sid,
                    "message": {"role": "user", "content": [{"type": "text", "text": "  "}]}
                }),
                serde_json::json!({
                    "type": "user",
                    "sessionId": sid,
                    "promptSource": "typed",
                    "message": {"role": "user", "content": [{"type": "text", "text": "the second thing said"}]}
                }),
                assistant(
                    "r2",
                    "2026-01-02T11:00:00Z",
                    sid,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 1, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
            ],
        );

        let read = claude_read(home);
        let titles: Vec<Option<String>> = read
            .batch
            .sessions
            .iter()
            .map(|s| s.title.clone())
            .collect();
        // The first file's own title wins over its opening message; the second has
        // no title record, so it uses the first message that is actually the
        // user's — past the meta record and the command echo.
        assert!(
            titles.contains(&Some("fixing the flaky test".to_string())),
            "{titles:?}"
        );
        assert!(
            titles.contains(&Some("the second thing said".to_string())),
            "{titles:?}"
        );
    }

    /// A title is one line, bounded, and dropped when it looks like a credential.
    ///
    /// All three rules exist because the source is free-form: an opening message
    /// is routinely several paragraphs, and one that begins with a pasted key
    /// would put that key in a table the app renders.
    #[test]
    fn a_title_is_one_bounded_line_and_never_a_credential() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let sid = "11111111-2222-3333-4444-555555555555";

        write_transcript(
            home,
            "-home-me-work-acme",
            "a.jsonl",
            &[
                serde_json::json!({
                    "type": "user",
                    "sessionId": sid,
                    "message": {"role": "user", "content": "first line\n\nsecond paragraph\ttabbed"}
                }),
                assistant(
                    "r1",
                    "2026-01-02T10:00:00Z",
                    sid,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 1, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
            ],
        );
        write_transcript(
            home,
            "-home-me-work-acme",
            "long.jsonl",
            &[
                serde_json::json!({
                    "type": "user", "sessionId": sid,
                    "message": {"role": "user", "content": "x".repeat(400)}
                }),
                assistant(
                    "r2",
                    "2026-01-02T10:00:00Z",
                    sid,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 1, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
            ],
        );
        write_transcript(
            home,
            "-home-me-work-acme",
            "secret.jsonl",
            &[
                serde_json::json!({
                    "type": "user", "sessionId": sid,
                    "message": {
                        "role": "user",
                        "content": "why does ghp_ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789 fail"
                    }
                }),
                assistant(
                    "r3",
                    "2026-01-02T10:00:00Z",
                    sid,
                    "/home/me/work/acme",
                    serde_json::json!({"input_tokens": 1, "output_tokens": 1}),
                    serde_json::json!([{"type": "text", "text": "ok"}]),
                ),
            ],
        );

        let read = claude_read(home);
        let titles: Vec<String> = read
            .batch
            .sessions
            .iter()
            .filter_map(|s| s.title.clone())
            .collect();

        assert!(
            titles
                .iter()
                .any(|t| t == "first line second paragraph tabbed"),
            "whitespace runs collapse: {titles:?}"
        );
        let long = titles
            .iter()
            .find(|t| t.starts_with("xxx"))
            .expect("the long one");
        assert!(long.ends_with('…'), "{long}");
        assert_eq!(
            long.chars().count(),
            121,
            "120 characters plus the ellipsis"
        );
        assert!(
            !titles.iter().any(|t| t.contains("ghp_")),
            "a title that looks like a credential is dropped, not stored: {titles:?}"
        );
    }

    /// Two assistant records: the session's tokens are their sum, the `Bash`
    /// `tool_use` is one tool call, and the `Skill` block is a skill — in `skills`
    /// and *not* in `tool_calls`, which is the rule stated in `read_file`.
    #[test]
    fn two_assistant_records_sum_their_tokens_and_split_skills_from_tools() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let cwd = home.join("repo");
        std::fs::create_dir_all(cwd.join(".git")).unwrap();

        write_transcript(
            home,
            "-home-me-repo",
            "s1.jsonl",
            &[
                serde_json::json!({ "type": "summary", "summary": "a session" }),
                assistant(
                    "u1",
                    "2024-01-01T00:00:00Z",
                    "s1",
                    &cwd.to_string_lossy(),
                    serde_json::json!({
                        "input_tokens": 100, "output_tokens": 50,
                        "cache_read_input_tokens": 10, "cache_creation_input_tokens": 5
                    }),
                    serde_json::json!([{ "type": "tool_use", "name": "Bash" }]),
                ),
                assistant(
                    "u2",
                    "2024-01-01T00:01:00Z",
                    "s1",
                    &cwd.to_string_lossy(),
                    serde_json::json!({ "input_tokens": 20, "output_tokens": 10 }),
                    serde_json::json!([
                        { "type": "tool_use", "name": "Skill" },
                        { "type": "text", "text": "hi" }
                    ]),
                ),
            ],
        );

        let read = claude_read(home);
        assert_eq!(read.batch.usage.len(), 2, "detail={:?}", read.detail);
        assert_eq!(read.batch.sessions.len(), 1);

        let session = &read.batch.sessions[0];
        assert_eq!(session.agent, "claude");
        assert_eq!(session.session_id, "s1");
        assert_eq!(session.project.as_deref(), Some("repo"));
        assert_eq!(session.started_at, "2024-01-01T00:00:00Z");
        assert_eq!(session.ended_at, "2024-01-01T00:01:00Z");
        assert_eq!(session.turns, 2);
        assert_eq!(session.input_tokens, 120);
        assert_eq!(session.output_tokens, 60);
        assert_eq!(session.cache_read_tokens, 10);
        assert_eq!(session.cache_creation_tokens, 5);

        // One tool call (Bash), one skill (Skill) — and never both.
        assert_eq!(
            session.tool_calls,
            vec![crate::history::HistoryCount {
                name: "Bash".into(),
                count: 1
            }]
        );
        assert_eq!(
            session.skills,
            vec![crate::history::HistoryCount {
                name: "Skill".into(),
                count: 1
            }]
        );

        // Anthropic-shaped: the cache buckets are outside `input_tokens`.
        assert!(read.batch.usage.iter().all(|r| !r.cache_inclusive));
    }

    /// A `cwd` that moves into a subdirectory of the repo mid-session does not
    /// move the project: both records walk up to the same `.git`.
    #[test]
    fn a_cwd_changing_mid_session_keeps_the_same_project_label() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let repo = home.join("proj");
        let sub = repo.join("crates").join("api");
        std::fs::create_dir_all(repo.join(".git")).unwrap();
        std::fs::create_dir_all(&sub).unwrap();

        write_transcript(
            home,
            "-home-me-proj",
            "s1.jsonl",
            &[
                assistant(
                    "u1",
                    "2024-01-01T00:00:00Z",
                    "s1",
                    &repo.to_string_lossy(),
                    serde_json::json!({ "input_tokens": 1, "output_tokens": 1 }),
                    serde_json::json!([]),
                ),
                assistant(
                    "u2",
                    "2024-01-01T00:01:00Z",
                    "s1",
                    &sub.to_string_lossy(),
                    serde_json::json!({ "input_tokens": 1, "output_tokens": 1 }),
                    serde_json::json!([]),
                ),
            ],
        );

        let read = claude_read(home);
        assert_eq!(read.batch.usage.len(), 2);
        assert_eq!(read.batch.usage[0].project.as_deref(), Some("proj"));
        assert_eq!(read.batch.usage[1].project.as_deref(), Some("proj"));
        assert_eq!(read.batch.sessions[0].project.as_deref(), Some("proj"));
    }

    /// A line that is not JSON is skipped; the records around it still parse, and
    /// the file is reported as read (a detail line), not as a skip.
    #[test]
    fn a_malformed_line_is_skipped_without_losing_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        let slug = home.join(".claude").join("projects").join("-home-me-repo");
        std::fs::create_dir_all(&slug).unwrap();
        let one = serde_json::to_string(&assistant(
            "u1",
            "2024-01-01T00:00:00Z",
            "s1",
            "",
            serde_json::json!({ "input_tokens": 7, "output_tokens": 3 }),
            serde_json::json!([]),
        ))
        .unwrap();
        let two = serde_json::to_string(&assistant(
            "u2",
            "2024-01-01T00:01:00Z",
            "s1",
            "",
            serde_json::json!({ "input_tokens": 7, "output_tokens": 3 }),
            serde_json::json!([]),
        ))
        .unwrap();
        std::fs::write(
            slug.join("s1.jsonl"),
            format!("{one}\n{{ not json\n{two}\n"),
        )
        .unwrap();

        let read = claude_read(home);
        assert!(read.skips.is_empty(), "a bad line is not a bad file");
        assert_eq!(read.batch.usage.len(), 2, "detail={:?}", read.detail);
        assert!(read.detail.iter().any(|d| d.contains("malformed line")));
    }

    /// A record with a `message` but no `usage` — a user turn, or an assistant
    /// record trimmed by the tool — contributes no usage row and no turn.
    #[test]
    fn a_record_with_no_usage_contributes_no_row() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_transcript(
            home,
            "-home-me-repo",
            "s1.jsonl",
            &[
                serde_json::json!({
                    "type": "user", "uuid": "u0", "timestamp": "2024-01-01T00:00:00Z",
                    "sessionId": "s1", "message": { "role": "user", "content": "hello" }
                }),
                assistant(
                    "u1",
                    "2024-01-01T00:01:00Z",
                    "s1",
                    "",
                    serde_json::json!({ "input_tokens": 4, "output_tokens": 2 }),
                    serde_json::json!([]),
                ),
            ],
        );

        let read = claude_read(home);
        assert_eq!(read.batch.usage.len(), 1, "detail={:?}", read.detail);
        assert_eq!(read.batch.sessions[0].turns, 1);
    }

    /// The idempotency key is deterministic from the file: reading the same
    /// fixture twice yields the same keys, so a re-import refreshes rather than
    /// duplicates.
    #[test]
    fn an_import_key_is_stable_across_two_runs() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path();
        write_transcript(
            home,
            "-home-me-repo",
            "s1.jsonl",
            &[assistant(
                "u1",
                "2024-01-01T00:00:00Z",
                "s1",
                "",
                serde_json::json!({ "input_tokens": 4, "output_tokens": 2 }),
                serde_json::json!([]),
            )],
        );

        let first = claude_read(home);
        let second = claude_read(home);
        assert_eq!(first.batch.usage[0].import_key, "claude-code:s1:u1");
        assert_eq!(
            first.batch.usage[0].import_key,
            second.batch.usage[0].import_key
        );
        assert_eq!(first.batch.sessions[0].import_key, "claude-code:s1");
        assert_eq!(
            first.batch.sessions[0].import_key,
            second.batch.sessions[0].import_key
        );
    }
}
