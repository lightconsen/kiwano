//! Importing an agent's own history: the rows the client parsed, applied.
//!
//! The client reads the files (they are in the user's home, and `migrate.local.md`
//! §5 #1/#2 puts both halves of that boundary on it: no paths in an API payload,
//! and the daemon never touches user files). What arrives here is a batch of
//! parsed rows, and this module answers three questions about them.
//!
//! **What does it cost?** The files carry tokens, not money. Each row is priced
//! at the **general rate** for its model — the same rung a provider with no
//! catalog entry reaches — because an imported request has no provider to have
//! declared a price. That makes every imported figure an estimate, and a machine
//! that has never synced the Hub prices nothing at all: the cost stays NULL
//! rather than becoming a guess, and a later re-import (which is idempotent)
//! fills it in once the mirror is there.
//!
//! **What must not be counted twice?** Both agents write their own session files
//! whether or not the traffic went through us, so every request served since a
//! takeover appears in *both* ledgers. The rule is a per-agent watermark: import
//! only rows **older than that agent's earliest metered row**
//! ([`Store::first_usage_at`]). It needs no bookkeeping of its own — the ledger
//! already holds the date — and it has the right shape at both ends: an agent
//! that was never taken over has no metered rows at all and gets its whole
//! history, and one that is taken over later stops being imported from that
//! moment on.
//!
//! **What happens if it runs twice?** Nothing. Each row carries an `import_key`
//! derived from the file it came out of, and the write is an upsert on the unique
//! index over it, so a second run refreshes the rows it already has instead of
//! duplicating them — and re-prices the ones that had no price the first time.

use crate::store::{Store, UsageRecord};
use kiwano_api::error::ApiError;
use kiwano_api::history::{HistoryBatch, HistorySessionRow, HistoryUsageRow};

/// What an import did, in the shape the cc-switch import reports in.
///
/// `Default` and [`HistoryImportReport::add`] exist because one scan is many
/// requests: the caller chunks the batch under the body limit, and the number it
/// shows the user is the sum of the pieces.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct HistoryImportReport {
    /// Rows written (or refreshed) in `usage`.
    pub usage_rows: usize,
    /// Sessions written (or refreshed).
    pub sessions: usize,
    /// Rows the watermark kept out: traffic this gateway already metered.
    pub skipped_by_watermark: usize,
    /// Rows priced at the general rate — the ones whose figure is an estimate.
    pub priced: usize,
    /// Rows left unpriced because the Hub's table has no rate for the model.
    pub unpriced: usize,
}

impl HistoryImportReport {
    /// Fold one chunk's report into another.
    pub fn add(&mut self, other: HistoryImportReport) {
        self.usage_rows += other.usage_rows;
        self.sessions += other.sessions;
        self.skipped_by_watermark += other.skipped_by_watermark;
        self.priced += other.priced;
        self.unpriced += other.unpriced;
    }
}

/// Apply one parsed batch. See the module doc for the three rules.
pub fn apply_history(
    store: &Store,
    pricing: &kiwano_adapters::model_pricing::PricingTable,
    batch: &HistoryBatch,
) -> Result<HistoryImportReport, ApiError> {
    let mut report = HistoryImportReport {
        usage_rows: 0,
        sessions: 0,
        skipped_by_watermark: 0,
        priced: 0,
        unpriced: 0,
    };

    // The watermark per agent, read once per batch rather than per row.
    let mut watermarks: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    let mut priceable = Vec::with_capacity(batch.usage.len());
    for row in &batch.usage {
        let watermark = match watermarks.get(&row.agent) {
            Some(w) => w.clone(),
            None => {
                let w = store.first_usage_at(&row.agent).map_err(ApiError::failed)?;
                watermarks.insert(row.agent.clone(), w.clone());
                w
            }
        };
        // Lexicographic, like every other `ts` bound in the store: the shapes
        // are the same because both sides write RFC3339 with an offset.
        if watermark.as_deref().is_some_and(|w| row.ts.as_str() >= w) {
            report.skipped_by_watermark += 1;
            continue;
        }
        let priced = price_row(pricing, row);
        match &priced.cost {
            Some(_) => report.priced += 1,
            None => report.unpriced += 1,
        }
        priceable.push(priced);
    }

    report.usage_rows = store
        .upsert_imported_usage(&priceable)
        .map_err(ApiError::failed)?;
    let sessions: Vec<crate::store::ImportedSession> =
        batch.sessions.iter().map(session_row).collect();
    report.sessions = store
        .upsert_imported_sessions(&sessions)
        .map_err(ApiError::failed)?;
    // The daemon stamps the scan itself rather than taking the client's word for
    // it: what the stamp means is "this ledger has been backfilled", which is a
    // fact about what just landed here.
    if let Err(e) = store.mark_history_scanned() {
        tracing::warn!(error = %e, "could not record the history scan time");
    }
    Ok(report)
}

