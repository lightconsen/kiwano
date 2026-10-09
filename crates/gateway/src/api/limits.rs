//! Declared prices and spending limits: what a user may say, and what gets
//! stored.
//!
//! Moved here from `kiwano_core::vm::limits` so the daemon's `add_provider` can
//! hold a form to the same rules the client does (`migrate.local.md` §10.13).
//! These return `String` on purpose: they are shared with paths that are still
//! the client's, and the daemon's `api::providers_add` maps them into
//! [`ApiError`] at its own boundary.

use crate::store::Store;
use kiwano_adapters::model_pricing::{DeclaredPrice, DeclaredPrices};
use kiwano_api::providers::{ProviderPriceInput, ProviderPricesInput};
use std::collections::HashSet;

pub fn known_limit_currencies(store: &Store) -> Vec<String> {
    let rates = store.hub_exchange_rates();
    if rates.is_empty() {
        vec!["USD".to_string(), "CNY".to_string()]
    } else {
        let mut codes: Vec<String> = rates.into_keys().collect();
        codes.sort();
        codes
    }
}

/// Normalize the user-entered per-period limit unit (tech.md §2.4 A).
///
/// With a limit set but no unit chosen, fall back to the legacy behavior of
/// counting "requests"; with no limit the unit is meaningless and stored as NULL.
/// Units are the two counting units plus a currency code from `known` (stored
/// uppercase; the v9 CHECK constraint enforces the same shape).
///
/// A currency this machine cannot price against is **refused** rather than
/// dropped: falling back to `requests` would quietly turn a money ceiling into a
/// request count, which is a different limit, not a smaller one.
pub fn normalize_limit_unit(
    unit: Option<&str>,
    has_limit: bool,
    known: &[String],
) -> Result<Option<String>, String> {
    if !has_limit {
        return Ok(None);
    }
    match unit {
        Some("wan_tokens") => Ok(Some("wan_tokens".into())),
        Some("requests") => Ok(Some("requests".into())),
        Some(u) => {
            let code = u.trim().to_ascii_uppercase();
            // Not a currency code at all: the legacy reading, count requests.
            if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                return Ok(Some("requests".into()));
            }
            if is_known_currency(&code, known) {
                Ok(Some(code))
            } else {
                Err(format!(
                    "a limit cannot be in {code}: this machine has no rate for it, and the limit is \
                     measured against costs priced in other currencies. Known: {}",
                    if known.is_empty() {
                        "none (the Hub has never been synced)".to_string()
                    } else {
                        known.join(", ")
                    }
                ))
            }
        }
        None => Ok(Some("requests".into())),
    }
}

/// Whether this machine can convert amounts in `code`.
///
/// One rule for two callers: the unit a spending limit is denominated in and the
/// currency declared prices are written in. Both end up compared against usage
/// the machine costs in whatever currency the price table names, and
/// `convert_amount` passes an unknown currency through unchanged rather than
/// inventing a rate — so an amount in one would be compared against a limit in
/// another as though the numbers meant the same thing.
fn is_known_currency(code: &str, known: &[String]) -> bool {
    known.iter().any(|k| k.eq_ignore_ascii_case(code))
}

/// One declared rate, validated, with a blank meaning zero.
///
/// The text is trimmed and kept as written: `0.80` and `0.8` are the same rate,
/// and the one the reader sees back should be the one they typed. Anything that
/// is not a non-negative number is refused — the price table parses these as f64
/// at cost time, so a stray character would otherwise turn into a NaN (or a
/// zero) in the recorded cost of every request to that provider.
fn declared_rate(raw: &str, model_id: &str, what: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok("0".to_string());
    }
    let value: f64 = text
        .parse()
        .map_err(|_| format!("the {what} rate of `{model_id}` is not a number: `{text}`"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "the {what} rate of `{model_id}` must be zero or more: `{text}`"
        ));
    }
    Ok(text.to_string())
}

/// Validate the declared prices and serialize them into the `providers.prices`
/// blob.
///
/// `None` (the field absent from the request) is "not speaking about prices": an
/// update keeps what is stored, exactly as it does for `advanced` and
/// `plan_query`. `Some` is an authoritative snapshot, so a bundle that holds no
/// model clears the column.
///
/// Both refusals below are refusals rather than silent drops, because either
/// would leave the provider quietly mispriced:
/// - **A currency this machine cannot convert** — the figures are what the
///   spending limit is measured against, and they are recorded in this currency.
/// - **A rate that is not a non-negative number** — see `declared_rate`.
///
/// A row with a blank model id is dropped (that is the form's empty tail row)
/// and a duplicated model id keeps the first, the same rule the modal applies to
/// a duplicated protocol when it saves the endpoint list.
pub fn normalize_declared_prices(
    input: Option<&ProviderPricesInput>,
    known: &[String],
) -> Result<Option<String>, String> {
    let Some(input) = input else {
        return Ok(None);
    };
    let currency = input.currency.trim().to_ascii_uppercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!("`{currency}` is not a currency code"));
    }
    if !is_known_currency(&currency, known) {
        return Err(format!(
            "prices cannot be in {currency}: this machine has no rate for it, and the spending \
             limit is measured against costs priced in other currencies. Known: {}",
            if known.is_empty() {
                "none (the Hub has never been synced)".to_string()
            } else {
                known.join(", ")
            }
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut models = Vec::new();
    for row in &input.models {
        let model_id = row.model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        // Case-insensitive, because that is how the table keys a model: two rows
        // differing only in case would collide there and one would win by write
        // order rather than by anything the user chose.
        if !seen.insert(model_id.to_ascii_lowercase()) {
            continue;
        }
        models.push(DeclaredPrice {
            model_id: model_id.to_string(),
            input: declared_rate(&row.input, model_id, "input")?,
            output: declared_rate(&row.output, model_id, "output")?,
            cache_read: declared_rate(
                row.cache_read.as_deref().unwrap_or(""),
                model_id,
                "cache read",
            )?,
            cache_creation: declared_rate(
                row.cache_creation.as_deref().unwrap_or(""),
                model_id,
                "cache write",
            )?,
        });
    }
    if models.is_empty() {
        return Ok(None);
    }
    Ok(Some(DeclaredPrices { currency, models }.to_json()))
}

/// The declared prices of an imported provider, validated for *this* machine.
///
/// A shared config carries the blob the exporting install wrote (the shape
/// `normalize_declared_prices` produces), and it arrives the same way a limit's
/// unit does: written where that currency could be converted, read where it may
/// not be. Same rule, then — refused rather than re-denominated, since dropping
/// it would let the provider's requests be costed by the Hub's table instead
/// without anyone having said so.
///
/// The one exception is a blob this build cannot read. That one says nothing to
/// honour, so there is no statement to refuse: it is treated as "none declared",
/// which is both the honest reading of a field written in a shape this build does
/// not know and the state most providers are in anyway.
pub fn import_declared_prices(
    raw: Option<&str>,
    known: &[String],
) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(parsed) = DeclaredPrices::parse(raw) else {
        return Ok(None);
    };
    // Back through the input shape, so an imported blob is held to exactly the
    // rule the form is — a hand-edited share file included.
    let input = ProviderPricesInput {
        currency: parsed.currency,
        models: parsed
            .models
            .into_iter()
            .map(|m| ProviderPriceInput {
                model_id: m.model_id,
                input: m.input,
                output: m.output,
                cache_read: Some(m.cache_read),
                cache_creation: Some(m.cache_creation),
            })
            .collect(),
    };
    normalize_declared_prices(Some(&input), known)
}
