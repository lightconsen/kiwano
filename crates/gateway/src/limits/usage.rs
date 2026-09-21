//! Reading an amount limit back out of the store: one provider's
//! `period_limit`, or one agent's ceiling measured across every provider it
//! used.
//!
//! Both answer in `PeriodLimit`, over the boundaries `period` draws. The unit
//! is the limit's own — `requests`, `wan_tokens`, or a currency code the
//! provider bills in — and money is converted per bucket before it is summed,
//! because a provider's usage can span more than one currency.

use chrono::Utc;

use crate::limits::period::{period_start, PeriodLimit};
use crate::store::{Provider, Store};

/// Read one provider's period limit and how much of it is spent. `None` when
/// there is no limit worth measuring (absent, or zero/negative).
pub fn period_limit_usage(
    store: &Store,
    p: &Provider,
) -> crate::error::Result<Option<PeriodLimit>> {
    let Some(limit) = p.period_limit.filter(|l| *l > 0.0) else {
        return Ok(None);
    };
    // A NULL unit normalizes to requests — the same source the ring percentage
    // reads, so v1 rows keep meaning what they always did.
    let unit = match p.limit_unit.as_deref() {
        Some("wan_tokens") => "wan_tokens",
        // A currency limit compares the period's cost, denominated in the
        // currency the provider bills in. Most of the time that is the only
        // currency in the sum — a provider's own models are priced in it — but
        // the cost of a model it does not price comes from the general row,
        // which may be in another one. So the buckets are converted rather than
        // added (see `used` below); what stays true is that the *limit* is
        // never converted, so the ceiling the user typed does not move.
        Some(u) if u.len() == 3 => u,
        _ => "requests",
    };
    let (since, period_key) = period_start(
        Utc::now().timestamp(),
        p.reset_period.as_deref(),
        store.ui_tz_offset_minutes(),
    );
    let used = match unit {
        "wan_tokens" => {
            let t = store.usage_totals_for_provider(&p.id, since.as_deref())?;
            (t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_creation_tokens)
                as f64
                / 10_000.0
        }
        u if u.len() == 3 => {
            // Converted before it is added, because a provider's usage can span
            // more than one currency: its own models are priced in its own
            // currency, but a model it does not price is billed from the general
            // row, which may be denominated in another. Summing the buckets raw
            // added USD to CNY at 1:1 — a `¥50` limit quietly became a ceiling
            // of nothing in particular, and it did so while both the app and the
            // gateway agreed on the wrong number.
            let buckets = store.usage_cost_by_currency(None, Some(&p.id), since.as_deref())?;
            kiwano_adapters::model_pricing::convert_cost_buckets(
                &buckets,
                u,
                &store.hub_exchange_rates(),
            )
        }
        _ => {
            store
                .usage_totals_for_provider(&p.id, since.as_deref())?
                .requests as f64
        }
    };
    Ok(Some(PeriodLimit {
        used,
        limit,
        unit: unit.to_string(),
        period_key,
    }))
}

