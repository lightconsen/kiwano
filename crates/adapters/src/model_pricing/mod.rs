//! Model pricing table + cost calculation, ported from cc-switch
//! (src-tauri/src/services/usage_stats.rs matching layer and
//! src-tauri/src/proxy/usage/calculator.rs, MIT License).
//!
//! Prices are per million tokens as TEXT decimals in the row's own currency,
//! plus top-level exchange rates used for the Dashboard's display conversion.
//! There is no bundled snapshot: both the UI and the gateway read the Hub's
//! `models.json` — through the served document, or through the `model_pricing`
//! mirror the GUI seeds from it. An install that has never synced therefore has
//! no prices at all and costs read as "—", the same posture the catalog takes
//! (`vm::load_catalog`).
//!
//! A row may also publish **time-of-day** pricing: its own rates are then the
//! peak ones, applied inside `peak_hours` in the *vendor's* clock, with
//! `off_peak` in force outside them (`is_peak`, `compute_cost_pair`).
//!
//! cc-switch uses rust_decimal; here prices are parsed to f64 and results are
//! rounded to 6 decimal places, which is ample for per-request USD amounts.
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`model_pricing::PricingTable`): the facade below
//! re-exports it, because `crates/core/src/{pricing,sync,vm}.rs`, the
//! `crates/gateway` modules and `app/src-tauri/src/catalog.rs` name it that way
//! and none of them is edited.
//!
//! `types`, `convert`, `peak_hours` and `candidates` are leaves — they read
//! the rows, rates and ids they are handed and no table. `table` composes the
//! candidates into the lookup ladder, and `cost` composes `peak_hours` and the
//! tiers into the bill.

pub mod candidates;
pub mod convert;
pub mod cost;
pub mod peak_hours;
pub mod table;
pub mod types;

// ── the public surface, re-exported so every `model_pricing::x` path still resolves ──

pub use candidates::{is_placeholder_pricing_model, normalize_model_id, pricing_candidates};
pub use convert::{convert_amount, convert_cost_buckets};
pub use cost::{compute_cost, compute_cost_pair};
pub use peak_hours::is_peak;
pub use table::PricingTable;
pub use types::{
    DeclaredPrice, DeclaredPrices, LongContextRates, ModelPriceEntry, ModelsDoc, OffPeakRates,
    PeakHours, PeakWindow, PriceTiers,
};

#[cfg(test)]
pub(crate) mod test_support {
    use crate::model_pricing::table::PricingTable;
    use crate::model_pricing::types::{
        DeclaredPrices, LongContextRates, ModelPriceEntry, OffPeakRates, PeakHours, PeakWindow,
    };

    /// A stand-in for a Hub document. There is no bundled snapshot to read any
    /// more, so the lookup tests carry the rows they look up: one general price
    /// with a provider's own beside it, and one model priced per provider only.
    pub(crate) fn table() -> PricingTable {
        r#"{
            "version": 1,
            "exchange_rates": {"USD": 1.0, "CNY": 7.1},
            "models": [
                {"model_id": "claude-opus-4-8", "display_name": "Claude Opus 4.8",
                 "input": "5", "output": "25", "cache_read": "0.5",
                 "cache_creation": "6.25", "currency": "USD"},
                {"model_id": "claude-3-5-haiku", "display_name": "Claude 3.5 Haiku",
                 "input": "0.8", "output": "4", "cache_read": "0.08",
                 "cache_creation": "1", "currency": "USD"},
                {"provider_id": "zenmux", "model_id": "claude-opus-4-8",
                 "display_name": "Claude Opus 4.8 (ZenMux)", "input": "6",
                 "output": "30", "cache_read": "0.6", "cache_creation": "7.5",
                 "currency": "USD"},
                {"provider_id": "kimi", "model_id": "kimi-k2",
                 "display_name": "Kimi K2 (Kimi)", "input": "1", "output": "4",
                 "cache_read": "0.1", "cache_creation": "1", "currency": "USD"},
                {"provider_id": "moonshot", "model_id": "kimi-k2",
                 "display_name": "Kimi K2 (Moonshot)", "input": "2", "output": "8",
                 "cache_read": "0.2", "cache_creation": "2", "currency": "USD"}
            ]
        }"#
        .parse()
        .expect("fixture table parses")
    }

    /// One provider's declared rates, as `Store::load_declared_prices` builds
    /// them: keyed by the local provider row id, and — unlike the Hub's table —
    /// alone in their own table.
    pub(crate) fn declared() -> PricingTable {
        let blob = r#"{
            "currency": "CNY",
            "models": [
                {"model_id": "kimi-k2", "input": "1.5", "output": "6"},
                {"model_id": "claude-sonnet-5", "input": "12", "output": "60",
                 "cache_read": "1.2", "cache_creation": "15"}
            ]
        }"#;
        let declared = DeclaredPrices::parse(blob).expect("fixture blob parses");
        PricingTable::from_entries(declared.entries("kimi-moonshot-4f2a1c"))
    }

    pub(crate) fn usage_entry() -> ModelPriceEntry {
        ModelPriceEntry {
            provider_id: String::new(),
            model_id: "test".into(),
            display_name: "Test".into(),
            input: "3.0".into(),
            output: "15.0".into(),
            cache_read: "0.3".into(),
            cache_creation: "3.75".into(),
            currency: "USD".into(),
            off_peak: None,
            peak_hours: None,
            long_context: None,
        }
    }

    /// The published DeepSeek schedule: weekdays 09:00–12:00 and 14:00–18:00,
    /// Beijing time. The instants below are what the vendor's clock makes of
    /// them; 2026-09-09 is a Wednesday and 2026-09-12 a Saturday.
    pub(crate) fn deepseek_hours() -> PeakHours {
        let window = |start: &str, end: &str| PeakWindow {
            days: ["mon", "tue", "wed", "thu", "fri"]
                .iter()
                .map(|d| d.to_string())
                .collect(),
            start: start.into(),
            end: end.into(),
        };
        PeakHours {
            tz_offset: 480,
            windows: vec![window("09:00", "12:00"), window("14:00", "18:00")],
        }
    }

    /// A row with tiers: the listed rates inside the windows, the discounted
    /// ones outside, and the pair's second element always the off-peak answer.
    pub(crate) fn tiered_entry() -> ModelPriceEntry {
        ModelPriceEntry {
            input: "9.0".into(),
            output: "27.0".into(),
            cache_read: "0.30".into(),
            cache_creation: "0".into(),
            off_peak: Some(OffPeakRates {
                input: "4.5".into(),
                output: "13.5".into(),
                cache_read: "0.15".into(),
                cache_creation: "0".into(),
            }),
            peak_hours: Some(deepseek_hours()),
            ..usage_entry()
        }
    }

    /// `off_peak` omits `cache_creation` in the published document; absent means
    /// 0, not "the peak rate stands in".
    /// A row that prices in length bands — MiniMax M3's published shape.
    pub(crate) fn banded_entry() -> ModelPriceEntry {
        ModelPriceEntry {
            input: "2.10".into(),
            output: "8.40".into(),
            cache_read: "0.42".into(),
            cache_creation: "0".into(),
            long_context: Some(LongContextRates {
                over: 512_000,
                input: "4.20".into(),
                output: "16.80".into(),
                cache_read: "0.84".into(),
                cache_creation: "0".into(),
            }),
            ..usage_entry()
        }
    }
}
