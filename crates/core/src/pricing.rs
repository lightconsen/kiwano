//! Model pricing sync + currency helpers.
//!
//! The Hub's models.json — cached verbatim by [`crate::sync`] — is the price
//! source, as it is for the catalog. This module keeps the GUI-readable
//! `model_pricing` SQLite table in sync with it (version-keyed upsert, changed
//! rows only, rows the document dropped pruned away) and provides the currency
//! metadata + conversion used across the usage/dashboard surfaces.

use crate::vm::Aux;
use kiwano_adapters::model_pricing::{ModelPriceEntry, ModelsDoc};
use std::collections::HashMap;

/// app_settings KV key holding the last-seeded models.json version.
const SEEDED_VERSION_KEY: &str = "pricing_seeded_version";

/// app_settings KV key holding the sha256 of the content last seeded. Absent
/// until the first Hub seed — an install that has never synced has nothing to
/// record, and one that seeded before this key existed re-seeds once.
const SEEDED_SHA_KEY: &str = "pricing_seeded_sha256";

/// Currency metadata for the Settings selector + UI conversion.
#[derive(serde::Serialize)]
pub struct CurrencyMetaVm {
    /// The codes the selector offers: the rate table's keys, plus the current
    /// preference if the table has no rate for it.
    pub currencies: Vec<String>,
    /// currency -> units of that currency per 1 USD (e.g. CNY: 7.1).
    pub exchange_rates: HashMap<String, f64>,
    /// The user's preferred display currency (Settings).
    pub preferred: String,
}

/// The currencies a user may display in. The rate table's keys are the ones
/// conversion can actually target; `models[].currency` says what things are
/// *priced* in, which is a different question — the published table prices
/// everything in USD, so deriving the list from it offered USD alone while the
/// default preference was CNY, and picking USD left nothing to switch back to.
///
/// The current preference is kept on the list even when no rate names it: a
/// selector that cannot show its own value is a dead end.
pub fn displayable_currencies(rates: &HashMap<String, f64>, preferred: &str) -> Vec<String> {
    let mut out: Vec<String> = rates.keys().cloned().collect();
    if !out.iter().any(|c| c == preferred) {
        out.push(preferred.to_string());
    }
    out.sort();
    out.dedup();
    out
}

#[derive(serde::Serialize)]
pub struct SeededReport {
    pub seeded: usize,
    pub version: i64,
    pub skipped: bool,
}

/// The price table to seed from, with the sha256 of the bytes it came from, or
/// `None` when the Hub has never been synced or its cache is unusable.
///
/// There is no compiled fallback: like the catalog, prices come from the Hub
/// alone. `None` is therefore "nothing to seed", never "an empty price
/// document" — the difference matters, because seeding an empty document
/// clears the table.
pub fn effective_doc(aux: &Aux) -> Option<(ModelsDoc, String)> {
    let (_, payload, sha, _) = aux.load_hub_models_cache()?;
    let doc = serde_json::from_str::<ModelsDoc>(&payload).ok()?;
    Some((doc, sha))
}

/// The exchange rates to convert with, or an empty table before the first sync
/// (every conversion then passes amounts through unchanged).
pub fn effective_rates(aux: &Aux) -> HashMap<String, f64> {
    effective_doc(aux)
        .map(|(doc, _)| doc.exchange_rates)
        .unwrap_or_default()
}

/// The content half of the seed gate: only the bytes the Hub publishes may skip
/// a seed. An install that seeded before digests were recorded has none, so it
/// re-seeds once — which is also what a Hub that republishes under a new digest
/// wants.
fn shas_match(fetched: &str, seeded: Option<&str>) -> bool {
    seeded == Some(fetched)
}

/// Delete the price rows whose `model_id` is not in `keep`; returns how many.
///
/// A temp table rather than `NOT IN (?, ?, …)`: the placeholder list is capped
/// by SQLite's variable limit (999 by default) while this is however many models
/// the Hub publishes. The connection outlives the call, so the table is dropped
/// rather than left behind.
///
/// An empty `keep` deletes every row, which is the intended reading of a
/// document that prices nothing. The caller logs it.
fn prune_absent(conn: &rusqlite::Connection, keep: &[ModelPriceEntry]) -> Result<usize, String> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS keep_price (model_id TEXT PRIMARY KEY);
         DELETE FROM keep_price;",
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = conn
            .prepare("INSERT OR IGNORE INTO keep_price (model_id) VALUES (?1)")
            .map_err(|e| e.to_string())?;
        for m in keep {
            stmt.execute(rusqlite::params![m.model_id])
                .map_err(|e| e.to_string())?;
        }
    }

    let removed = conn
        .execute(
            "DELETE FROM model_pricing
              WHERE model_id NOT IN (SELECT model_id FROM keep_price)",
            [],
        )
        .map_err(|e| e.to_string())?;
    conn.execute_batch("DROP TABLE IF EXISTS keep_price")
        .map_err(|e| e.to_string())?;
    Ok(removed)
}

