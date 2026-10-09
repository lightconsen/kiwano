//! Usage alerts: the Dashboard's warnings about spend, plan windows, anomalies
//! and agent ceilings.
//!
//! Moved from `kiwano_core::vm::alerts` (`migrate.local.md` §10.21). It reads
//! nothing but the daemon's tables, so it moved whole — with one classification
//! changed: the **dedup markers** (`alert_sent:*`) were the app's because the app
//! was what produced the alerts. The daemon produces them now, so it remembers
//! them, and `migrate.local.md` §9.1 is corrected to match.

use crate::store::time::{rfc3339_from_unix, unix_now};
use crate::store::{Billing, Provider, Store};
use kiwano_api::dashboard::UsageAlertVm;
use kiwano_api::error::ApiError;
use kiwano_api::settings::SettingsVm;

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

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
/// Notification only — the acting half is `crate::limits::evaluate`, which runs
/// whether or not the user wants to be told.
///
/// `mark` controls the once-per-identity dedup write. The app passes `true` — it
/// raises one notification per provider per reset period, and recording that is
/// the point of the key. A read-only caller passes `false`, because consuming
/// the dedup would suppress the notification the app is about to raise: a
/// `--json` poll from a script must not silently eat the user's alert.
pub fn check_usage_alerts(store: &Store, mark: bool) -> Result<Vec<UsageAlertVm>, ApiError> {
    check_usage_alerts_at(store, mark, unix_now())
}

/// What every alert source reads, resolved once per poll.
struct AlertCtx<'a> {
    store: &'a Store,
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
) -> Result<(), ApiError> {
    let Some(pl) = crate::limits::period_limit_usage(ctx.store, p).map_err(ApiError::failed)?
    else {
        return Ok(());
    };
    // Notify at most once per reset period (app_settings KV dedup).
    let key = format!("alert_sent:{}", p.id);
    if ctx.settings.cost_alert
        && pl.used >= pl.limit
        && first_notice(ctx.store, &key, &pl.period_key, ctx.mark)?
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
        if let Some((start, end)) = crate::limits::period_span_secs(
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
                    if first_notice(ctx.store, &key, &pl.period_key, ctx.mark)? {
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
) -> Result<(), ApiError> {
    if !ctx.settings.cost_alert || p.billing != Billing::Subscription {
        return Ok(());
    }
    // Bound to locals rather than chained: the hit borrows the report,
    // so the report has to outlive it in this scope.
    let limits = crate::limits::PlanLimits::parse(p.plan_limits.as_deref());
    let report = crate::plan_quota::cached_report(ctx.store, &p.id);
    let hit = match (limits.as_ref(), report.as_ref()) {
        (Some(limits), Some(report)) => crate::limits::window_over(report, limits),
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
            crate::limits::period_start(unix_now(), Some("day"), ctx.store.ui_tz_offset_minutes())
                .1
        ),
    };
    let key = format!("alert_sent:plan:{}", p.id);
    if first_notice(ctx.store, &key, &identity, ctx.mark)? {
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
) -> Result<(), ApiError> {
    let key = format!("alert_sent:anomaly:{rule}");
    if first_notice(ctx.store, &key, day_key, ctx.mark)? {
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
fn push_anomaly_alerts(ctx: &AlertCtx, out: &mut Vec<UsageAlertVm>) -> Result<(), ApiError> {
    let hour_start = ctx.now - ctx.now.rem_euclid(3600);
    let recent = ctx
        .store
        .traffic_stats(
            &rfc3339_from_unix(hour_start - 3600),
            &rfc3339_from_unix(hour_start),
        )
        .map_err(ApiError::failed)?;
    let base = ctx
        .store
        .traffic_stats(
            &rfc3339_from_unix(ctx.now - 7 * 86_400),
            &rfc3339_from_unix(hour_start - 3600),
        )
        .map_err(ApiError::failed)?;
    let base_hours = 167.0_f64; // 7 days minus the recent hour
    let day_key =
        crate::limits::period_start(ctx.now, Some("day"), ctx.store.ui_tz_offset_minutes()).1;
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
fn push_agent_limit_alerts(ctx: &AlertCtx, out: &mut Vec<UsageAlertVm>) -> Result<(), ApiError> {
    for l in ctx.store.list_agent_limits().map_err(ApiError::failed)? {
        if let Some(pl) =
            crate::limits::agent_limit_usage(ctx.store, &l).map_err(ApiError::failed)?
        {
            if pl.used >= pl.limit {
                let key = format!("alert_sent:agent:{}:{}", l.agent, l.period);
                if first_notice(ctx.store, &key, &pl.period_key, ctx.mark)? {
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
pub fn check_usage_alerts_at(
    store: &Store,
    mark: bool,
    now: i64,
) -> Result<Vec<UsageAlertVm>, ApiError> {
    // The blob comes from the store — the same row `update_settings` patches —
    // so a feature toggle the user just set is visible to this check. The `aux`
    // is still here for the dedup markers, which only the client writes.
    let settings = super::settings::ui_settings(store)?;
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
        settings,
        mark,
        now,
    };
    let mut alerts = Vec::new();
    // Per provider, and in this order: the amount pair, then the plan window.
    // `!p.enabled` is checked once here, so a parked provider is skipped for
    // every source below.
    for p in ctx.store.list_providers().map_err(ApiError::failed)? {
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
fn first_notice(store: &Store, key: &str, identity: &str, mark: bool) -> Result<bool, ApiError> {
    // **The daemon's KV, not the app's.** These markers were the app's because
    // the app was what produced the alerts (`migrate.local.md` §9.1 listed them
    // as local). It is not any more: the daemon decides what is worth saying, so
    // it is also what remembers having said it — a marker on the client would
    // re-raise every alert the moment the user opened the app on their other
    // machine.
    if store.app_setting(key).as_deref() == Some(identity) {
        return Ok(false);
    }
    if mark {
        store
            .set_app_setting(key, identity)
            .map_err(ApiError::failed)?;
    }
    Ok(true)
}
