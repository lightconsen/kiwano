//! Model pricing sync + currency helpers.
//!
//! The Hub's models.json — cached verbatim by [`crate::sync`] — is the price
//! source, as it is for the catalog. This module keeps the GUI-readable
//! `model_pricing` SQLite table in sync with it (version-keyed upsert, changed
//! rows only, rows the document dropped pruned away) and provides the currency
//! metadata used across the usage/dashboard surfaces.

use crate::vm::Aux;
/// Re-exported: the app's `list_model_prices` command hands these rows to the
/// frontend, and the app deliberately depends on `kiwano-core` alone rather than
/// reaching into the adapters crate for a type.
pub use kiwano_adapters::model_pricing::ModelPriceEntry;
use kiwano_adapters::model_pricing::ModelsDoc;
/// Re-exported for the same reason, and for a better one: the gateway's limit
/// check converts with these too, and one implementation is the only way the
/// figure it enforces and the figure the app shows can be the same number.
pub use kiwano_adapters::model_pricing::{convert_amount, convert_cost_buckets};
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
    let (version, payload, sha, _) = aux.load_hub_models_cache()?;
    match serde_json::from_str::<ModelsDoc>(&payload) {
        Ok(doc) => Some((doc, sha)),
        Err(e) => {
            // Not the same as "the Hub published nothing". A cached document
            // that will not parse is a price table that has quietly stopped
            // updating: the seed reports itself skipped, the old rows stay, and
            // the gateway goes on billing them. The sync validates a document
            // before caching it, so reaching this means the stored row changed
            // underneath the app — a truncated write, or a schema this build no
            // longer reads. Say so, because the symptom is otherwise invisible.
            tracing::warn!(
                version,
                error = %e,
                "cached pricing document does not parse; the previously seeded prices stand"
            );
            None
        }
    }
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

/// The canonical `(provider_id, model_id)` key of a document row.
///
/// Both halves trimmed and lowercased, because that is what the in-memory index
/// looks up: a row stored as the Hub spelled it would never be found. The seed's
/// insert and its prune go through here together — normalizing in only one of
/// them would delete the row the other just wrote, every sync.
fn price_key(m: &ModelPriceEntry) -> (String, String) {
    (
        m.provider_id.trim().to_ascii_lowercase(),
        m.model_id.trim().to_ascii_lowercase(),
    )
}