/// Upsert the price rows into the shared `model_pricing` table.
///
/// Gated on version **and** content hash. Each half covers what the other
/// misses: the version alone would skip a price change that shipped without a
/// bump (nothing enforces the bump), and the hash alone would skip a
/// deliberate re-announce whose rows happen to be identical. A run is skipped
/// only when the version is not newer *and* the content is byte-identical.
///
/// Content is what decides in the end: a Hub rollback that actually changes
/// prices is applied even at a lower version, because the published bytes —
/// not the counter — are the source of truth.
///
/// Rows are written only when a column actually changes (WHERE guard), so a
/// forced re-seed is write-free in practice and `seeded` stays honest.
pub fn seed_model_pricing(aux: &Aux) -> Result<SeededReport, String> {
    // Nothing published, nothing to seed — and crucially, nothing to prune.
    let Some((doc, sha)) = effective_doc(aux) else {
        return Ok(SeededReport {
            seeded: 0,
            version: 0,
            skipped: true,
        });
    };
    let seeded_version = aux
        .get_setting(SEEDED_VERSION_KEY)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let seeded_sha = aux.get_setting(SEEDED_SHA_KEY);
    if doc.version <= seeded_version && shas_match(&sha, seeded_sha.as_deref()) {
        return Ok(SeededReport {
            seeded: 0,
            version: doc.version,
            skipped: true,
        });
    }

    // Provenance of these rows. Every row the seeder writes is Hub-sourced now
    // that the compiled snapshot is gone; the column stays because legacy
    // installs still carry `bundled` rows, and the guard below is what
    // relabels them.
    let source = "hub";

    let mut guard = aux.conn.lock().expect("aux mutex poisoned");
    let mut seeded = 0usize;
    let removed;
    // One transaction for the upserts and the prune: they are one statement
    // about the table's contents, and a crash between them would leave it
    // half-seeded and half-pruned. Committed before the settings below, which
    // take the same mutex.
    {
        let tx = guard.transaction().map_err(|e| e.to_string())?;
        for m in &doc.models {
            let n = tx
                .execute(
                    "INSERT INTO model_pricing (model_id, display_name, input, output,
                                                cache_read, cache_creation, currency, source)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                     ON CONFLICT(model_id) DO UPDATE SET
                        display_name = ?2, input = ?3, output = ?4,
                        cache_read = ?5, cache_creation = ?6, currency = ?7, source = ?8
                     -- COALESCE: legacy rows may carry a NULL source, and
                     -- `NULL <> ?` is NULL (falsy), which would silently skip them.
                     WHERE display_name <> ?2 OR input <> ?3 OR output <> ?4
                        OR cache_read <> ?5 OR cache_creation <> ?6 OR currency <> ?7
                        OR COALESCE(source, '') <> ?8",
                    rusqlite::params![
                        m.model_id,
                        m.display_name,
                        m.input,
                        m.output,
                        m.cache_read,
                        m.cache_creation,
                        m.currency,
                        source,
                    ],
                )
                .map_err(|e| e.to_string())?;
            seeded += n;
        }

        // Rows this document no longer carries are removed, so the table ends up
        // *equal* to the document rather than the union of every document ever
        // seeded. Without this an upsert-only seed is a ratchet, and a model the
        // Hub drops stays priced forever: the data repo cut its table from 192
        // rows to 39, and every client that had synced the larger one kept
        // costing the 153 vendor prices that were deliberately deleted.
        //
        // Deliberately not scoped by `source`. An install seeded from the old
        // bundled snapshot holds `source = 'bundled'` rows, and sparing those is
        // exactly how the table stops matching the document. The target is
        // "local table == this document", whoever wrote the row.
        removed = prune_absent(&tx, &doc.models)?;
        tx.commit().map_err(|e| e.to_string())?;
    }
    drop(guard);

    // Say what happened when it is not nothing. Clearing the whole table is the
    // case worth shouting about: it is the correct reading of a document that
    // prices nothing, and it is also indistinguishable from a Hub accident if
    // nobody writes it down.
    if removed > 0 {
        if doc.models.is_empty() {
            tracing::warn!(
                removed,
                "the price document is empty; every local price row was removed"
            );
        } else {
            tracing::info!(
                removed,
                kept = doc.models.len(),
                "price rows no longer in the document were removed"
            );
        }
    }

    aux.set_setting(SEEDED_VERSION_KEY, &doc.version.to_string())
        .map_err(|e| e.to_string())?;
    // Record the content half so an unchanged Hub document is not re-seeded.
    aux.set_setting(SEEDED_SHA_KEY, &sha)
        .map_err(|e| e.to_string())?;
    Ok(SeededReport {
        seeded,
        version: doc.version,
        skipped: false,
    })
}

