//! Model pricing sync + currency helpers.
//!
//! The bundled models.json (compiled into kiwano-adapters) is the price
//! source. This module keeps the GUI-readable `model_pricing` SQLite table in
//! sync with it (version-keyed upsert, changed rows only) and provides the
//! currency metadata + conversion used across the usage/dashboard surfaces.

use crate::vm::Aux;
use kiwano_adapters::model_pricing::ModelsDoc;
use std::collections::HashMap;

/// app_settings KV key holding the last-seeded models.json version.
const SEEDED_VERSION_KEY: &str = "pricing_seeded_version";

/// app_settings KV key holding the sha256 of the content last seeded, when that
/// content came from the Hub. Absent for a bundled seed — the compiled snapshot
/// has no published digest — which keeps the legacy version-only gate for
/// installs that have never synced.
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
/// *priced* in, which is a different question — every bundled model is priced
/// in USD, so deriving the list from it offered USD alone while the default
/// preference was CNY, and picking USD left nothing to switch back to.
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

/// Read the bundled price table.
pub(crate) fn bundled_doc() -> ModelsDoc {
    serde_json::from_str(kiwano_adapters::model_pricing::MODELS_JSON)
        .expect("bundled models.json is valid")
}

/// The price table to seed from, with the sha256 of its content when it came
/// from the Hub. Prefers the Hub cache (a remote refresh wins over the compiled
/// snapshot) and falls back to bundled on absence *or* on a corrupt cache —
/// the same posture as `vm::load_catalog`. Never fails.
pub(crate) fn effective_doc(aux: &Aux) -> (ModelsDoc, Option<String>) {
    if let Some((_, payload, sha, _)) = aux.load_hub_models_cache() {
        if let Ok(doc) = serde_json::from_str::<ModelsDoc>(&payload) {
            return (doc, Some(sha));
        }
    }
    (bundled_doc(), None)
}

/// The content half of the seed gate. Only compared when both sides exist, so
/// an install that has never seen the Hub keeps the original version-only
/// behaviour (and a switch back to the bundled snapshot re-seeds once).
fn shas_match(fetched: Option<&str>, seeded: Option<&str>) -> bool {
    match (fetched, seeded) {
        (Some(a), Some(b)) => a == b,
        (Some(_), None) => false, // Hub content we have never seeded
        (None, Some(_)) => false, // source changed from the Hub back to bundled
        (None, None) => true,     // pure version gate (legacy)
    }
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
    let (doc, sha) = effective_doc(aux);
    let seeded_version = aux
        .get_setting(SEEDED_VERSION_KEY)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    let seeded_sha = aux.get_setting(SEEDED_SHA_KEY);
    if doc.version <= seeded_version && shas_match(sha.as_deref(), seeded_sha.as_deref()) {
        return Ok(SeededReport {
            seeded: 0,
            version: doc.version,
            skipped: true,
        });
    }

    // Provenance of these rows — the Hub refresh and the compiled snapshot are
    // different sources, and the column exists to tell them apart.
    let source = if sha.is_some() { "hub" } else { "bundled" };

    let conn = aux.conn.lock().expect("aux mutex poisoned");
    let mut seeded = 0usize;
    for m in &doc.models {
        let n = conn
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
    drop(conn);

    aux.set_setting(SEEDED_VERSION_KEY, &doc.version.to_string())
        .map_err(|e| e.to_string())?;
    // Record the content half so an unchanged Hub document is not re-seeded;
    // a bundled seed clears it, keeping the legacy version-only gate.
    match sha.as_deref() {
        Some(s) => aux
            .set_setting(SEEDED_SHA_KEY, s)
            .map_err(|e| e.to_string())?,
        None => {
            aux.delete_setting(SEEDED_SHA_KEY)
                .map_err(|e| e.to_string())?;
        }
    }
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

/// Persist the preferred display currency into the ui settings JSON.
/// (Writes go through vm::update_settings, which validates the code.)
#[tauri::command]
pub fn get_currency_meta(
    state: tauri::State<'_, crate::AppState>,
) -> Result<CurrencyMetaVm, String> {
    // The effective doc, not the bundled one: a Hub-supplied currency must be
    // selectable, and a bundled-only currency must not be offered once the Hub
    // table has replaced it — the list has to describe what was seeded.
    let doc = effective_doc(&state.aux).0;
    let preferred = preferred_currency(&state.aux);
    let currencies = displayable_currencies(&doc.exchange_rates, &preferred);
    Ok(CurrencyMetaVm {
        currencies,
        exchange_rates: doc.exchange_rates,
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

    #[test]
    fn displayable_currencies_come_from_the_rate_table() {
        let list = displayable_currencies(&rates(), "CNY");
        assert_eq!(list, vec!["CNY".to_string(), "USD".to_string()]);
    }

    #[test]
    fn a_preference_with_no_rate_still_stays_selectable() {
        // The reported bug in miniature: every bundled model is priced in USD,
        // so a list built from those offered USD alone while the default
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
        kiwano_gateway::store::Store::open(&path).unwrap();
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
        assert!(shas_match(Some("a"), Some("a")));
        assert!(!shas_match(Some("a"), Some("b")));
        assert!(!shas_match(Some("a"), None), "Hub content never seeded");
        assert!(
            !shas_match(None, Some("a")),
            "source changed back to bundled"
        );
        assert!(shas_match(None, None), "legacy version-only gate");
    }

    /// An install that never saw the Hub keeps the original behaviour: the
    /// bundled snapshot has no published digest, so the version alone gates it.
    #[test]
    fn seed_gate_version_only_legacy() {
        let (aux, _dir) = test_env();
        let first = seed_model_pricing(&aux).unwrap();
        assert!(!first.skipped);
        assert!(first.seeded > 0);
        assert_eq!(aux.get_setting(SEEDED_SHA_KEY), None);

        let second = seed_model_pricing(&aux).unwrap();
        assert!(second.skipped);
        assert_eq!(second.seeded, 0);
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
    fn effective_doc_prefers_hub_then_bundled() {
        let aux = Aux::open_in_memory().unwrap();
        let (doc, sha) = effective_doc(&aux);
        assert_eq!(doc.version, bundled_doc().version);
        assert_eq!(sha, None);

        cache_hub(&aux, 99, "1", &sha_a());
        let (doc, sha) = effective_doc(&aux);
        assert_eq!(doc.version, 99);
        assert_eq!(sha.as_deref(), Some(sha_a().as_str()));
    }

    #[test]
    fn effective_doc_corrupt_cache_falls_back() {
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_models_cache(9, "not json", &sha_a(), "2026-01-01T00:00:00Z")
            .unwrap();
        let (doc, sha) = effective_doc(&aux);
        assert_eq!(doc.version, bundled_doc().version);
        assert_eq!(sha, None, "an unusable cache must not arm the gate");
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
