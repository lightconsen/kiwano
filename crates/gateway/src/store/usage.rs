//! The `usage` table: the metered rows the gateway writes and every
//! aggregation the dashboard, the limit evaluation and the quota strategy
//! read back out of it.
//!
//! `usage_filters` and `usage_bucketed` are the two shared query shapes:
//! `usage_filters` builds the WHERE fragment and its positional params for
//! eight of the methods here (and `logs::count_request_logs`), and
//! `usage_bucketed` is the one query behind `usage_daily`/`usage_hourly`.
//! Both are `pub(crate)`: the first is reached from `logs`.

use crate::error::Result;
use crate::store::types::{
    CostBucket, DailyUsage, ProviderCostBucket, ProviderUsage, TrafficStats, UsageRecord,
    UsageTotals,
};
use crate::store::Store;
use rusqlite::params;

impl Store {
    // ---- usage -----------------------------------------------------------

    pub fn record_usage(&self, u: &UsageRecord) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO usage (ts, agent, provider_id, client_key_id, model, input_tokens,
                                output_tokens, cache_read_tokens, cache_creation_tokens,
                                latency_ms, status, cost, cost_currency, cost_off_peak,
                                project, session_id, import_key)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)",
            params![
                u.ts,
                u.agent,
                u.provider_id,
                u.client_key_id,
                u.model,
                u.input_tokens,
                u.output_tokens,
                u.cache_read_tokens,
                u.cache_creation_tokens,
                u.latency_ms,
                u.status,
                u.cost,
                u.cost_currency,
                u.cost_off_peak,
                u.project,
                u.session_id,
                u.import_key,
            ],
        )?;
        Ok(())
    }

    /// Cost sums grouped by currency (rows without a computed cost are
    /// skipped). Used to build currency-aware usage/dashboard figures.
    pub fn usage_cost_by_currency(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<(Option<String>, f64)>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT cost_currency, SUM(cost) FROM usage
             WHERE 1=1{cond} AND cost IS NOT NULL
             GROUP BY cost_currency",
        ))?;
        let mut out = Vec::new();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, f64>(1)?))
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Per-provider cost sums with their currency (dashboard by-provider
    /// breakdown needs the split before converting to the preferred currency),
    /// each beside what the same rows would have cost off-peak.
    ///
    /// `cost_off_peak` is written equal to `cost` for a model with no schedule,
    /// so the difference is the premium paid for running at peak times and
    /// nothing else — which is what makes it a sum over every priced row rather
    /// than a second query with its own row set.
    pub fn usage_cost_by_provider(
        &self,
        agent: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<ProviderCostBucket>> {
        let (cond, params) = Self::usage_filters(agent, None, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT provider_id, cost_currency, SUM(cost), SUM(cost_off_peak) FROM usage
             WHERE 1=1{cond} AND cost IS NOT NULL
             GROUP BY provider_id, cost_currency",
        ))?;
        let mut out = Vec::new();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            let cost: f64 = row.get(2)?;
            // A NULL sum means no row here carries an off-peak figure — rows an
            // older gateway wrote into a v13 database. Reading that as "what you
            // paid" states no premium, which is the honest answer; reading it as
            // 0 would report the whole cost as one.
            Ok(ProviderCostBucket {
                provider_id: row.get(0)?,
                currency: row.get(1)?,
                cost,
                cost_off_peak: row.get::<_, Option<f64>>(3)?.unwrap_or(cost),
            })
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// [`Self::usage_cost_by_currency`] with the off-peak sum beside it.
    ///
    /// A sibling rather than two more columns on that method: it has seven
    /// callers — a spending-limit evaluation and three tests among them — and
    /// none of them asked for this.
    pub fn usage_cost_with_off_peak_by_currency(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<CostBucket>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT cost_currency, SUM(cost), SUM(cost_off_peak) FROM usage
             WHERE 1=1{cond} AND cost IS NOT NULL
             GROUP BY cost_currency",
        ))?;
        let mut out = Vec::new();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            let cost: f64 = row.get(1)?;
            Ok(CostBucket {
                currency: row.get(0)?,
                cost,
                // See `usage_cost_by_provider`: an absent sum is "no premium",
                // not "the whole cost was one".
                cost_off_peak: row.get::<_, Option<f64>>(2)?.unwrap_or(cost),
            })
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// WHERE fragment + positional params shared by usage-table aggregations
    /// (agent / provider / time window). Fragment indexes are 1-based and
    /// ordered, so callers splice it after `WHERE 1=1` and bind via
    /// `params_from_iter`. Params are owned: binding trait-object lifetimes
    /// to the borrowed inputs fights the borrows inside `if let` scopes.
    pub(crate) fn usage_filters(
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> (String, Vec<String>) {
        let mut cond = String::new();
        let mut params: Vec<String> = Vec::new();
        if let Some(a) = agent {
            params.push(a.to_string());
            cond.push_str(&format!(" AND agent = ?{}", params.len()));
        }
        if let Some(p) = provider_id {
            params.push(p.to_string());
            cond.push_str(&format!(" AND provider_id = ?{}", params.len()));
        }
        if let Some(s) = since {
            params.push(s.to_string());
            cond.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        // Half-open, like every other window in this crate: the dashboard's
        // previous-period comparison asks for the rows that *were* in the last
        // period, and a boundary row counted twice would inflate it.
        if let Some(u) = until {
            params.push(u.to_string());
            cond.push_str(&format!(" AND ts < ?{}", params.len()));
        }
        (cond, params)
    }

    /// Aggregated totals, optionally filtered by agent, provider and/or a
    /// start timestamp.
    pub fn usage_totals(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<UsageTotals> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}",
        ))?;
        Ok(stmt.query_row(rusqlite::params_from_iter(params), UsageTotals::from_row)?)
    }

    /// Request-log traffic over [since, until): counts and mean latency. The
    /// anomaly detector's raw material — unlike `usage`, request_logs also
    /// holds the requests that never reached a provider, which is where a
    /// misconfiguration storm shows up. Bounds compare lexicographically like
    /// every other `ts` filter; latency is averaged over the rows that have
    /// one.
    pub fn traffic_stats(&self, since: &str, until: &str) -> Result<TrafficStats> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(conn.query_row(
            "SELECT COUNT(*), COALESCE(SUM(status_code >= 400),0), AVG(latency_ms)
             FROM request_logs WHERE ts >= ?1 AND ts < ?2",
            params![since, until],
            |row| {
                Ok(TrafficStats {
                    requests: row.get::<_, i64>(0)? as u64,
                    errors: row.get::<_, i64>(1)? as u64,
                    avg_latency_ms: row.get(2)?,
                })
            },
        )?)
    }

    /// Aggregated totals for one provider, optionally since a timestamp.
    /// Read by the quota strategy (tech.md §4.7 quota).
    pub fn usage_totals_for_provider(
        &self,
        provider_id: &str,
        since: Option<&str>,
    ) -> Result<UsageTotals> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE provider_id = ?1 AND (?2 IS NULL OR ts >= ?2)",
        )?;
        Ok(stmt.query_row(params![provider_id, since], UsageTotals::from_row)?)
    }

    /// Aggregated totals for one client key, optionally since a timestamp.
    /// Read by the client-key spend ceilings (limits v29).
    ///
    /// A sibling of `usage_totals_for_provider` rather than a ninth filter on
    /// `usage_filters`: that helper has eight readers, none of which asked for
    /// this one, and widening it would put the credential in the WHERE clause of
    /// every dashboard query. Rows written before the column are NULL and belong
    /// to nobody's key, which is also the reading that keeps a pre-v29 database
    /// from suddenly charging its history to a key.
    pub fn usage_totals_for_client_key(
        &self,
        client_key_id: &str,
        since: Option<&str>,
    ) -> Result<UsageTotals> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE client_key_id = ?1 AND (?2 IS NULL OR ts >= ?2)",
        )?;
        Ok(stmt.query_row(params![client_key_id, since], UsageTotals::from_row)?)
    }

    /// Cost sums of one client key, grouped by currency. The money half of
    /// [`Store::usage_totals_for_client_key`], and grouped for the same reason
    /// the agent-keyed one is: one key's traffic can span providers, and those
    /// bill in different currencies.
    pub fn usage_cost_by_currency_for_client_key(
        &self,
        client_key_id: &str,
        since: Option<&str>,
    ) -> Result<Vec<(Option<String>, f64)>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT cost_currency, SUM(cost) FROM usage
             WHERE client_key_id = ?1 AND (?2 IS NULL OR ts >= ?2) AND cost IS NOT NULL
             GROUP BY cost_currency",
        )?;
        let rows = stmt
            .query_map(params![client_key_id, since], |row| {
                Ok((row.get::<_, Option<String>>(0)?, row.get::<_, f64>(1)?))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// The instant this gateway's **own** first row for this agent was written.
    ///
    /// The watermark an import measures against: everything older than this
    /// happened before this gateway was in the path, so it is the agent's own
    /// history and nobody else has counted it — and everything at or after it is
    /// already in this table, so importing it would count the same request twice.
    /// The agents write their session files either way, which is what makes the
    /// overlap real rather than theoretical.
    ///
    /// **`import_key IS NULL` is the load-bearing half of that query.** An
    /// imported row is not evidence that the agent was ever metered, and if one
    /// counted, an import would move its own watermark: the first chunk's rows
    /// would become the boundary, and every later chunk of the same scan would be
    /// "already metered" and skipped. That is not a hypothetical — it is what the
    /// first run against a real home did, importing 806 rows out of 118,784 and
    /// reporting the rest as traffic it had already counted.
    ///
    /// `None` for an agent this gateway has never served: all of its history is
    /// pre-gateway, and all of it is importable.
    pub fn first_usage_at(&self, agent: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(conn.query_row(
            "SELECT MIN(ts) FROM usage WHERE agent = ?1 AND import_key IS NULL",
            params![agent],
            |row| row.get::<_, Option<String>>(0),
        )?)
    }

    /// Write a batch of imported rows, refreshing any the store already has.
    ///
    /// Upsert on `import_key`, which is what makes an import repeatable: the key
    /// is derived from the file the row came out of, so a second scan of the same
    /// session refreshes those rows instead of adding a second copy of them. The
    /// refresh is worth something on its own — a machine that had no prices the
    /// first time gets its costs filled in — and it is why the conflict clause
    /// carries the priced columns and not just the tokens.
    ///
    /// The gateway's own rows are never touched: their `import_key` is NULL, and
    /// SQLite treats NULLs in a unique index as distinct.
    ///
    /// Returns how many rows the batch wrote (inserted or refreshed).
    pub fn upsert_imported_usage(&self, rows: &[UsageRecord]) -> Result<usize> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        let mut written = 0usize;
        for u in rows {
            tx.execute(
                "INSERT INTO usage (ts, agent, provider_id, client_key_id, model, input_tokens,
                                    output_tokens, cache_read_tokens, cache_creation_tokens,
                                    latency_ms, status, cost, cost_currency, cost_off_peak,
                                    project, session_id, import_key)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17)
                 ON CONFLICT(import_key) DO UPDATE SET
                     ts = ?1, agent = ?2, provider_id = ?3, client_key_id = ?4, model = ?5,
                     input_tokens = ?6, output_tokens = ?7, cache_read_tokens = ?8,
                     cache_creation_tokens = ?9, latency_ms = ?10, status = ?11, cost = ?12,
                     cost_currency = ?13, cost_off_peak = ?14, project = ?15, session_id = ?16",
                params![
                    u.ts,
                    u.agent,
                    u.provider_id,
                    u.client_key_id,
                    u.model,
                    u.input_tokens,
                    u.output_tokens,
                    u.cache_read_tokens,
                    u.cache_creation_tokens,
                    u.latency_ms,
                    u.status,
                    u.cost,
                    u.cost_currency,
                    u.cost_off_peak,
                    u.project,
                    u.session_id,
                    u.import_key,
                ],
            )?;
            written += 1;
        }
        tx.commit()?;
        Ok(written)
    }

    /// When each client key last carried a request, by handle.
    ///
    /// The question a key list has to answer before anyone revokes one, and the
    /// only honest source for it: the credential itself carries no timestamp, and
    /// a key's own row cannot say whether it is in use. A key that never made a
    /// request is simply absent — which is a state a reader has to render
    /// differently from "unknown", and the reason this returns a map rather than
    /// giving every key a value.
    ///
    /// One query for the whole listing rather than one per key: the group is the
    /// shape the caller wants, and `idx_usage_client_key_ts` (v29) is exactly
    /// this pair. Rows written before the column carry no handle and so answer
    /// for nobody.
    pub fn client_key_last_used(&self) -> Result<std::collections::HashMap<String, String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT client_key_id, MAX(ts) FROM usage
             WHERE client_key_id IS NOT NULL GROUP BY client_key_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?;
        let mut out = std::collections::HashMap::new();
        for row in rows {
            let (key_id, ts) = row?;
            out.insert(key_id, ts);
        }
        Ok(out)
    }

    /// Totals grouped by provider, optionally filtered by agent, provider
    /// and/or since.
    pub fn usage_by_provider(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<ProviderUsage>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT provider_id, COUNT(*), COALESCE(SUM(input_tokens),0),
                    COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}
             GROUP BY provider_id ORDER BY COUNT(*) DESC",
        ))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(ProviderUsage {
                provider_id: row.get(0)?,
                totals: UsageTotals {
                    requests: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                },
            })
        };
        let mut out = Vec::new();
        for row in stmt.query_map(rusqlite::params_from_iter(params), map_row)? {
            out.push(row?);
        }
        Ok(out)
    }

    /// Distinct agents with usage in the window, busiest first.
    ///
    /// The Dashboard's agent breakdown iterates the registry, which is fine
    /// while every agent is a constant — but an agent the user defined (and
    /// later deleted) exists only as the `agent` column of these rows. This is
    /// how its traffic stays on the page after the route is gone.
    pub fn usage_agents(&self, since: Option<&str>) -> Result<Vec<String>> {
        let (cond, params) = Self::usage_filters(None, None, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT agent FROM usage WHERE 1=1{cond}
             GROUP BY agent ORDER BY COUNT(*) DESC, agent ASC",
        ))?;
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| row.get(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Daily aggregation (UTC day = first 10 chars of the RFC3339 ts).
    /// Daily totals, bucketed by the caller's local day: `tz_offset_minutes`
    /// is minutes east of UTC, applied as an SQLite modifier before the date is
    /// taken (stored timestamps are UTC). Keys are `YYYY-MM-DD`.
    pub fn usage_daily(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        tz_offset_minutes: i64,
    ) -> Result<Vec<DailyUsage>> {
        self.usage_bucketed(agent, provider_id, since, tz_offset_minutes, "%Y-%m-%d")
    }

    /// Hourly aggregation — same filters and columns as [`Self::usage_daily`],
    /// but `strftime` stops at the hour, so keys are `YYYY-MM-DDTHH`. This is
    /// what the dashboard's "today" window plots; sharing the query with the
    /// daily one keeps the two from drifting in what they filter or sum.
    pub fn usage_hourly(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        tz_offset_minutes: i64,
    ) -> Result<Vec<DailyUsage>> {
        self.usage_bucketed(agent, provider_id, since, tz_offset_minutes, "%Y-%m-%dT%H")
    }

    /// The one bucketing query behind `usage_daily`/`usage_hourly`: `fmt` is
    /// the `strftime` format that names a bucket, and the returned `DailyUsage`
    /// carries that key in `day` whatever its granularity.
    fn usage_bucketed(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        tz_offset_minutes: i64,
        fmt: &str,
    ) -> Result<Vec<DailyUsage>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, None);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT strftime('{fmt}', ts, '{tz_offset_minutes:+} minutes') AS bucket, COUNT(*),
                    COALESCE(SUM(input_tokens),0),
                    COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}
             GROUP BY bucket ORDER BY bucket ASC",
        ))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(DailyUsage {
                day: row.get(0)?,
                totals: UsageTotals {
                    requests: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                },
            })
        };
        let mut out = Vec::new();
        for row in stmt.query_map(rusqlite::params_from_iter(params), map_row)? {
            out.push(row?);
        }
        Ok(out)
    }
    /// Average `latency_ms` over a window, optionally per provider and/or
    /// agent. `from`/`to` are RFC3339 (store ts strings compare
    /// lexicographically).
    pub fn avg_latency(
        &self,
        provider: Option<&str>,
        agent: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Option<i64> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut sql =
            String::from("SELECT AVG(latency_ms) FROM usage WHERE latency_ms IS NOT NULL");
        // Owned params: binding trait objects to borrowed &str inside `if let`
        // scopes fights the borrows (same as Store::usage_filters)
        let mut params: Vec<String> = Vec::new();
        if let Some(p) = provider {
            params.push(p.to_string());
            sql.push_str(&format!(" AND provider_id = ?{}", params.len()));
        }
        if let Some(a) = agent {
            params.push(a.to_string());
            sql.push_str(&format!(" AND agent = ?{}", params.len()));
        }
        if let Some(f) = from {
            params.push(f.to_string());
            sql.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        if let Some(t) = to {
            params.push(t.to_string());
            sql.push_str(&format!(" AND ts < ?{}", params.len()));
        }
        let mut stmt = conn.prepare(&sql).ok()?;
        let avg: Option<f64> = stmt
            .query_row(rusqlite::params_from_iter(params), |r| {
                r.get::<_, Option<f64>>(0)
            })
            .ok()?;
        avg.map(|a| a.round() as i64)
    }

    /// Per-UTC-day token totals (input+output) for one provider.
    pub fn provider_daily(&self, provider_id: &str, since: &str) -> Vec<(String, i64)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut stmt = match conn.prepare(
            "SELECT SUBSTR(ts, 1, 10) AS day,
                    SUM(input_tokens + output_tokens)
             FROM usage WHERE provider_id = ?1 AND ts >= ?2
             GROUP BY day ORDER BY day ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let map = |r: &rusqlite::Row| -> rusqlite::Result<(String, i64)> {
            Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        };
        let rows = stmt.query_map(rusqlite::params![provider_id, since], map);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

/// The listing question: which key is still in use, and which has never been.
#[test]
fn last_used_is_the_latest_request_per_key_and_absent_for_an_idle_one() {
    use crate::store::UsageRecord;
    let (_dir, store) = crate::store::test_support::temp_store();
    let id = store.upsert_client_key("kw-ag-claude-a", "claude").unwrap();
    let row = |ts: &str, key: Option<&str>| UsageRecord {
        ts: ts.into(),
        agent: "claude".into(),
        provider_id: Some("p1".into()),
        client_key_id: key.map(str::to_string),
        model: None,
        input_tokens: 1,
        output_tokens: 1,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        latency_ms: None,
        status: "ok".into(),
        cost: None,
        cost_currency: None,
        cost_off_peak: None,
        project: None,
        session_id: None,
        import_key: None,
    };
    store
        .record_usage(&row("2026-01-01T00:00:00+00:00", Some(&id)))
        .unwrap();
    store
        .record_usage(&row("2026-03-01T00:00:00+00:00", Some(&id)))
        .unwrap();
    // A row from before the column exists attributes to no key at all, so it
    // cannot make a key look used.
    store
        .record_usage(&row("2026-09-01T00:00:00+00:00", None))
        .unwrap();

    let seen = store.client_key_last_used().unwrap();
    assert_eq!(
        seen.get(&id).map(String::as_str),
        Some("2026-03-01T00:00:00+00:00"),
        "the latest request, not the first"
    );

    // A key nobody has used is absent rather than dated — the state a reader has
    // to be able to tell from "used a long time ago".
    let idle = store.upsert_client_key("kw-ag-claude-b", "claude").unwrap();
    assert!(!store.client_key_last_used().unwrap().contains_key(&idle));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::temp_store;

    #[test]
    fn usage_record_and_aggregates() {
        let (_dir, store) = temp_store();
        let mk = |ts: &str, provider: &str, input: i64, output: i64| UsageRecord {
            ts: ts.to_string(),
            agent: "claude".to_string(),
            provider_id: Some(provider.to_string()),
            client_key_id: None,
            model: Some("claude-sonnet-4-5".to_string()),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(120),
            status: "ok".to_string(),
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            project: None,
            session_id: None,
            import_key: None,
        };

        store
            .record_usage(&mk("2026-09-06T10:00:00+00:00", "p1", 100, 200))
            .unwrap();
        store
            .record_usage(&mk("2026-09-06T11:00:00+00:00", "p1", 10, 20))
            .unwrap();
        store
            .record_usage(&mk("2026-09-07T10:00:00+00:00", "p2", 7, 3))
            .unwrap();
        store
            .record_usage(&UsageRecord {
                ts: "2026-09-07T12:00:00+00:00".into(),
                agent: "codex".into(),
                provider_id: Some("p1".into()),
                client_key_id: None,
                model: None,
                input_tokens: 5,
                output_tokens: 5,
                cache_read_tokens: 40,
                cache_creation_tokens: 2,
                latency_ms: None,
                status: "error".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
                project: None,
                session_id: None,
                import_key: None,
            })
            .unwrap();

        let totals = store.usage_totals(None, None, None).unwrap();
        assert_eq!(totals.requests, 4);
        assert_eq!(totals.input_tokens, 122);
        assert_eq!(totals.output_tokens, 228);
        assert_eq!(totals.cache_read_tokens, 40);
        assert_eq!(totals.cache_creation_tokens, 2);

        let agent_totals = store.usage_totals(Some("claude"), None, None).unwrap();
        assert_eq!(agent_totals.requests, 3);
        assert_eq!(agent_totals.input_tokens, 117);

        // Provider filter spans agents (2 claude + 1 codex rows on p1).
        let provider_totals = store.usage_totals(None, Some("p1"), None).unwrap();
        assert_eq!(provider_totals.requests, 3);
        // Combined agent + provider filter ANDs both conditions.
        let combined = store
            .usage_totals(Some("claude"), Some("p1"), None)
            .unwrap();
        assert_eq!(combined.requests, 2);

        let since_totals = store
            .usage_totals(None, None, Some("2026-09-07T00:00:00+00:00"))
            .unwrap();
        assert_eq!(since_totals.requests, 2);

        let by_provider = store.usage_by_provider(Some("claude"), None, None).unwrap();
        assert_eq!(by_provider.len(), 2);
        assert_eq!(by_provider[0].provider_id.as_deref(), Some("p1"));
        assert_eq!(by_provider[0].totals.requests, 2);
        assert_eq!(by_provider[0].totals.output_tokens, 220);

        let one_provider = store.usage_by_provider(None, Some("p2"), None).unwrap();
        assert_eq!(one_provider.len(), 1);
        assert_eq!(one_provider[0].provider_id.as_deref(), Some("p2"));

        // tz 0 = UTC: the timestamps below are already UTC dates, so the
        // buckets are unchanged. A non-zero offset is covered by the dashboard
        // test in `kiwano-core`'s `vm::dashboard`, where the window and the
        // chart must agree on it.
        let daily = store.usage_daily(Some("claude"), None, None, 0).unwrap();
        assert_eq!(daily.len(), 2);
        assert_eq!(daily[0].day, "2026-09-06");
        assert_eq!(daily[1].day, "2026-09-07");
        assert_eq!(daily[1].totals.requests, 1);

        let empty = store.usage_totals(Some("gemini"), None, None).unwrap();
        assert_eq!(empty.requests, 0);
        assert_eq!(empty.input_tokens, 0);
    }
}
