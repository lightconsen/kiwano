//! Persisting a metered sample: the `usage` row, the full request log, and
//! the side task a stream hands its sample to when it ends.

use std::sync::Arc;

use tokio::sync::mpsc;

use crate::forward::pricing::compute_sample_cost;
use crate::forward::sample::UsageSample;
use crate::server::GatewayState;
use crate::store::RequestLogNew;

pub(crate) async fn record_pending_usage(
    state: Arc<GatewayState>,
    mut rx: mpsc::Receiver<UsageSample>,
) {
    if let Some(sample) = rx.recv().await {
        record_sample(&state, sample);
    }
}

pub(crate) fn record_sample(state: &GatewayState, sample: UsageSample) {
    let mut sample = sample;
    let log = sample.log.take();
    // The two things the `usage` table has no column for: they describe this
    // request's own log row rather than the metered totals, and `into_record`
    // consumes the sample below.
    let reasoning_tokens = sample.usage.reasoning_tokens;
    let usage_missing = sample.usage_missing;
    let (cost, cost_off_peak, cost_currency) = compute_sample_cost(state, &sample);
    let record = sample.into_record(cost, cost_off_peak, cost_currency);
    tracing::info!(
        agent = %record.agent,
        provider_id = %record.provider_id,
        input = record.input_tokens,
        output = record.output_tokens,
        cache_read = record.cache_read_tokens,
        cache_creation = record.cache_creation_tokens,
        reasoning = reasoning_tokens,
        usage_missing = usage_missing,
        latency_ms = record.latency_ms,
        status = %record.status,
        cost = record.cost,
        "usage captured"
    );
    if let Err(e) = state.store.record_usage(&record) {
        tracing::warn!(error = %e, "failed to persist usage record");
    } else {
        // The numbers on disk just moved, and nothing else would tell the app:
        // one tick per recorded request is what lets its screens re-read rather
        // than poll (see `GatewayState::notify_usage`).
        state.notify_usage();
    }
    if let Some(log) = log {
        // The finding's first line, read before the entry takes the notes.
        let dlp_line = log
            .request_notes
            .as_ref()
            .and_then(|n| n.lines().find(|l| l.starts_with("dlp: ")))
            .map(str::to_string);
        let entry = RequestLogNew {
            ts: log.capture.ts,
            method: log.capture.method,
            path: log.capture.path,
            query: log.capture.query,
            agent: Some(record.agent.clone()),
            attribution: Some(log.attribution),
            provider_id: Some(record.provider_id.clone()),
            model: record.model,
            status_code: log.status_code as i64,
            error_kind: log.error_kind,
            error_message: log.error_message,
            session_id: log.capture.session_id,
            is_streaming: log.is_streaming,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_read_tokens: record.cache_read_tokens,
            cache_creation_tokens: record.cache_creation_tokens,
            reasoning_tokens,
            usage_missing,
            latency_ms: record.latency_ms,
            first_token_ms: log.first_token_ms,
            request_headers: log.capture.request_headers,
            response_headers: log.response_headers,
            request_body: log.capture.request_body,
            response_body: log.response_body,
            request_size: log.capture.request_size,
            response_size: log.response_size,
            truncated: log.capture.truncated || log.truncated,
            cost: record.cost,
            cost_currency: record.cost_currency,
            cost_off_peak: record.cost_off_peak,
            request_notes: log.request_notes,
        };
        let log_id = match state.store.insert_request_log(&entry) {
            Ok(id) => id,
            Err(e) => {
                tracing::warn!(error = %e, "failed to persist request log");
                return;
            }
        };
        // A credential finding is a fact that happened once, and its
        // details are the one source: the log row's id is what the app
        // deep-links, the note's first line is what it shows. Sent here —
        // after attribution and persistence — rather than at the scan,
        // which runs before either is known.
        if let Some(note) = dlp_line {
            state.notify_event(crate::server::GatewayEvent::DlpFinding {
                log_id,
                agent: record.agent,
                provider_id: Some(record.provider_id),
                note,
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::TEST_AT;
    use crate::meter::Usage;

    #[test]
    fn record_sample_costs_priced_models_and_skips_unknown() {
        use crate::store::Protocol;

        // The `model_pricing` mirror is the only price source (there is no
        // compiled snapshot behind it), so the rows this test costs against
        // have to be seeded into it — exactly as the GUI seeder would.
        let store = crate::store::Store::open_in_memory().expect("store");
        store
            .upsert_model_pricing(&kiwano_adapters::model_pricing::ModelPriceEntry {
                long_context: None,
                provider_id: String::new(),
                model_id: "claude-opus-4-8".into(),
                off_peak: None,
                peak_hours: None,
                display_name: "Claude Opus 4.8".into(),
                input: "5".into(),
                output: "25".into(),
                cache_read: "0.5".into(),
                cache_creation: "6.25".into(),
                currency: "USD".into(),
            })
            .unwrap();
        let state = crate::server::GatewayState::new(store).expect("state");
        state
            .store
            .insert_provider(&crate::store::Provider {
                id: "p1".into(),
                name: "p1".into(),
                catalog_id: None,
                // No declared prices: this fixture is priced by the Hub's table.
                prices: None,
                protocol: Protocol::Anthropic,
                base_url: "https://a.example.com".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk".into()),
                model_default: None,
                billing: crate::store::Billing::Metered,
                period_limit: None,
                limit_unit: None,
                plan_query: None,
                plan_limits: None,
                timeout_secs: None,
                retries: None,
                headers: None,
                reset_period: None,
                enabled: true,
                created_at: crate::store::now_rfc3339(),
                updated_at: crate::store::now_rfc3339(),
            })
            .unwrap();

        // claude-opus-4-8 is priced in the mirror above (input 5 USD/M).
        let sample = |model: Option<&'static str>| UsageSample {
            agent: "claude".into(),
            provider_id: "p1".into(),
            catalog_id: None,
            started_unix: TEST_AT,
            model: model.map(str::to_string),
            usage: Usage {
                reasoning_tokens: 0,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        // One recorded request is one tick on the admin plane's event stream,
        // and that tick is the whole of what keeps the app's numbers live —
        // nothing else would notice the day it stopped happening.
        let mut ticks = state.usage_ticks();
        record_sample(&state, sample(Some("claude-opus-4-8")));
        assert!(ticks.try_recv().is_ok(), "a recorded request ticks");
        record_sample(&state, sample(Some("totally-unpriced-model")));
        record_sample(&state, sample(None));
        // Unpriced is still metered, and still moves the count the app shows.
        assert!(ticks.try_recv().is_ok(), "and so does one it cannot price");

        let totals = state.store.usage_totals(None, None, None).unwrap();
        assert_eq!(totals.requests, 3);
        // Only the priced model contributes; cost is stored in the price
        // entry's currency (1M fresh input @ 5 USD/M = 5.0).
        let costs = state
            .store
            .usage_cost_by_currency(None, None, None)
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].0.as_deref(), Some("USD"));
        assert!((costs[0].1 - 5.0).abs() < 1e-9, "got {}", costs[0].1);
    }

    /// The price a request is costed at follows the provider it went through.
    ///
    /// The Hub prices a model per catalog entry, so one model can cost
    /// differently at two providers — and a provider with no catalog entry is
    /// costed at the general price, not at a neighbour's subsidised one. This
    /// pins the whole chain: the stored `catalog_id`, the lookup, and the
    /// recorded cost.
    #[test]
    fn record_sample_uses_the_providers_own_price() {
        use crate::store::Protocol;

        let store = crate::store::Store::open_in_memory().expect("store");
        let priced =
            |provider_id: &str, input: &str| kiwano_adapters::model_pricing::ModelPriceEntry {
                long_context: None,
                provider_id: provider_id.into(),
                model_id: "m1".into(),
                off_peak: None,
                peak_hours: None,
                display_name: "M1".into(),
                input: input.into(),
                output: "0".into(),
                cache_read: "0".into(),
                cache_creation: "0".into(),
                currency: "USD".into(),
            };
        store.upsert_model_pricing(&priced("", "1")).unwrap();
        store.upsert_model_pricing(&priced("kimi", "3")).unwrap();
        for (id, catalog_id) in [("p-kimi", Some("kimi")), ("p-manual", None)] {
            store
                .insert_provider(&crate::store::Provider {
                    id: id.into(),
                    name: id.into(),
                    catalog_id: catalog_id.map(str::to_string),
                    // No declared prices: this fixture is priced by the Hub's table.
                    prices: None,
                    protocol: Protocol::Anthropic,
                    base_url: "https://a.example.com".into(),
                    api_path: None,
                    endpoints: Vec::new(),
                    api_key: Some("sk".into()),
                    model_default: None,
                    billing: crate::store::Billing::Metered,
                    period_limit: None,
                    limit_unit: None,
                    plan_query: None,
                    plan_limits: None,
                    timeout_secs: None,
                    retries: None,
                    headers: None,
                    reset_period: None,
                    enabled: true,
                    created_at: crate::store::now_rfc3339(),
                    updated_at: crate::store::now_rfc3339(),
                })
                .unwrap();
        }
        let state = crate::server::GatewayState::new(store).expect("state");

        let sample = |provider_id: &str, catalog_id: Option<&str>| UsageSample {
            agent: "claude".into(),
            provider_id: provider_id.into(),
            catalog_id: catalog_id.map(str::to_string),
            started_unix: TEST_AT,
            model: Some("m1".into()),
            usage: Usage {
                reasoning_tokens: 0,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        record_sample(&state, sample("p-kimi", Some("kimi")));
        record_sample(&state, sample("p-manual", None));

        let cost_of = |provider_id: &str| {
            let costs = state
                .store
                .usage_cost_by_currency(None, Some(provider_id), None)
                .unwrap();
            costs[0].1
        };
        assert!(
            (cost_of("p-kimi") - 3.0).abs() < 1e-9,
            "the shelf provider pays its own rate"
        );
        assert!(
            (cost_of("p-manual") - 1.0).abs() < 1e-9,
            "a hand-added provider pays the general rate"
        );
    }

    /// The prices a user declared for their own provider cost its requests, and
    /// they cost them **first** — ahead of the catalog entry's published rates
    /// and ahead of the general row.
    ///
    /// The case this exists for is a provider the Hub cannot price at all: a
    /// hand-added one, which used to be costed at whatever the general table
    /// happened to say about the model (or at nothing). The second half is the
    /// boundary of that: a model the user did *not* declare still falls through
    /// to the Hub's table, so declaring one price does not price the rest at a
    /// neighbour's rate.
    #[test]
    fn record_sample_uses_the_prices_the_user_declared() {
        use crate::store::Protocol;

        let store = crate::store::Store::open_in_memory().expect("store");
        let priced = |provider_id: &str, model_id: &str, input: &str| {
            kiwano_adapters::model_pricing::ModelPriceEntry {
                long_context: None,
                provider_id: provider_id.into(),
                model_id: model_id.into(),
                off_peak: None,
                peak_hours: None,
                display_name: model_id.into(),
                input: input.into(),
                output: "0".into(),
                cache_read: "0".into(),
                cache_creation: "0".into(),
                currency: "USD".into(),
            }
        };
        // The Hub prices m1 generally and per catalog entry; m2 only generally.
        store.upsert_model_pricing(&priced("", "m1", "1")).unwrap();
        store.upsert_model_pricing(&priced("", "m2", "2")).unwrap();
        store
            .upsert_model_pricing(&priced("kimi", "m1", "3"))
            .unwrap();

        let declared = kiwano_adapters::model_pricing::DeclaredPrices {
            currency: "USD".into(),
            models: vec![kiwano_adapters::model_pricing::DeclaredPrice {
                model_id: "m1".into(),
                input: "9".into(),
                output: "0".into(),
                cache_read: "0".into(),
                cache_creation: "0".into(),
            }],
        }
        .to_json();
        let mk =
            |id: &str, catalog_id: Option<&str>, prices: Option<&str>| crate::store::Provider {
                id: id.into(),
                name: id.into(),
                catalog_id: catalog_id.map(str::to_string),
                prices: prices.map(str::to_string),
                protocol: Protocol::Anthropic,
                base_url: "https://a.example.com".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk".into()),
                model_default: None,
                billing: crate::store::Billing::Metered,
                period_limit: None,
                limit_unit: None,
                plan_query: None,
                plan_limits: None,
                timeout_secs: None,
                retries: None,
                headers: None,
                reset_period: None,
                enabled: true,
                created_at: crate::store::now_rfc3339(),
                updated_at: crate::store::now_rfc3339(),
            };
        // A hand-added provider that declared prices, one that declared none (the
        // control, and what proves the rows are keyed by the row id rather than
        // shared), and one that carries both a declaration and a catalog entry.
        for p in [
            mk("p-manual", None, Some(&declared)),
            mk("p-other", None, None),
            mk("p-kimi", Some("kimi"), Some(&declared)),
        ] {
            store.insert_provider(&p).unwrap();
        }

        let state = crate::server::GatewayState::new(store).expect("state");
        let cost_of = |provider_id: &str| {
            let costs = state
                .store
                .usage_cost_by_currency(None, Some(provider_id), None)
                .unwrap();
            costs[0].1
        };
        // 1M input tokens of m1 at each rate.
        let sample = |provider_id: &str, catalog_id: Option<&str>, model: &str| UsageSample {
            agent: "claude".into(),
            provider_id: provider_id.into(),
            catalog_id: catalog_id.map(str::to_string),
            started_unix: TEST_AT,
            model: Some(model.into()),
            usage: Usage {
                reasoning_tokens: 0,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        record_sample(&state, sample("p-manual", None, "m1"));
        record_sample(&state, sample("p-manual", None, "m2"));
        record_sample(&state, sample("p-other", None, "m1"));
        record_sample(&state, sample("p-kimi", Some("kimi"), "m1"));

        // The user's own figure for m1 (9, not the general row's 1), and the
        // Hub's general figure for the m2 they left out — a declaration prices
        // the models it names and nothing else.
        assert!(
            (cost_of("p-manual") - 11.0).abs() < 1e-9,
            "declared m1 at 9/MTok, plus the general 2/MTok for the undeclared m2"
        );
        assert!(
            (cost_of("p-other") - 1.0).abs() < 1e-9,
            "a provider that declared nothing is priced by the Hub's table"
        );
        // A declaration beats the catalog entry it was linked to: the user's
        // statement about what this provider charges is the specific one.
        assert!(
            (cost_of("p-kimi") - 9.0).abs() < 1e-9,
            "a declared price outranks the catalog entry's published one"
        );
    }

    /// Prices declared *after* the gateway is up take effect on the next
    /// `/reload` — which is the call the app makes after every provider save
    /// (`after_mutation`), and so the only path by which a price the user just
    /// typed reaches a running daemon.
    ///
    /// Without the reload, prices would land on disk and the daemon would keep
    /// costing with the table it booted from: a spend limit measured against
    /// figures the user had already corrected.
    #[test]
    fn reload_publishes_the_prices_declared_since_startup() {
        use crate::store::Protocol;

        let store = crate::store::Store::open_in_memory().expect("store");
        store
            .upsert_model_pricing(&kiwano_adapters::model_pricing::ModelPriceEntry {
                long_context: None,
                provider_id: String::new(),
                model_id: "m1".into(),
                off_peak: None,
                peak_hours: None,
                display_name: "M1".into(),
                input: "1".into(),
                output: "0".into(),
                cache_read: "0".into(),
                cache_creation: "0".into(),
                currency: "USD".into(),
            })
            .unwrap();
        store
            .insert_provider(&crate::store::Provider {
                id: "p-own".into(),
                name: "Own".into(),
                catalog_id: None,
                prices: None,
                protocol: Protocol::OpenAI,
                base_url: "https://a.example.com".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk".into()),
                model_default: None,
                billing: crate::store::Billing::Metered,
                period_limit: None,
                limit_unit: None,
                plan_query: None,
                plan_limits: None,
                timeout_secs: None,
                retries: None,
                headers: None,
                reset_period: None,
                enabled: true,
                created_at: crate::store::now_rfc3339(),
                updated_at: crate::store::now_rfc3339(),
            })
            .unwrap();

        let state = crate::server::GatewayState::new(store).expect("state");
        let sample = UsageSample {
            agent: "claude".into(),
            provider_id: "p-own".into(),
            catalog_id: None,
            started_unix: TEST_AT,
            model: Some("m1".into()),
            usage: Usage {
                reasoning_tokens: 0,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        // Before anything is declared: the Hub's general rate.
        record_sample(&state, sample.clone());

        // The user declares what this provider charges, exactly as a save does.
        let mut p = state.store.get_provider("p-own").unwrap().unwrap();
        p.prices = Some(
            kiwano_adapters::model_pricing::DeclaredPrices {
                currency: "USD".into(),
                models: vec![kiwano_adapters::model_pricing::DeclaredPrice {
                    model_id: "m1".into(),
                    input: "9".into(),
                    output: "0".into(),
                    cache_read: "0".into(),
                    cache_creation: "0".into(),
                }],
            }
            .to_json(),
        );
        state.store.update_provider(&p).unwrap();
        state.reload_routes().expect("reload");

        record_sample(
            &state,
            UsageSample {
                started_unix: TEST_AT + 1,
                ..sample
            },
        );

        // 1/MTok for the request before the reload, 9/MTok for the one after —
        // and 2 would be the sum had the reload published nothing.
        let costs = state
            .store
            .usage_cost_by_currency(None, Some("p-own"), None)
            .unwrap();
        assert!(
            (costs[0].1 - 10.0).abs() < 1e-9,
            "expected 1 before the reload and 9 after, got {}",
            costs[0].1
        );
    }

    /// A row with time-of-day tiers bills the tier the request **started** in.
    ///
    /// The same tokens, the same provider key, two instants: inside the
    /// published window the listed rate, outside it the discounted one. This is
    /// the whole money path — the sample's clock, the table's tiers, and the
    /// recorded figure — and it is what the App got wrong by ignoring
    /// `peak_hours` (it billed the peak always).
    #[test]
    fn record_sample_bills_the_tier_the_request_started_in() {
        use crate::store::Protocol;

        const PEAK_AT: i64 = 1_788_919_200; // Wed 2026-09-09 10:00 Beijing
        const OFF_AT: i64 = 1_788_930_000; // the same day's 13:00 lunch gap

        let store = crate::store::Store::open_in_memory().expect("store");
        store
            .upsert_model_pricing(&kiwano_adapters::model_pricing::ModelPriceEntry {
                long_context: None,
                provider_id: String::new(),
                model_id: "m1".into(),
                display_name: "M1".into(),
                input: "9.0".into(),
                output: "27.0".into(),
                cache_read: "0.30".into(),
                cache_creation: "0".into(),
                currency: "CNY".into(),
                off_peak: Some(kiwano_adapters::model_pricing::OffPeakRates {
                    input: "4.5".into(),
                    output: "13.5".into(),
                    cache_read: "0.15".into(),
                    cache_creation: "0".into(),
                }),
                peak_hours: Some(kiwano_adapters::model_pricing::PeakHours {
                    tz_offset: 480,
                    windows: vec![kiwano_adapters::model_pricing::PeakWindow {
                        days: ["mon", "tue", "wed", "thu", "fri"]
                            .iter()
                            .map(|d| d.to_string())
                            .collect(),
                        start: "09:00".into(),
                        end: "12:00".into(),
                    }],
                }),
            })
            .unwrap();
        // Two providers, so the two samples are two figures rather than one sum.
        for id in ["p-peak", "p-off"] {
            store
                .insert_provider(&crate::store::Provider {
                    id: id.into(),
                    name: id.into(),
                    catalog_id: None,
                    // No declared prices: this fixture is priced by the Hub's table.
                    prices: None,
                    protocol: Protocol::Anthropic,
                    base_url: "https://a.example.com".into(),
                    api_path: None,
                    endpoints: Vec::new(),
                    api_key: Some("sk".into()),
                    model_default: None,
                    billing: crate::store::Billing::Metered,
                    period_limit: None,
                    limit_unit: None,
                    plan_query: None,
                    plan_limits: None,
                    timeout_secs: None,
                    retries: None,
                    headers: None,
                    reset_period: None,
                    enabled: true,
                    created_at: crate::store::now_rfc3339(),
                    updated_at: crate::store::now_rfc3339(),
                })
                .unwrap();
        }
        let state = crate::server::GatewayState::new(store).expect("state");

        let sample = |provider_id: &str, started_unix: i64| UsageSample {
            agent: "claude".into(),
            provider_id: provider_id.into(),
            catalog_id: None,
            started_unix,
            model: Some("m1".into()),
            usage: Usage {
                reasoning_tokens: 0,
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        record_sample(&state, sample("p-peak", PEAK_AT));
        record_sample(&state, sample("p-off", OFF_AT));

        let cost_of = |provider_id: &str| {
            state
                .store
                .usage_cost_by_currency(None, Some(provider_id), None)
                .unwrap()[0]
                .1
        };
        assert!(
            (cost_of("p-peak") - 9.0).abs() < 1e-9,
            "1M at the listed (peak) rate: {}",
            cost_of("p-peak")
        );
        assert!(
            (cost_of("p-off") - 4.5).abs() < 1e-9,
            "1M at the off-peak rate: {}",
            cost_of("p-off")
        );
    }

    /// The band is what a *real request* is billed at, not only what the cost
    /// function says in isolation: over the threshold the recorded cost is the
    /// block's rate, under it the listed one. This is the path a forwarded
    /// request takes — `record_sample` → `compute_sample_cost` →
    /// `compute_cost_pair` — and the only one that ends in `usage.cost`.
    #[test]
    fn record_sample_bills_the_band_a_long_request_falls_into() {
        use crate::store::Protocol;
        use kiwano_adapters::model_pricing::{LongContextRates, ModelPriceEntry};

        let store = crate::store::Store::open_in_memory().expect("store");
        // MiniMax M3's published shape: ¥2.10/¥8.40, and ¥4.20/¥16.80 above 512k.
        store
            .upsert_model_pricing(&ModelPriceEntry {
                provider_id: String::new(),
                model_id: "m1".into(),
                display_name: "M1".into(),
                input: "2.10".into(),
                output: "8.40".into(),
                cache_read: "0.42".into(),
                cache_creation: "0".into(),
                currency: "CNY".into(),
                off_peak: None,
                peak_hours: None,
                long_context: Some(LongContextRates {
                    over: 512_000,
                    input: "4.20".into(),
                    output: "16.80".into(),
                    cache_read: "0.84".into(),
                    cache_creation: "0".into(),
                }),
            })
            .unwrap();
        // One provider per size, so the two samples stay two figures.
        for id in ["p-long", "p-short"] {
            store
                .insert_provider(&crate::store::Provider {
                    id: id.into(),
                    name: id.into(),
                    catalog_id: None,
                    // No declared prices: this fixture is priced by the Hub's table.
                    prices: None,
                    protocol: Protocol::Anthropic,
                    base_url: "https://a.example.com".into(),
                    api_path: None,
                    endpoints: Vec::new(),
                    api_key: Some("sk".into()),
                    model_default: None,
                    billing: crate::store::Billing::Metered,
                    period_limit: None,
                    limit_unit: None,
                    plan_query: None,
                    plan_limits: None,
                    timeout_secs: None,
                    retries: None,
                    headers: None,
                    reset_period: None,
                    enabled: true,
                    created_at: crate::store::now_rfc3339(),
                    updated_at: crate::store::now_rfc3339(),
                })
                .unwrap();
        }
        let state = crate::server::GatewayState::new(store).expect("state");

        let sample = |provider_id: &str, input_tokens: i64| UsageSample {
            agent: "claude".into(),
            provider_id: provider_id.into(),
            catalog_id: None,
            started_unix: 1_788_919_200,
            model: Some("m1".into()),
            usage: Usage {
                input_tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                reasoning_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            usage_missing: false,
            log: None,
        };
        record_sample(&state, sample("p-long", 600_000));
        record_sample(&state, sample("p-short", 100_000));

        let cost_of = |provider_id: &str| {
            state
                .store
                .usage_cost_by_currency(None, Some(provider_id), None)
                .unwrap()[0]
                .1
        };
        assert!(
            (cost_of("p-long") - 4.20 * 0.6).abs() < 1e-9,
            "600k at the block's rate: {}",
            cost_of("p-long")
        );
        assert!(
            (cost_of("p-short") - 2.10 * 0.1).abs() < 1e-9,
            "100k at the listed rate: {}",
            cost_of("p-short")
        );
    }
}
