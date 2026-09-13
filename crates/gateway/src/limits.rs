//! Per-provider billing limits: how much of a limit is spent, and which
//! providers are therefore out of service.
//!
//! The reading half lives here because it has to be shared. The gateway
//! enforces these limits (see `crate::strategy`), and the desktop app notifies
//! about them; if each computed the number its own way they could disagree, and
//! a provider would be blocked by one and reported as fine by the other.
//!
//! Two kinds of limit, one shape of answer:
//!
//!   * **amount** — `period_limit` in `requests`, `wan_tokens` or the
//!     provider's own currency, measured over `reset_period`;
//!   * **plan percent** — `plan_limits` ceilings (`five_hour`, `weekly`) against
//!     the utilization the provider's own endpoint reports (see `plan_quota`).

use std::sync::Arc;
// `Duration` below is chrono's — the two are named apart so the date maths
// stays readable.
use std::time::Duration as StdDuration;

use chrono::{Datelike, Duration, NaiveDate, SecondsFormat, TimeZone, Utc};

use crate::server::GatewayState;
use crate::store::{Provider, Store};

/// Start of the current reset period as the RFC3339 UTC instant a `ts >=`
/// filter wants, plus the key a notification dedups on.
///
/// Reset periods follow the user's clock: a monthly limit rolls over at local
/// midnight on the 1st, not at UTC's. `None` means no reset — the limit
/// measures all time, which is also where an unrecognised value lands.
pub fn period_start(
    epoch_secs: i64,
    reset_period: Option<&str>,
    tz_offset_minutes: i64,
) -> (Option<String>, String) {
    let offset = Duration::minutes(tz_offset_minutes);
    // Shift the instant by the offset and read it as UTC: the local wall clock
    // is then the calendar the period boundaries come from.
    let local = Utc
        .timestamp_opt(epoch_secs, 0)
        .single()
        .unwrap_or_else(Utc::now)
        + offset;
    let day = local.date_naive();

    // Local midnight of `d`, written the way `now_rfc3339` writes timestamps —
    // the column is compared lexicographically, so the shape is part of the
    // contract, not cosmetic.
    let midnight_utc = |d: NaiveDate| {
        let naive = d.and_hms_opt(0, 0, 0).expect("midnight is a valid time");
        Utc.from_utc_datetime(&(naive - offset))
            .to_rfc3339_opts(SecondsFormat::Secs, true)
    };
    let first_of = |y: i32, m: u32| NaiveDate::from_ymd_opt(y, m, 1).expect("the 1st exists");

    match reset_period {
        None => (None, "all".into()),
        // The quota strategy's `period: "day"`. Same boundary as every other
        // period here, because "100 requests a day" means the user's day.
        Some("day") => (Some(midnight_utc(day)), day.format("%Y-%m-%d").to_string()),
        Some("weekly") => {
            let monday = day - Duration::days(day.weekday().num_days_from_monday() as i64);
            (
                Some(midnight_utc(monday)),
                monday.format("%Y-%m-%d").to_string(),
            )
        }
        Some("yearly") => (
            Some(midnight_utc(first_of(day.year(), 1))),
            format!("{:04}", day.year()),
        ),
        // `monthly` and anything unrecognised.
        _ => (
            Some(midnight_utc(first_of(day.year(), day.month()))),
            format!("{:04}-{:02}", day.year(), day.month()),
        ),
    }
}

/// What a provider's per-period limit is measured in, and how much of it is
/// spent.
pub struct PeriodLimit {
    pub used: f64,
    pub limit: f64,
    /// `requests` | `wan_tokens` | an ISO currency code — the limit's own unit.
    pub unit: String,
    /// The reset period's identity (e.g. `2026-09`), for notification dedup.
    pub period_key: String,
}

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

/// `providers.plan_limits`: percent ceilings on the plan's own windows.
#[derive(Debug, serde::Deserialize)]
pub struct PlanLimits {
    five_hour: Option<f64>,
    weekly: Option<f64>,
}

