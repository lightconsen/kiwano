//! The in-memory price lookup: `PricingTable` and its matching ladder.
//!
//! Rows are keyed by `(provider_id, model_id)`, both normalized; an empty
//! `provider_id` is the general price. `find` is the ladder a request walks
//! (exact candidates, then a gated prefix scan, with the provider breaking
//! ties at every rung, and the general row behind each of them);
//! `find_declared` is the same ladder without the last rung, for the prices a
//! user declared for their own provider row.
//!
//! Two orderings here are load-bearing and must not be touched: `build` sorts
//! the providers of a model so that `""` sorts first (the fallback relies on
//! it), and sorts the table keys by `(len, name)` so the prefix scan finds the
//! shortest match.

use crate::model_pricing::candidates::{pricing_candidates, should_try_pricing_prefix_match};
use crate::model_pricing::types::{ModelPriceEntry, ModelsDoc};
use std::collections::HashMap;

/// In-memory price lookup built from models.json.
///
/// Keyed by `(provider_id, model_id)`; both are normalized/lowercase and a row
/// whose `provider_id` is empty is the general price.
#[derive(Debug, Clone, Default)]
pub struct PricingTable {
    /// provider_id -> model_id -> entry.
    rows: HashMap<String, HashMap<String, ModelPriceEntry>>,
    /// model_id -> the providers that price it, ascending, which is what makes
    /// `fallback` deterministic and try the general row first ("" sorts first).
    providers_by_model: HashMap<String, Vec<String>>,
    /// Sorted model_id keys for the gated prefix scan (shortest match wins).
    keys: Vec<String>,
    /// Currency conversion rates for display (e.g. USD -> CNY).
    exchange_rates: HashMap<String, f64>,
    /// models.json version (seed-versioning key for the SQLite seeder).
    pub version: i64,
}

/// Parse a models.json document (`json.parse::<PricingTable>()`).
impl std::str::FromStr for PricingTable {
    type Err = serde_json::Error;

    fn from_str(json: &str) -> Result<Self, Self::Err> {
        let doc: ModelsDoc = serde_json::from_str(json)?;
        Ok(Self::build(doc.models, doc.exchange_rates, doc.version))
    }
}

impl PricingTable {
    /// Index the rows. Shared by the JSON document and the mirror-read path so
    /// the two can never disagree about what a key means.
    fn build(
        entries: Vec<ModelPriceEntry>,
        exchange_rates: HashMap<String, f64>,
        version: i64,
    ) -> Self {
        let mut rows: HashMap<String, HashMap<String, ModelPriceEntry>> =
            HashMap::with_capacity(entries.len());
        let mut providers_by_model: HashMap<String, Vec<String>> = HashMap::new();
        for entry in entries {
            let provider = entry.provider_id.trim().to_ascii_lowercase();
            let model = entry.model_id.trim().to_ascii_lowercase();
            rows.entry(provider.clone())
                .or_default()
                .insert(model.clone(), entry);
            providers_by_model.entry(model).or_default().push(provider);
        }
        for providers in providers_by_model.values_mut() {
            providers.sort();
            providers.dedup();
        }
        let mut keys: Vec<String> = providers_by_model.keys().cloned().collect();
        keys.sort_by_key(|k| (k.len(), k.clone()));
        Self {
            rows,
            providers_by_model,
            keys,
            exchange_rates,
            version,
        }
    }

    /// Build a table from rows read back from the `model_pricing` mirror. The
    /// gateway resolves prices in memory, so this is how a Hub-refreshed price
    /// table reaches a forwarded request. Exchange rates stay empty: the mirror
    /// stores rows, not the rates above them, and the one place the gateway
    /// converts money — comparing a provider's spend against its limit — reads
    /// them from the cached document instead.
    ///
    /// `version` is 0 — it keys the SQLite seeder, not in-memory lookups.
    pub fn from_entries(entries: Vec<ModelPriceEntry>) -> Self {
        Self::build(entries, HashMap::new(), 0)
    }

    pub fn exchange_rates(&self) -> &HashMap<String, f64> {
        &self.exchange_rates
    }

    /// Exact-key hit: `provider`'s own row for `model`.
    fn get_exact(&self, provider: &str, model: &str) -> Option<&ModelPriceEntry> {
        self.rows
            .get(provider)
            .and_then(|by_model| by_model.get(model))
    }

