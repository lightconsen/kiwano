//! The price table the Hub publishes, resolved and seeded into the mirror.
//!
//! Moved here from `kiwano_core::pricing` when the daemon took over the sync's
//! apply step (`migrate.local.md` §10.14): the cache it reads and the
//! `model_pricing` mirror it writes are both the daemon's tables, so a sync's
//! "what changed" is decided by the side that bills from it. The `String`
//! errors are what the CLI reports — the daemon's `sync` maps them at its
//! boundary.

use crate::store::Store;
use kiwano_adapters::model_pricing::{ModelPriceEntry, ModelsDoc};
use std::collections::HashMap;

const SEEDED_VERSION_KEY: &str = "pricing_seeded_version";
/// app_settings KV holding the sha256 of the models.json the seeder last wrote
/// from, so a version-bump re-seed and a content-change re-seed are
/// distinguishable without comparing the whole table.
const SEEDED_SHA_KEY: &str = "pricing_seeded_sha256";

pub struct SeededReport {
    pub seeded: usize,
    pub version: i64,
    pub skipped: bool,
}

pub fn effective_doc(store: &Store) -> Option<(ModelsDoc, String)> {
    let (version, payload, sha, _) = store.hub_models_cache()?;
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

pub fn effective_rates(store: &Store) -> HashMap<String, f64> {
    effective_doc(store)
        .map(|(doc, _)| doc.exchange_rates)
        .unwrap_or_default()
}

fn shas_match(fetched: &str, seeded: Option<&str>) -> bool {
    seeded == Some(fetched)
}

#[cfg(test)]
mod gate_tests {
    use super::*;

    /// Only the bytes the Hub publishes may skip a seed: a digest we cannot
    /// match is a digest we cannot trust, and re-seeding costs one pass.
    #[test]
    fn shas_match_requires_both_sides() {
        assert!(shas_match("a", Some("a")));
        assert!(!shas_match("a", Some("b")));
        assert!(!shas_match("a", None), "published bytes never seeded");
    }
}

fn price_key(m: &ModelPriceEntry) -> (String, String) {
    (
        m.provider_id.trim().to_ascii_lowercase(),
        m.model_id.trim().to_ascii_lowercase(),
    )
}

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

pub fn seed_model_pricing(store: &Store) -> Result<SeededReport, String> {
    // Nothing published, nothing to seed — and crucially, nothing to prune.
    let Some((doc, sha)) = effective_doc(store) else {
        return Ok(SeededReport {
            seeded: 0,
            version: 0,
            skipped: true,
        });
    };
    let seeded_version = store
        .app_setting(SEEDED_VERSION_KEY)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let seeded_sha = store.app_setting(SEEDED_SHA_KEY);
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

    let mut conn = store.conn.lock().expect("store mutex poisoned");
    let mut seeded = 0usize;
    let removed;
    // One transaction for the upserts and the prune: they are one statement
    // about the table's contents, and a crash between them would leave it
    // half-seeded and half-pruned. Committed before the settings below, which
    // take the same mutex.
    {
        let tx = conn.transaction().map_err(|e| e.to_string())?;
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
    drop(conn);

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

    store
        .set_app_setting(SEEDED_VERSION_KEY, &doc.version.to_string())
        .map_err(|e| e.to_string())?;
    // Record the content half so an unchanged Hub document is not re-seeded.
    store
        .set_app_setting(SEEDED_SHA_KEY, &sha)
        .map_err(|e| e.to_string())?;
    Ok(SeededReport {
        seeded,
        version: doc.version,
        skipped: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A Hub document with one priced model.
    fn doc_json(version: i64, price: &str) -> String {
        format!(
            r#"{{"version":{version},"generated_at":"2026-01-01",
                "exchange_rates":{{"USD":1.0}},
                "models":[{{"model_id":"m1","display_name":"M1","input":"{price}",
                  "output":"2","cache_read":"0","cache_creation":"0","currency":"USD"}}]}}"#
        )
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

    fn doc_json_ids(version: i64, ids: &[&str]) -> String {
        let rows: Vec<(&str, &str)> = ids.iter().map(|id| ("", *id)).collect();
        doc_json_rows(version, &rows)
    }

    fn sha_a() -> String {
        "a".repeat(64)
    }

    /// How `ids` would be stored: no provider, so the general price.
    fn general_keys(ids: &[&str]) -> Vec<(String, String)> {
        ids.iter()
            .map(|id| (String::new(), id.to_string()))
            .collect()
    }

    fn cache_hub(store: &Store, version: i64, price: &str, sha: &str) {
        store
            .save_hub_models_cache(
                version,
                &doc_json(version, price),
                sha,
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
    }

    fn tiers_of(store: &Store, model_id: &str) -> Option<String> {
        store
            .conn
            .lock()
            .unwrap()
            .query_row(
                "SELECT tiers FROM model_pricing WHERE model_id = ?1",
                rusqlite::params![model_id],
                |r| r.get::<_, Option<String>>(0),
            )
            .unwrap()
    }

    fn priced_keys(store: &Store) -> Vec<(String, String)> {
        let conn = store.conn.lock().unwrap();
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
    /// schedule (the column is NULL, never `"{}"`).

    #[test]
    fn a_row_the_document_drops_stops_being_priced() {
        let (store, _dir) = test_env();

        store
            .save_hub_models_cache(1, &doc_json_ids(1, &["a", "b", "c"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert_eq!(priced_keys(&store), general_keys(&["a", "b", "c"]));

        store
            .save_hub_models_cache(2, &doc_json_ids(2, &["a", "b"]), &"b".repeat(64), "t")
            .unwrap();
        let report = seed_model_pricing(&store).unwrap();
        assert!(!report.skipped, "a newer version re-seeds");
        assert_eq!(
            priced_keys(&store),
            general_keys(&["a", "b"]),
            "the dropped row is gone, not merely untouched"
        );
    }
    #[test]
    fn a_row_that_gains_a_provider_replaces_the_general_one() {
        let (store, _dir) = test_env();

        store
            .save_hub_models_cache(1, &doc_json_ids(1, &["m1"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert_eq!(priced_keys(&store), general_keys(&["m1"]));

        store
            .save_hub_models_cache(
                2,
                &doc_json_rows(2, &[("kimi", "m1"), ("moonshot", "m1")]),
                &"b".repeat(64),
                "t",
            )
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert_eq!(
            priced_keys(&store),
            vec![
                ("kimi".to_string(), "m1".to_string()),
                ("moonshot".to_string(), "m1".to_string()),
            ],
            "both providers are priced, and the general row is gone"
        );
    }
    #[test]
    fn the_stored_key_is_normalized() {
        let (store, _dir) = test_env();
        let kimi_m1 = vec![("kimi".to_string(), "m1".to_string())];

        store
            .save_hub_models_cache(
                1,
                &doc_json_rows(1, &[("  Kimi  ", "m1")]),
                &"a".repeat(64),
                "t",
            )
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert_eq!(priced_keys(&store), kimi_m1);

        // The same row, spelled differently, under a new digest so the gate
        // re-seeds: nothing to write, and — the trap — nothing to prune. An
        // insert that normalizes while the prune does not would delete the row
        // it had just decided was unchanged.
        store
            .save_hub_models_cache(
                2,
                &doc_json_rows(2, &[("KIMI", "m1")]),
                &"b".repeat(64),
                "t",
            )
            .unwrap();
        let report = seed_model_pricing(&store).unwrap();
        assert!(!report.skipped, "a new digest re-seeds");
        assert_eq!(
            report.seeded, 0,
            "the row is already what the document says"
        );
        assert_eq!(priced_keys(&store), kimi_m1, "and it survived the prune");
    }
    /// which is why `seed_model_pricing` logs the removal.
    #[test]
    fn an_empty_document_clears_the_table() {
        let (store, _dir) = test_env();

        store
            .save_hub_models_cache(1, &doc_json_ids(1, &["a", "b"]), &"a".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert_eq!(priced_keys(&store).len(), 2);

        store
            .save_hub_models_cache(2, &doc_json_ids(2, &[]), &"b".repeat(64), "t")
            .unwrap();
        seed_model_pricing(&store).unwrap();
        assert!(priced_keys(&store).is_empty());
    }
    /// arrangement. The `TempDir` must stay alive for the file to exist.
    fn test_env() -> (crate::store::Store, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path().join("kiwano.db")).unwrap();
        (store, dir)
    }

    fn sha_b() -> String {
        "b".repeat(64)
    }
    /// Pretend the Hub delivered this doc.
    #[test]
    fn seed_without_a_published_document_does_nothing() {
        let (store, _dir) = test_env();
        let first = seed_model_pricing(&store).unwrap();
        assert!(first.skipped);
        assert_eq!(first.seeded, 0);
        assert_eq!(store.app_setting(SEEDED_SHA_KEY), None);

        // …and once there are rows, they survive the document going away.
        cache_hub(&store, 1, "1", &sha_a());
        seed_model_pricing(&store).unwrap();
        assert_eq!(priced_keys(&store).len(), 1);

        store
            .conn
            .lock()
            .unwrap()
            .execute("DELETE FROM hub_models_cache", [])
            .unwrap();
        let after = seed_model_pricing(&store).unwrap();
        assert!(after.skipped);
        assert_eq!(priced_keys(&store).len(), 1, "the rows are left alone");
    }
    #[test]
    fn a_tiers_only_change_reseeds_the_row() {
        let (store, _dir) = test_env();
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
        cache_hub(&store, 1, "1", &sha_a());
        assert_eq!(seed_model_pricing(&store).unwrap().seeded, 1);
        assert_eq!(tiers_of(&store, "m1"), None);
        // …so a forced re-seed of the same bytes stays write-free.
        assert!(seed_model_pricing(&store).unwrap().skipped);

        // Only the discount moves.
        store
            .save_hub_models_cache(2, &doc("4.5"), &"c".repeat(64), "t")
            .unwrap();
        assert_eq!(seed_model_pricing(&store).unwrap().seeded, 1);

        store
            .save_hub_models_cache(3, &doc("3.5"), &"d".repeat(64), "t")
            .unwrap();
        assert_eq!(
            seed_model_pricing(&store).unwrap().seeded,
            1,
            "the tiers column changed"
        );
        let stored = tiers_of(&store, "m1").expect("the schedule reached the mirror");
        assert!(stored.contains(r#""in":"3.5""#), "{stored}");
        assert!(stored.contains("peak_hours"), "{stored}");
    }
    #[test]
    fn a_length_band_reaches_the_mirror_and_comes_back() {
        let (store, dir) = test_env();
        let doc = r#"{"version":1,"generated_at":"t","exchange_rates":{"USD":1.0},"models":[
            {"model_id":"m1","display_name":"M1","input":"2.10","output":"8.40",
             "cache_read":"0.42","cache_creation":"0","currency":"CNY",
             "long_context":{"over":512000,"in":"4.20","out":"16.80","cache_read":"0.84"}}]}"#;
        // Written whole rather than through `cache_hub`, whose third argument is
        // a price for its own single-row fixture.
        store
            .save_hub_models_cache(1, doc, &sha_a(), "2026-01-01T00:00:00Z")
            .unwrap();
        assert_eq!(seed_model_pricing(&store).unwrap().seeded, 1);

        let stored = tiers_of(&store, "m1").expect("the band reached the mirror");
        assert!(stored.contains("long_context"), "{stored}");
        assert!(stored.contains("512000"), "{stored}");

        // Read back the way the gateway reads it: `apply_tiers` unpacks the blob
        // into the entry, and the block's absent `cache_creation` is a zero rate
        // rather than a band that failed to parse.
        let store = crate::store::Store::open(dir.path().join("kiwano.db")).unwrap();
        let rows = store.load_model_pricing().unwrap();
        let band = rows[0].long_context.as_ref().expect("the band comes back");
        assert_eq!(band.over, 512_000);
        assert_eq!(band.input, "4.20");
        assert_eq!(band.cache_read, "0.84");
        assert_eq!(band.cache_creation, "0");
    }
    #[test]
    fn seed_gate_content_change_same_version() {
        let (store, _dir) = test_env();
        cache_hub(&store, 1, "1", &sha_a());
        let first = seed_model_pricing(&store).unwrap();
        assert!(!first.skipped);
        assert_eq!(first.seeded, 1);

        cache_hub(&store, 1, "9", &sha_b());
        let second = seed_model_pricing(&store).unwrap();
        assert!(!second.skipped, "a content change must re-seed");
        assert_eq!(second.seeded, 1, "the changed row is rewritten");

        // Same content again → skipped.
        assert!(seed_model_pricing(&store).unwrap().skipped);
    }
    #[test]
    fn seed_gate_version_bump_always_reseeds() {
        let (store, _dir) = test_env();
        cache_hub(&store, 1, "1", &sha_a());
        assert!(!seed_model_pricing(&store).unwrap().skipped);

        cache_hub(&store, 2, "1", &sha_b());
        let bumped = seed_model_pricing(&store).unwrap();
        assert!(!bumped.skipped);
        // …and stays write-free, because the WHERE guard sees no column change.
        assert_eq!(bumped.seeded, 0);
    }
    #[test]
    fn seed_gate_skips_identical_content_at_any_version() {
        let (store, _dir) = test_env();
        cache_hub(&store, 5, "1", &sha_a());
        assert!(!seed_model_pricing(&store).unwrap().skipped);

        cache_hub(&store, 4, "1", &sha_a());
        assert!(seed_model_pricing(&store).unwrap().skipped);
    }
    #[test]
    fn seed_gate_applies_changed_content_at_a_lower_version() {
        let (store, _dir) = test_env();
        cache_hub(&store, 5, "1", &sha_a());
        assert!(!seed_model_pricing(&store).unwrap().skipped);

        cache_hub(&store, 4, "9", &sha_b());
        let rolled_back = seed_model_pricing(&store).unwrap();
        assert!(!rolled_back.skipped);
        assert_eq!(rolled_back.seeded, 1);
    }
    #[test]
    fn effective_doc_is_none_until_the_hub_is_synced() {
        let store = crate::store::Store::open_in_memory().unwrap();
        assert!(effective_doc(&store).is_none());
        assert!(
            effective_rates(&store).is_empty(),
            "no rates to convert with"
        );

        cache_hub(&store, 99, "1", &sha_a());
        let (doc, sha) = effective_doc(&store).unwrap();
        assert_eq!(doc.version, 99);
        assert_eq!(sha, sha_a());
    }
    #[test]
    fn effective_doc_ignores_an_unusable_cache() {
        let store = crate::store::Store::open_in_memory().unwrap();
        store
            .save_hub_models_cache(9, "not json", &sha_a(), "2026-01-01T00:00:00Z")
            .unwrap();
        assert!(
            effective_doc(&store).is_none(),
            "an unparseable cache is no document at all"
        );
    }
    #[test]
    fn seeded_rows_record_their_source() {
        let (store, _dir) = test_env();
        cache_hub(&store, 1, "1", &sha_a());
        seed_model_pricing(&store).unwrap();
        let conn = store.conn.lock().unwrap();
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
