//! Full request captures: the `request_logs` metadata table, its sibling
//! body table, the paged list, the export and the detail view.
//!
//! The types, the WHERE fragment, the column list and the row mapper are
//! one unit and stay together here. `REQUEST_LOG_COLUMNS` names the columns
//! in the order `request_log_from_row` reads them by index, and both must
//! agree with the column order of the `INSERT` in `insert_request_log` —
//! nothing in the compiler checks that, so a split that separates them
//! compiles and fails at runtime.

use crate::error::Result;
use crate::store::Store;
use chrono::Utc;
use rusqlite::{params, OptionalExtension};
use serde::Serialize;

/// One complete data-plane request awaiting persistence (tech.md: full request
/// logging). Bodies are optional — disabled capture or oversize truncation.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLogNew {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// Attributed agent; None for failures before attribution (e.g. 404).
    pub agent: Option<String>,
    /// Attribution method: `key` / `path_fallback` (see router Attribution).
    pub attribution: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// Status code the client ultimately received.
    pub status_code: i64,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub is_streaming: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// A slice of `output_tokens`; 0 when the provider does not report it.
    pub reasoning_tokens: i64,
    /// True when no usage object arrived at all — see `RequestLogEntry`.
    pub usage_missing: bool,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    /// Header maps serialized as JSON with credential headers redacted.
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    /// lossy-UTF8 request body (already truncated to the capture cap).
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    /// Original byte sizes (before any truncation).
    pub request_size: i64,
    pub response_size: i64,
    pub truncated: bool,
    /// Computed request cost in the price entry's currency (migration v9).
    pub cost: Option<f64>,
    pub cost_currency: Option<String>,
    /// The off-peak equivalent of `cost` (migration v13) — see `UsageRecord`.
    pub cost_off_peak: Option<f64>,
    /// What the compat shim changed about this request, one line per action
    /// (migration v26). `None` when nothing was touched — the common case.
    pub request_notes: Option<String>,
}

/// Metadata row of `request_logs` (list view — never includes bodies).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestLogEntry {
    pub id: i64,
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub agent: Option<String>,
    pub attribution: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub status_code: i64,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub is_streaming: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    /// A slice of `output_tokens`, never an addition to them; 0 when the
    /// provider does not break its thinking out.
    pub reasoning_tokens: i64,
    /// The upstream reported no usage object at all, so every token column on
    /// this row is a zero that means "unknown" rather than "none".
    pub usage_missing: bool,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_size: i64,
    pub response_size: i64,
    pub truncated: bool,
    /// What the request cost, in `cost_currency`. Carried from the row's own
    /// currency (never converted here) so an export can state it as recorded.
    pub cost: Option<f64>,
    pub cost_currency: Option<String>,
    /// Its off-peak equivalent (migration v13) — see `UsageRecord`. Not in the
    /// CSV export: that file states what happened, not what could have.
    pub cost_off_peak: Option<f64>,
    /// What the compat shim changed about this request (migration v26), one
    /// line per action; `None` when nothing was touched.
    pub request_notes: Option<String>,
}

/// One row of an export: the metadata every export carries, plus the captured
/// bodies when the caller asked for them (the export is the only reader that
/// can ask — the list view never does).
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLogExportRow {
    pub entry: RequestLogEntry,
    /// `None` when the export was run without bodies, or when this row has
    /// none stored (a pre-forward failure, or a pruned body table row).
    pub request_body: Option<String>,
    pub response_body: Option<String>,
}

impl RequestLogExportRow {
    /// The metadata-only shape: bodies deliberately absent.
    pub fn from_entry(entry: RequestLogEntry) -> Self {
        RequestLogExportRow {
            entry,
            request_body: None,
            response_body: None,
        }
    }
}

/// Detail view: metadata + the captured bodies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestLogDetail {
    #[serde(flatten)]
    pub entry: RequestLogEntry,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
}

