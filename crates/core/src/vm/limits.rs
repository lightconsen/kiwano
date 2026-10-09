//! Declared prices and spending limits — served by the daemon now.
//!
//! The rules moved to `kiwanod::api::limits` (`migrate.local.md` §10.13): the
//! daemon holds a provider-add to the same constraints the client did, and a
//! copy here would be the second one. Re-exported, so the `vm::` paths that
//! name them still resolve.

pub use kiwanod::api::limits::{
    import_declared_prices, known_limit_currencies, normalize_declared_prices, normalize_limit_unit,
};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::provider_edit::{add_provider, update_provider, NewProviderInput};
    use crate::vm::test_support::{catalog_input, no_vars, prices, store};
    use crate::vm::Aux;
    use kiwano_api::providers::ProviderPriceInput;
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
        assert_eq!(known_limit_currencies(&s), vec!["USD", "CNY"]);
        assert!(add_provider(&s, &limited_input("USD")).is_ok());
        assert!(
            add_provider(&s, &limited_input("cny")).is_ok(),
            "and it is not case-sensitive"
        );
        let err = match add_provider(&s, &limited_input("EUR")) {
            Err(e) => e,
            Ok(_) => panic!("EUR has no rate on this machine"),
        };
        assert!(
            err.contains("EUR") && err.contains("CNY"),
            "the refusal names the currency and what it does know: {err}"
        );

        // Synced: the table's own list, in order.
        let (_dir, synced) = store_with_hub_rates(r#"{"USD":1.0,"CNY":7.1,"EUR":0.9}"#);
        assert_eq!(known_limit_currencies(&synced), vec!["CNY", "EUR", "USD"]);
        assert!(add_provider(&synced, &limited_input("eur")).is_ok());
        assert!(add_provider(&synced, &limited_input("JPY")).is_err());

        // The counting units are not currencies, and a limit in one is what the
        // vast majority of providers have.
        for unit in ["requests", "wan_tokens"] {
            assert!(
                add_provider(&s, &limited_input(unit)).is_ok(),
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
        let vm = add_provider(&s, &input).unwrap();

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
