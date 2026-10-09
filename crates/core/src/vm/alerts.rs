//! Usage alerts — produced by the daemon.
//!
//! Moved to `kiwanod::api::alerts` (`migrate.local.md` §10.21): it reads nothing
//! but the daemon's tables. One classification changed with it — the dedup
//! markers (`alert_sent:*`) were the app's, because the app was what produced the
//! alerts; the daemon produces them now, so it remembers them (§9.1 corrected).
//!
//! The wrapper keeps the signature every caller knows. The tests are the alert
//! rules themselves (a forecast before the limit, an anomaly against the same
//! hour's baseline, a ceiling that the gateway enforces silently) and they run
//! against the daemon's implementation.

use kiwanod::api::alerts as daemon;
use kiwanod::store::Store;

pub use kiwano_api::dashboard::UsageAlertVm;

pub fn check_usage_alerts(
    store: &Store,
    _aux: &crate::vm::Aux,
    mark: bool,
) -> Result<Vec<UsageAlertVm>, String> {
    daemon::check_usage_alerts(store, mark).map_err(|e| e.to_string())
}

/// The same check against a **fixed** clock, which is what the tests below need:
/// a forecast or an anomaly is about a window, and "now" has to be a value the
/// test chose.
#[cfg(test)]
fn check_usage_alerts_at(
    store: &Store,
    _aux: &crate::vm::Aux,
    mark: bool,
    now: i64,
) -> Result<Vec<UsageAlertVm>, String> {
    daemon::check_usage_alerts_at(store, mark, now).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::settings::update_settings;
    use crate::vm::test_support::{no_vars, provider, store, store_and_aux_on_one_file, usage_row};
    use crate::vm::time::{rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::Billing;

    #[test]
    fn cost_alert_fires_once_per_period_and_respects_toggle() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("kimi-1", "Kimi", Billing::Subscription);
        p.period_limit = Some(100.0);
        p.limit_unit = Some("requests".into());
        p.reset_period = Some("monthly".into());
        s.insert_provider(&p).unwrap();

        // 40/100 → below threshold
        for _ in 0..40 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // 100/100 → threshold hit
        for _ in 0..60 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].provider_id, "kimi-1");
        assert_eq!(alerts[0].unit, "requests");

        // same-period dedup: the second check returns nothing
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // toggle off → silent
        let patch = serde_json::json!({ "cost_alert": false });
        update_settings(&s, &aux, &patch, &no_vars()).unwrap();
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());
    }

    /// A plan ceiling is the other way a provider goes out of service, and it
    /// used to do so silently: the gateway took it out of the routes, the UI had
    /// the branch and the strings for the notice, and nothing ever produced one.
    #[test]
    fn a_plan_window_ceiling_notifies_once_per_window() {
        use kiwanod::plan_quota::{cache_write, PlanQuotaReport, PlanTierVm};
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("glm-1", "GLM", Billing::Subscription);
        p.plan_limits = Some(serde_json::json!({ "five_hour": 90.0 }).to_string());
        s.insert_provider(&p).unwrap();

        let report = |util: f64, resets: &str| PlanQuotaReport {
            provider_id: "glm-1".into(),
            template: "zhipu".into(),
            success: true,
            error: None,
            note: None,
            tiers: vec![
                PlanTierVm {
                    name: "five_hour".into(),
                    utilization: util,
                    resets_at: Some(resets.into()),
                    used: None,
                    limit: None,
                    unit: None,
                },
                // Present and quiet: only the window that is over should speak.
                PlanTierVm {
                    name: "weekly_limit".into(),
                    utilization: 10.0,
                    resets_at: None,
                    used: None,
                    limit: None,
                    unit: None,
                },
            ],
            queried_at: 0,
            cached: false,
        };

        // Under the ceiling: nothing to say.
        cache_write(&s, "glm-1", &report(80.0, "2026-09-14T10:00:00Z"));
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // Over it: one notice, carrying the window's own percentage against the
        // ceiling the user set.
        cache_write(&s, "glm-1", &report(95.0, "2026-09-14T10:00:00Z"));
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].unit, "plan_pct");
        assert_eq!(alerts[0].used, 95.0);
        assert_eq!(alerts[0].limit, 90.0);

        // Same window, same news: silent, the way the money cap is.
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // The window rolls over and is over its ceiling again. The reset time is
        // the identity, so this is what re-arms the dedup rather than letting one
        // hit go quiet for good.
        cache_write(&s, "glm-1", &report(97.0, "2026-09-14T15:00:00Z"));
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].used, 97.0);
    }

    /// The dedup key is shared with the desktop notification, so a read-only
    /// caller must be able to ask without consuming the alert the app is about
    /// to raise. A cron poll that silenced the user's notification would be a
    /// bug nobody would connect to the poll.
    #[test]
    fn cost_alert_can_be_read_without_consuming_the_dedup() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("kimi-1", "Kimi", Billing::Subscription);
        p.period_limit = Some(10.0);
        p.limit_unit = Some("requests".into());
        p.reset_period = Some("monthly".into());
        s.insert_provider(&p).unwrap();
        for _ in 0..10 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }

        // Read-only: the alert is reported every time, and the dedup is
        // untouched — which is what `--mark-notified` is for.
        let first = check_usage_alerts(&s, &aux, false).unwrap();
        let second = check_usage_alerts(&s, &aux, false).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1, "a read must not consume the alert");

        // The app's marking call still works and still dedups.
        assert_eq!(check_usage_alerts(&s, &aux, true).unwrap().len(), 1);
        assert!(
            check_usage_alerts(&s, &aux, true).unwrap().is_empty(),
            "marking is what suppresses the repeat"
        );
        assert!(
            check_usage_alerts(&s, &aux, false).unwrap().is_empty(),
            "…for read-only callers too, once it has been marked"
        );
    }

    #[test]
    fn cost_alert_skips_unlimited_rows_and_converts_currency_limits() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        let mut payg = provider("ds-1", "DeepSeek", Billing::Metered);
        payg.period_limit = Some(5.0); // NULL unit normalizes to requests
        s.insert_provider(&payg).unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("CNY".into()); // currency limit: compares spent cost
        s.insert_provider(&cny).unwrap();

        // 6 rows each: payg hits the request threshold; the CNY limit compares
        // the period's spent cost (¥10/row → ¥60) against ¥50.
        for _ in 0..6 {
            s.record_usage(&usage_row("ds-1")).unwrap();
            let mut row = usage_row("glm-1");
            row.cost = Some(10.0);
            row.cost_currency = Some("CNY".into());
            s.record_usage(&row).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].provider_id, "ds-1");
        assert_eq!(alerts[1].provider_id, "glm-1");
        assert_eq!(alerts[1].unit, "CNY");
        assert!((alerts[1].used - 60.0).abs() < 1e-6);
    }

    // ── Features-panel alerts (all opt-in; each test also covers the gate) ──

    /// A request-log row at a chosen instant — the anomaly rules read "the last
    /// completed hour", so the rows' timestamps are the fixture, not now.
    fn log_row_at(
        ts: String,
        status_code: i64,
        latency_ms: Option<i64>,
    ) -> kiwanod::store::RequestLogNew {
        kiwanod::store::RequestLogNew {
            ts,
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some("claude".into()),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: None,
            status_code,
            error_kind: None,
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms,
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 0,
            response_size: 0,
            truncated: false,
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            request_notes: None,
        }
    }

    #[test]
    fn cost_forecast_fires_before_the_limit_is_hit() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("ds-1", "DeepSeek", Billing::Metered);
        p.period_limit = Some(100.0);
        p.limit_unit = Some("CNY".into());
        p.reset_period = Some("monthly".into());
        s.insert_provider(&p).unwrap();

        // "Now" pinned to 30% into the current month, so the projection is
        // exact regardless of the day the suite runs. The usage rows carry the
        // real clock — they land in the current month either way, which is all
        // the usage side of the check asks.
        let real_now = unix_now();
        let (start, end) = kiwanod::limits::period_span_secs(real_now, Some("monthly"), 0).unwrap();
        let now = start + ((end - start) as f64 * 0.3) as i64;
        // ¥40 spent at 30% elapsed → projection ¥133 against a ¥100 limit.
        for _ in 0..4 {
            let mut row = usage_row("ds-1");
            row.cost = Some(10.0);
            row.cost_currency = Some("CNY".into());
            s.record_usage(&row).unwrap();
        }

        // Flag off (the default): silent, and under the limit so the plain
        // cost alert has nothing to say either.
        assert!(check_usage_alerts_at(&s, &aux, true, now)
            .unwrap()
            .is_empty());

        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "feat_cost_forecast": true }),
            &no_vars(),
        )
        .unwrap();
        let alerts = check_usage_alerts_at(&s, &aux, true, now).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "cost_forecast");
        assert!(alerts[0].message.contains("on pace"));
        assert!(alerts[0].message.contains("CNY"));
        assert!(
            check_usage_alerts_at(&s, &aux, true, now)
                .unwrap()
                .is_empty(),
            "deduped for the rest of the period"
        );
    }

    #[test]
    fn anomaly_alert_on_an_error_spike() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let real_now = unix_now();
        let hour_start = real_now - real_now.rem_euclid(3600);
        let now = hour_start + 1800; // mid-hour: "the last completed hour" is fixed
                                     // A quiet baseline week: a trickle of successes, no errors.
        for i in 0..20 {
            s.insert_request_log(&log_row_at(
                rfc3339(hour_start - 3600 - (i + 1) * 7200),
                200,
                Some(100),
            ))
            .unwrap();
        }
        // The last completed hour: six failures out of eight — the shape of the
        // 09-09 protocol_mismatch storm the rule exists to catch.
        for i in 0..6 {
            s.insert_request_log(&log_row_at(
                rfc3339(hour_start - 3600 + i * 60),
                502,
                Some(100),
            ))
            .unwrap();
        }
        for i in 0..2 {
            s.insert_request_log(&log_row_at(
                rfc3339(hour_start - 3600 + 300 + i * 60),
                200,
                Some(100),
            ))
            .unwrap();
        }

        assert!(check_usage_alerts_at(&s, &aux, true, now)
            .unwrap()
            .is_empty());
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "feat_anomaly_alerts": true }),
            &no_vars(),
        )
        .unwrap();
        let alerts = check_usage_alerts_at(&s, &aux, true, now).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "anomaly");
        assert!(alerts[0].message.contains("Error spike"));
        assert!(
            check_usage_alerts_at(&s, &aux, true, now)
                .unwrap()
                .is_empty(),
            "once per day per rule"
        );
    }

    #[test]
    fn anomaly_alert_on_a_latency_outlier() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let real_now = unix_now();
        let hour_start = real_now - real_now.rem_euclid(3600);
        let now = hour_start + 1800;
        for i in 0..10 {
            s.insert_request_log(&log_row_at(
                rfc3339(hour_start - 7200 - i * 3600),
                200,
                Some(100),
            ))
            .unwrap();
        }
        // Ten requests at ten times the baseline's latency.
        for i in 0..10 {
            s.insert_request_log(&log_row_at(
                rfc3339(hour_start - 3600 + i * 60),
                200,
                Some(1000),
            ))
            .unwrap();
        }
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "feat_anomaly_alerts": true }),
            &no_vars(),
        )
        .unwrap();
        let alerts = check_usage_alerts_at(&s, &aux, true, now).unwrap();
        assert_eq!(alerts.len(), 1);
        assert!(alerts[0].message.contains("Latency outlier"));
    }

    #[test]
    fn agent_limit_alert_covers_the_gateways_silent_refusal() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered))
            .unwrap();
        s.replace_agent_limits(
            "claude",
            &[kiwanod::store::AgentLimit {
                agent: "claude".into(),
                period: "day".into(),
                period_limit: 5.0,
                limit_unit: None, // requests
                created_at: String::new(),
                updated_at: String::new(),
            }],
        )
        .unwrap();
        for _ in 0..6 {
            s.record_usage(&usage_row("p1")).unwrap();
        }

        // The gateway is already refusing this agent; with the flag off the
        // user is never told.
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "feat_agent_limit_alerts": true }),
            &no_vars(),
        )
        .unwrap();
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].kind, "agent_limit");
        assert_eq!(alerts[0].provider_name, "claude");
        assert!(alerts[0].message.contains("budget"));
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());
    }

    #[test]
    fn currency_limits_convert_with_the_hub_rate_table() {
        let (_dir, s, aux) = store_and_aux_on_one_file();
        // A Hub price table whose CNY rate is nothing like the real one (the Hub
        // quotes several CNY per USD).
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 6.0 },
            "models": []
        })
        .to_string();
        // Through the store: the cache is the daemon's table now.
        s.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("CNY".into());
        s.insert_provider(&cny).unwrap();

        // Dollars, against a limit denominated in yuan — which happens whenever
        // a provider serves a model it does not price itself and the general row
        // is in someone else's currency.
        let mut row = usage_row("glm-1");
        row.cost = Some(10.0);
        row.cost_currency = Some("USD".into());
        s.record_usage(&row).unwrap();

        // 10 USD is 60 CNY at the cached rate, over the 50 CNY limit, so the
        // alert fires. Adding the buckets raw — the old behaviour — read 10 and
        // stayed silent, and so did converting at any rate at or below 5, which
        // is why the rate is quoted high enough to decide the outcome.
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert!(
            alerts.iter().any(|a| a.provider_id == "glm-1"),
            "10 USD at 6.0 CNY/USD is 60 CNY, over the 50 CNY limit"
        );
    }
}