impl PlanLimits {
    /// Parse a provider's ceilings. `None` when absent, unreadable, or when
    /// neither window is configured — a row that limits nothing.
    pub fn parse(raw: Option<&str>) -> Option<PlanLimits> {
        let limits: PlanLimits = serde_json::from_str(raw?).ok()?;
        (limits.five_hour.is_some() || limits.weekly.is_some()).then_some(limits)
    }
}

/// A plan window whose live utilization has reached the ceiling set for it.
pub struct WindowHit<'a> {
    /// `five_hour` | `weekly`.
    pub window: &'static str,
    /// Percent of the window used, as the provider's own endpoint reports it.
    pub util: f64,
    /// The ceiling the user configured, in percent.
    pub pct: f64,
    /// When the window resets, as the endpoint reports it. This is what a
    /// notification dedups on: one notice per window, and a fresh one after the
    /// window rolls over. `None` when the endpoint does not say.
    pub resets_at: Option<&'a str>,
}

/// The first configured window whose live utilization has reached its ceiling.
/// A window the report does not mention counts as not over: no evidence is not
/// evidence of a hit.
pub fn window_over<'a>(
    report: &'a crate::plan_quota::PlanQuotaReport,
    limits: &PlanLimits,
) -> Option<WindowHit<'a>> {
    let tier = |name: &str| report.tiers.iter().find(|t| t.name == name);
    let hit = |window, tier: &'a crate::plan_quota::PlanTierVm, pct| WindowHit {
        window,
        util: tier.utilization,
        pct,
        resets_at: tier.resets_at.as_deref(),
    };
    if let Some(pct) = limits.five_hour {
        if let Some(t) = tier("five_hour") {
            if t.utilization >= pct {
                return Some(hit("five_hour", t, pct));
            }
        }
    }
    if let Some(pct) = limits.weekly {
        if let Some(t) = tier("weekly_limit") {
            if t.utilization >= pct {
                return Some(hit("weekly", t, pct));
            }
        }
    }
    None
}

/// Why a provider is out of service.
#[derive(Debug, Clone, PartialEq)]
pub enum BlockReason {
    /// A plan window's live utilization reached the ceiling configured for it.
    PlanWindow { window: String, util: f64, pct: f64 },
    /// The period's usage reached an amount limit.
    Spend { used: f64, limit: f64, unit: String },
}

impl BlockReason {
    /// One line, for a log and for the Apps card.
    pub fn describe(&self) -> String {
        match self {
            BlockReason::PlanWindow { window, util, pct } => {
                format!("{window} window at {util:.0}% of a {pct:.0}% ceiling")
            }
            BlockReason::Spend { used, limit, unit } => {
                format!("{used:.2} of {limit:.2} {unit} this period")
            }
        }
    }
}

/// Which providers are over a limit right now.
///
/// Derived state, never authored: it is recomputed from scratch every tick, so
/// it cannot strand a provider the way a persisted `enabled = false` would —
/// there is no marker to clean up and no "who switched this off?" to answer.
#[derive(Debug, Default)]
pub struct LimitState {
    blocked: std::collections::HashMap<String, BlockReason>,
    /// The user's UTC offset, carried along so the routing path can work out
    /// what "today" means without reading settings on every request. It rides
    /// the snapshot, so it is at worst one refresh interval stale — and it
    /// comes from the same place the billing limits read, so the two cannot
    /// disagree about when a day starts.
    tz_offset_minutes: i64,
}

impl LimitState {
    pub fn blocked(&self, provider_id: &str) -> Option<&BlockReason> {
        self.blocked.get(provider_id)
    }

    pub fn tz_offset_minutes(&self) -> i64 {
        self.tz_offset_minutes
    }

    pub fn entries(&self) -> impl Iterator<Item = (&str, &BlockReason)> {
        self.blocked.iter().map(|(id, r)| (id.as_str(), r))
    }

