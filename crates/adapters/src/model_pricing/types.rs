//! The wire shapes: one row of `models.json`, its tiers, and the prices a
//! user declared for their own provider.
//!
//! Every struct here is serde-derived, and these types ride **both** the
//! Tauri IPC boundary and the SQLite `model_pricing` mirror, so a field
//! reorder is a silent break. `ModelsDoc` is the byte the Hub serves and is
//! read only, so it derives `Deserialize` alone.
//!
//! A leaf: nothing here reads a table.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A row's rates outside its peak hours.
///
/// The document spells these `in` / `out` — short keys, and not ours to rename:
/// the published shape is what the Hub and every other client share. The rates
/// are read as they are written, so a vendor that discounts input but not output
/// is expressible.
///
/// Both cache rates are defaulted rather than required: `models/README.md` says
/// an absent cache field means 0, and a parse failure here is not a local
/// problem — it fails the whole document, which the seeder answers by keeping
/// the *previous* prices for every model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffPeakRates {
    #[serde(rename = "in")]
    pub input: String,
    #[serde(rename = "out")]
    pub output: String,
    #[serde(default = "zero_rate")]
    pub cache_read: String,
    #[serde(default = "zero_rate")]
    pub cache_creation: String,
}

/// Rates that apply once a request's input passes a size — the second band some
/// vendors price in (MiniMax M3, most of Alibaba's models).
///
/// The row's **own** rates are the listed band, which is the *cheap* one: the
/// data repo's README makes that explicit ("the listed band is the cheaper one"),
/// because the headline price a reader sees should be the one most requests pay.
/// A client that ignores this field therefore bills a long request at the low
/// band — understating rather than overstating, the deliberate trade recorded
/// there.
///
/// There is no off-peak counterpart here, and that is a property of the document
/// rather than of this type: a band carries one set of rates. So a row that
/// publishes both a schedule and a band has nothing to compare against inside the
/// band — see `compute_cost_pair`, which treats it as flat rather than inventing
/// a discount the vendor never published.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LongContextRates {
    /// Input tokens **above** which these rates apply. The one published figure
    /// that is a number rather than a TEXT decimal.
    pub over: u64,
    #[serde(rename = "in")]
    pub input: String,
    #[serde(rename = "out")]
    pub output: String,
    /// Same rule as the row's own rates: a cache rate the document leaves out is
    /// zero, never the band's `in`/`out`. An omitted rate is an absence of a
    /// charge, and reading it as "the same as input" would multiply the bill.
    #[serde(default = "zero_rate")]
    pub cache_read: String,
    #[serde(default = "zero_rate")]
    pub cache_creation: String,
}

/// The cache rates the document leaves out default to 0, as the data repo's
/// README states — not to the peak rate, which would invent a charge.
fn zero_rate() -> String {
    "0".to_string()
}

/// When a row's own (peak) rates apply, in the **vendor's** clock.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeakHours {
    /// Minutes east of UTC. Required when a schedule is published: these windows
    /// are business hours somewhere, and judging "is it peak now" against the
    /// reader's own timezone would silently pick the wrong rate.
    #[serde(default)]
    pub tz_offset: i32,
    #[serde(default)]
    pub windows: Vec<PeakWindow>,
}

/// One peak window. `start`/`end` are `HH:MM` in the vendor's clock and the
/// window is half-open — `[start, end)` — which is what lets adjacent windows
/// compose without a gap or an overlap. A window never wraps midnight; the
/// document writes `22:00–02:00` as two windows.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeakWindow {
    #[serde(default)]
    pub days: Vec<String>,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
}

/// The two halves of a time-of-day price, as they are stored between the
/// document and the mirror: one JSON blob, so the seeder and the mirror reader
/// cannot disagree about the shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PriceTiers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_peak: Option<OffPeakRates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_hours: Option<PeakHours>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<LongContextRates>,
}

