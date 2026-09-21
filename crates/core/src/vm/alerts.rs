//! Usage and cost alerts — the per-period limit, the forecast, the anomaly rules,
//! and the per-agent limit notices. Every source dedups through `first_notice`,
//! so a widget that polls keeps its meaning.

use crate::vm::settings::{ui_settings, SettingsVm};
use crate::vm::time::{rfc3339, unix_now};
use crate::vm::{e2s, Aux};
use kiwanod::store::{Billing, Provider, Store};
use serde::Serialize;

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

#[derive(Serialize)]
pub struct UsageAlertVm {
    pub provider_id: String,
    pub provider_name: String,
    pub used: f64,
    pub limit: f64,
    /// requests | wan_tokens | 3-letter ISO currency code
    pub unit: String,
    /// Which check raised this: `provider_limit` | `plan_window` |
    /// `cost_forecast` | `anomaly` | `agent_limit`. The first two are formatted
    /// by the frontend from the numbers; the rest carry their text in
    /// `message`.
    pub kind: String,
    /// Pre-built notification text for the feature alerts (English, the same
    /// convention as insights findings); empty for the two legacy kinds.
    pub message: String,
}

/// Alerts for providers that have spent their allowance, which the frontend
/// turns into system notifications.
///
/// Two ways to be over, and both are raised here: an **amount** limit
/// (`period_limit` — requests, tokens, or the provider's own currency) and a
/// **plan window** ceiling (`plan_limits`, measured against what the provider's
/// endpoint reports). Both are read through `kiwanod::limits`, the same code the
/// gateway enforces with, so a notice can never describe a different number from
/// the block.
///
/// Notification only — the acting half is `kiwanod::limits::evaluate`, which runs
/// whether or not the user wants to be told.
///
/// `mark` controls the once-per-identity dedup write. The app passes `true` — it
/// raises one notification per provider per reset period, and recording that is
/// the point of the key. A read-only caller passes `false`, because consuming
/// the dedup would suppress the notification the app is about to raise: a
/// `--json` poll from a script must not silently eat the user's alert.
pub fn check_usage_alerts(
    store: &Store,
    aux: &Aux,
    mark: bool,
) -> Result<Vec<UsageAlertVm>, String> {
    check_usage_alerts_at(store, aux, mark, unix_now())
}

/// What every alert source reads, resolved once per poll.
struct AlertCtx<'a> {
    store: &'a Store,
    aux: &'a Aux,
    settings: SettingsVm,
    /// Whether `first_notice` may consume the dedup. A read-only caller passes
    /// false and cannot eat the alert the app is about to raise.
    mark: bool,
    now: i64,
}

/// The per-period cost limit and the forecast that precedes it, for one provider.
///
/// The two are one function because they share `period_limit_usage` and their
/// order is the returned order: the limit notice reads first, then "on pace to
/// pass it".
fn push_amount_alerts(
    ctx: &AlertCtx,
    p: &Provider,
    out: &mut Vec<UsageAlertVm>,
) -> Result<(), String> {
    let Some(pl) = kiwanod::limits::period_limit_usage(ctx.store, p).map_err(e2s)? else {
        return Ok(());
    };
    // Notify at most once per reset period (app_settings KV dedup).
    let key = format!("alert_sent:{}", p.id);
    if ctx.settings.cost_alert
        && pl.used >= pl.limit
        && first_notice(ctx.aux, &key, &pl.period_key, ctx.mark)?
    {
        out.push(UsageAlertVm {
            provider_id: p.id.clone(),
            provider_name: p.name.clone(),
            used: (pl.used * 100.0).round() / 100.0,
            limit: pl.limit,
            unit: pl.unit.clone(),
            kind: "provider_limit".into(),
            message: String::new(),
        });
    }
    // Cost forecast (Features panel): the month's spend slope projects
    // past the limit — said *before* the limit is hit, which is the
    // whole point. Currency limits with a reset period only: requests
    // do not slope the same way, and a no-reset limit has no end to
    // project toward.
    if ctx.settings.feat_cost_forecast && pl.used < pl.limit && pl.unit.len() == 3 {
        if let Some((start, end)) = kiwanod::limits::period_span_secs(
            ctx.now,
            p.reset_period.as_deref(),
            ctx.store.ui_tz_offset_minutes(),
        ) {
            let elapsed = (ctx.now - start) as f64 / (end - start) as f64;
            // The first tenth of a period is noise; the 5% margin keeps
            // a projection that lands exactly on the limit quiet.
            if elapsed >= 0.1 {
                let projected = pl.used / elapsed;
                if projected > pl.limit * 1.05 {
                    let key = format!("alert_sent:forecast:{}", p.id);
                    if first_notice(ctx.aux, &key, &pl.period_key, ctx.mark)? {
                        out.push(UsageAlertVm {
                            provider_id: p.id.clone(),
                            provider_name: p.name.clone(),
                            used: (pl.used * 100.0).round() / 100.0,
                            limit: pl.limit,
                            unit: pl.unit.clone(),
                            kind: "cost_forecast".into(),
                            message: format!(
                                "{}: on pace for {:.0} {} this period — past the {:.0} limit",
                                p.name, projected, pl.unit, pl.limit
                            ),
                        });
                    }
                }
            }
        }
    }
    Ok(())
}

