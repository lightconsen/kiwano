//! Model pricing sync + currency helpers.
//!
//! The Hub's models.json — cached verbatim by [`crate::sync`] — is the price
//! source, as it is for the catalog. This module keeps the GUI-readable
//! `model_pricing` SQLite table in sync with it (version-keyed upsert, changed
//! rows only, rows the document dropped pruned away) and provides the currency
//! metadata used across the usage/dashboard surfaces.

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

/// The price table to seed from, with the sha256 of the bytes it came from, or
/// `None` when the Hub has never been synced or its cache is unusable.
///
/// There is no compiled fallback: like the catalog, prices come from the Hub
/// alone. `None` is therefore "nothing to seed", never "an empty price
/// document" — the difference matters, because seeding an empty document
/// clears the table.
pub fn effective_doc(store: &kiwanod::store::Store) -> Option<(ModelsDoc, String)> {
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

/// The exchange rates to convert with, or an empty table before the first sync
/// (every conversion then passes amounts through unchanged).
pub fn effective_rates(store: &kiwanod::store::Store) -> HashMap<String, f64> {
    effective_doc(store)
        .map(|(doc, _)| doc.exchange_rates)
        .unwrap_or_default()
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
/// Seed the price mirror from the cached Hub document — served by the daemon
/// (`kiwanod::api::pricing_sync::seed_model_pricing`): the mirror is the table
/// the gateway bills from, and the cache it reads is the daemon's.
pub fn seed_model_pricing(store: &kiwanod::store::Store) -> Result<SeededReport, String> {
    kiwanod::api::pricing_sync::seed_model_pricing(store)
}

// The report type travels with the function that produces it.
pub use kiwanod::api::pricing_sync::SeededReport;

/// The user's preferred display currency (default CNY). Stored inside the
/// ui settings blob (same source the Settings page round-trips).
pub fn preferred_currency(store: &kiwanod::store::Store) -> String {
    kiwanod::api::settings::ui_settings(store)
        .map(|s| s.preferred_currency)
        .unwrap_or_else(|_| crate::vm::default_preferred_currency())
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
/// Served by the daemon (`kiwanod::api::pricing::list_model_prices`) — it is the
/// side that charges with this table, so it is the side that serves it.
pub fn list_model_prices(store: &kiwanod::store::Store) -> Result<Vec<ModelPriceEntry>, String> {
    kiwanod::api::pricing::list_model_prices(store).map_err(|e| e.to_string())
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

pub fn currency_meta(store: &kiwanod::store::Store) -> Result<CurrencyMetaVm, String> {
    let rates = effective_rates(store);
    let preferred = preferred_currency(store);
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

    /// The conversion and display helpers — the client's half. The seed and the
    /// mirror's own rows are tested where they live now
    /// (`kiwanod::api::pricing_sync`).
    /// Every `(provider_id, model_id)` the local price table currently holds.    /// The tiers blob a row carries, as stored: `None` means the row has no    /// A row the document drops has to stop being priced.
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
    fn rates() -> HashMap<String, f64> {
        HashMap::from([("USD".to_string(), 1.0), ("CNY".to_string(), 7.1)])
    }
}