/// One row of models.json (prices = currency per million tokens, TEXT decimals).
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ModelPriceEntry {
    /// The catalog provider entry this price belongs to. The Hub prices a model
    /// per provider, so the same `model_id` may appear once per provider at
    /// different rates (a subsidy, a margin, an off-peak tariff).
    ///
    /// Empty means "not specific to a provider": that is what documents and
    /// rows written before this field existed carry, and the lookup treats them
    /// as the general price.
    #[serde(default)]
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub input: String,
    pub output: String,
    pub cache_read: String,
    pub cache_creation: String,
    pub currency: String,
    /// The discounted rates in force outside `peak_hours`. The row's own rates
    /// above are the **peak** ones — they apply inside the windows.
    #[serde(default)]
    pub off_peak: Option<OffPeakRates>,
    #[serde(default)]
    pub peak_hours: Option<PeakHours>,
    /// The rates that apply once the request's input passes `over`. The rates
    /// above are then the band below it.
    #[serde(default)]
    pub long_context: Option<LongContextRates>,
}

impl ModelPriceEntry {
    /// The tiers as they travel to the mirror, or `None` when the row has none.
    /// "No tiers" is NULL rather than `"{}"`, so a forced re-seed of a tiered
    /// document stays write-free for the rows that never had any.
    pub fn tiers_json(&self) -> Option<String> {
        let tiers = PriceTiers {
            off_peak: self.off_peak.clone(),
            peak_hours: self.peak_hours.clone(),
            long_context: self.long_context.clone(),
        };
        if tiers.off_peak.is_none() && tiers.peak_hours.is_none() && tiers.long_context.is_none() {
            return None;
        }
        serde_json::to_string(&tiers).ok()
    }

    /// Read the tiers back from the mirror, **tolerating** a blob this build
    /// cannot parse: an unreadable row means peak-only for that row, never an
    /// error. An error here would reach `resolve_pricing`, which answers a read
    /// failure with an empty table — every cost NULL, which is far worse than
    /// one row billed at its peak rate.
    pub fn apply_tiers(&mut self, raw: Option<&str>) {
        let Some(raw) = raw else { return };
        let Ok(tiers) = serde_json::from_str::<PriceTiers>(raw) else {
            return;
        };
        self.off_peak = tiers.off_peak;
        self.peak_hours = tiers.peak_hours;
        self.long_context = tiers.long_context;
    }
}

/// The prices a user declared for one of their own providers — the shape of the
/// `providers.prices` column, and of the JSON the add/edit dialog sends back.
///
/// This is the second source of prices, and the only one a hand-added provider
/// has: the Hub prices the models of *its* catalog entries, and a provider that
/// names no entry is otherwise costed at the general rate for a model the Hub
/// happens to know, or recorded unpriced. Since the user is the only one who can
/// say what such a provider charges, they say it here.
///
/// One currency per provider rather than one per row: these are what one vendor
/// charges, and a provider billing different models in different currencies is
/// not a thing. `parse` tolerates a blob this build cannot read — see its note.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DeclaredPrices {
    /// ISO code every figure below is denominated in, uppercase.
    pub currency: String,
    #[serde(default)]
    pub models: Vec<DeclaredPrice>,
}

/// One model's declared rates, per million tokens as TEXT decimals like every
/// other price in this module (the arithmetic parses them; the display does not
/// round them on the way through).
///
/// The cache rates have no `Option`: a user who does not know what their vendor
/// charges for a cache read writes nothing, and nothing is zero — the same
/// reading the document takes for a rate it leaves out (`zero_rate`), and the
/// one the form states plainly. Charging the input rate instead would invent a
/// charge the vendor may not make, which is the failure mode that fires a
/// spending limit early.
#[derive(Debug, Clone, PartialEq, Deserialize, Serialize)]
pub struct DeclaredPrice {
    pub model_id: String,
    pub input: String,
    pub output: String,
    #[serde(default = "zero_rate")]
    pub cache_read: String,
    #[serde(default = "zero_rate")]
    pub cache_creation: String,
}

impl DeclaredPrices {
    /// Read a stored blob, tolerating one this build cannot parse.
    ///
    /// Malformed means "no declared prices for this provider", never an error.
    /// The caller is `Store::load_declared_prices`, which the gateway answers a
    /// failure of with an empty table — so erroring here would cost *every*
    /// provider its declared prices over one bad row, exactly the trade
    /// `ModelPriceEntry::apply_tiers` refuses.
    pub fn parse(raw: &str) -> Option<Self> {
        serde_json::from_str(raw).ok()
    }