/// The percent half — a plan window whose live utilization has reached the
/// ceiling the user set for it. These are the same numbers the gateway blocks on
/// (`limits::evaluate`), read through the same cached report and the same
/// `window_over`, so the notice and the block agree. Until this existed nothing
/// raised it: the UI had the branch and the strings, and no producer, so a plan
/// ceiling took the provider out of service silently.
fn push_plan_window_alert(
    ctx: &AlertCtx,
    p: &Provider,
    out: &mut Vec<UsageAlertVm>,
) -> Result<(), String> {
    if !ctx.settings.cost_alert || p.billing != Billing::Subscription {
        return Ok(());
    }
    // Bound to locals rather than chained: the hit borrows the report,
    // so the report has to outlive it in this scope.
    let limits = kiwanod::limits::PlanLimits::parse(p.plan_limits.as_deref());
    let report = kiwanod::plan_quota::cached_report(ctx.store, &p.id);
    let hit = match (limits.as_ref(), report.as_ref()) {
        (Some(limits), Some(report)) => kiwanod::limits::window_over(report, limits),
        _ => None,
    };
    let Some(hit) = hit else {
        return Ok(());
    };
    // One notice per window, and a fresh one once it rolls over.
    let identity = match hit.resets_at {
        Some(at) => format!("{}@{at}", hit.window),
        // The endpoint does not say when the window resets, so
        // re-arm daily: a window that stays over should not notify
        // again on every poll, but it must not go quiet forever.
        //
        // This reads the wall clock rather than `ctx.now`, and that looks wrong
        // next to the forecast above. It is left as it was: the tests pin `now`
        // only for the forecast and anomaly paths, so changing it here would
        // move untested behaviour, which is a separate change from this one.
        None => format!(
            "{}@{}",
            hit.window,
            kiwanod::limits::period_start(
                unix_now(),
                Some("day"),
                ctx.store.ui_tz_offset_minutes()
            )
            .1
        ),
    };
    let key = format!("alert_sent:plan:{}", p.id);
    if first_notice(ctx.aux, &key, &identity, ctx.mark)? {
        out.push(UsageAlertVm {
            provider_id: p.id.clone(),
            provider_name: p.name.clone(),
            used: (hit.util * 100.0).round() / 100.0,
            limit: hit.pct,
            unit: "plan_pct".into(),
            kind: "plan_window".into(),
            message: String::new(),
        });
    }
    Ok(())
}

/// One anomaly notice: recorded under `alert_sent:anomaly:{rule}` against the
/// local day, so each rule fires at most once a day.
fn push_anomaly(
    ctx: &AlertCtx,
    rule: &str,
    message: String,
    day_key: &str,
    out: &mut Vec<UsageAlertVm>,
) -> Result<(), String> {
    let key = format!("alert_sent:anomaly:{rule}");
    if first_notice(ctx.aux, &key, day_key, ctx.mark)? {
        out.push(UsageAlertVm {
            provider_id: String::new(),
            provider_name: String::new(),
            used: 0.0,
            limit: 0.0,
            unit: String::new(),
            kind: "anomaly".into(),
            message,
        });
    }
    Ok(())
}