/// Filter for `list_request_logs`. `status` is "ok" (<400) or "error" (>=400).
/// `from`/`to` are RFC3339 UTC bounds, half-open (`from <= ts < to`) — stored
/// timestamps are RFC3339 text, which orders lexicographically, so the range is
/// a plain string comparison.
#[derive(Debug, Clone, Default)]
pub struct RequestLogFilter<'a> {
    pub agent: Option<&'a str>,
    pub provider_id: Option<&'a str>,
    pub status: Option<&'a str>,
    pub from: Option<&'a str>,
    pub to: Option<&'a str>,
}

/// The WHERE every request-log read shares, so a page and an export can never
/// disagree about which rows they cover. Fixed positional params keep the SQL
/// simple: an absent filter binds NULL and drops out.
const REQUEST_LOG_WHERE: &str = " WHERE (?1 IS NULL OR agent = ?1)
                                  AND (?2 IS NULL OR provider_id = ?2)
                                  AND (?3 IS NULL OR (?3 = 'ok' AND status_code < 400)
                                                   OR (?3 = 'error' AND status_code >= 400))
                                  AND (?4 IS NULL OR ts >= ?4)
                                  AND (?5 IS NULL OR ts < ?5)";

/// The columns `request_log_from_row` reads, in the order it reads them.
const REQUEST_LOG_COLUMNS: &str = "id, ts, method, path, query, agent, attribution, provider_id,
                                   model, status_code, error_kind, error_message, session_id,
                                   is_streaming, input_tokens, output_tokens, cache_read_tokens,
                                   cache_creation_tokens, latency_ms, first_token_ms,
                                   request_headers, response_headers, request_size, response_size,
                                   truncated, cost, cost_currency, cost_off_peak,
                                   reasoning_tokens, usage_missing, request_notes";

/// How many columns [`REQUEST_LOG_COLUMNS`] names. The body join appends two
/// more, and the only way to read them by index without counting commas is to
/// count them here — an off-by-one silently reads the wrong field into
/// `request_body` (the CSV export test caught exactly that when v13 added one).
const REQUEST_LOG_COLUMN_COUNT: usize = 31;

/// Ceiling on one export. A local log can be large, and the CSV is built in
/// memory before it is written, so the read is capped rather than unbounded;
/// callers detect the cap by asking for one row more than they will write.
pub const EXPORT_ROW_CAP: i64 = 100_000;

impl Store {
    // ---- request logs (full captures) ------------------------------------