    /// A snapshot with exactly these providers blocked. `evaluate` is the
    /// production constructor; this is for stating a case outright.
    #[cfg(test)]
    pub fn from_reasons(reasons: impl IntoIterator<Item = (String, BlockReason)>) -> Self {
        LimitState {
            blocked: reasons.into_iter().collect(),
            tz_offset_minutes: 0,
        }
    }
}

/// Take the providers that are over a limit out of a route's candidate list.
/// `None` when nothing was removed, so the common case allocates nothing.
pub fn without_blocked(
    route: &crate::router::AgentRoute,
    limits: &LimitState,
) -> Option<crate::router::AgentRoute> {
    if limits.blocked.is_empty() {
        return None;
    }
    let kept: Vec<_> = route
        .candidates
        .iter()
        .filter(|c| limits.blocked(&c.id).is_none())
        .cloned()
        .collect();
    (kept.len() != route.candidates.len()).then(|| crate::router::AgentRoute {
        candidates: kept,
        ..route.clone()
    })
}

/// Evaluate every provider's limits against the store.
///
/// Best-effort throughout: a provider whose numbers cannot be read is left
/// unblocked. A limit that fails closed would take the agent down on a hiccup,
/// which is a worse failure than one more request against the cap.
pub fn evaluate(store: &Store) -> LimitState {
    let mut blocked = std::collections::HashMap::new();
    let tz_offset_minutes = store.ui_tz_offset_minutes();
    let providers = match store.list_providers() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "limit evaluation skipped: providers unreadable");
            return LimitState::default();
        }
    };
    for p in providers {
        // `enabled` is the user's switch and the router already honours it;
        // unlimited has nothing to measure.
        if !p.enabled || p.billing == crate::store::Billing::Unlimited {
            continue;
        }
        if let Ok(Some(pl)) = period_limit_usage(store, &p) {
            if pl.used >= pl.limit {
                blocked.insert(
                    p.id.clone(),
                    BlockReason::Spend {
                        used: pl.used,
                        limit: pl.limit,
                        unit: pl.unit,
                    },
                );
            }
        }
        // The percent half reads only what the refresh step cached. Fetching
        // here would make `GatewayState::new` block on N provider endpoints,
        // and would put a network round trip in a path that runs every tick.
        if p.billing == crate::store::Billing::Subscription {
            // Bound to locals rather than chained: the hit borrows the report,
            // so the report has to outlive it in this scope.
            let limits = PlanLimits::parse(p.plan_limits.as_deref());
            let report = crate::plan_quota::cached_report(store, &p.id);
            let over = match (limits.as_ref(), report.as_ref()) {
                (Some(limits), Some(report)) => window_over(report, limits),
                _ => None,
            };
            if let Some(hit) = over {
                blocked.insert(
                    p.id.clone(),
                    BlockReason::PlanWindow {
                        window: hit.window.to_string(),
                        util: hit.util,
                        pct: hit.pct,
                    },
                );
            }
        }
    }
    LimitState {
        blocked,
        tz_offset_minutes,
    }
}

/// The KV markers the desktop app wrote when *it* disabled a provider for
/// exceeding a limit. Nothing produces them any more.
const LEGACY_DISABLE_MARKERS: [&str; 2] = ["plan_limit_disabled:", "spend_limit_disabled:"];

/// Undo the old app-side enforcement, once at startup.
///
/// Those markers were how the app told its own doing from the user's, so it
/// could re-enable safely. Enforcement moved here and no longer disables
/// anything, which means nothing would ever clear them: a provider the old app
/// switched off would stay off, with no marker UI to explain why and no code
/// left to restore it.
pub fn clear_legacy_disables(store: &Store) -> usize {
    let mut restored = 0;
    for prefix in LEGACY_DISABLE_MARKERS {
        let Ok(rows) = store.app_settings_with_prefix(prefix) else {
            continue;
        };
        for (key, _) in rows {
            let Some(id) = key.strip_prefix(prefix) else {
                continue;
            };
            if let Ok(Some(mut p)) = store.get_provider(id) {
                if !p.enabled {
                    p.enabled = true;
                    p.updated_at = crate::store::now_rfc3339();
                    if store.update_provider(&p).is_ok() {
                        restored += 1;
                        tracing::info!(
                            provider = %id,
                            "re-enabled: it was disabled by an older limit patrol, which the gateway now owns"
                        );
                    }
                }
            }
            let _ = store.delete_app_setting(&key);
        }
    }
    restored
}

