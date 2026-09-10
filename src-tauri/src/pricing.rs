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

/// Currency metadata for the Settings selector + UI conversion.
#[derive(serde::Serialize)]
pub struct CurrencyMetaVm {
    /// ISO codes present in the bundled price table (rates map keys).
    pub currencies: Vec<String>,
    /// currency -> units of that currency per 1 USD (e.g. CNY: 7.1).
    pub exchange_rates: HashMap<String, f64>,
    /// The user's preferred display currency (Settings).
    pub preferred: String,
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

/// Upsert the bundled price rows into the shared `model_pricing` table.
/// Version-gated: a no-op unless models.json carries a newer version than the
/// `pricing_seeded_version` KV. Rows are written only when a column actually
/// changes (WHERE guard), so repeated runs stay write-free.
pub fn seed_model_pricing(aux: &Aux) -> Result<SeededReport, String> {
    let doc = bundled_doc();
    let seeded_version = aux
        .get_setting(SEEDED_VERSION_KEY)
        .and_then(|v| v.parse::<i64>().ok())
        .unwrap_or(0);
    if doc.version <= seeded_version {
        return Ok(SeededReport {
            seeded: 0,
            version: doc.version,
            skipped: true,
        });
    }

    let conn = aux.conn.lock().expect("aux mutex poisoned");
    let mut seeded = 0usize;
    for m in &doc.models {
        let n = conn
            .execute(
                "INSERT INTO model_pricing (model_id, display_name, input, output,
                                            cache_read, cache_creation, currency, source)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'cc-switch')
                 ON CONFLICT(model_id) DO UPDATE SET
                    display_name = ?2, input = ?3, output = ?4,
                    cache_read = ?5, cache_creation = ?6, currency = ?7
                 WHERE display_name <> ?2 OR input <> ?3 OR output <> ?4
                    OR cache_read <> ?5 OR cache_creation <> ?6 OR currency <> ?7",
                rusqlite::params![
                    m.model_id,
                    m.display_name,
                    m.input,
                    m.output,
                    m.cache_read,
                    m.cache_creation,
                    m.currency,
                ],
            )
            .map_err(|e| e.to_string())?;
        seeded += n;
    }
    drop(conn);

    aux.set_setting(SEEDED_VERSION_KEY, &doc.version.to_string())
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

/// Persist the preferred display currency into the ui settings JSON.
/// (Writes go through vm::update_settings, which validates the code.)
#[tauri::command]
pub fn get_currency_meta(
    state: tauri::State<'_, crate::AppState>,
) -> Result<CurrencyMetaVm, String> {
    let doc = bundled_doc();
    let mut currencies: Vec<String> = doc.models.iter().map(|m| m.currency.clone()).collect();
    currencies.sort();
    currencies.dedup();
    Ok(CurrencyMetaVm {
        currencies,
        exchange_rates: doc.exchange_rates,
        preferred: preferred_currency(&state.aux),
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
    fn convert_via_usd_pivot() {
        let r = rates();
        assert!((convert_amount(7.1, "CNY", "USD", &r) - 1.0).abs() < 1e-9);
        assert!((convert_amount(1.0, "USD", "CNY", &r) - 7.1).abs() < 1e-9);
        assert_eq!(convert_amount(3.0, "CNY", "CNY", &r), 3.0);
        // Unknown currency passes through unchanged.
        assert_eq!(convert_amount(5.0, "EUR", "USD", &r), 5.0);
    }
}
