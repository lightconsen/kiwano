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
    /// Rows left unpriced because neither the Hub's table nor a declaration the
    /// user made has a rate for the model.
    pub unpriced: usize,
    /// Which models those were, most rows first.
    ///
    /// A count alone leaves the reader with nothing to do about it: the names are
    /// what tell them whether this is a model nobody prices, a plan's own name
    /// for a model the plan grants, or the placeholder a client writes when it is
    /// not talking to a model at all. Capped, because the report is a line in a
    /// terminal and the tail is not actionable.
    #[serde(default)]
    pub unpriced_models: Vec<UnpricedModel>,
}

/// One model some rows could not be priced for.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct UnpricedModel {
    /// `None` for a row whose request named no model at all.
    pub model: Option<String>,
    pub rows: usize,
}

/// How many names the report carries. See [`HistoryImportReport::unpriced_models`].
const UNPRICED_MODELS_REPORTED: usize = 5;

impl HistoryImportReport {
    /// Fold one chunk's report into another.
    pub fn add(&mut self, other: HistoryImportReport) {
        self.usage_rows += other.usage_rows;
        self.sessions += other.sessions;
        self.skipped_by_watermark += other.skipped_by_watermark;
        self.priced += other.priced;
        self.unpriced += other.unpriced;
        // Summed by name across chunks, then re-ranked and re-capped: one scan is
        // many requests, and a model's rows are not all in the same one.
        //
        // Summed with `entry().or_default() +=`, **not** collected into a map:
        // `collect()` keeps the last value for a repeated key, which would report
        // the final chunk's counts as if they were the scan's — 12 unpriced rows
        // named out of 494, on the run that found this.
        let mut counted: std::collections::HashMap<Option<String>, usize> =
            std::collections::HashMap::new();
        for u in self.unpriced_models.drain(..).chain(other.unpriced_models) {
            *counted.entry(u.model).or_default() += u.rows;
        }
        let mut ranked: Vec<UnpricedModel> = counted
            .drain()
            .map(|(model, rows)| UnpricedModel { model, rows })
            .collect();
        ranked.sort_by(|a, b| b.rows.cmp(&a.rows).then_with(|| a.model.cmp(&b.model)));
        ranked.truncate(UNPRICED_MODELS_REPORTED);
        self.unpriced_models = ranked;
    }
}

/// Apply one parsed batch. See the module doc for the three rules.
///
/// Two price tables, in the order a request consults them: what the user declared
/// for a model (their own statement, and so the more specific one) and then the
/// Hub's table. A declaration normally belongs to a provider row, and an imported
/// row has no provider — so here it is looked up **by model alone**, across every
/// provider the user declared prices on. That is the escape hatch for a name the
/// Hub cannot price: someone who knows what `kimi-for-coding` is being charged
/// can say so, and the import will use it instead of leaving the rows blank.
pub fn apply_history(
    store: &Store,
    pricing: &kiwano_adapters::model_pricing::PricingTable,
    declared: &kiwano_adapters::model_pricing::PricingTable,
    batch: &HistoryBatch,
) -> Result<HistoryImportReport, ApiError> {
    let mut report = HistoryImportReport::default();

    // The watermark per agent, read once per batch rather than per row.
    let mut watermarks: std::collections::HashMap<String, Option<String>> =
        std::collections::HashMap::new();
    // The machine every row in this batch came from: one fact about the scan, so
    // it is read once and stamped on both halves (usage and sessions).
    let machine = batch.source_machine.as_deref();
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
        let priced = price_row(pricing, declared, row, machine);
        match &priced.cost {
            Some(_) => report.priced += 1,
            None => {
                report.unpriced += 1;
                // Kept by name rather than counted: see `unpriced_models`.
                let entry = match report
                    .unpriced_models
                    .iter_mut()
                    .find(|u| u.model == row.model)
                {
                    Some(entry) => entry,
                    None => {
                        report.unpriced_models.push(UnpricedModel {
                            model: row.model.clone(),
                            rows: 0,
                        });
                        report.unpriced_models.last_mut().expect("just pushed")
                    }
                };
                entry.rows += 1;
            }
        }
        priceable.push(priced);
    }

    report.usage_rows = store
        .upsert_imported_usage(&priceable)
        .map_err(ApiError::failed)?;
    let sessions: Vec<crate::store::ImportedSession> = batch
        .sessions
        .iter()
        .map(|row| session_row(row, machine))
        .collect();
    report.sessions = store
        .upsert_imported_sessions(&sessions)
        .map_err(ApiError::failed)?;
    // The daemon stamps the scan itself rather than taking the client's word for
    // *when* — but it stamps only the agents the client says it read, because only
    // the client knows that. A row-less agent still gets stamped: "read it, there
    // was nothing" is a fact, and without it the client re-reads it every launch.
    if let Err(e) = store.mark_history_scanned(&batch.scanned_agents) {
        tracing::warn!(error = %e, "could not record the history scan time");
    }
    Ok(report)
}

