//! The Usage cell: the period's totals, its cost in the provider's own
//! currency, the quota ring when the row has a limit to ring against, and the
//! sparkline when it does not.

use super::money::provider_cost;
use super::types::{QuotaVm, UsageVm};
use crate::vm::Aux;
use kiwanod::store::{Billing, Provider, Store, UsageTotals};

/// Usage cell for one provider.
pub(crate) fn usage_vm(
    store: &Store,
    aux: &Aux,
    p: &Provider,
    totals: Option<&UsageTotals>,
    since7: &str,
) -> Option<UsageVm> {
    let t = totals?;
    // The cost stays in the currency this provider's usage was priced in: it
    // is read next to that provider's own limits, and converting it into the
    // user's display currency made the two disagree. Rolling several
    // providers into one number is the Dashboard's job, and converting there
    // is what the display currency is for.
    let cost_buckets = store
        .usage_cost_by_currency(None, Some(&p.id), Some(since7))
        .unwrap_or_default();
    let (cost, cost_currency) = provider_cost(&cost_buckets);
    let quota = match (p.billing, p.limit_unit.as_deref()) {
        // A subscription's period limit and a metered provider's spending cap
        // are the same arithmetic: this period's usage against a number in the
        // provider's own unit. Building it only for Subscription left the
        // pay-as-you-go branches above unreachable — the Apps list drew no ring
        // and its tooltip said "no limit set" while a limit sat in the row.
        // Unlimited has nothing to measure, so it stays None.
        (Billing::Subscription | Billing::Metered, unit) => p.period_limit.map(|limit| {
            let (used, unit) = match unit {
                Some("wan_tokens") => (
                    (t.input_tokens
                        + t.output_tokens
                        + t.cache_read_tokens
                        + t.cache_creation_tokens) as f64
                        / 10_000.0,
                    "wan_tokens",
                ),
                // A currency limit rings against the period's cost as recorded:
                // the limit is denominated in the provider's own currency (the
                // price table's), so no rate is involved. Converting would make
                // the threshold move with the exchange rate.
                Some(u) if u.len() == 3 => (cost.unwrap_or(0.0), u),
                _ => (t.requests as f64, "requests"),
            };
            QuotaVm {
                used: (used * 100.0).round() / 100.0,
                limit,
                unit: unit.to_string(),
                resets_at: None, // reset-cycle tracking lands with the quota strategy (P2)
            }
        }),
        _ => None,
    };
    let spark = match quota {
        None => normalize_spark(
            &aux.provider_daily(&p.id, since7)
                .into_iter()
                .map(|(_, v)| v)
                .collect::<Vec<_>>(),
        ),
        Some(_) => None,
    };
    Some(UsageVm {
        requests: t.requests,
        input_tokens: t.input_tokens,
        cache_read_tokens: t.cache_read_tokens,
        cache_creation_tokens: t.cache_creation_tokens,
        output_tokens: t.output_tokens,
        cost: cost.map(|c| (c * 1e6).round() / 1e6),
        cost_currency,
        latency_ms: aux.avg_latency(Some(&p.id), None, Some(since7), None),
        quota,
        spark,
    })
}

// ── Sparkline normalization: y coords in the 80×14 viewBox, 1..13 ──

fn normalize_spark(values: &[i64]) -> Option<Vec<f64>> {
    let max = values.iter().max().copied()?;
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .map(|v| {
                if max == 0 {
                    12.0
                } else {
                    (12.0 - 10.0 * (*v as f64 / max as f64)).clamp(2.0, 12.0)
                }
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{provider, store};
    use crate::vm::time::{local_day_start, rfc3339, unix_now};

    #[test]
    fn a_spending_limit_shows_up_for_every_billing_that_has_one() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let since7 = local_day_start(0, now - 6 * 86_400);

        // A metered provider with a 50 CNY cap and 30 CNY of cost this period.
        let mut metered = provider("payg-1", "Payg", Billing::Metered);
        metered.period_limit = Some(50.0);
        metered.limit_unit = Some("CNY".into());
        s.insert_provider(&metered).unwrap();
        s.record_usage(&kiwanod::store::UsageRecord {
            ts: rfc3339(now - 60),
            agent: "claude".into(),
            provider_id: "payg-1".into(),
            model: Some("demo-model".into()),
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(214),
            status: "ok".into(),
            cost: Some(30.0),
            cost_currency: Some("CNY".into()),
            cost_off_peak: None,
        })
        .unwrap();
        let totals = s.usage_totals(None, Some("payg-1"), Some(&since7)).unwrap();
        let payg = usage_vm(&s, &aux, &metered, Some(&totals), &since7).unwrap();
        let q = payg.quota.expect("a payg cap is a quota too");
        assert_eq!((q.used, q.limit), (30.0, 50.0));
        assert_eq!(q.unit, "CNY", "denominated in the provider's own currency");

        // Unlimited has nothing to measure, limit or no limit.
        let mut unl = provider("unl-1", "Local", Billing::Unlimited);
        unl.period_limit = Some(50.0);
        s.insert_provider(&unl).unwrap();
        let totals = s.usage_totals(None, Some("unl-1"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &unl, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());

        // A metered provider with no cap has nothing to ring against, so the
        // card falls back to the usage trend.
        let bare = provider("payg-2", "Bare", Billing::Metered);
        s.insert_provider(&bare).unwrap();
        let totals = s.usage_totals(None, Some("payg-2"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &bare, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());
    }
}
