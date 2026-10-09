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
