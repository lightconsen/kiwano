//! The Usage cell — implemented in the daemon's crate.
//!
//! Same reasoning as the health cell beside it: the rule follows the shape, and
//! the shape is the daemon's now (`migrate.local.md` §10.7, §10.21). The tests
//! below pin the behaviour through that implementation.

#[cfg(test)]
pub(crate) use kiwanod::api::providers_view::usage_vm;

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{provider, store};
    use crate::vm::time::{local_day_start, rfc3339, unix_now};
    use kiwanod::store::Billing;

    #[test]
    fn a_spending_limit_shows_up_for_every_billing_that_has_one() {
        let s = store();
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
        let payg = usage_vm(&s, &metered, Some(&totals), &since7).unwrap();
        let q = payg.quota.expect("a payg cap is a quota too");
        assert_eq!((q.used, q.limit), (30.0, 50.0));
        assert_eq!(q.unit, "CNY", "denominated in the provider's own currency");

        // Unlimited has nothing to measure, limit or no limit.
        let mut unl = provider("unl-1", "Local", Billing::Unlimited);
        unl.period_limit = Some(50.0);
        s.insert_provider(&unl).unwrap();
        let totals = s.usage_totals(None, Some("unl-1"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &unl, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());

        // A metered provider with no cap has nothing to ring against, so the
        // card falls back to the usage trend.
        let bare = provider("payg-2", "Bare", Billing::Metered);
        s.insert_provider(&bare).unwrap();
        let totals = s.usage_totals(None, Some("payg-2"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &bare, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());
    }
}