/// The user's preferred display currency (default CNY). Stored inside the
/// ui settings blob (same source the Settings page round-trips).
pub fn preferred_currency(aux: &Aux) -> String {
    crate::vm::ui_settings(aux).preferred_currency
}

/// Currencies to offer, the rates to convert with, and the preferred one.
///
/// No Tauri involvement: the app's `get_currency_meta` command is a wrapper that
/// hands over `state.aux`, and a command-line caller passes its own.
///
/// The rates are the Hub's, so the list describes what was actually seeded
/// rather than what some snapshot once offered.
pub fn currency_meta(aux: &Aux) -> Result<CurrencyMetaVm, String> {
    let rates = effective_rates(aux);
    let preferred = preferred_currency(aux);
    let currencies = displayable_currencies(&rates, &preferred);
    Ok(CurrencyMetaVm {
        currencies,
        exchange_rates: rates,
        preferred,
    })
}

/// Convert an amount between currencies using the models.json rates
/// (`rates[currency]` = units per 1 USD; USD pivots). Unknown currencies
/// return the amount unchanged (no rate — display the raw number).
pub fn convert_amount(amount: f64, from: &str, to: &str, rates: &HashMap<String, f64>) -> f64 {
    if from == to {
        return amount;
    }
    let (Some(per_usd_from), Some(per_usd_to)) = (rates.get(from), rates.get(to)) else {
        return amount;
    };
    if *per_usd_from == 0.0 {
        return amount;
    }
    let usd = amount / per_usd_from;
    usd * per_usd_to
}

