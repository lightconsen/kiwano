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
    /// What the session is about (migration v34) — see
    /// [`crate::store::ImportedSession::title`]. `None` for a file that named
    /// neither a title nor an opening message.
    pub title: Option<String>,
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
    /// Which machine's files this session was imported from (migration v32) — a
    /// best-effort **label**, `None` for a machine that could not name itself.
    /// Only an imported row has one; the traffic half never does.
    pub imported_from: Option<String>,
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
const SESSION_COLUMNS: &str = "agent, project, title, session_id, started_at, ended_at, turns,
     input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens, tool_calls, skills,
     imported_from";

fn session_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<SessionRow> {
    Ok(SessionRow {
        agent: row.get(0)?,
        project: row.get(1)?,
        title: row.get(2)?,
        session_id: row.get(3)?,
        started_at: row.get(4)?,
        ended_at: row.get(5)?,
        turns: row.get(6)?,
        input_tokens: row.get(7)?,
        output_tokens: row.get(8)?,
        cache_read_tokens: row.get(9)?,
        cache_creation_tokens: row.get(10)?,
        tool_calls: row.get(11)?,
        skills: row.get(12)?,
        imported_from: row.get(13)?,
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
                "INSERT INTO sessions (import_key, agent, project, title, session_id, started_at, ended_at,
                                       turns, input_tokens, output_tokens, cache_read_tokens,
                                       cache_creation_tokens, tool_calls, skills, imported_from)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15)
                 ON CONFLICT(import_key) DO UPDATE SET
                     agent = ?2, project = ?3, title = ?4, session_id = ?5, started_at = ?6,
                     ended_at = ?7, turns = ?8, input_tokens = ?9, output_tokens = ?10,
                     cache_read_tokens = ?11, cache_creation_tokens = ?12, tool_calls = ?13,
                     skills = ?14, imported_from = ?15",
                params![
                    r.import_key,
                    r.agent,
                    r.project,
                    r.title,
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
                    r.imported_from,
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
    /// pattern), and `since` bounds **`ended_at`** — the session's last activity.
    ///
    /// Not `started_at`, which is what this used to do, and the difference is a
    /// session that ran across the window's edge: a conversation begun ten days
    /// ago and worked on yesterday belongs in a seven-day list, and the traffic
    /// half of the same view already answers that way (it includes a session with
    /// *any* request in the window). Two halves of one list measuring two
    /// different questions is the kind of disagreement this store does not
    /// tolerate anywhere else.
    ///
    /// It is strictly a widening: `ended_at >= started_at` always, so every
    /// session the old bound included is still included.
    pub fn list_sessions(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<SessionRow>> {
        self.sessions_where(agent, project, since, None)
    }

    /// One session id's imported rows — a **list**, not one row.
    ///
    /// `sessions.import_key` is `"{agent}:{session}"`, so a single id can be two
    /// agents' rows and this table keeps both. The list view shows them as two
    /// rows; anything wanting "the session with this id" has to pick, and the
    /// sort here (`agent ASC`) is what that pick is made on.
    pub fn session_rows_by_id(&self, session_id: &str) -> Result<Vec<SessionRow>> {
        self.sessions_where(None, None, None, Some(session_id))
    }

    /// The one query behind both readers above: same columns, same predicates,
    /// one more optional bound. Two copies of this WHERE is how a filter comes
    /// to mean one thing in the list and another in the lookup.
    fn sessions_where(
        &self,
        agent: Option<&str>,
        project: Option<&str>,
        since: Option<&str>,
        session_id: Option<&str>,
    ) -> Result<Vec<SessionRow>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {SESSION_COLUMNS} FROM sessions
             WHERE (?1 IS NULL OR agent = ?1)
               AND (?2 IS NULL OR project = ?2)
               AND (?3 IS NULL OR ended_at >= ?3)
               AND (?4 IS NULL OR session_id = ?4)
             -- A total order, and the last term is why: two rows can share a
             -- session id (one per agent) and a start, and without a final
             -- tiebreak the order between them is whatever the planner returns —
             -- which is how the by-id lookup would settle on a different row on
             -- different runs.
             ORDER BY started_at DESC, session_id ASC, agent ASC"
        ))?;
        let rows = stmt
            .query_map(params![agent, project, since, session_id], session_from_row)?
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
        self.traffic_sessions_where(agent, since, None)
    }

    /// The gateway's own reading of one session, when it routed any of it.
    ///
    /// No index carries `request_logs(session_id)`, so this is two full scans —
    /// and still cheaper than the list read the app already makes on every open,
    /// because the `GROUP BY` collapses to one row.
    pub fn traffic_session_by_id(&self, session_id: &str) -> Result<Option<TrafficSession>> {
        Ok(self
            .traffic_sessions_where(None, None, Some(session_id))?
            .into_iter()
            .next())
    }

    fn traffic_sessions_where(
        &self,
        agent: Option<&str>,
        since: Option<&str>,
        session_id: Option<&str>,
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
                   AND (?3 IS NULL OR session_id = ?3)
                 GROUP BY session_id
                 ORDER BY MAX(ts) DESC, session_id ASC",
            )?;
            let rows = stmt.query_map(params![agent, since, session_id], |row| {
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
               AND (?3 IS NULL OR session_id = ?3)
             GROUP BY session_id, cost_currency",
        )?;
        let mut by_session: std::collections::HashMap<String, Vec<SessionCurrencyCost>> =
            std::collections::HashMap::new();
        let rows = money.query_map(params![agent, since, session_id], |row| {
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

    /// A session with its own span: `started` and `ended` are separate because
    /// the window filter reads the **end**, and a fixture that made them equal
    /// could not tell the two apart.
    fn session(key: &str, project: &str, started: &str) -> ImportedSession {
        let ended = started.replace("T00:00:00", "T06:00:00");
        ImportedSession {
            import_key: key.into(),
            agent: "claude".into(),
            project: Some(project.into()),
            title: None,
            session_id: format!("s-{key}"),
            started_at: started.into(),
            ended_at: ended,
            turns: 3,
            input_tokens: 100,
            output_tokens: 10,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: Some(r#"[{"name":"Bash","count":2}]"#.into()),
            skills: None,
            imported_from: None,
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
        // `since` bounds the session's **last activity**, not its start. The
        // bound sits inside `b`'s own span (00:00–06:00): a start-based filter
        // would drop a conversation that was still being worked on inside the
        // window, which is the bug this bound exists to not have.
        let recent = store
            .list_sessions(None, None, Some("2026-02-01T03:00:00+00:00"))
            .unwrap();
        assert_eq!(recent.len(), 1, "{:?}", recent.len());
        assert_eq!(recent[0].session_id, "s-claude-code:b");

        // And a bound past everything still returns nothing rather than falling
        // back to "unbounded".
        assert!(store
            .list_sessions(None, None, Some("2027-01-01T00:00:00+00:00"))
            .unwrap()
            .is_empty());
    }

    /// The by-id reads: the imported rows come back as a **list** (one id can be
    /// two agents' rows), and the traffic side answers `None` rather than an
    /// empty aggregate when the gateway never routed the session.
    /// The by-id reads: the imported side answers a **list** (one id can be two
    /// agents' rows) and the traffic side answers `None` rather than a zeroed
    /// aggregate when the gateway never routed the session.
    #[test]
    fn the_by_id_reads_answer_what_they_have_and_nothing_for_the_rest() {
        let (_dir, store) = crate::store::test_support::temp_store();
        // Two agents, one session id: what `import_key = "{agent}:{session}"`
        // makes possible, and why the imported lookup cannot be an Option.
        let mut second = session("codex:x", "kiwano", "2026-01-01T00:00:00+00:00");
        second.session_id = "s-claude-code:a".into();
        second.agent = "codex".into();
        store
            .upsert_imported_sessions(&[
                session("claude-code:a", "kiwano", "2026-01-01T00:00:00+00:00"),
                second,
            ])
            .unwrap();

        let rows = store.session_rows_by_id("s-claude-code:a").unwrap();
        assert_eq!(rows.len(), 2, "both agents' rows, sorted by agent");
        assert_eq!(rows[0].agent, "claude", "the pick the detail page makes");
        assert!(store
            .session_rows_by_id("nothing-knows-this")
            .unwrap()
            .is_empty());

        assert!(
            store
                .traffic_session_by_id("s-claude-code:a")
                .unwrap()
                .is_none(),
            "no request log names it: None, not an empty aggregate"
        );
    }

    /// v32: the machine a session was imported from round-trips through the
    /// session upsert, and a **re-import refreshes it** — the label is a fact
    /// about the scan, so it must be refreshed on conflict like the counts are.
    #[test]
    fn a_sessions_machine_round_trips_and_is_refreshed_on_re_import() {
        let (_dir, store) = crate::store::test_support::temp_store();
        let mut row = session("claude-code:a", "kiwano", "2026-01-01T00:00:00+00:00");
        row.imported_from = Some("buildbox".into());
        store.upsert_imported_sessions(&[row]).unwrap();
        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(listed[0].imported_from.as_deref(), Some("buildbox"));

        // The same session, re-imported from a machine that has since been
        // renamed: the upsert replaces the label rather than leaving the first.
        let mut renamed = session("claude-code:a", "kiwano", "2026-01-01T00:00:00+00:00");
        renamed.imported_from = Some("renamed".into());
        store.upsert_imported_sessions(&[renamed]).unwrap();
        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(listed.len(), 1, "still one session");
        assert_eq!(listed[0].imported_from.as_deref(), Some("renamed"));

        // A session from a machine that could not name itself reads NULL.
        let mut unnamed = session("claude-code:b", "acme", "2026-02-01T00:00:00+00:00");
        unnamed.imported_from = None;
        store.upsert_imported_sessions(&[unnamed]).unwrap();
        let listed = store.list_sessions(None, None, None).unwrap();
        let b = listed
            .iter()
            .find(|s| s.session_id == "s-claude-code:b")
            .unwrap();
        assert_eq!(b.imported_from, None);
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