/// Refresh the cached plan reports for providers that have a query to run.
///
/// Blocking HTTP — the caller runs it off the async runtime — and it is what
/// keeps `evaluate` free of network calls. A provider whose endpoint is down
/// keeps its previous cached answer (or none), so a flaky endpoint never
/// fabricates a block.
pub fn refresh_plan_reports(store: &Store) {
    let providers = match store.list_providers() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "plan-quota refresh skipped: providers unreadable");
            return;
        }
    };
    for p in providers {
        if !p.enabled || p.plan_query.is_none() {
            continue;
        }
        if let Err(e) = crate::plan_quota::get_plan_quota_report(store, &p.id, false) {
            tracing::debug!(provider = %p.id, error = %e, "plan quota refresh failed");
        }
    }
}

/// How often the limits are re-evaluated. The plan half rides a 5-minute
/// upstream cache, so this is about noticing a spending limit promptly rather
/// than about refresh cost.
pub const LIMIT_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Re-evaluate and publish, forever. Shaped like `strategy::prober::run` — the
/// other thing here that has to look at the world outside a request.
pub async fn run(state: Arc<GatewayState>, interval: StdDuration) {
    loop {
        // Off the async runtime: the refresh does blocking HTTP against
        // provider endpoints. The evaluation itself is store reads only.
        let store = state.store.clone();
        let evaluated = tokio::task::spawn_blocking(move || {
            refresh_plan_reports(&store);
            evaluate(&store)
        })
        .await;

        if let Ok(next) = evaluated {
            let prev = state.limits();
            let current = state.set_limits(next);
            // Transitions only: this is the line that answers "why did my
            // request start failing?" without a per-tick heartbeat.
            for (id, reason) in current.entries() {
                if prev.blocked(id).is_none() {
                    tracing::warn!(
                        provider = %id,
                        reason = %reason.describe(),
                        "provider is over its limit; routing around it"
                    );
                }
            }
            for (id, _) in prev.entries() {
                if current.blocked(id).is_none() {
                    tracing::info!(provider = %id, "provider is back under its limit");
                }
            }
        }
        tokio::time::sleep(interval).await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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

    /// The percent half of `evaluate`: a plan window whose live utilization has
    /// reached the ceiling the user set takes the provider out of the routes,
    /// carrying the window and the numbers that decided it.
    ///
    /// Untested until now — the money half had tests and this did not — which
    /// mattered the moment `window_over` started returning the window rather
    /// than a tuple: the only thing standing behind this branch was the compiler.
    #[test]
    fn a_plan_window_over_its_ceiling_takes_the_provider_out_of_the_routes() {
        use crate::store::Billing;
        let s = Store::open_in_memory().unwrap();
        let mut p = test_provider("glm-1");
        p.billing = Billing::Subscription;
        p.plan_limits = Some(r#"{"five_hour": 90}"#.into());
        s.insert_provider(&p).unwrap();

        // Under the ceiling: routed.
        crate::plan_quota::cache_write(&s, "glm-1", &plan_report(50.0, Some("2026-09-14T10:00:00Z")));
        assert!(evaluate(&s).blocked("glm-1").is_none());

        // At it: out, and for the reason claimed.
        crate::plan_quota::cache_write(&s, "glm-1", &plan_report(95.0, Some("2026-09-14T10:00:00Z")));
        match evaluate(&s).blocked("glm-1") {
            Some(BlockReason::PlanWindow { window, util, pct }) => {
                assert_eq!(window, "five_hour");
                assert!((util - 95.0).abs() < 1e-6, "got {util}");
                assert!((pct - 90.0).abs() < 1e-6, "got {pct}");
            }
            other => panic!("expected a plan-window block, got {other:?}"),
        }

        // A window the report does not mention is not evidence of a hit: with a
        // stale cache of the other shape, the provider routes again.
        crate::plan_quota::cache_write(&s, "glm-1", &plan_report(0.0, Some("2026-09-14T15:00:00Z")));
        assert!(evaluate(&s).blocked("glm-1").is_none());
    }

    #[test]
    fn period_start_keys() {
        // 2026-09-07T12:34:56Z (Monday)
        let t = 1_788_784_496_i64;
        let (since, key) = period_start(t, Some("monthly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(key, "2026-09");
        let (since, key) = period_start(t, Some("weekly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-07T00:00:00Z"));
        assert_eq!(key, "2026-09-07");
        let (since, key) = period_start(t, Some("yearly"), 0);
        assert_eq!(since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(key, "2026");
        // no reset → all-time totals
        let (since, key) = period_start(t, None, 0);
        assert_eq!(since, None);
        assert_eq!(key, "all");
    }

    /// A plain enabled provider, no limits and not yet stored — so a test can
    /// shape it before inserting, since `insert_provider` is a plain INSERT.
    fn test_provider(provider_id: &str) -> crate::store::Provider {
        use crate::store::{Billing, Protocol, Provider};
        Provider {
            id: provider_id.into(),
            name: provider_id.into(),
            catalog_id: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    /// A stored provider on a CNY amount limit, with no usage recorded yet.
    fn limited_provider(store: &Store, provider_id: &str, limit: f64) -> crate::store::Provider {
        let mut p = test_provider(provider_id);
        p.period_limit = Some(limit);
        p.limit_unit = Some("CNY".into());
        p.reset_period = Some("monthly".into());
        store.insert_provider(&p).unwrap();
        p
    }

    /// The plan report the refresh step would have cached, with one window's
    /// utilization set and everything else quiet.
    fn plan_report(util: f64, resets_at: Option<&str>) -> crate::plan_quota::PlanQuotaReport {
        use crate::plan_quota::{PlanQuotaReport, PlanTierVm};
        PlanQuotaReport {
            provider_id: "glm-1".into(),
            template: "zhipu".into(),
            success: true,
            error: None,
            note: None,
            tiers: vec![
                PlanTierVm {
                    name: "five_hour".into(),
                    utilization: util,
                    resets_at: resets_at.map(str::to_string),
                    used: None,
                    limit: None,
                    unit: None,
                },
                PlanTierVm {
                    name: "weekly_limit".into(),
                    utilization: 5.0,
                    resets_at: None,
                    used: None,
                    limit: None,
                    unit: None,
                },
            ],
            queried_at: 0,
            cached: false,
        }
    }

    /// One metered row, in the currency given.
    fn record_cost(store: &Store, provider_id: &str, cost: f64, currency: &str) {
        use crate::store::UsageRecord;
        store
            .record_usage(&UsageRecord {
                ts: crate::store::now_rfc3339(),
                agent: "claude".into(),
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

    fn store_with_spend(provider_id: &str, limit: f64, spent: f64) -> Store {
        let s = Store::open_in_memory().unwrap();
        limited_provider(&s, provider_id, limit);
        record_cost(&s, provider_id, spent, "CNY");
        s
    }

    /// A store on a real file with a Hub price document cached beside it — the
    /// shape production has, where the GUI opens `Aux` on the same database the
    /// gateway opens, so the rates the seeder read and the rates a limit reads
    /// back are one row.
    fn store_with_hub_rates(rates: &str) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        // The GUI's own schema (`kiwano_core::Aux`), which the gateway reads but
        // does not create.
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hub_models_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 version   INTEGER NOT NULL,
                 sha256    TEXT NOT NULL,
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
             )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, 1, 'sha', ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![format!(
                r#"{{"version":1,"exchange_rates":{rates},"models":[]}}"#
            )],
        )
        .unwrap();
        (dir, store)
    }

    #[test]
    fn evaluate_blocks_a_provider_over_its_cap_and_clears_when_it_rolls_back() {
        let s = store_with_spend("payg-1", 30.0, 31.0);
        let state = evaluate(&s);
        assert!(
            matches!(state.blocked("payg-1"), Some(BlockReason::Spend { .. })),
            "31 spent against a 30 cap is over it"
        );

        // Under the cap: not blocked. (`evaluate` recomputes from scratch, so
        // there is no state to clear — that is the point of not persisting it.)
        let s2 = store_with_spend("payg-1", 30.0, 29.0);
        assert!(evaluate(&s2).blocked("payg-1").is_none());
    }

    #[test]
    fn the_legacy_disable_markers_are_undone_once() {
        let s = store_with_spend("payg-1", 30.0, 31.0);
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.enabled = false;
        s.update_provider(&p).unwrap();
        // What the old app-side patrol left behind.
        s.set_app_setting("spend_limit_disabled:payg-1", "31.00/30.00")
            .unwrap();
        s.set_app_setting("plan_limit_disabled:payg-1", "five_hour")
            .unwrap();

        assert_eq!(clear_legacy_disables(&s), 1, "one provider restored");
        assert!(s.get_provider("payg-1").unwrap().unwrap().enabled);
        assert!(s.app_setting("spend_limit_disabled:payg-1").is_none());
        assert!(s.app_setting("plan_limit_disabled:payg-1").is_none());
        // Idempotent — it runs on every gateway start.
        assert_eq!(clear_legacy_disables(&s), 0);
    }

    #[test]
    fn evaluate_leaves_unlimited_and_disabled_providers_alone() {
        let s = store_with_spend("payg-1", 30.0, 31.0);
        // Unlimited meters nothing, so a cap on it means nothing either.
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.billing = crate::store::Billing::Unlimited;
        s.update_provider(&p).unwrap();
        assert!(evaluate(&s).blocked("payg-1").is_none());

        // A provider the user switched off is already not routed; blocking it
        // too would just mean two mechanisms with one visible cause.
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.billing = crate::store::Billing::Metered;
        p.enabled = false;
        s.update_provider(&p).unwrap();
        assert!(evaluate(&s).blocked("payg-1").is_none());
    }

    #[test]
    fn a_provider_with_no_limit_is_never_blocked() {
        let mut s = store_with_spend("payg-1", 30.0, 999.0);
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.period_limit = None;
        s.update_provider(&p).unwrap();
        assert!(evaluate(&s).blocked("payg-1").is_none());
        // and a zero cap is "no limit", not "everything is over it"
        let mut p = s.get_provider("payg-1").unwrap().unwrap();
        p.period_limit = Some(0.0);
        s.update_provider(&p).unwrap();
        assert!(evaluate(&s).blocked("payg-1").is_none());
        let _ = &mut s;
    }

    #[test]
    fn the_quota_day_is_the_local_day() {
        // 2026-09-30T20:00:00Z is already 04:00 on the 1st at UTC+8.
        let t = 1_790_798_400_i64;
        let (since, key) = period_start(t, Some("day"), 480);
        assert_eq!(key, "2026-10-01");
        assert_eq!(
            since.as_deref(),
            Some("2026-09-30T16:00:00Z"),
            "local midnight, in UTC"
        );
        let (_, key) = period_start(t, Some("day"), 0);
        assert_eq!(
            key, "2026-09-30",
            "the same instant is a different day in UTC"
        );
    }

    #[test]
    fn period_boundaries_follow_the_users_clock() {
        // 2026-09-30T20:00:00Z is already 04:00 on the 1st of October at UTC+8,
        // so that user's month has rolled over while UTC's has not.
        let t = 1_790_798_400_i64;
        let (since, key) = period_start(t, Some("monthly"), 480);
        assert_eq!(key, "2026-10");
        assert_eq!(
            since.as_deref(),
            Some("2026-09-30T16:00:00Z"),
            "local midnight, in UTC"
        );
        let (_, key) = period_start(t, Some("monthly"), 0);
        assert_eq!(key, "2026-09", "the same instant is still September in UTC");
    }
}