    /// The general price for `model`, or failing that the lowest-`provider_id`
    /// row that prices it.
    ///
    /// This tail is tolerance, not policy. The Hub prices a model per provider
    /// entry, and a local provider that has not been matched to its catalog
    /// entry (`Provider.catalog_id`) can only be found by model. The ordering is
    /// the point: `""` is the lowest key, so the general row wins when there is
    /// one, and the rest is stable — a model-keyed table returned whichever row
    /// happened to be seeded last, so a cost could move without any price moving.
    fn fallback(&self, model: &str) -> Option<&ModelPriceEntry> {
        self.providers_by_model
            .get(model)?
            .iter()
            .find_map(|provider| self.get_exact(provider, model))
    }

    /// One id, two rungs — the provider's price, else the fallback. `own_only`
    /// drops the second rung; see `find_declared` for when that is the answer.
    fn lookup(&self, provider: &str, model: &str, own_only: bool) -> Option<&ModelPriceEntry> {
        let exact = self.get_exact(provider, model);
        if own_only {
            exact
        } else {
            exact.or_else(|| self.fallback(model))
        }
    }

    /// Gated prefix scan: shortest table key that starts with `candidate-`,
    /// resolved for this provider. The caller applies
    /// `should_try_pricing_prefix_match` first.
    fn get_prefix(
        &self,
        provider: &str,
        candidate: &str,
        own_only: bool,
    ) -> Option<&ModelPriceEntry> {
        let mut prefix = String::with_capacity(candidate.len() + 1);
        prefix.push_str(candidate);
        prefix.push('-');
        let key = self.keys.iter().find(|k| k.starts_with(&prefix))?;
        self.lookup(provider, key, own_only)
    }

    /// Resolve pricing for a raw (upstream) model id using cc-switch's matching
    /// ladder: exact candidate hits first, then a gated prefix scan, with the
    /// provider breaking ties at every rung.
    ///
    /// `provider_id` is the *catalog* entry id, not the local provider's row id
    /// (a provider added from the shelf is named whatever the user called it).
    /// An empty string asks for the general price, which is also what a provider
    /// with no catalog entry gets.
    pub fn find(&self, provider_id: &str, model_id: &str) -> Option<&ModelPriceEntry> {
        self.find_with(provider_id, model_id, false)
    }

    /// Resolve a price the **user declared** for this very provider row, or
    /// `None` when they declared none for this model.
    ///
    /// The one difference from `find` is the missing last rung: a declared miss
    /// must stay a miss. Falling back would answer "the user priced nothing for
    /// this model" with another provider's rate and — since the declared rung is
    /// consulted *first* — swallow the catalog price that ought to have been the
    /// answer for a provider that has both. The ladder above it is the same one:
    /// aliases and date snapshots resolve, so a declared `claude-sonnet-5` still
    /// covers the `claude-sonnet-5-20250929` a request names.
    pub fn find_declared(&self, provider_id: &str, model_id: &str) -> Option<&ModelPriceEntry> {
        self.find_with(provider_id, model_id, true)
    }