/// One parsed row plus what the price table says about it.
fn price_row(
    pricing: &kiwano_adapters::model_pricing::PricingTable,
    row: &HistoryUsageRow,
) -> UsageRecord {
    // An empty provider id asks for the general rate — the same rung a provider
    // with no catalog entry reaches. No model, or no rate for it: NULL, which is
    // what "unpriced" looks like everywhere else in the ledger.
    let priced = row
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .and_then(|model| pricing.find("", model))
        .and_then(|entry| {
            let pair = kiwano_adapters::model_pricing::compute_cost_pair(
                entry,
                unix_of(&row.ts),
                row.input_tokens.max(0) as u64,
                row.output_tokens.max(0) as u64,
                row.cache_read_tokens.max(0) as u64,
                row.cache_creation_tokens.max(0) as u64,
                row.cache_inclusive,
            );
            pair.map(|(cost, off_peak)| (cost, off_peak, entry.currency.clone()))
        });

    let (cost, cost_off_peak, cost_currency) = match priced {
        Some((cost, off_peak, currency)) => (Some(cost), Some(off_peak), Some(currency)),
        None => (None, None, None),
    };
    UsageRecord {
        ts: row.ts.clone(),
        agent: row.agent.clone(),
        // A request that went straight to a vendor: no provider, and NULL is how
        // this table says so (migration v31).
        provider_id: None,
        client_key_id: None,
        model: row.model.clone(),
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_read_tokens: row.cache_read_tokens,
        cache_creation_tokens: row.cache_creation_tokens,
        // The files do not record a latency, and inventing one would put a
        // number in the dashboard's p50 that nothing measured.
        latency_ms: None,
        status: "ok".into(),
        cost,
        cost_off_peak,
        cost_currency,
        project: row.project.clone(),
        session_id: row.session_id.clone(),
        import_key: Some(row.import_key.clone()),
    }
}

/// The Unix second a row's timestamp names, for the price table's date windows.
/// An unparseable stamp prices at "now", which is the row's own tie-break only —
/// the price rows are otherwise month-bounded, so the worst case is a price band
/// chosen by the day the import ran rather than the day of the request.
fn unix_of(ts: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.timestamp())
        .unwrap_or_else(|_| chrono::Utc::now().timestamp())
}

