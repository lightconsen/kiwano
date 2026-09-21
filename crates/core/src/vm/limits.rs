//! Spending limits and declared prices: which currencies this machine can reason
//! about, and how a limit unit or a price row is normalized.

use kiwano_adapters::model_pricing::{DeclaredPrice, DeclaredPrices};
use kiwanod::store::Store;
use serde::Deserialize;
use std::collections::HashSet;

/// The prices a user declared for a provider, as the modal form sends them: one
/// currency for every figure, one row per model.
///
/// Asked for on a pay-as-you-go provider the form is the user's own (a hand-added
/// one, or an edit) because the Hub prices the models of *its* catalog entries —
/// a provider that names none has no published price to be costed at, so the user
/// is the only one who can say what it charges. These figures are also what its
/// spending limit is measured against.
#[derive(Deserialize)]
pub struct ProviderPricesInput {
    /// ISO code the figures are denominated in. The form fills it from the same
    /// picker the spending limit uses: both are about what this provider bills.
    pub currency: String,
    #[serde(default)]
    pub models: Vec<ProviderPriceInput>,
}

/// One model's declared rates, per million tokens, as the user typed them.
#[derive(Deserialize)]
pub struct ProviderPriceInput {
    pub model_id: String,
    /// Kept as text: every rate in the price table is a TEXT decimal, and
    /// re-printing one from the parsed f64 would rewrite what the user wrote.
    pub input: String,
    pub output: String,
    /// Blank is zero — "this vendor charges nothing for that bucket", which is
    /// what the form's hint says. Charging the input rate instead would invent a
    /// charge and overstate the spend a limit is measured against.
    #[serde(default)]
    pub cache_read: Option<String>,
    #[serde(default)]
    pub cache_creation: Option<String>,
}