/// Delete the price rows whose key is not in `keep`; returns how many.
///
/// A temp table rather than `NOT IN (?, ?, …)`: the placeholder list is capped
/// by SQLite's variable limit (999 by default) while this is however many rows
/// the Hub publishes. The connection outlives the call, so the table is dropped
/// rather than left behind.
///
/// The key is the full `(provider_id, model_id)` pair. A pre-v11 row is the
/// general one (`provider_id = ''`), and it has to go when the document prices
/// that model per provider instead: a stale general row is not inert, it is what
/// every unmatched provider falls back to.
///
/// An empty `keep` deletes every row, which is the intended reading of a
/// document that prices nothing. The caller logs it.
fn prune_absent(conn: &rusqlite::Connection, keep: &[ModelPriceEntry]) -> Result<usize, String> {
    conn.execute_batch(
        "CREATE TEMP TABLE IF NOT EXISTS keep_price (
             provider_id TEXT NOT NULL DEFAULT '',
             model_id    TEXT NOT NULL,
             PRIMARY KEY (provider_id, model_id)
         );
         DELETE FROM keep_price;",
    )
    .map_err(|e| e.to_string())?;

    {
        let mut stmt = conn
            .prepare("INSERT OR IGNORE INTO keep_price (provider_id, model_id) VALUES (?1, ?2)")
            .map_err(|e| e.to_string())?;
        for m in keep {
            let (provider_id, model_id) = price_key(m);
            stmt.execute(rusqlite::params![provider_id, model_id])
                .map_err(|e| e.to_string())?;
        }
    }

    let removed = conn
        .execute(
            "DELETE FROM model_pricing
              WHERE NOT EXISTS (SELECT 1 FROM keep_price k
                                 WHERE k.provider_id = model_pricing.provider_id
                                   AND k.model_id = model_pricing.model_id)",
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
            let (provider_id, model_id) = price_key(m);
            let n = tx
                .execute(
                    "INSERT INTO model_pricing (provider_id, model_id, display_name, input, output,
                                                cache_read, cache_creation, currency, source, tiers)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                     ON CONFLICT(provider_id, model_id) DO UPDATE SET
                        display_name = ?3, input = ?4, output = ?5,
                        cache_read = ?6, cache_creation = ?7, currency = ?8, source = ?9,
                        tiers = ?10
                     -- COALESCE on both nullable columns: a row may carry NULL
                     -- in `source` (legacy) or in `tiers` (a model with no
                     -- schedule), and `NULL <> ?` is NULL (falsy) — which would
                     -- silently skip the very update that adds one.
                     WHERE display_name <> ?3 OR input <> ?4 OR output <> ?5
                        OR cache_read <> ?6 OR cache_creation <> ?7 OR currency <> ?8
                        OR COALESCE(source, '') <> ?9
                        OR COALESCE(tiers, '') <> COALESCE(?10, '')",
                    rusqlite::params![
                        provider_id,
                        model_id,
                        m.display_name,
                        m.input,
                        m.output,
                        m.cache_read,
                        m.cache_creation,
                        m.currency,
                        source,
                        m.tiers_json(),
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
/// The rows the gateway charges with, for the Models page's per-model detail.
///
/// The catalog carries one representative price per provider; what each of a
/// provider's models costs lives here — and so does each row's own tier
/// schedule, which for any model other than the flagship is the only place it
/// is stated. Read-only, and small enough to hand over whole.
pub fn list_model_prices(store: &kiwanod::store::Store) -> Result<Vec<ModelPriceEntry>, String> {
    store.load_model_pricing().map_err(|e| e.to_string())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    fn rates() -> HashMap<String, f64> {
        HashMap::from([("USD".to_string(), 1.0), ("CNY".to_string(), 7.1)])
    }

    /// A price document carrying `rows` as `(provider_id, model_id)`, as the
    /// Hub would publish it.
    fn doc_json_rows(version: i64, rows: &[(&str, &str)]) -> String {
        let models: Vec<serde_json::Value> = rows
            .iter()
            .map(|(provider_id, id)| {
                serde_json::json!({
                    "provider_id": provider_id, "model_id": id, "display_name": id,
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

    /// `doc_json_rows` for a document of provider-less ids — the pre-v11 shape.
    fn doc_json_ids(version: i64, ids: &[&str]) -> String {
        let rows: Vec<(&str, &str)> = ids.iter().map(|id| ("", *id)).collect();
        doc_json_rows(version, &rows)
    }

    /// How `ids` would be stored: no provider, so the general price.
    fn general_keys(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (String::new(), id.to_string()))
            .collect()
    }

    /// Every `(provider_id, model_id)` the local price table currently holds.
    /// The tiers blob a row carries, as stored: `None` means the row has no
    /// schedule (the column is NULL, never `"{}"`).
    fn tiers_of(aux: &Aux, model_id: &str) -> Option<String> {
        aux.conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT tiers FROM model_pricing WHERE model_id = ?1",
                rusqlite::params![model_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
    }

    fn priced_keys(aux: &Aux) -> Vec<(String, String)> {
        let conn = aux.conn.lock().unwrap();
        let mut stmt = conn
            .prepare(
                "SELECT provider_id, model_id FROM model_pricing ORDER BY provider_id, model_id",
            )
            .unwrap();
        let keys = stmt
            .query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))
            .unwrap()
            .filter_map(Result::ok)
            .collect();
        keys
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
        assert_eq!(priced_keys(&aux), general_keys(&["a", "b", "c"]));

        aux.save_hub_models_cache(2, &doc_json_ids(2, &["a", "b"]), &"b".repeat(64), "t")
            .unwrap();
        let report = seed_model_pricing(&aux).unwrap();
        assert!(!report.skipped, "a newer version re-seeds");
        assert_eq!(
            priced_keys(&aux),
            general_keys(&["a", "b"]),
            "the dropped row is gone, not merely untouched"
        );
    }

    /// A model the document starts pricing per provider replaces the general
    /// row rather than sitting beside it. The general row is not inert — it is
    /// what every provider without a price of its own falls back to — so leaving
    /// it would keep handing out a price the Hub had withdrawn.
    #[test]
    fn a_row_that_gains_a_provider_replaces_the_general_one() {
        let (aux, _dir) = test_env();

        aux.save_hub_models_cache(1, &doc_json_ids(1, &["m1"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert_eq!(priced_keys(&aux), general_keys(&["m1"]));

        aux.save_hub_models_cache(
            2,
            &doc_json_rows(2, &[("kimi", "m1"), ("moonshot", "m1")]),
            &"b".repeat(64),
            "t",
        )
        .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert_eq!(
            priced_keys(&aux),
            vec![
                ("kimi".to_string(), "m1".to_string()),
                ("moonshot".to_string(), "m1".to_string()),
            ],
            "both providers are priced, and the general row is gone"
        );
    }

    /// The stored key is the normalized one, so a row spelled differently is the
    /// same row — which is what stops the insert and the prune from disagreeing
    /// about whether it belongs: an unnormalized prune would delete the row the
    /// insert had just normalized, on every sync, leaving the table empty.
    #[test]
    fn the_stored_key_is_normalized() {
        let (aux, _dir) = test_env();
        let kimi_m1 = vec![("kimi".to_string(), "m1".to_string())];

        aux.save_hub_models_cache(
            1,
            &doc_json_rows(1, &[("  Kimi  ", "m1")]),
            &"a".repeat(64),
            "t",
        )
        .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert_eq!(priced_keys(&aux), kimi_m1);

        // The same row, spelled differently, under a new digest so the gate
        // re-seeds: nothing to write, and — the trap — nothing to prune. An
        // insert that normalizes while the prune does not would delete the row
        // it had just decided was unchanged.
        aux.save_hub_models_cache(
            2,
            &doc_json_rows(2, &[("KIMI", "m1")]),
            &"b".repeat(64),
            "t",
        )
        .unwrap();
        let report = seed_model_pricing(&aux).unwrap();
        assert!(!report.skipped, "a new digest re-seeds");
        assert_eq!(
            report.seeded, 0,
            "the row is already what the document says"
        );
        assert_eq!(priced_keys(&aux), kimi_m1, "and it survived the prune");
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
        assert_eq!(priced_keys(&aux).len(), 2);

        aux.save_hub_models_cache(2, &doc_json_ids(2, &[]), &"b".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&aux).unwrap();
        assert!(priced_keys(&aux).is_empty());
    }

    /// Nothing to convert sums to a positive zero: the number reaches screens
    /// and JSON, and `-0.0` reads as a bug in both.
    #[test]
    fn empty_cost_buckets_sum_to_positive_zero() {
        let total = convert_cost_buckets(&[], "CNY", &rates());
        assert_eq!(total, 0.0);
        assert!(total.is_sign_positive(), "not -0.0: {total:?}");
        // …and a bucket whose amount is zero keeps the sign too, whichever
        // currency it is in.
        let zero = convert_cost_buckets(&[(Some("USD".into()), 0.0)], "CNY", &rates());
        assert!(zero.is_sign_positive(), "{zero:?}");
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
        assert_eq!(priced_keys(&aux).len(), 1);

        aux.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM hub_models_cache", [])
            .unwrap();
        let after = seed_model_pricing(&aux).unwrap();
        assert!(after.skipped);
        assert_eq!(priced_keys(&aux).len(), 1, "the rows are left alone");
    }

    /// A row that gains a schedule, or whose only change is inside it, has to be
    /// written: `tiers` is nullable, and the guard would compare `NULL <> '…'`
    /// — NULL, hence falsy — if it were not COALESCEd on both sides. Without it
    /// the very update that adds time-of-day pricing would be skipped, and the
    /// install would keep billing peak forever.
    #[test]
    fn a_tiers_only_change_reseeds_the_row() {
        let (aux, _dir) = test_env();
        let doc = |off_peak_in: &str| {
            format!(
                r#"{{"version":1,"generated_at":"t","exchange_rates":{{"USD":1.0}},"models":[
                   {{"model_id":"m1","display_name":"M1","input":"9","output":"27",
                     "cache_read":"0.3","cache_creation":"0","currency":"USD",
                     "off_peak":{{"in":"{off_peak_in}","out":"13.5","cache_read":"0.15"}},
                     "peak_hours":{{"tz_offset":480,"windows":[
                       {{"days":["mon"],"start":"09:00","end":"12:00"}}]}}}}]}}"#
            )
        };

        // A row with no tiers stores NULL, not "{}"…
        cache_hub(&aux, 1, "1", &sha_a());
        assert_eq!(seed_model_pricing(&aux).unwrap().seeded, 1);
        assert_eq!(tiers_of(&aux, "m1"), None);
        // …so a forced re-seed of the same bytes stays write-free.
        assert!(seed_model_pricing(&aux).unwrap().skipped);

        // Only the discount moves.
        aux.save_hub_models_cache(2, &doc("4.5"), &"c".repeat(64), "t")
            .unwrap();
        assert_eq!(seed_model_pricing(&aux).unwrap().seeded, 1);

        aux.save_hub_models_cache(3, &doc("3.5"), &"d".repeat(64), "t")
            .unwrap();
        assert_eq!(
            seed_model_pricing(&aux).unwrap().seeded,
            1,
            "the tiers column changed"
        );
        let stored = tiers_of(&aux, "m1").expect("the schedule reached the mirror");
        assert!(stored.contains(r#""in":"3.5""#), "{stored}");
        assert!(stored.contains("peak_hours"), "{stored}");
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