    /// The blob as the `providers.prices` column holds it.
    ///
    /// Serialization cannot fail — every field is a `String`, or a `Vec` of
    /// structs holding them — so the expect is a statement about the type rather
    /// than a case to handle.
    pub fn to_json(&self) -> String {
        serde_json::to_string(self).expect("a declared-price blob holds only strings")
    }

    /// The rows as price-table entries, keyed by the **local provider id**.
    ///
    /// That key is what makes this a separate table rather than more rows in the
    /// Hub's one: `PricingTable`'s other keys are catalog entry ids, and the two
    /// namespaces are the whole reason `Provider.catalog_id` is kept apart from
    /// `Provider.id` (`<slug>-<hex>`). Merging them would let a catalog row and
    /// a local row shadow each other by string equality.
    pub fn entries(&self, provider_id: &str) -> Vec<ModelPriceEntry> {
        self.models
            .iter()
            .map(|m| ModelPriceEntry {
                provider_id: provider_id.to_string(),
                model_id: m.model_id.clone(),
                // The model id is the only name a declared row has; nothing else
                // knows this vendor's display spelling for it.
                display_name: m.model_id.clone(),
                input: m.input.clone(),
                output: m.output.clone(),
                cache_read: m.cache_read.clone(),
                cache_creation: m.cache_creation.clone(),
                currency: self.currency.clone(),
                // Bands and time-of-day tiers are the Hub's vocabulary, not
                // something the form collects: a declared row is flat.
                off_peak: None,
                peak_hours: None,
                long_context: None,
            })
            .collect()
    }
}

/// Top-level models.json document.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDoc {
    pub version: i64,
    /// Informational only, so it tolerates being absent: the Hub serves this
    /// document remotely, and one omitted field must not strand every client
    /// on a parse error. `version` and `exchange_rates` stay required — their
    /// absence is a real defect, and rejecting it keeps the previous cache.
    #[serde(default)]
    pub generated_at: String,
    pub exchange_rates: HashMap<String, f64>,
    pub models: Vec<ModelPriceEntry>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn declared_blob_reads_back_with_zero_cache_rates_by_default() {
        let parsed = DeclaredPrices::parse(
            r#"{"currency":"CNY","models":[{"model_id":"kimi-k2","input":"1.5","output":"6"}]}"#,
        )
        .expect("parses");
        assert_eq!(parsed.currency, "CNY");
        // A rate the user left out is zero, not the input rate: the form says
        // so, and inventing a charge is what fires a spending limit early.
        assert_eq!(parsed.models[0].cache_read, "0");
        assert_eq!(parsed.models[0].cache_creation, "0");
        // A blob this build cannot read is "no declared prices", never an
        // error: one bad row must not cost every provider its prices.
        assert!(DeclaredPrices::parse("not json").is_none());
        assert_eq!(
            DeclaredPrices::parse(r#"{"currency":"CNY"}"#)
                .unwrap()
                .models,
            vec![]
        );
    }

    #[test]
    fn absent_off_peak_cache_creation_is_zero() {
        let doc: ModelsDoc = serde_json::from_str(
            r#"{"version": 1, "exchange_rates": {"USD": 1.0}, "models": [
                {"model_id": "m", "display_name": "M", "input": "9", "output": "27",
                 "cache_read": "0.3", "cache_creation": "1", "currency": "USD",
                 "off_peak": {"in": "4.5", "out": "13.5", "cache_read": "0.15"}}]}"#,
        )
        .expect("the published shape parses");
        let e = &doc.models[0];
        assert_eq!(e.off_peak.as_ref().unwrap().cache_creation, "0");
        // …and a document whose off_peak has no cache fields at all still parses,
        // rather than failing every price this install has.
        let sparse: ModelsDoc = serde_json::from_str(
            r#"{"version": 1, "exchange_rates": {"USD": 1.0}, "models": [
                {"model_id": "m", "display_name": "M", "input": "9", "output": "27",
                 "cache_read": "0.3", "cache_creation": "1", "currency": "USD",
                 "off_peak": {"in": "4.5", "out": "13.5"}}]}"#,
        )
        .expect("absent cache rates default to 0");
        assert_eq!(sparse.models[0].off_peak.as_ref().unwrap().cache_read, "0");
    }
}