/// A machine that has never synced has no table at all, and then it is the two
/// currencies the Hub publishes rates against. Empty is not "anything goes": the
/// reason for the rule is that no rate exists, and that is true of every third
/// currency as well.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::provider_edit::{add_provider, update_provider, NewProviderInput};
    use crate::vm::test_support::{catalog_input, no_vars, prices, store};
    use crate::vm::Aux;
    use kiwanod::store::Store;

    /// A pay-as-you-go provider with a spending limit in `unit`.
    fn limited_input(unit: &str) -> NewProviderInput {
        let mut input = catalog_input("Limited", "https://api.limited.example");
        input.billing_config.limit_value = Some(50.0);
        input.billing_config.limit_unit = Some(unit.into());
        input
    }

    /// A store that has synced the Hub, so its rate table is not empty. Written
    /// through a second connection because the cache belongs to the GUI's schema,
    /// which the gateway reads and does not create (see `limits::tests`).
    fn store_with_hub_rates(rates: &str) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hub_models_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 version   INTEGER NOT NULL,
                 sha256    TEXT NOT NULL,
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
             )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, 1, 'sha', ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![format!(
                r#"{{"version":1,"exchange_rates":{rates},"models":[]}}"#
            )],
        )
        .unwrap();
        (dir, store)
    }

    // A limit's currency has to be one this machine can convert. The limit is
    // measured against costs priced in other currencies, and `convert_amount` hands
    // a currency it has no rate for back **unchanged** — added to the others at
    // 1:1 — so a limit denominated in one fires at the wrong time.
    //
    // The rule lives in the normalizer, which is where every writer passes: the
    // dialog and the CLI both build a `NewProviderInput`, and the share importer
    // calls it directly.
    #[test]
    fn a_limit_currency_must_be_one_this_machine_can_convert() {
        // Never synced: no table at all, so the two currencies the Hub publishes
        // rates against. Empty is not "anything goes" — no rate exists for a third
        // one either.
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        assert_eq!(known_limit_currencies(&s), vec!["USD", "CNY"]);
        assert!(add_provider(&s, &aux, &limited_input("USD")).is_ok());
        assert!(
            add_provider(&s, &aux, &limited_input("cny")).is_ok(),
            "and it is not case-sensitive"
        );
        let err = match add_provider(&s, &aux, &limited_input("EUR")) {
            Err(e) => e,
            Ok(_) => panic!("EUR has no rate on this machine"),
        };
        assert!(
            err.contains("EUR") && err.contains("CNY"),
            "the refusal names the currency and what it does know: {err}"
        );

        // Synced: the table's own list, in order.
        let (_dir, synced) = store_with_hub_rates(r#"{"USD":1.0,"CNY":7.1,"EUR":0.9}"#);
        let aux2 = Aux::open_in_memory().unwrap();
        assert_eq!(known_limit_currencies(&synced), vec!["CNY", "EUR", "USD"]);
        assert!(add_provider(&synced, &aux2, &limited_input("eur")).is_ok());
        assert!(add_provider(&synced, &aux2, &limited_input("JPY")).is_err());

        // The counting units are not currencies, and a limit in one is what the
        // vast majority of providers have.
        for unit in ["requests", "wan_tokens"] {
            assert!(
                add_provider(&s, &aux, &limited_input(unit)).is_ok(),
                "{unit} is a counting unit"
            );
        }
    }

    // ── Declared prices (the Custom form's Prices section) ──────────────────

    /// A bundle as the form sends it.
    fn price_row(model_id: &str, input: &str, output: &str) -> ProviderPriceInput {
        ProviderPriceInput {
            model_id: model_id.into(),
            input: input.into(),
            output: output.into(),
            cache_read: None,
            cache_creation: None,
        }
    }

    /// What the prices section hands the backend, both ways: a provider typed in
    /// by hand carries the figures to its row, and the dialog reads them back.
    #[test]
    fn declared_prices_round_trip_through_add_and_edit() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        let mut input = catalog_input("Manual", "https://api.manual.example");
        input.billing_config.limit_value = Some(50.0);
        input.billing_config.limit_unit = Some("CNY".into());
        input.prices = Some(prices(
            "cny",
            vec![
                price_row("kimi-k2", "1.5", "6"),
                price_row("glm-4.6", "2", "8"),
            ],
        ));
        let vm = add_provider(&s, &aux, &input).unwrap();

        // The currency is stored uppercase, and the figures come back as typed —
        // the dialog is the only thing that can correct them, so it has to see
        // what it sent.
        let stored = vm.prices.expect("the VM carries the declared prices");
        assert_eq!(stored["currency"], "CNY");
        assert_eq!(stored["models"][0]["model_id"], "kimi-k2");
        assert_eq!(stored["models"][0]["input"], "1.5");
        // …and the gateway's own reader agrees, keyed by the row's id.
        let rows = s.load_declared_prices().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.provider_id == vm.id));
        assert!(rows.iter().all(|r| r.currency == "CNY"));

        // An edit that says nothing about prices keeps them.
        let mut edit = catalog_input("Manual", "https://api.manual.example");
        edit.prices = None;
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &edit,
            &no_vars(),
        )
        .unwrap();
        assert!(vm.prices.is_some(), "absent means keep");

        // An edit with an empty bundle clears them: that is the form's state when
        // the provider leaves pay-as-you-go.
        let mut cleared = catalog_input("Manual", "https://api.manual.example");
        cleared.prices = Some(prices("CNY", vec![]));
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &cleared,
            &no_vars(),
        )
        .unwrap();
        assert!(vm.prices.is_none());
        assert!(s.load_declared_prices().unwrap().is_empty());
    }

    /// What the section refuses, and why each refusal is one rather than a
    /// silent drop: both would leave the provider quietly mispriced.
    #[test]
    fn declared_prices_refuse_a_currency_or_a_rate_that_cannot_be_used() {
        let known = known_limit_currencies(&store());

        // A currency this machine cannot convert: these figures are what the
        // spending limit is measured against.
        let err =
            normalize_declared_prices(Some(&prices("EUR", vec![price_row("m", "1", "2")])), &known)
                .expect_err("EUR has no rate on this machine");
        assert!(err.contains("EUR") && err.contains("CNY"), "{err}");
        assert!(normalize_declared_prices(Some(&prices("YEN", vec![])), &known).is_err());

        // A rate the price table cannot parse as a number would reach the cost
        // arithmetic as NaN — in every request to this provider.
        for bad in ["abc", "-1", "1e999"] {
            let bundle = prices("USD", vec![price_row("m", bad, "2")]);
            assert!(
                normalize_declared_prices(Some(&bundle), &known).is_err(),
                "`{bad}` is not a rate"
            );
        }
    }

    /// The rows the section drops rather than stores: a blank model id (the empty
    /// tail row the form always leaves) and a duplicate.
    #[test]
    fn declared_prices_drop_blank_and_duplicate_rows() {
        let known = known_limit_currencies(&store());
        let bundle = prices(
            "USD",
            vec![
                price_row("", "9", "9"),
                price_row("kimi-k2", "1.5", "6"),
                price_row("Kimi-K2", "99", "99"),
                price_row("glm-4.6", "2", "8"),
            ],
        );
        let blob = normalize_declared_prices(Some(&bundle), &known)
            .unwrap()
            .expect("two rows survive");
        let parsed = kiwano_adapters::model_pricing::DeclaredPrices::parse(&blob).unwrap();
        // Case-insensitively deduplicated, first wins: the table keys a model in
        // lowercase, so keeping both would make one of them win by write order.
        assert_eq!(parsed.models.len(), 2);
        assert_eq!(parsed.models[0].input, "1.5");

        // A rate the user left out is zero, not the input rate.
        let bundle = prices("USD", vec![price_row("m", "1", "2")]);
        let blob = normalize_declared_prices(Some(&bundle), &known)
            .unwrap()
            .unwrap();
        let parsed = kiwano_adapters::model_pricing::DeclaredPrices::parse(&blob).unwrap();
        assert_eq!(parsed.models[0].cache_read, "0");
        assert_eq!(parsed.models[0].cache_creation, "0");

        // Nothing to say, in both of the ways the form says it.
        assert!(normalize_declared_prices(None, &known).unwrap().is_none());
        assert!(
            normalize_declared_prices(Some(&prices("USD", vec![])), &known)
                .unwrap()
                .is_none()
        );
    }

    /// A shared config carries the declared prices with the provider, and they
    /// are validated for the importing machine the way a limit's unit is — same
    /// rule, because the limit is measured against them.
    #[test]
    fn a_shared_provider_keeps_its_declared_prices() {
        let known = known_limit_currencies(&store());
        let blob = normalize_declared_prices(
            Some(&prices("CNY", vec![price_row("kimi-k2", "1.5", "6")])),
            &known,
        )
        .unwrap();

        let carried = import_declared_prices(blob.as_deref(), &known).unwrap();
        assert_eq!(carried, blob, "unchanged where the currency converts");

        // A blob this build cannot read is dropped, not fatal: it says nothing
        // to honour, and failing the import would cost the provider its row.
        assert_eq!(
            import_declared_prices(Some("{not json"), &known).unwrap(),
            None
        );
        assert_eq!(import_declared_prices(None, &known).unwrap(), None);

        // A currency this machine cannot convert is refused, exactly as the same
        // file's limit unit would be: the provider is about to be costed in it.
        let foreign = r#"{"currency":"EUR","models":[{"model_id":"m","input":"1","output":"2"}]}"#;
        assert!(import_declared_prices(Some(foreign), &known).is_err());
    }
}
