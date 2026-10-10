//! The two halves of "a session".
//!
//! Session-shaped facts — how many turns, which tools, which skills — have no
//! home in `usage`, whose row is a request. Keeping them apart is what stops
//! either table from becoming the house of two different things.
//!
//! The `sessions` table below is the **imported** half: what an agent's own
//! files said, and the gateway does not write it. Its own notion of a session
//! lives in `request_logs.session_id`, per request — the **traffic** half, read
//! back as an aggregate by [`Store::traffic_sessions`]. That read lives here,
//! beside the imported one, because the two are the inputs of one merged view
//! (`api::sessions`): a reader can see both sources of a session without leaving
//! the module. `store::logs` is the wrong home for it — that module is one
//! request row and its full capture, and its column list and WHERE fragment are
//! a single unit whose own doc warns against splitting them.

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

/// One session as the **traffic** side sees it: what the gateway routed under
/// one `request_logs.session_id` (migration v9).
///
/// This is the precise record — provider, cost, tokens, status, per request —
/// and it is deliberately a different shape from [`SessionRow`]: a row here is a
/// session's *traffic*, and only the files can say its project, its turns or the
/// tools it used.
#[derive(Debug, Clone, PartialEq)]
pub struct TrafficSession {
    pub session_id: String,
    /// The span of the rows the gateway saw (`MIN`/`MAX` of their `ts`). Not the
    /// session's whole life: one that began before a takeover has imported rows
    /// older than this, which is why the merge prefers the imported span.
    pub first_ts: String,
    pub last_ts: String,
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// The distinct agent labels involved, sorted. Usually one; a session whose
    /// id two agents happened to share is why this is a list rather than a field.
    pub agents: Vec<String>,
    /// The distinct provider ids involved, sorted. Usually one. A pre-forward
    /// failure's row has no provider and contributes none.
    pub providers: Vec<String>,
    /// What the session cost, per currency. Never converted, and never added
    /// across currencies — the amounts are in the money they were spent in.
    pub cost: Vec<SessionCurrencyCost>,
    /// Rows that carry tokens but no cost. They are part of `requests` and their
    /// tokens are in the buckets above; `cost` is short by them, and this is how
    /// the reader is told rather than shown a total that is quietly too small.
    pub unpriced_rows: i64,
}

/// One session's spend in one currency — the money half of a [`TrafficSession`].
#[derive(Debug, Clone, PartialEq)]
pub struct SessionCurrencyCost {
    /// `None` for a row that has a cost but no recorded currency; the merge
    /// drops those from the money list (there is no currency to show it in)
    /// while the tokens still count.
    pub currency: Option<String>,
    pub cost: f64,
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