    /// Persist one completed request: metadata + bodies in a single
    /// transaction so a list row never appears without its bodies.
    pub fn insert_request_log(&self, r: &RequestLogNew) -> Result<i64> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO request_logs (ts, method, path, query, agent, attribution,
                                       provider_id, model, status_code, error_kind,
                                       error_message, session_id, is_streaming,
                                       input_tokens, output_tokens, cache_read_tokens,
                                       cache_creation_tokens, latency_ms, first_token_ms,
                                       request_headers, response_headers,
                                       request_size, response_size, truncated,
                                       cost, cost_currency, cost_off_peak,
                                       reasoning_tokens, usage_missing, request_notes)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26, ?27,
                     ?28, ?29, ?30)",
            params![
                r.ts,
                r.method,
                r.path,
                r.query,
                r.agent,
                r.attribution,
                r.provider_id,
                r.model,
                r.status_code,
                r.error_kind,
                r.error_message,
                r.session_id,
                r.is_streaming as i64,
                r.input_tokens,
                r.output_tokens,
                r.cache_read_tokens,
                r.cache_creation_tokens,
                r.latency_ms,
                r.first_token_ms,
                r.request_headers,
                r.response_headers,
                r.request_size,
                r.response_size,
                r.truncated as i64,
                r.cost,
                r.cost_currency,
                r.cost_off_peak,
                r.reasoning_tokens,
                r.usage_missing as i64,
                r.request_notes,
            ],
        )?;
        let id = tx.last_insert_rowid();
        if r.request_body.is_some() || r.response_body.is_some() {
            tx.execute(
                "INSERT INTO request_bodies (log_id, request_body, response_body)
                 VALUES (?1, ?2, ?3)",
                params![id, r.request_body, r.response_body],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Paged list (newest first) with optional filters; returns rows + total.
    pub fn list_request_logs(
        &self,
        page: i64,
        page_size: i64,
        filter: RequestLogFilter<'_>,
    ) -> Result<(Vec<RequestLogEntry>, i64)> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs{REQUEST_LOG_WHERE}"),
            params![
                filter.agent,
                filter.provider_id,
                filter.status,
                filter.from,
                filter.to
            ],
            |r| r.get(0),
        )?;

        let mut stmt = conn.prepare(&format!(
            "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs{REQUEST_LOG_WHERE}
             ORDER BY id DESC LIMIT ?6 OFFSET ?7",
        ))?;
        let offset = (page - 1).max(0) * page_size;
        let rows = stmt
            .query_map(
                params![
                    filter.agent,
                    filter.provider_id,
                    filter.status,
                    filter.from,
                    filter.to,
                    page_size,
                    offset
                ],
                request_log_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((rows, total))
    }

    /// Every row the filter matches, newest first — the export's read. Unpaged
    /// on purpose: a page size is a display concern, and letting one cap an
    /// export would drop rows silently. `limit` is the explicit cap instead,
    /// so the caller can say so rather than produce a quietly short file.
    pub fn export_request_logs(
        &self,
        filter: RequestLogFilter<'_>,
        limit: i64,
    ) -> Result<Vec<RequestLogEntry>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs{REQUEST_LOG_WHERE}
             ORDER BY id DESC LIMIT ?6",
        ))?;
        let rows = stmt
            .query_map(
                params![
                    filter.agent,
                    filter.provider_id,
                    filter.status,
                    filter.from,
                    filter.to,
                    limit
                ],
                request_log_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The same export read, with the captured bodies attached.
    ///
    /// `request_bodies` is keyed by `log_id`, not a column of `request_logs`,
    /// so the bodies come from a LEFT JOIN rather than from the shared
    /// [`REQUEST_LOG_COLUMNS`] slice — a row whose body was never stored (a
    /// pre-forward failure, a pruned body table row) still exports, with NULL
    /// bodies, instead of disappearing from the file. Row order and the filter
    /// are identical to [`Self::export_request_logs`]; only the two extra
    /// columns differ.
    pub fn export_request_logs_with_bodies(
        &self,
        filter: RequestLogFilter<'_>,
        limit: i64,
    ) -> Result<Vec<RequestLogExportRow>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {REQUEST_LOG_COLUMNS}, b.request_body, b.response_body
             FROM request_logs LEFT JOIN request_bodies b ON b.log_id = request_logs.id
             {REQUEST_LOG_WHERE}
             ORDER BY request_logs.id DESC LIMIT ?6",
        ))?;
        let rows = stmt
            .query_map(
                params![
                    filter.agent,
                    filter.provider_id,
                    filter.status,
                    filter.from,
                    filter.to,
                    limit
                ],
                |row| {
                    Ok(RequestLogExportRow {
                        entry: request_log_from_row(row)?,
                        request_body: row.get(REQUEST_LOG_COLUMN_COUNT)?,
                        response_body: row.get(REQUEST_LOG_COLUMN_COUNT + 1)?,
                    })
                },
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// COUNT of data-plane requests (every row: forwarded + pre-forward
    /// failures), optionally filtered by agent, provider and/or a window.
    /// `since`/`until` are half-open, which is what lets the dashboard ask for
    /// the *previous* period as one query. The dashboard headline reads this
    /// instead of `usage_totals`: usage rows only cover forwarded requests, so
    /// pre-forward failures would silently vanish from the top stat while the
    /// Logs card below still shows them.
    pub fn count_request_logs(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        until: Option<&str>,
    ) -> Result<i64> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since, until);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs WHERE 1=1{cond}"),
            rusqlite::params_from_iter(params),
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// One request with bodies (detail view); None if the id is unknown.
    pub fn get_request_log(&self, id: i64) -> Result<Option<RequestLogDetail>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let entry = {
            let mut stmt = conn.prepare(&format!(
                "SELECT {REQUEST_LOG_COLUMNS} FROM request_logs WHERE id = ?1"
            ))?;
            stmt.query_row(params![id], request_log_from_row)
                .optional()?
        };
        let Some(entry) = entry else {
            return Ok(None);
        };
        let bodies: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT request_body, response_body FROM request_bodies WHERE log_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(Some(RequestLogDetail {
            entry,
            request_body: bodies.as_ref().and_then(|b| b.0.clone()),
            response_body: bodies.as_ref().and_then(|b| b.1.clone()),
        }))
    }

    /// Delete rows older than `retain_days`; bodies cascade. Returns removed count.
    /// `None` prunes nothing, which is what an install that never set a
    /// retention gets — and, before this took an `Option`, what a *failed read*
    /// of the config silently turned into the opposite of: the caller's
    /// `unwrap_or_default()` produced a zero, and zero days keeps nothing at
    /// all (`DELETE … WHERE ts < now`). A transient error at startup deleted
    /// the log.
    pub fn prune_request_logs(&self, retain_days: Option<u32>) -> Result<usize> {
        let Some(days) = retain_days else {
            return Ok(0);
        };
        let cutoff = (Utc::now() - chrono::Duration::days(days as i64)).to_rfc3339();
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM request_logs WHERE ts < ?1", params![cutoff])?;
        Ok(n)
    }

    /// Delete every logged request (GUI "clear log" action).
    pub fn clear_request_logs(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(conn.execute("DELETE FROM request_logs", [])?)
    }
}