/// Read one agent's own ceiling and how much of it is spent. `None` when there is
/// no limit worth measuring (no row, or zero/negative).
///
/// The same three units a provider's limit takes, measured against the same
/// period boundaries — but scoped to the agent, across every provider it used.
/// That is the whole point of a limit here: a route can span providers, and "this
/// agent may spend ¥50 a day" is a statement about the agent, not about any one of
/// them.
pub fn agent_limit_usage(
    store: &Store,
    limit: &crate::store::AgentLimit,
) -> crate::error::Result<Option<PeriodLimit>> {
    // `is_finite` as well: a NaN would slip through every comparison and
    // become a ceiling that is never reached.
    if !limit.period_limit.is_finite() || limit.period_limit <= 0.0 {
        return Ok(None);
    }
    let unit = match limit.limit_unit.as_deref() {
        Some("wan_tokens") => "wan_tokens",
        Some(u) if u.len() == 3 => u,
        _ => "requests",
    };
    // `all` is the stored spelling of "no reset"; `period_start` says that with
    // `None`, and treats anything it does not recognise as monthly — so the
    // mapping has to happen here rather than by passing the string through.
    let reset = (limit.period != "all").then_some(limit.period.as_str());
    let (since, period_key) =
        period_start(Utc::now().timestamp(), reset, store.ui_tz_offset_minutes());
    let used = match unit {
        "wan_tokens" => {
            let t = store.usage_totals(Some(&limit.agent), None, since.as_deref())?;
            (t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_creation_tokens)
                as f64
                / 10_000.0
        }
        // Converted before it is added, for the reason the provider limits give:
        // the agent's traffic spans providers, and those bill in different
        // currencies. The ceiling itself is never converted.
        u if u.len() == 3 => {
            let buckets =
                store.usage_cost_by_currency(Some(&limit.agent), None, since.as_deref())?;
            kiwano_adapters::model_pricing::convert_cost_buckets(
                &buckets,
                u,
                &store.hub_exchange_rates(),
            )
        }
        _ => {
            store
                .usage_totals(Some(&limit.agent), None, since.as_deref())?
                .requests as f64
        }
    };
    Ok(Some(PeriodLimit {
        used,
        limit: limit.period_limit,
        unit: unit.to_string(),
        period_key,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::state::evaluate;
    use crate::limits::test_support::{
        agent_limit, limited_provider, record_cost, set_limits, store_with_hub_rates, test_provider,
    };

    /// A provider's spend can span currencies: its own models are priced in its
    /// own currency, but a model it does not price is billed from the general
    /// row, which may be denominated in another. The limit is the user's, in the
    /// provider's currency, so each bucket is converted on the way in.
    ///
    /// The same 10 USD + 5 CNY of usage is run under two Hub rates, giving 25
    /// and then 65 — a raw sum would read 15 in both cases, which is what makes
    /// the second assertion the one that proves a rate was applied at all.
    #[test]
    fn a_money_limit_converts_each_currency_before_it_sums_them() {
        let (_low_dir, low) = store_with_hub_rates(r#"{"USD":1.0,"CNY":2.0}"#);
        let (_high_dir, high) = store_with_hub_rates(r#"{"USD":1.0,"CNY":6.0}"#);
        for (store, expected) in [(&low, 25.0), (&high, 65.0)] {
            // The limit itself is never converted: ¥50 stays ¥50.
            let p = limited_provider(store, "ds-1", 50.0);
            record_cost(store, "ds-1", 10.0, "USD");
            record_cost(store, "ds-1", 5.0, "CNY");
            let used = period_limit_usage(store, &p).unwrap().unwrap().used;
            assert!(
                (used - expected).abs() < 1e-6,
                "10 USD + 5 CNY should measure {expected}, got {used}"
            );
        }
    }

    /// The ordinary case needs no rate: a provider whose usage is all in its own
    /// currency converts to itself, which is also all a never-synced install
    /// with no rates cached can measure.
    #[test]
    fn a_money_limit_in_one_currency_needs_no_rates() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("kiwano.db")).unwrap();
        let p = limited_provider(&store, "ds-1", 50.0);
        record_cost(&store, "ds-1", 5.0, "CNY");
        let used = period_limit_usage(&store, &p).unwrap().unwrap().used;
        assert!((used - 5.0).abs() < 1e-6, "got {used}");
    }

    /// A cost row attributed to a named agent — `record_cost` is always claude,
    /// and the point of an agent limit is that it is one agent's number.
    fn record_agent_cost(store: &Store, agent: &str, provider_id: &str, cost: f64, currency: &str) {
        use crate::store::UsageRecord;
        store
            .record_usage(&UsageRecord {
                ts: crate::store::now_rfc3339(),
                agent: agent.into(),
                provider_id: provider_id.into(),
                model: None,
                input_tokens: 0,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: Some(cost),
                cost_currency: Some(currency.into()),
                cost_off_peak: None,
            })
            .unwrap();
    }

    /// An agent's ceiling belongs to the agent, not to any one provider: a route
    /// spans providers, so the number is the sum across them — and another
    /// agent's traffic is not part of it. That second half is the whole reason
    /// this is not just the provider limit again.
    #[test]
    fn an_agent_limit_sums_its_providers_and_ignores_other_agents() {
        let (_dir, store) = store_with_hub_rates(r#"{"USD":1.0,"CNY":2.0}"#);
        for id in ["ds-1", "kimi-1"] {
            store.insert_provider(&test_provider(id)).unwrap();
        }
        record_agent_cost(&store, "claude", "ds-1", 10.0, "USD");
        record_agent_cost(&store, "claude", "kimi-1", 5.0, "CNY");
        record_agent_cost(&store, "codex", "ds-1", 99.0, "USD");

        let limit = agent_limit("claude", 50.0, Some("CNY"), "monthly");
        set_limits(&store, "claude", vec![limit.clone()]);

        // 10 USD at the cached 2 CNY/USD, plus 5 CNY — converted before it sums,
        // the same way a provider's money limit does it, and the codex row is
        // simply not this agent's.
        let pl = agent_limit_usage(&store, &limit).unwrap().unwrap();
        assert!((pl.used - 25.0).abs() < 1e-6, "got {}", pl.used);
        assert_eq!(pl.unit, "CNY");

        // The measurement is not the gate; `evaluate` is what turns it into one.
        assert!(
            evaluate(&store).agent_blocked("claude").is_none(),
            "under the ceiling"
        );
        record_agent_cost(&store, "claude", "ds-1", 20.0, "USD");
        let state = evaluate(&store);
        let reason = state
            .agent_blocked("claude")
            .expect("40 USD is past a 50 CNY ceiling");
        assert!(reason.describe().contains("CNY"), "{}", reason.describe());
        assert!(
            state.agent_blocked("codex").is_none(),
            "the other agent's spend is not this agent's"
        );
    }

    /// No row is no ceiling; a zero limit is not a ceiling of zero. Same rule a
    /// provider's `period_limit` follows, so "0" cannot mean "refuse everything".
    #[test]
    fn an_absent_or_zero_agent_limit_measures_nothing() {
        let (_dir, store) = store_with_hub_rates(r#"{"USD":1.0}"#);
        assert!(store.agent_limits_for("claude").unwrap().is_empty());

        let zero = agent_limit("claude", 0.0, None, "day");
        assert!(agent_limit_usage(&store, &zero).unwrap().is_none());

        // A stored zero window is still no ceiling, and does not block.
        set_limits(&store, "claude", vec![zero]);
        assert!(evaluate(&store).agent_blocked("claude").is_none());
    }
}