    /// The gateway's own sessions, most recently active first.
    ///
    /// Grouped by `session_id`, skipping NULL: a request with no session is not
    /// a session, and the files' own sessions are in `sessions`, not here.
    /// Bounded by `agent` and by `since` — both applied to the *rows*, so a
    /// session with any activity in the window appears whole within it.
    ///
    /// Two reads rather than one. Tokens sum per session; money sums per
    /// (session, currency). A single `GROUP BY` cannot carry both without
    /// repeating the token totals once per currency, and un-repeating that in
    /// Rust is exactly where such an aggregate silently double-counts.
    pub fn traffic_sessions(
        &self,
        agent: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<TrafficSession>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut sessions: Vec<TrafficSession> = {
            let mut stmt = conn.prepare(
                "SELECT session_id, MIN(ts), MAX(ts), COUNT(*),
                        COALESCE(SUM(input_tokens), 0), COALESCE(SUM(output_tokens), 0),
                        COALESCE(SUM(cache_read_tokens), 0),
                        COALESCE(SUM(cache_creation_tokens), 0),
                        COALESCE(SUM(CASE WHEN cost IS NULL THEN 1 ELSE 0 END), 0),
                        GROUP_CONCAT(DISTINCT agent), GROUP_CONCAT(DISTINCT provider_id)
                 FROM request_logs
                 WHERE session_id IS NOT NULL
                   AND (?1 IS NULL OR agent = ?1)
                   AND (?2 IS NULL OR ts >= ?2)
                 GROUP BY session_id
                 ORDER BY MAX(ts) DESC, session_id ASC",
            )?;
            let rows = stmt.query_map(params![agent, since], |row| {
                Ok(TrafficSession {
                    session_id: row.get(0)?,
                    first_ts: row.get(1)?,
                    last_ts: row.get(2)?,
                    requests: row.get(3)?,
                    input_tokens: row.get(4)?,
                    output_tokens: row.get(5)?,
                    cache_read_tokens: row.get(6)?,
                    cache_creation_tokens: row.get(7)?,
                    unpriced_rows: row.get(8)?,
                    agents: split_distinct(row.get::<_, Option<String>>(9)?),
                    providers: split_distinct(row.get::<_, Option<String>>(10)?),
                    cost: Vec::new(),
                })
            })?;
            rows.collect::<rusqlite::Result<Vec<_>>>()?
        };

        let mut money = conn.prepare(
            "SELECT session_id, cost_currency, COALESCE(SUM(cost), 0)
             FROM request_logs
             WHERE session_id IS NOT NULL AND cost IS NOT NULL
               AND (?1 IS NULL OR agent = ?1)
               AND (?2 IS NULL OR ts >= ?2)
             GROUP BY session_id, cost_currency",
        )?;
        let mut by_session: std::collections::HashMap<String, Vec<SessionCurrencyCost>> =
            std::collections::HashMap::new();
        let rows = money.query_map(params![agent, since], |row| {
            Ok((
                row.get::<_, String>(0)?,
                SessionCurrencyCost {
                    currency: row.get(1)?,
                    cost: row.get(2)?,
                },
            ))
        })?;
        for row in rows {
            let (id, bucket) = row?;
            by_session.entry(id).or_default().push(bucket);
        }
        for session in &mut sessions {
            if let Some(mut buckets) = by_session.remove(&session.session_id) {
                buckets.sort_by(|a, b| a.currency.cmp(&b.currency));
                session.cost = buckets;
            }
        }
        Ok(sessions)
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

    /// Record that `agent`'s history has been scanned, as of `at`.
    ///
    /// **Per agent**, and that is the whole point: what a client needs to know is
    /// not "has this ledger been backfilled" but "which agents have I already
    /// read". A single flag would answer the first question and quietly mean "and
    /// never read the rest": adding a reader for a third agent would leave every
    /// install that had already scanned never scanning it, because the flag was
    /// set before that reader existed.
    pub fn record_history_scan(&self, agent: &str, at: &str) -> Result<()> {
        self.set_gateway_setting(&history_scan_key(agent), at)
    }

    /// Which agents have been scanned, and when.
    pub fn history_scans(&self) -> Result<std::collections::HashMap<String, String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key, value FROM gateway_settings
             WHERE key LIKE ?1 ESCAPE '\\'",
        )?;
        let prefix = format!("{HISTORY_SCAN_PREFIX}%");
        let rows = stmt.query_map(params![prefix], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (key, value) = row?;
            if let Some(agent) = key.strip_prefix(HISTORY_SCAN_PREFIX) {
                if !agent.is_empty() && !value.trim().is_empty() {
                    out.insert(agent.to_string(), value);
                }
            }
        }
        Ok(out)
    }

    /// Stamp a scan that just happened, for each agent it covered.
    pub fn mark_history_scanned(&self, agents: &[String]) -> Result<()> {
        let now = now_rfc3339();
        for agent in agents {
            if !agent.trim().is_empty() {
                self.record_history_scan(agent, &now)?;
            }
        }
        Ok(())
    }
}