    /// The matching ladder: exact candidate hits first, then a gated prefix
    /// scan, with the provider breaking ties at every rung — and, unless
    /// `own_only`, the general/any-provider fallback behind each of them.
    fn find_with(
        &self,
        provider_id: &str,
        model_id: &str,
        own_only: bool,
    ) -> Option<&ModelPriceEntry> {
        let provider = provider_id.trim().to_ascii_lowercase();
        let candidates = pricing_candidates(model_id);
        for candidate in &candidates {
            if let Some(entry) = self.lookup(&provider, candidate, own_only) {
                return Some(entry);
            }
        }
        for candidate in &candidates {
            if should_try_pricing_prefix_match(candidate) {
                if let Some(entry) = self.get_prefix(&provider, candidate, own_only) {
                    return Some(entry);
                }
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use crate::model_pricing::test_support::{declared, table};

    // ---- table lookup ----------------------------------------------------

    #[test]
    fn find_resolves_exact_normalized_ids() {
        let t = table();
        let entry = t.find("", "anthropic/claude-opus-4-8:beta").unwrap();
        assert_eq!(entry.model_id, "claude-opus-4-8");
        assert_eq!(entry.currency, "USD");
    }

    #[test]
    fn find_resolves_date_snapshot_via_candidates() {
        let t = table();
        // The full snapshot id has no row of its own; the candidate pass strips
        // the date and lands on the family row.
        let entry = t.find("", "claude-3-5-haiku-20241022").unwrap();
        assert_eq!(entry.model_id, "claude-3-5-haiku");
    }

    #[test]
    fn find_unknown_model_is_none() {
        let t = table();
        assert!(t.find("", "totally-made-up-model").is_none());
        assert!(t.find("", "unknown").is_none());
        assert!(t.find("", "").is_none());
        // A known provider asking for a model nobody prices is still a miss.
        assert!(t.find("kimi", "totally-made-up-model").is_none());
    }

    /// The reason the key carries a provider at all: two providers may price the
    /// same model differently, and each request costs at its own provider's rate.
    #[test]
    fn find_uses_the_price_of_the_provider_that_was_asked_for() {
        let t = table();
        assert_eq!(t.find("kimi", "kimi-k2").unwrap().input, "1");
        assert_eq!(t.find("moonshot", "kimi-k2").unwrap().input, "2");
        // A provider's own row beats the general one for the same model.
        assert_eq!(t.find("", "claude-opus-4-8").unwrap().input, "5");
        assert_eq!(t.find("zenmux", "claude-opus-4-8").unwrap().input, "6");
    }

    /// Falling back is the point of the ladder: an unmatched provider (a manual
    /// one, or one added before it carried a catalog id) must still be costed.
    /// The order is fixed — general row first, then lowest provider_id — so a
    /// cost never depends on which row was seeded last.
    #[test]
    fn find_falls_back_to_the_general_row_then_by_provider_id() {
        let t = table();
        // claude-opus-4-8 has a general row and a zenmux one; the general row
        // is what an unlisted provider gets ("" sorts below "zenmux").
        let entry = t.find("no-such-provider", "claude-opus-4-8").unwrap();
        assert_eq!(entry.provider_id, "", "the general price, not zenmux's");
        assert_eq!(entry.input, "5");
        // claude-3-5-haiku has no provider row at all: general row either way.
        assert_eq!(t.find("zenmux", "claude-3-5-haiku").unwrap().input, "0.8");
        // kimi-k2 exists only per provider, so the fallback picks by name.
        assert_eq!(t.find("no-such-provider", "kimi-k2").unwrap().input, "1");
    }

    // ---- declared prices (the providers.prices column) -------------------

    #[test]
    fn declared_entries_carry_the_currency_and_the_provider_row_id() {
        let t = declared();
        let entry = t.get_exact("kimi-moonshot-4f2a1c", "kimi-k2").unwrap();
        assert_eq!(entry.currency, "CNY");
        assert_eq!(entry.input, "1.5");
        // Nothing else knows this vendor's spelling for the model.
        assert_eq!(entry.display_name, "kimi-k2");
        // Flat: bands and time-of-day tiers are the Hub's vocabulary.
        assert!(entry.off_peak.is_none() && entry.peak_hours.is_none());
        assert!(entry.long_context.is_none());
    }

    #[test]
    fn find_declared_answers_from_the_declared_rows_and_never_another_provider() {
        let declared = declared();
        assert_eq!(
            declared
                .find_declared("kimi-moonshot-4f2a1c", "kimi-k2")
                .unwrap()
                .input,
            "1.5"
        );
        // A model nobody declared is a miss, and has to stay one: the declared
        // rung is consulted *first*, so an answer here would become the price of
        // every request to this provider.
        assert!(declared
            .find_declared("kimi-moonshot-4f2a1c", "kimi-k3")
            .is_none());
        // …and the row id is the key, so another provider's table is empty.
        assert!(declared
            .find_declared("some-other-1b2c3d", "kimi-k2")
            .is_none());
    }

    #[test]
    fn find_declared_walks_the_same_ladder_as_find() {
        let declared = declared();
        // A request names a dated snapshot; the user declared the family.
        assert!(declared
            .find_declared("kimi-moonshot-4f2a1c", "claude-sonnet-5-20250929")
            .is_some());
        // Namespaced and mixed-case ids reduce to the declared one.
        assert!(declared
            .find_declared("kimi-moonshot-4f2a1c", "anthropic/claude-sonnet-5")
            .is_some());
        assert!(declared
            .find_declared("kimi-moonshot-4f2a1c", "Kimi-K2")
            .is_some());
    }
}