/// Anomaly detection (Features panel): the last completed hour against the
/// trailing-7-day baseline. All three rules fire at most once a day each, and in
/// this order — errors, then latency, then burst.
fn push_anomaly_alerts(ctx: &AlertCtx, out: &mut Vec<UsageAlertVm>) -> Result<(), String> {
    let hour_start = ctx.now - ctx.now.rem_euclid(3600);
    let recent = ctx
        .store
        .traffic_stats(&rfc3339(hour_start - 3600), &rfc3339(hour_start))
        .map_err(e2s)?;
    let base = ctx
        .store
        .traffic_stats(&rfc3339(ctx.now - 7 * 86_400), &rfc3339(hour_start - 3600))
        .map_err(e2s)?;
    let base_hours = 167.0_f64; // 7 days minus the recent hour
    let day_key =
        kiwanod::limits::period_start(ctx.now, Some("day"), ctx.store.ui_tz_offset_minutes()).1;
    // Error-rate spike. The 5-error floor keeps a quiet machine from
    // "spiking" on a single failure; the 10% floor does the same for a
    // zero baseline.
    let recent_err_rate = recent.errors as f64 / recent.requests.max(1) as f64;
    let base_err_rate = base.errors as f64 / base.requests.max(1) as f64;
    if recent.errors >= 5 && recent_err_rate > f64::max(3.0 * base_err_rate, 0.10) {
        push_anomaly(
            ctx,
            "errors",
            format!(
                "Error spike: {} of the last hour's {} requests failed ({:.0}% — 7-day baseline {:.0}%)",
                recent.errors,
                recent.requests,
                recent_err_rate * 100.0,
                base_err_rate * 100.0
            ),
            &day_key,
            out,
        )?;
    }
    // Latency outlier: the hour's mean is 3× the baseline's. Needs traffic
    // on both sides — a baseline of no measurements is not a baseline.
    if let (Some(recent_avg), Some(base_avg)) = (recent.avg_latency_ms, base.avg_latency_ms) {
        if recent.requests >= 10 && base_avg > 0.0 && recent_avg > 3.0 * base_avg {
            push_anomaly(
                ctx,
                "latency",
                format!(
                    "Latency outlier: {:.1}s average in the last hour, vs a {:.1}s 7-day baseline",
                    recent_avg / 1000.0,
                    base_avg / 1000.0
                ),
                &day_key,
                out,
            )?;
        }
    }
    // Traffic burst: the hour tripled the baseline's hourly rate. The
    // baseline floor (a request every other hour over the week) keeps a
    // fresh install's first real use from reading as a burst.
    let base_hourly = base.requests as f64 / base_hours;
    if recent.requests >= 20 && base.requests >= 84 && recent.requests as f64 > 3.0 * base_hourly {
        push_anomaly(
            ctx,
            "burst",
            format!(
                "Traffic burst: {} requests in the last hour — 3× the {:.1}/h 7-day baseline",
                recent.requests, base_hourly
            ),
            &day_key,
            out,
        )?;
    }
    Ok(())
}

/// Agent budgets (Features panel): the gateway already refuses the request;
/// this is the notification that refusal never produced.
fn push_agent_limit_alerts(ctx: &AlertCtx, out: &mut Vec<UsageAlertVm>) -> Result<(), String> {
    for l in ctx.store.list_agent_limits().map_err(e2s)? {
        if let Some(pl) = kiwanod::limits::agent_limit_usage(ctx.store, &l).map_err(e2s)? {
            if pl.used >= pl.limit {
                let key = format!("alert_sent:agent:{}:{}", l.agent, l.period);
                if first_notice(ctx.aux, &key, &pl.period_key, ctx.mark)? {
                    out.push(UsageAlertVm {
                        provider_id: String::new(),
                        provider_name: l.agent.clone(),
                        used: (pl.used * 100.0).round() / 100.0,
                        limit: pl.limit,
                        unit: pl.unit.clone(),
                        kind: "agent_limit".into(),
                        message: format!(
                            "Agent '{}' reached its {} budget: {:.0} of {:.0} {}",
                            l.agent, l.period, pl.used, pl.limit, pl.unit
                        ),
                    });
                }
            }
        }
    }
    Ok(())
}

/// `check_usage_alerts` against a caller-chosen "now" — the tests' way in:
/// the forecast divides by how much of the period has elapsed, and the
/// anomaly rules read the last completed hour, so neither can be asserted
/// against a wall clock that keeps moving.
fn check_usage_alerts_at(
    store: &Store,
    aux: &Aux,
    mark: bool,
    now: i64,
) -> Result<Vec<UsageAlertVm>, String> {
    let settings = ui_settings(aux);
    if !settings.cost_alert
        && !settings.feat_cost_forecast
        && !settings.feat_anomaly_alerts
        && !settings.feat_agent_limit_alerts
    {
        return Ok(Vec::new());
    }
    // The gate above is the only read of `settings` that is not through the
    // context; it is moved in rather than cloned.
    let ctx = AlertCtx {
        store,
        aux,
        settings,
        mark,
        now,
    };
    let mut alerts = Vec::new();
    // Per provider, and in this order: the amount pair, then the plan window.
    // `!p.enabled` is checked once here, so a parked provider is skipped for
    // every source below.
    for p in ctx.store.list_providers().map_err(e2s)? {
        if !p.enabled {
            continue;
        }
        push_amount_alerts(&ctx, &p, &mut alerts)?;
        push_plan_window_alert(&ctx, &p, &mut alerts)?;
    }
    if ctx.settings.feat_anomaly_alerts {
        push_anomaly_alerts(&ctx, &mut alerts)?;
    }
    if ctx.settings.feat_agent_limit_alerts {
        push_agent_limit_alerts(&ctx, &mut alerts)?;
    }
    Ok(alerts)
}

/// Record `identity` under `key`, answering whether this is the first time it
/// has been seen. With `mark` false the answer is given without consuming the
/// dedup, so a read-only caller cannot eat the alert the app is about to raise.
fn first_notice(aux: &Aux, key: &str, identity: &str, mark: bool) -> Result<bool, String> {
    if aux.get_setting(key).as_deref() == Some(identity) {
        return Ok(false);
    }
    if mark {
        aux.set_setting(key, identity).map_err(e2s)?;
    }
    Ok(true)
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
        aux.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
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