/// A session row as the store wants it: the counts become JSON, because a
/// per-session list of names is a payload and not a column.
fn session_row(row: &HistorySessionRow) -> crate::store::ImportedSession {
    let encode = |counts: &[kiwano_api::history::HistoryCount]| -> Option<String> {
        (!counts.is_empty())
            .then(|| serde_json::to_string(counts).unwrap_or_else(|_| "[]".to_string()))
    };
    crate::store::ImportedSession {
        import_key: row.import_key.clone(),
        agent: row.agent.clone(),
        project: row.project.clone(),
        session_id: row.session_id.clone(),
        started_at: row.started_at.clone(),
        ended_at: row.ended_at.clone(),
        turns: row.turns,
        input_tokens: row.input_tokens,
        output_tokens: row.output_tokens,
        cache_read_tokens: row.cache_read_tokens,
        cache_creation_tokens: row.cache_creation_tokens,
        tool_calls: encode(&row.tool_calls),
        skills: encode(&row.skills),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwano_api::history::HistoryCount;

    fn row(agent: &str, ts: &str, key: &str) -> HistoryUsageRow {
        HistoryUsageRow {
            agent: agent.into(),
            ts: ts.into(),
            model: Some("gpt-4o".into()),
            project: Some("kiwano".into()),
            session_id: Some("s-1".into()),
            import_key: key.into(),
            input_tokens: 100,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            cache_inclusive: true,
        }
    }

    fn session(key: &str) -> HistorySessionRow {
        HistorySessionRow {
            import_key: key.into(),
            agent: "claude".into(),
            project: Some("kiwano".into()),
            session_id: "s-1".into(),
            started_at: "2026-01-01T00:00:00+00:00".into(),
            ended_at: "2026-01-01T01:00:00+00:00".into(),
            turns: 4,
            input_tokens: 400,
            output_tokens: 80,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            tool_calls: vec![HistoryCount {
                name: "Bash".into(),
                count: 3,
            }],
            skills: vec![HistoryCount {
                name: "tweet".into(),
                count: 1,
            }],
        }
    }

    /// A store holding an agent, with no prices anywhere: the state a machine is
    /// in before the Hub has ever been synced.
    fn store_unpriced() -> crate::store::Store {
        let s = crate::store::Store::open_in_memory().unwrap();
        s.upsert_client_key("kw-ag-claude-a", "claude").unwrap();
        s
    }

    /// The same store after a Hub sync, whose document prices `gpt-4o` under the
    /// **general** provider — the row a provider-less request reaches.
    fn store_priced() -> crate::store::Store {
        let s = store_unpriced();
        s.save_hub_models_cache(
            1,
            &serde_json::json!({
                "version": 1,
                "generated_at": "2026-09-13T00:00:00Z",
                "exchange_rates": { "USD": 1.0 },
                "models": [{
                    "provider_id": "", "model_id": "gpt-4o", "display_name": "GPT-4o",
                    "input": "2.5", "output": "10",
                    "cache_read": "1.25", "cache_creation": "0",
                    "currency": "USD",
                }],
            })
            .to_string(),
            &"a".repeat(64),
            "2026-09-13T00:00:00Z",
        )
        .unwrap();
        crate::api::pricing_sync::seed_model_pricing(&s).expect("the document seeds");
        s
    }

    /// The table the daemon serves: what `GatewayState::pricing()` would hold.
    fn pricing_of(store: &crate::store::Store) -> kiwano_adapters::model_pricing::PricingTable {
        kiwano_adapters::model_pricing::PricingTable::from_entries(
            store.load_model_pricing().unwrap_or_default(),
        )
    }

    /// Rows older than the agent's first metered row are imported; rows at or
    /// after it are not, because that traffic is already in the ledger.
    #[test]
    fn the_watermark_is_the_agents_first_metered_row() {
        let store = store_priced();
        // The gateway metered a request for `claude` in March.
        store
            .record_usage(&UsageRecord {
                ts: "2026-03-01T00:00:00+00:00".into(),
                agent: "claude".into(),
                provider_id: Some("p1".into()),
                client_key_id: None,
                model: Some("gpt-4o".into()),
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
            })
            .unwrap();

        let batch = HistoryBatch {
            usage: vec![
                row("claude", "2026-01-01T00:00:00+00:00", "claude-code:s:1"),
                row("claude", "2026-03-01T00:00:00+00:00", "claude-code:s:2"),
                row("claude", "2026-04-01T00:00:00+00:00", "claude-code:s:3"),
                // An agent this gateway has never served: everything it has is
                // imported, which is the other half of the rule.
                row("codex", "2026-04-01T00:00:00+00:00", "codex:s:1"),
            ],
            sessions: vec![session("claude-code:s")],
        };
        let report = apply_history(&store, &pricing_of(&store), &batch).unwrap();

        assert_eq!(report.usage_rows, 2, "one before the watermark, one codex");
        assert_eq!(report.skipped_by_watermark, 2);
        assert_eq!(report.sessions, 1);
        // The general rate priced them at all: this is an estimate, and it is
        // the one a provider-less row gets.
        assert_eq!(report.priced, 2);
        assert_eq!(report.unpriced, 0);

        let totals = store.usage_totals(Some("claude"), None, None).unwrap();
        assert_eq!(totals.requests, 2, "the metered row plus the imported one");
    }

    /// A second run of the same batch changes nothing — and a run after the Hub
    /// sync fills in a price the first run could not know.
    #[test]
    fn a_re_import_refreshes_rather_than_duplicating_and_re_prices() {
        // First run on a machine that has never synced: the rows land unpriced.
        let store = store_unpriced();
        let batch = HistoryBatch {
            usage: vec![row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1")],
            sessions: vec![],
        };
        let first = apply_history(&store, &pricing_of(&store), &batch).unwrap();
        assert_eq!(first.usage_rows, 1);
        assert_eq!(first.unpriced, 1, "nothing to price it with");
        assert_eq!(
            store
                .usage_totals(Some("codex"), None, None)
                .unwrap()
                .requests,
            1
        );

        // The same batch again, on a machine that has since synced: still one
        // row, and now it has a cost.
        let store = store_priced();
        let second = apply_history(&store, &pricing_of(&store), &batch).unwrap();
        assert_eq!(second.usage_rows, 1);
        assert_eq!(second.priced, 1);
        assert_eq!(
            store
                .usage_totals(Some("codex"), None, None)
                .unwrap()
                .requests,
            1,
            "the upsert refreshed the row rather than adding one"
        );
        let buckets = store.usage_cost_by_provider(None, None).unwrap();
        let imported = buckets
            .iter()
            .find(|b| b.provider_id.is_none())
            .expect("the imported bucket");
        assert!(imported.cost > 0.0, "the second run priced it");
    }

    /// An imported row names no provider, and the bucket that says so is where
    /// the dashboard's split finds it — not dropped, or the parts would not sum.
    #[test]
    fn an_imported_row_is_the_no_provider_bucket() {
        let store = store_priced();
        let batch = HistoryBatch {
            usage: vec![row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1")],
            sessions: vec![],
        };
        apply_history(&store, &pricing_of(&store), &batch).unwrap();

        let by_provider = store.usage_by_provider(None, None, None).unwrap();
        assert_eq!(by_provider.len(), 1);
        assert_eq!(by_provider[0].provider_id, None);
        assert_eq!(by_provider[0].totals.requests, 1);
    }

    /// A scan arrives in chunks, and **every** chunk lands.
    ///
    /// This is the case the first run against a real home failed: the watermark
    /// was read per batch against a table the import itself was writing, so the
    /// first chunk's rows became the boundary and the rest of the scan was
    /// reported as traffic already metered (806 rows imported out of 118,784).
    /// The fixture is therefore two calls — what the client does for one scan —
    /// with the second chunk's rows *newer* than the first's, which is the shape
    /// that exposes it.
    #[test]
    fn every_chunk_of_one_scan_lands() {
        let store = store_priced();
        let first = HistoryBatch {
            usage: vec![
                row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1"),
                row("codex", "2026-01-02T00:00:00+00:00", "codex:s:2"),
            ],
            sessions: vec![],
        };
        let second = HistoryBatch {
            usage: vec![
                row("codex", "2026-01-03T00:00:00+00:00", "codex:s:3"),
                row("codex", "2026-01-04T00:00:00+00:00", "codex:s:4"),
            ],
            sessions: vec![],
        };
        let a = apply_history(&store, &pricing_of(&store), &first).unwrap();
        let b = apply_history(&store, &pricing_of(&store), &second).unwrap();
        assert_eq!((a.usage_rows, a.skipped_by_watermark), (2, 0));
        assert_eq!(
            (b.usage_rows, b.skipped_by_watermark),
            (2, 0),
            "the second chunk is not skipped: imported rows do not move the watermark"
        );
        assert_eq!(
            store
                .usage_totals(Some("codex"), None, None)
                .unwrap()
                .requests,
            4
        );

        // And the gateway's own row still draws the line where it should: a
        // request it metered on the 3rd makes that day and later the agent's own
        // traffic, so a re-scan of the same files imports only what is older.
        store
            .record_usage(&UsageRecord {
                ts: "2026-01-03T00:00:00+00:00".into(),
                agent: "codex".into(),
                provider_id: Some("p1".into()),
                client_key_id: None,
                model: Some("gpt-4o".into()),
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
            })
            .unwrap();
        let again = apply_history(&store, &pricing_of(&store), &second).unwrap();
        assert_eq!(
            (again.usage_rows, again.skipped_by_watermark),
            (0, 2),
            "the gateway metered that traffic, so the files' copy of it is skipped"
        );
    }

    /// The session half lands too, counts and all.
    #[test]
    fn sessions_are_written_with_their_counts() {
        let store = store_priced();
        let batch = HistoryBatch {
            usage: vec![],
            sessions: vec![session("claude-code:s")],
        };
        let report = apply_history(&store, &pricing_of(&store), &batch).unwrap();
        assert_eq!(report.sessions, 1);
        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].turns, 4);
        assert_eq!(listed[0].project.as_deref(), Some("kiwano"));
        assert!(listed[0].tool_calls.as_deref().unwrap().contains("Bash"));
        assert!(listed[0].skills.as_deref().unwrap().contains("tweet"));
    }
}