fn request_log_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogEntry> {
    Ok(RequestLogEntry {
        id: row.get(0)?,
        ts: row.get(1)?,
        method: row.get(2)?,
        path: row.get(3)?,
        query: row.get(4)?,
        agent: row.get(5)?,
        attribution: row.get(6)?,
        provider_id: row.get(7)?,
        model: row.get(8)?,
        status_code: row.get(9)?,
        error_kind: row.get(10)?,
        error_message: row.get(11)?,
        session_id: row.get(12)?,
        is_streaming: row.get::<_, i64>(13)? != 0,
        input_tokens: row.get(14)?,
        output_tokens: row.get(15)?,
        cache_read_tokens: row.get(16)?,
        cache_creation_tokens: row.get(17)?,
        latency_ms: row.get(18)?,
        first_token_ms: row.get(19)?,
        request_headers: row.get(20)?,
        response_headers: row.get(21)?,
        request_size: row.get(22)?,
        response_size: row.get(23)?,
        truncated: row.get::<_, i64>(24)? != 0,
        cost: row.get(25)?,
        cost_currency: row.get(26)?,
        cost_off_peak: row.get(27)?,
        reasoning_tokens: row.get(28)?,
        usage_missing: row.get::<_, i64>(29)? != 0,
        request_notes: row.get(30)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{sample_log, temp_store};
    use crate::store::time::now_rfc3339;

    #[test]
    fn request_log_date_range_is_half_open() {
        let (_dir, store) = temp_store();
        // The bound is `from <= ts < to`, and the shapes matter: real rows come
        // from chrono (`...+00:00`, sometimes fractional), the seeded fixture
        // writes `...Z`, and the two sort differently against a `.000Z` bound.
        for ts in [
            "2026-09-06T23:59:59+00:00", // before
            "2026-09-07T00:00:00+00:00", // exactly at `from` — included
            "2026-09-07T12:00:00+00:00",
            "2026-09-07T23:59:59.500+00:00", // fractional, still inside
            "2026-09-08T00:00:00+00:00",     // exactly at `to` — excluded
        ] {
            store
                .insert_request_log(&sample_log(ts, Some("claude"), 200))
                .unwrap();
        }

        let window = RequestLogFilter {
            from: Some("2026-09-07T00:00:00+00:00"),
            to: Some("2026-09-08T00:00:00+00:00"),
            ..Default::default()
        };
        let (rows, total) = store.list_request_logs(1, 10, window.clone()).unwrap();
        assert_eq!(total, 3, "the day's rows, and neither edge crossed");
        let ts: Vec<&str> = rows.iter().map(|r| r.ts.as_str()).collect();
        assert!(
            ts.contains(&"2026-09-07T00:00:00+00:00"),
            "from is inclusive"
        );
        assert!(
            ts.contains(&"2026-09-07T23:59:59.500+00:00"),
            "fractional rows sort inside"
        );
        assert!(
            !ts.contains(&"2026-09-08T00:00:00+00:00"),
            "to is exclusive"
        );
        assert!(!ts.contains(&"2026-09-06T23:59:59+00:00"));

        // A `.000Z` bound is not usable: '.' sorts after '+' at that index, so
        // the bound lands *past* a row stored as `+00:00` at the same instant
        // and drops it. The count alone hides this — same total, different set.
        let loose = RequestLogFilter {
            from: Some("2026-09-07T00:00:00.000Z"),
            ..Default::default()
        };
        let (rows, total) = store.list_request_logs(1, 10, loose).unwrap();
        assert_eq!(total, 3);
        assert!(
            !rows.iter().any(|r| r.ts == "2026-09-07T00:00:00+00:00"),
            "the row sitting exactly on the bound is the one it loses"
        );
    }

    #[test]
    fn request_log_date_range_composes_with_the_other_filters() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T11:00:00+00:00",
                Some("claude"),
                502,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log("2026-09-07T12:00:00+00:00", Some("codex"), 502))
            .unwrap();
        store
            .insert_request_log(&sample_log(
                "2026-09-08T10:00:00+00:00",
                Some("claude"),
                502,
            ))
            .unwrap();

        let (_, total) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    agent: Some("claude"),
                    status: Some("error"),
                    from: Some("2026-09-07T00:00:00+00:00"),
                    to: Some("2026-09-08T00:00:00+00:00"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(total, 1, "all four predicates at once, on the right ?N");
    }

    #[test]
    fn export_reads_past_one_page_and_stops_at_the_cap() {
        let (_dir, store) = temp_store();
        for i in 0..7 {
            store
                .insert_request_log(&sample_log(
                    &format!("2026-09-07T0{i}:00:00+00:00"),
                    Some("claude"),
                    200,
                ))
                .unwrap();
        }
        store
            .insert_request_log(&sample_log("2026-09-06T10:00:00+00:00", Some("codex"), 200))
            .unwrap();

        let claude = RequestLogFilter {
            agent: Some("claude"),
            ..Default::default()
        };
        let rows = store.export_request_logs(claude.clone(), 100).unwrap();
        assert_eq!(rows.len(), 7, "every match, not one page of them");
        assert!(rows[0].ts > rows[6].ts, "newest first, like the list");

        assert_eq!(
            store.export_request_logs(claude, 3).unwrap().len(),
            3,
            "the cap is what limits it"
        );
    }

    #[test]
    fn export_with_bodies_joins_the_separate_body_table() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        // A second row with no body stored: the LEFT JOIN must keep it.
        let mut bodyless = sample_log("2026-09-07T11:00:00+00:00", Some("claude"), 502);
        bodyless.request_body = None;
        bodyless.response_body = None;
        store.insert_request_log(&bodyless).unwrap();

        let rows = store
            .export_request_logs_with_bodies(RequestLogFilter::default(), 100)
            .unwrap();
        assert_eq!(rows.len(), 2, "a row without a body still exports");
        assert_eq!(
            rows[0].entry.status_code, 502,
            "newest first, as in the list"
        );
        assert_eq!(rows[0].request_body, None);
        assert_eq!(rows[0].response_body, None);
        assert_eq!(
            rows[1].request_body.as_deref(),
            Some(r#"{"model":"claude-sonnet-4-5"}"#)
        );
        assert_eq!(rows[1].response_body.as_deref(), Some(r#"{"ok":true}"#));
        // Metadata rides along unchanged — same columns as the bodyless read.
        assert_eq!(
            rows[1].entry,
            store
                .export_request_logs(RequestLogFilter::default(), 100)
                .unwrap()[1]
        );
    }

    #[test]
    fn request_log_insert_list_detail_roundtrip() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log("2026-09-07T11:00:00+00:00", Some("codex"), 502))
            .unwrap();
        // Pre-attribution failure: no agent/provider at all.
        store
            .insert_request_log(&sample_log("2026-09-07T12:00:00+00:00", None, 404))
            .unwrap();

        let (rows, total) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].status_code, 404); // newest first

        let detail = store.get_request_log(rows[0].id).unwrap().unwrap();
        assert_eq!(detail.entry.status_code, 404);
        assert_eq!(detail.entry.error_kind.as_deref(), Some("upstream_error"));
        assert_eq!(
            detail.request_body.as_deref(),
            Some(r#"{"model":"claude-sonnet-4-5"}"#)
        );
        assert_eq!(detail.response_body.as_deref(), Some(r#"{"ok":true}"#));

        // Filters.
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    agent: Some("claude"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 1);
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    status: Some("error"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 2); // 502 + 404
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    status: Some("ok"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 1);

        // Pagination.
        let (rows, _) = store
            .list_request_logs(2, 2, RequestLogFilter::default())
            .unwrap();
        assert_eq!(rows.len(), 1);

        // A row with no body stored (a pre-forward failure) still lists; its
        // detail has no bodies.
        let mut bodyless = sample_log("2026-09-07T13:00:00+00:00", Some("pi"), 200);
        bodyless.request_body = None;
        bodyless.response_body = None;
        let id = store.insert_request_log(&bodyless).unwrap();
        let detail = store.get_request_log(id).unwrap().unwrap();
        assert_eq!(detail.request_body, None);
        assert_eq!(detail.response_body, None);

        assert!(store.get_request_log(9999).unwrap().is_none());
    }

    #[test]
    fn count_request_logs_windows_by_ts() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-01T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                503,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log("2026-09-08T10:00:00+00:00", None, 404))
            .unwrap();

        assert_eq!(store.count_request_logs(None, None, None, None).unwrap(), 3);
        // Failures count too — the dashboard headline uses this.
        assert_eq!(
            store
                .count_request_logs(None, None, Some("2026-09-07T00:00:00Z"), None)
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .count_request_logs(None, None, Some("2026-09-09T00:00:00Z"), None)
                .unwrap(),
            0
        );
        // Agent / provider filters: the agentless row (no provider either) is
        // only counted in the unfiltered totals.
        assert_eq!(
            store
                .count_request_logs(Some("claude"), None, None, None)
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .count_request_logs(None, Some("p1"), None, None)
                .unwrap(),
            2
        );
    }

    #[test]
    fn request_log_prune_and_clear() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-08-01T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log(now_rfc3339().as_str(), Some("claude"), 200))
            .unwrap();

        // Old row (plus its bodies) goes; fresh row stays.
        assert_eq!(store.prune_request_logs(Some(30)).unwrap(), 1);
        let (rows, total) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(total, 1);
        let detail = store.get_request_log(rows[0].id).unwrap().unwrap();
        assert_eq!(
            detail.request_body.as_deref(),
            Some(r#"{"model":"claude-sonnet-4-5"}"#)
        );

        assert_eq!(store.clear_request_logs().unwrap(), 1);
        assert_eq!(
            store
                .list_request_logs(1, 10, RequestLogFilter::default())
                .unwrap()
                .1,
            0
        );
    }
}