/// Sum per-currency cost buckets into one amount denominated in `to`.
pub fn convert_cost_buckets(
    buckets: &[(Option<String>, f64)],
    to: &str,
    rates: &HashMap<String, f64>,
) -> f64 {
    buckets
        .iter()
        .map(|(currency, cost)| match currency {
            Some(c) => convert_amount(*cost, c, to, rates),
            None => 0.0,
        })
        .sum()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rates() -> HashMap<String, f64> {
        HashMap::from([("USD".to_string(), 1.0), ("CNY".to_string(), 7.1)])
    }

    /// A price document carrying `ids`, as the Hub would publish it.
    fn doc_json_ids(version: i64, ids: &[&str]) -> String {
        let models: Vec<serde_json::Value> = ids
            .iter()
            .map(|id| {
                serde_json::json!({
                    "model_id": id, "display_name": id,
                    "input": "1", "output": "2",
                    "cache_read": "0", "cache_creation": "0",
                    "currency": "USD",
                })
            })
            .collect();
        serde_json::json!({
            "version": version,
            "generated_at": "2026-09-13T00:00:00Z",
            "exchange_rates": { "USD": 1.0 },
            "models": models,
        })
        .to_string()
    }

    /// Every `model_id` the local price table currently holds.
    fn priced_ids(aux: &Aux) -> Vec<String> {
        let conn = aux.conn.lock().unwrap();
        let mut stmt = conn
            .prepare("SELECT model_id FROM model_pricing ORDER BY model_id")
            .unwrap();
        let ids = stmt
            .query_map([], |r| r.get::<_, String>(0))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        ids
    }

    /// A row the document drops has to stop being priced.
    ///
    /// The seed was upsert-only, which made the table a ratchet: the data repo
    /// cut its table from 192 rows to 39, and every client that had synced the
    /// larger one went on costing all 153 vendor prices it had deleted — while
    /// the shelf, reading a catalog that *had* been migrated, showed nothing of
    /// the kind.
    #[test]
    fn a_row_the_document_drops_stops_being_priced() {
        let (aux, _dir) = test_env();

        aux.save_hub_models_cache(1, &doc_json_ids(1, &["a", "b", "c"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert_eq!(priced_ids(&aux), ["a", "b", "c"]);

        aux.save_hub_models_cache(2, &doc_json_ids(2, &["a", "b"]), &"b".repeat(64), "t")
            .unwrap();
        let report = seed_model_pricing(&aux).unwrap();
        assert!(!report.skipped, "a newer version re-seeds");
        assert_eq!(
            priced_ids(&aux),
            ["a", "b"],
            "the dropped row is gone, not merely untouched"
        );
    }

    /// An empty document means "nothing is priced", and it clears the table.
    /// That is the right reading and also indistinguishable from a Hub accident,
    /// which is why `seed_model_pricing` logs the removal.
    #[test]
    fn an_empty_document_clears_the_table() {
        let (aux, _dir) = test_env();

        aux.save_hub_models_cache(1, &doc_json_ids(1, &["a", "b"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert_eq!(priced_ids(&aux).len(), 2);

        aux.save_hub_models_cache(2, &doc_json_ids(2, &[]), &"b".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert!(priced_ids(&aux).is_empty());
    }

    #[test]
    fn displayable_currencies_come_from_the_rate_table() {
        let list = displayable_currencies(&rates(), "CNY");
        assert_eq!(list, vec!["CNY".to_string(), "USD".to_string()]);
    }

    #[test]
    fn a_preference_with_no_rate_still_stays_selectable() {
        // The reported bug in miniature: every published model is priced in
        // USD, so a list built from those offered USD alone while the default
        // preference was CNY — pick USD and there was nothing to switch back
        // to. Whatever is selected has to stay on its own list.
        let usd_only = HashMap::from([("USD".to_string(), 1.0)]);
        let list = displayable_currencies(&usd_only, "CNY");
        assert!(
            list.contains(&"CNY".to_string()),
            "the selection survives: {list:?}"
        );
        assert!(list.contains(&"USD".to_string()));
    }

    #[test]
    fn convert_via_usd_pivot() {
        let r = rates();
        assert!((convert_amount(7.1, "CNY", "USD", &r) - 1.0).abs() < 1e-9);
        assert!((convert_amount(1.0, "USD", "CNY", &r) - 7.1).abs() < 1e-9);
        assert_eq!(convert_amount(3.0, "CNY", "CNY", &r), 3.0);
        // Unknown currency passes through unchanged.
        assert_eq!(convert_amount(5.0, "EUR", "USD", &r), 5.0);
    }

    // ── seed gate ────────────────────────────────────────────────────────

    /// Production opens `Aux` and the gateway `Store` on the *same* file, and
    /// `model_pricing` is created by the Store's migrations — `Aux` alone has
    /// no such table. Mirror that here so the seed tests exercise the real
    /// arrangement. The `TempDir` must stay alive for the file to exist.
    fn test_env() -> (Aux, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        kiwanod::store::Store::open(&path).unwrap();
        (Aux::open(&path).unwrap(), dir)
    }

    fn sha_a() -> String {
        "a".repeat(64)
    }

    fn sha_b() -> String {
        "b".repeat(64)
    }

    /// A one-row price doc; `price` is the knob the content-change tests turn.
    fn doc_json(version: i64, price: &str) -> String {
        format!(
            r#"{{"version":{version},"generated_at":"2026-01-01",
                "exchange_rates":{{"USD":1.0}},
                "models":[{{"model_id":"m1","display_name":"M1","input":"{price}",
                  "output":"2","cache_read":"0","cache_creation":"0","currency":"USD"}}]}}"#
        )
    }

    /// Pretend the Hub delivered this doc.
    fn cache_hub(aux: &Aux, version: i64, price: &str, sha: &str) {
        aux.save_hub_models_cache(
            version,
            &doc_json(version, price),
            sha,
            "2026-01-01T00:00:00Z",
        )
        .unwrap();
    }

    #[test]
    fn shas_match_matrix() {
        assert!(shas_match("a", Some("a")));
        assert!(!shas_match("a", Some("b")));
        assert!(!shas_match("a", None), "published bytes never seeded");
    }

    /// Nothing published is not the same as "nothing is priced": the seed has
    /// nothing to say and leaves the table exactly as it found it. The
    /// distinction is load-bearing — the seed prunes rows the document does not
    /// carry, so treating a missing document as an empty one would wipe the
    /// local table on any install whose cache went missing.
    #[test]
    fn seed_without_a_published_document_does_nothing() {
        let (aux, _dir) = test_env();
        let first = seed_model_pricing(&aux).unwrap();
        assert!(first.skipped);
        assert_eq!(first.seeded, 0);
        assert_eq!(aux.get_setting(SEEDED_SHA_KEY), None);

        // …and once there are rows, they survive the document going away.
        cache_hub(&aux, 1, "1", &sha_a());
        seed_model_pricing(&aux).unwrap();
        assert_eq!(priced_ids(&aux).len(), 1);

        aux.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM hub_models_cache", [])
            .unwrap();
        let after = seed_model_pricing(&aux).unwrap();
        assert!(after.skipped);
        assert_eq!(priced_ids(&aux).len(), 1, "the rows are left alone");
    }

    /// The headline case: content changed, version did not. Nothing enforces a
    /// version bump, so a version-only gate would silently keep the old prices.
    #[test]
    fn seed_gate_content_change_same_version() {
        let (aux, _dir) = test_env();
        cache_hub(&aux, 1, "1", &sha_a());
        let first = seed_model_pricing(&aux).unwrap();
        assert!(!first.skipped);
        assert_eq!(first.seeded, 1);

        cache_hub(&aux, 1, "9", &sha_b());
        let second = seed_model_pricing(&aux).unwrap();
        assert!(!second.skipped, "a content change must re-seed");
        assert_eq!(second.seeded, 1, "the changed row is rewritten");

        // Same content again → skipped.
        assert!(seed_model_pricing(&aux).unwrap().skipped);
    }

    /// A bump re-seeds even when the rows are byte-identical: the version half
    /// must never be blocked by the hash half.
    #[test]
    fn seed_gate_version_bump_always_reseeds() {
        let (aux, _dir) = test_env();
        cache_hub(&aux, 1, "1", &sha_a());
        assert!(!seed_model_pricing(&aux).unwrap().skipped);

        cache_hub(&aux, 2, "1", &sha_b());
        let bumped = seed_model_pricing(&aux).unwrap();
        assert!(!bumped.skipped);
        // …and stays write-free, because the WHERE guard sees no column change.
        assert_eq!(bumped.seeded, 0);
    }

    /// Identical content is skipped whatever the version says, so a re-publish
    /// of the same table costs nothing.
    #[test]
    fn seed_gate_skips_identical_content_at_any_version() {
        let (aux, _dir) = test_env();
        cache_hub(&aux, 5, "1", &sha_a());
        assert!(!seed_model_pricing(&aux).unwrap().skipped);

        cache_hub(&aux, 4, "1", &sha_a());
        assert!(seed_model_pricing(&aux).unwrap().skipped);
    }

    /// …but a rollback that changes prices still lands: the published bytes are
    /// the source of truth, and refusing it would strand clients on the table
    /// the Hub just withdrew.
    #[test]
    fn seed_gate_applies_changed_content_at_a_lower_version() {
        let (aux, _dir) = test_env();
        cache_hub(&aux, 5, "1", &sha_a());
        assert!(!seed_model_pricing(&aux).unwrap().skipped);

        cache_hub(&aux, 4, "9", &sha_b());
        let rolled_back = seed_model_pricing(&aux).unwrap();
        assert!(!rolled_back.skipped);
        assert_eq!(rolled_back.seeded, 1);
    }

    #[test]
    fn effective_doc_is_none_until_the_hub_is_synced() {
        let aux = Aux::open_in_memory().unwrap();
        assert!(effective_doc(&aux).is_none());
        assert!(effective_rates(&aux).is_empty(), "no rates to convert with");

        cache_hub(&aux, 99, "1", &sha_a());
        let (doc, sha) = effective_doc(&aux).unwrap();
        assert_eq!(doc.version, 99);
        assert_eq!(sha, sha_a());
    }

    #[test]
    fn effective_doc_ignores_an_unusable_cache() {
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_models_cache(9, "not json", &sha_a(), "2026-01-01T00:00:00Z")
            .unwrap();
        assert!(
            effective_doc(&aux).is_none(),
            "an unparseable cache is no document at all"
        );
    }

    /// Provenance: rows seeded from the Hub are labelled as such.
    #[test]
    fn seeded_rows_record_their_source() {
        let (aux, _dir) = test_env();
        cache_hub(&aux, 1, "1", &sha_a());
        seed_model_pricing(&aux).unwrap();
        let conn = aux.conn.lock().unwrap();
        let source: String = conn
            .query_row(
                "SELECT source FROM model_pricing WHERE model_id = 'm1'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(source, "hub");
    }
}