/// A `GROUP_CONCAT(DISTINCT x)` cell as a sorted, de-duplicated list.
///
/// NULLs are already dropped by the aggregate; an empty cell means no row had
/// the value. The separator is the comma `GROUP_CONCAT` uses by default, which
/// is safe here because neither an agent label nor a provider id can contain
/// one.
fn split_distinct(joined: Option<String>) -> Vec<String> {
    let mut seen: Vec<String> = joined
        .filter(|s| !s.is_empty())
        .map(|s| s.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    seen.sort();
    seen.dedup();
    seen
}

/// The prefix one agent's scan stamp lives under, in `gateway_settings`
/// (`history.scan.<agent>`).
///
/// Daemon-side rather than in the client's own database, because the *fact* it
/// records is about the daemon's ledger: what was scanned is what was imported,
/// and a client that reinstalled should not have to be told twice. It is also
/// **per client machine**, which is why two machines do not suppress each other's
/// scans — each has its own client asking, and a machine that has just installed
/// has no stamps here at all.
pub const HISTORY_SCAN_PREFIX: &str = "history.scan.";

/// The settings key one agent's scan stamp lives under.
fn history_scan_key(agent: &str) -> String {
    format!("{HISTORY_SCAN_PREFIX}{agent}")
}

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

    /// Scan stamps are **per agent**, which is what lets a reader added later be
    /// backfilled instead of being skipped because the ledger was read once before
    /// that reader existed.
    #[test]
    fn a_scan_stamp_is_recorded_per_agent() {
        let (_dir, store) = crate::store::test_support::temp_store();
        assert!(store.history_scans().unwrap().is_empty());

        store.mark_history_scanned(&["claude".to_string()]).unwrap();
        let scans = store.history_scans().unwrap();
        assert_eq!(scans.len(), 1);
        assert!(scans.contains_key("claude"), "the agent that was read");
        assert!(
            !scans.contains_key("codex"),
            "and not one that was not: a new reader has to be able to tell"
        );

        // An agent that yielded nothing is still stamped — the fact is "read it,
        // there was nothing", and without it the client re-reads it every launch.
        store.mark_history_scanned(&["codex".to_string()]).unwrap();
        assert_eq!(store.history_scans().unwrap().len(), 2);
    }

    /// The traffic aggregate's numbers, including the two cases the money rules
    /// exist for: a row with no cost (tokens, no money — counted as unpriced) and
    /// a session that spent in two currencies (two amounts, never one sum).
    #[test]
    fn the_traffic_aggregate_counts_a_session_and_keeps_its_money_split() {
        let (_dir, store) = crate::store::test_support::temp_store();
        let row = |ts: &str, session: Option<&str>, cost: Option<f64>, currency: Option<&str>| {
            let mut log = crate::store::test_support::sample_log(ts, Some("claude"), 200);
            log.session_id = session.map(str::to_string);
            log.provider_id = Some("p-1".into());
            log.input_tokens = 100;
            log.output_tokens = 10;
            log.cost = cost;
            log.cost_currency = currency.map(str::to_string);
            log
        };
        for log in [
            row(
                "2026-09-07T10:00:00+00:00",
                Some("s-1"),
                Some(1.5),
                Some("USD"),
            ),
            row(
                "2026-09-07T11:00:00+00:00",
                Some("s-1"),
                Some(9.0),
                Some("CNY"),
            ),
            // The unpriced row: its tokens count, its money does not.
            row("2026-09-07T12:00:00+00:00", Some("s-1"), None, None),
            row(
                "2026-09-08T10:00:00+00:00",
                Some("s-2"),
                Some(2.0),
                Some("USD"),
            ),
        ] {
            store.insert_request_log(&log).unwrap();
        }
        // A request the gateway could not attribute to any session: nobody's.
        let mut orphan = row("2026-09-09T10:00:00+00:00", None, None, None);
        orphan.input_tokens = 7;
        store.insert_request_log(&orphan).unwrap();

        let sessions = store.traffic_sessions(None, None).unwrap();
        assert_eq!(sessions.len(), 2, "the session-less row is not a session");
        assert_eq!(sessions[0].session_id, "s-2", "most recently active first");

        let s1 = sessions.iter().find(|s| s.session_id == "s-1").unwrap();
        assert_eq!(s1.requests, 3);
        assert_eq!(s1.input_tokens, 300);
        assert_eq!(s1.output_tokens, 30);
        assert_eq!(s1.first_ts, "2026-09-07T10:00:00+00:00");
        assert_eq!(s1.last_ts, "2026-09-07T12:00:00+00:00");
        assert_eq!(s1.agents, vec!["claude".to_string()]);
        assert_eq!(s1.providers, vec!["p-1".to_string()]);
        assert_eq!(
            s1.unpriced_rows, 1,
            "the costless row is counted, not hidden"
        );
        // Two currencies, kept apart and sorted; never added.
        assert_eq!(s1.cost.len(), 2);
        assert_eq!(s1.cost[0].currency.as_deref(), Some("CNY"));
        assert!((s1.cost[0].cost - 9.0).abs() < 1e-9);
        assert_eq!(s1.cost[1].currency.as_deref(), Some("USD"));
        assert!((s1.cost[1].cost - 1.5).abs() < 1e-9);

        // The window and the agent bound the rows the aggregate sees.
        let recent = store
            .traffic_sessions(None, Some("2026-09-08T00:00:00+00:00"))
            .unwrap();
        assert_eq!(recent.len(), 1);
        assert_eq!(recent[0].session_id, "s-2");
        assert!(store
            .traffic_sessions(Some("codex"), None)
            .unwrap()
            .is_empty());
    }
}
