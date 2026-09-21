//! A provider's own money: the currency its figures are denominated in — the
//! unit the agent's spending-limit picker offers — and what its usage cost in
//! that currency. Never converted: both are read beside the provider's own
//! limits rather than in the user's display currency.

use crate::vm::catalog::CatalogEntryVm;
use kiwanod::store::Provider;
use std::collections::HashMap;

/// The currencies a spending limit may be denominated in.
///
/// The Hub's rate table, because that is what makes the limit comparable with the
/// costs it is measured against: `convert_cost_buckets` needs a rate for both
/// sides, and a currency without one is **added** to the others at 1:1 — the bug
/// that once let a `¥50` limit mean nothing in particular.
///
/// The currency a provider's figures are denominated in.
///
/// Declared prices win: they are what cost this provider's requests, and a
/// spending limit is measured against that cost. Then the catalog entry it was
/// added from — the authority on what a provider bills in, and the field the Hub
/// added for exactly this (its own default is USD, so a matched entry carries a
/// currency either way). A provider added by hand, with no entry behind it and
/// nothing declared, falls back to USD as well: the price table's base, and the
/// only honest answer when nothing said otherwise.
pub(crate) fn provider_currency(p: &Provider, entries: &[CatalogEntryVm]) -> String {
    if let Some(c) = p
        .prices
        .as_deref()
        .and_then(|s| {
            serde_json::from_str::<kiwano_adapters::model_pricing::DeclaredPrices>(s).ok()
        })
        .map(|d| d.currency)
        .filter(|c| !c.is_empty())
    {
        return c;
    }
    p.catalog_id
        .as_deref()
        .and_then(|id| entries.iter().find(|e| e.id == id))
        .map(|e| e.currency.clone())
        .unwrap_or_else(|| "USD".to_string())
}

/// Cost of one provider, in the currency its usage was priced in.
///
/// No conversion: a provider bills in one currency and this number is read
/// beside that provider's own limits. Should usage ever be priced in more than
/// one currency (a price-table currency change mid-period), the currency
/// carrying the most money names the total — the alternatives are folding
/// other currencies in at a rate nobody asked for, or inventing a second line
/// for a case that does not occur in practice. Unpriced rows (`None`)
/// contribute nothing, exactly as they did when the sum was converted.
pub(crate) fn provider_cost(buckets: &[(Option<String>, f64)]) -> (Option<f64>, Option<String>) {
    let mut per_currency: HashMap<&str, f64> = HashMap::new();
    for (currency, cost) in buckets {
        let Some(currency) = currency.as_deref() else {
            continue;
        };
        *per_currency.entry(currency).or_default() += cost;
    }
    match per_currency.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((currency, total)) => (Some(total), Some(currency.to_string())),
        None => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::catalog::catalog_snapshot;
    use crate::vm::test_support::provider;
    use crate::vm::Aux;
    use kiwanod::store::Billing;

    /// The currency a provider's figures are denominated in — the field the
    /// agent's spending-limit picker offers as a unit, so it decides which money
    /// a ceiling can be written in.
    ///
    /// Declared prices win over the entry: a user who typed their own rates also
    /// typed what they are in, and those are the figures costing the requests the
    /// limit is measured against. Neither source means USD — the price table's
    /// base, and the only honest answer when nothing named another.
    #[test]
    fn a_provider_carries_the_currency_its_figures_are_in() {
        const CUR: &str = r#"{"total":2,"entries":[
            {"id":"cn-1","name":"CN One","tag":"third","rating":3,"billing":"payg",
             "currency":"CNY",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.cn1.example"}]},
            {"id":"us-1","name":"US One","tag":"third","rating":3,"billing":"payg",
             "currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.us1.example"}]}
        ]}"#;
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_cache(CUR, "2026-09-07T00:00:00Z").unwrap();
        let entries = catalog_snapshot(&aux).entries;

        // Added from an entry: what that entry bills in.
        let mut cn = provider("cn", "CN One", Billing::Metered);
        cn.catalog_id = Some("cn-1".into());
        assert_eq!(provider_currency(&cn, &entries), "CNY");

        // Declared prices, over an entry that says USD.
        let mut declared = provider("decl", "Declared", Billing::Metered);
        declared.catalog_id = Some("us-1".into());
        declared.prices = Some(r#"{"currency":"CNY","models":[]}"#.into());
        assert_eq!(provider_currency(&declared, &entries), "CNY");

        // Hand-added, and an id the catalog no longer carries: the same answer,
        // because neither names a currency at all.
        assert_eq!(
            provider_currency(&provider("bare", "Bare", Billing::Metered), &entries),
            "USD"
        );
        let mut stale = provider("stale", "Stale", Billing::Metered);
        stale.catalog_id = Some("gone".into());
        assert_eq!(provider_currency(&stale, &entries), "USD");
    }

    #[test]
    fn provider_cost_stays_in_its_own_currency() {
        let (cost, currency) = provider_cost(&[
            (Some("USD".into()), 1.5),
            (Some("USD".into()), 0.5),
            (None, 9.0), // unpriced row: no currency, contributes nothing
        ]);
        assert_eq!(cost, Some(2.0));
        assert_eq!(currency.as_deref(), Some("USD"));

        // Mixed currencies: the one carrying the most money names the total.
        let (cost, currency) =
            provider_cost(&[(Some("USD".into()), 1.0), (Some("CNY".into()), 40.0)]);
        assert_eq!(cost, Some(40.0));
        assert_eq!(currency.as_deref(), Some("CNY"));

        assert_eq!(provider_cost(&[]), (None, None));
        assert_eq!(provider_cost(&[(None, 3.0)]), (None, None));
    }
}
