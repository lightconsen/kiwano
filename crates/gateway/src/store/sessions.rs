//! The `sessions` table: what an agent's own files said about one session.
//!
//! Session-shaped facts — how many turns, which tools, which skills — have no
//! home in `usage`, whose row is a request. Keeping them apart is what stops
//! either table from becoming the house of two different things.
//!
//! Everything here is **imported**: the gateway does not write this table. Its
//! own notion of a session lives in `request_logs.session_id`, per request, and
//! that is the traffic's story rather than the agent's.

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::ImportedSession;
use crate::store::Store;
use rusqlite::params;

/// One stored session as a reader sees it — the import key is the row's identity
/// and is not part of it.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SessionRow {
    pub agent: String,
    pub project: Option<String>,
    pub session_id: String,
    pub started_at: String,
    pub ended_at: String,
    pub turns: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// JSON arrays of `{"name","count"}`, as stored.
    pub tool_calls: Option<String>,
    pub skills: Option<String>,
}

/// The columns every read shares, in the order `session_from_row` expects.
const SESSION_COLUMNS: &str = "agent, project, session_id, started_at, ended_at, turns,
     input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, tool_calls, skills";

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        agent: row.get(0)?,
        project: row.get(1)?,
        session_id: row.get(2)?,
        started_at: row.get(3)?,
        ended_at: row.get(4)?,
        turns: row.get(5)?,
        input_tokens: row.get(6)?,
        output_tokens: row.get(7)?,
        cache_read_tokens: row.get(8)?,
        cache_creation_tokens: row.get(9)?,
        tool_calls: row.get(10)?,
        skills: row.get(11)?,
    })
}

impl Store {
    /// Write a batch of imported sessions, refreshing any this store already has.
    ///
    /// Upsert on the primary key (`import_key`), for the reason the usage half
    /// gives: a re-import of the same files is the same sessions, and a second
    /// copy of each would make the session list grow every time a scan ran.
    pub fn upsert_imported_sessions(&self, rows: &[ImportedSession]) -> Result<usize> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        let mut written = 0usize;
        for r in rows {
            tx.execute(
                "INSERT INTO sessions (import_key, agent, project, session_id, started_at, ended_at,
                                       turns, input_tokens, output_tokens, cache_read_tokens,
                                       cache_creation_tokens, tool_calls, skills)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
                 ON CONFLICT(import_key) DO UPDATE SET
                     agent = ?2, project = ?3, session_id = ?4, started_at = ?5, ended_at = ?6,
                     turns = ?7, input_tokens = ?8, output_tokens = ?9, cache_read_tokens = ?10,
                     cache_creation_tokens = ?11, tool_calls = ?12, skills = ?13",
                params![
                    r.import_key,
                    r.agent,
                    r.project,
                    r.session_id,
                    r.started_at,
                    r.ended_at,
                    r.turns,
                    r.input_tokens,
                    r.output_tokens,
                    r.cache_read_tokens,
                    r.cache_creation_tokens,
                    r.tool_calls,
                    r.skills,
                ],
            )?;
            written += 1;
        }
        tx.commit()?;
        Ok(written)
    }

    /// The imported sessions, most recent first.
    ///
    /// `project` and `agent` are exact matches (a label the user reads, not a
    /// pattern), and `since` bounds `started_at` the way every other `ts` filter
    /// in the store does.
    pub fn list_sessions(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions
             WHERE (?1 IS NULL OR agent = ?1)
               AND (?2 IS NULL OR project = ?2)
               AND (?3 IS NULL OR started_at >= ?3)
             ORDER BY started_at DESC, session_id ASC"
        ))?;
        let rows = stmt
            .query_map(params![agent, project, since], session_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// How many sessions this store holds, and when the newest one ran — the pair
    /// a caller reporting "what is in here" wants.
    pub fn session_summary(&self) -> Result<(i64, Option<String>)> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(
            conn.query_row("SELECT COUNT(*), MAX(ended_at) FROM sessions", [], |row| {
                Ok((row.get(0)?, row.get(1)?))
            })?,
        )
    }

    /// When this store last imported anything, if it ever has — the marker a
    /// client uses to decide whether a scan is worth running again.
    pub fn record_history_scan(&self, at: &str) -> Result<()> {
        self.set_gateway_setting(HISTORY_SCAN_KEY, at)
    }

    pub fn last_history_scan(&self) -> Option<String> {
        self.gateway_setting(HISTORY_SCAN_KEY)
            .filter(|v| !v.trim().is_empty())
    }

    /// Stamp a scan that just happened. Convenience for the caller that only has
    /// the store.
    pub fn mark_history_scanned(&self) -> Result<String> {
        let now = now_rfc3339();
        self.record_history_scan(&now)?;
        Ok(now)
    }
}

/// Where the last history scan's timestamp lives (`gateway_settings`).
///
/// Daemon-side rather than in the client's own database, because the *fact* it
/// records is about the daemon's ledger: what was scanned is what was imported,
/// and a client that reinstalled should not have to be told twice.
pub const HISTORY_SCAN_KEY: &str = "history.last_scan_at";

#[cfg(test)]
mod tests {
    use super::*;

    fn session(key: &str, project: &str, started: &str) -> ImportedSession {
        ImportedSession {
            import_key: key.into(),
            agent: "claude".into(),
            project: Some(project.into()),
            session_id: format!("s-{key}"),
            started_at: started.into(),
            ended_at: started.into(),
            turns: 3,
            input_tokens: 100,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: Some(r#"[{"name":"Bash","count":2}]"#.into()),
            skills: None,
        }
    }

    #[test]
    fn importing_the_same_sessions_twice_leaves_one_of_each() {
        let (_dir, store) = crate::store::test_support::temp_store();
        let rows = vec![
            session("claude-code:a", "kiwano", "2026-01-01T00:00:00+00:00"),
            session("claude-code:b", "acme", "2026-02-01T00:00:00+00:00"),
        ];
        assert_eq!(store.upsert_imported_sessions(&rows).unwrap(), 2);
        assert_eq!(store.upsert_imported_sessions(&rows).unwrap(), 2);
        assert_eq!(store.list_sessions(None, None, None).unwrap().len(), 2);
        assert_eq!(store.session_summary().unwrap().0, 2);

        // Most recent first, and the filters are exact matches.
        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(listed[0].session_id, "s-claude-code:b");
        assert_eq!(
            store
                .list_sessions(Some("claude"), Some("acme"), None)
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .list_sessions(None, Some("nothing"), None)
            .unwrap()
            .is_empty());
        assert_eq!(
            store
                .list_sessions(None, None, Some("2026-01-15T00:00:00+00:00"))
                .unwrap()
                .len(),
            1
        );
    }

    #[test]
    fn a_scan_timestamp_is_recorded_only_once_it_happens() {
        let (_dir, store) = crate::store::test_support::temp_store();
        assert_eq!(store.last_history_scan(), None);
        let stamped = store.mark_history_scanned().unwrap();
        assert_eq!(store.last_history_scan().as_deref(), Some(stamped.as_str()));
    }
}