/// One parsed row plus what the price tables say about it.
///
/// The order is the request path's: what the user declared for a model first,
/// then the Hub's rate. On a row with a provider that distinction is about
/// precedence between two statements about the *same* provider; here it is
/// stronger, because the declaration is the only one of the two that can know
/// what a name like `kimi-for-coding` is being charged.
///
/// An empty provider id asks each table for the model alone — the general rate in
/// the Hub's, and the user's own row on whichever provider they declared it.
fn price_row(
    pricing: &kiwano_adapters::model_pricing::PricingTable,
    declared: &kiwano_adapters::model_pricing::PricingTable,
    row: &HistoryUsageRow,
    machine: Option<&str>,
) -> UsageRecord {
    // No model, or no rate for it anywhere: NULL, which is what "unpriced" looks
    // like everywhere else in the ledger.
    let priced = row
        .model
        .as_deref()
        .filter(|m| !m.is_empty())
        .and_then(|model| declared.find("", model).or_else(|| pricing.find("", model)))
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
        // The machine this scan came from — one label for every row the batch
        // produced (v32). `None` for a client that could not name itself.
        imported_from: machine.map(str::to_string),
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
fn session_row(row: &HistorySessionRow, machine: Option<&str>) -> crate::store::ImportedSession {
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
        // The same batch-level machine as the usage rows, for the same reason.
        imported_from: machine.map(str::to_string),
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
                imported_from: None,
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
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let report = apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();

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
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let first = apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();
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
        let second = apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();
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
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();

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
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let second = HistoryBatch {
            usage: vec![
                row("codex", "2026-01-03T00:00:00+00:00", "codex:s:3"),
                row("codex", "2026-01-04T00:00:00+00:00", "codex:s:4"),
            ],
            sessions: vec![],
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let hub = pricing_of(&store);
        let none = kiwano_adapters::model_pricing::PricingTable::default();
        let a = apply_history(&store, &hub, &none, &first).unwrap();
        let b = apply_history(&store, &hub, &none, &second).unwrap();
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
                imported_from: None,
            })
            .unwrap();
        let again = apply_history(&store, &hub, &none, &second).unwrap();
        assert_eq!(
            (again.usage_rows, again.skipped_by_watermark),
            (0, 2),
            "the gateway metered that traffic, so the files' copy of it is skipped"
        );
    }

    /// The report names the models it could not price, because a count alone
    /// leaves the reader with nothing to act on.
    #[test]
    fn the_report_names_the_models_it_could_not_price() {
        let store = store_priced();
        let mut unknown = row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1");
        unknown.model = Some("a-plan-name".into());
        let mut unknown2 = row("codex", "2026-01-02T00:00:00+00:00", "codex:s:2");
        unknown2.model = Some("a-plan-name".into());
        let mut nameless = row("codex", "2026-01-03T00:00:00+00:00", "codex:s:3");
        nameless.model = None;
        let batch = HistoryBatch {
            usage: vec![unknown, unknown2, nameless],
            sessions: vec![],
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let report = apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();
        assert_eq!(report.unpriced, 3);
        assert_eq!(report.unpriced_models.len(), 2);
        assert_eq!(
            report.unpriced_models[0].model.as_deref(),
            Some("a-plan-name")
        );
        assert_eq!(report.unpriced_models[0].rows, 2);
        assert_eq!(report.unpriced_models[1].model, None, "a row with no model");
    }

    /// Two chunks of one scan add up their per-model counts instead of the
    /// second replacing the first.
    ///
    /// The failure this pins is quiet and plausible-looking: a merge that keeps
    /// the last value for a repeated key reports the final chunk's counts as the
    /// scan's, which on a real history named 12 rows out of 494 unpriced.
    #[test]
    fn per_model_counts_add_up_across_chunks() {
        let store = store_priced();
        let hub = pricing_of(&store);
        let none = kiwano_adapters::model_pricing::PricingTable::default();
        let unpriced = |key: &str, model: &str| {
            let mut r = row("codex", "2026-01-01T00:00:00+00:00", key);
            r.model = Some(model.into());
            r
        };
        let first = apply_history(
            &store,
            &hub,
            &none,
            &HistoryBatch {
                usage: vec![
                    unpriced("codex:a", "a-plan-name"),
                    unpriced("codex:b", "a-plan-name"),
                ],
                sessions: vec![],
                scanned_agents: Vec::new(),
                source_machine: None,
            },
        )
        .unwrap();
        let second = apply_history(
            &store,
            &hub,
            &none,
            &HistoryBatch {
                usage: vec![
                    unpriced("codex:c", "a-plan-name"),
                    unpriced("codex:d", "other"),
                ],
                sessions: vec![],
                scanned_agents: Vec::new(),
                source_machine: None,
            },
        )
        .unwrap();
        assert_eq!(first.unpriced_models[0].rows, 2);

        let mut total = HistoryImportReport::default();
        total.add(first);
        total.add(second);
        assert_eq!(total.unpriced, 4);
        assert_eq!(
            total.unpriced_models[0].model.as_deref(),
            Some("a-plan-name"),
            "the most rows first"
        );
        assert_eq!(
            total.unpriced_models[0].rows, 3,
            "two from the first chunk and one from the second, summed"
        );
        assert_eq!(total.unpriced_models[1].rows, 1);
    }

    /// What the user declares for a model reaches an imported row — the escape
    /// hatch for a name the Hub cannot price, and the reason a plan's own model
    /// name does not need a table of guesses in the code.
    #[test]
    fn a_declared_price_prices_an_imported_row() {
        let store = store_unpriced();
        // The user says what their provider charges for the plan's own name. It is
        // declared on a provider row, as declarations are.
        let mut p = crate::limits::test_support::test_provider("kimi-plan");
        p.prices = Some(
            r#"{"currency":"USD","models":[{"model_id":"kimi-for-coding","input":"1.5","output":"6","cache_read":"0.15","cache_creation":"1.8"}]}"#
                .into(),
        );
        store.insert_provider(&p).unwrap();

        let mut r = row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1");
        r.model = Some("kimi-for-coding".into());
        let batch = HistoryBatch {
            usage: vec![r],
            sessions: vec![],
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        // The Hub's table is empty here: the declaration is the only thing that
        // can price it, which is exactly the case this exists for.
        let hub = kiwano_adapters::model_pricing::PricingTable::default();
        let declared = kiwano_adapters::model_pricing::PricingTable::from_entries(
            store.load_declared_prices().unwrap(),
        );
        let report = apply_history(&store, &hub, &declared, &batch).unwrap();
        assert_eq!((report.priced, report.unpriced), (1, 0));
        let buckets = store.usage_cost_by_provider(None, None).unwrap();
        let cost = buckets
            .iter()
            .find(|b| b.provider_id.is_none())
            .expect("the imported bucket")
            .cost;
        // 100 input at 1.5/M + 20 output at 6/M, cache-inclusive means the whole
        // input counts as input.
        assert!(
            (cost - (100.0 * 1.5 / 1e6 + 20.0 * 6.0 / 1e6)).abs() < 1e-9,
            "{cost}"
        );
    }

    /// Only the agents the batch names are recorded as scanned — the fact the
    /// daemon cannot derive, because an agent with no history produces no rows.
    #[test]
    fn a_scan_is_stamped_for_the_agents_the_batch_names() {
        let store = store_unpriced();
        let batch = HistoryBatch {
            usage: vec![],
            sessions: vec![],
            scanned_agents: vec!["claude".into()],
            source_machine: None,
        };
        apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();
        let scans = store.history_scans().unwrap();
        assert!(scans.contains_key("claude"), "{scans:?}");
        assert!(
            !scans.contains_key("codex"),
            "an agent nobody read is not recorded as read: that stamp is what a \
             later reader checks"
        );
    }

    /// The session half lands too, counts and all.
    #[test]
    fn sessions_are_written_with_their_counts() {
        let store = store_priced();
        let batch = HistoryBatch {
            usage: vec![],
            sessions: vec![session("claude-code:s")],
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        let report = apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &batch,
        )
        .unwrap();
        assert_eq!(report.sessions, 1);
        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].turns, 4);
        assert_eq!(listed[0].project.as_deref(), Some("kiwano"));
        assert!(listed[0].tool_calls.as_deref().unwrap().contains("Bash"));
        assert!(listed[0].skills.as_deref().unwrap().contains("tweet"));
    }

    /// The machine a batch names reaches **both** halves it writes — every row
    /// comes from the one scan, so the label is stamped on the usage rows and the
    /// session rows alike. A batch that named no machine leaves NULL, which is
    /// what an unnamed client produces.
    #[test]
    fn the_machine_a_batch_names_reaches_both_halves() {
        let store = store_priced();
        let named = HistoryBatch {
            usage: vec![row("codex", "2026-01-01T00:00:00+00:00", "codex:s:1")],
            sessions: vec![session("claude-code:s")],
            scanned_agents: Vec::new(),
            source_machine: Some("buildbox".into()),
        };
        apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &named,
        )
        .unwrap();

        let listed = store.list_sessions(None, None, None).unwrap();
        assert_eq!(
            listed[0].imported_from.as_deref(),
            Some("buildbox"),
            "the session row carries the scan's machine"
        );
        let conn = store.conn.lock().unwrap();
        let usage_machine: Option<String> = conn
            .query_row(
                "SELECT imported_from FROM usage WHERE import_key IS NOT NULL",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(
            usage_machine.as_deref(),
            Some("buildbox"),
            "and so does the usage row"
        );
        drop(conn);

        // A second scan from a machine that could not name itself: the columns
        // are NULL rather than empty text.
        let unnamed = HistoryBatch {
            usage: vec![row("codex", "2026-01-02T00:00:00+00:00", "codex:s:2")],
            sessions: vec![],
            scanned_agents: Vec::new(),
            source_machine: None,
        };
        apply_history(
            &store,
            &pricing_of(&store),
            &kiwano_adapters::model_pricing::PricingTable::default(),
            &unnamed,
        )
        .unwrap();
        let conn = store.conn.lock().unwrap();
        let unnamed_machine: Option<String> = conn
            .query_row(
                "SELECT imported_from FROM usage WHERE import_key = 'codex:s:2'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(unnamed_machine, None);
    }
}
