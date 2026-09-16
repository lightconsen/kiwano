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
    Spend {
        used: f64,
        limit: f64,
        unit: String,
        /// Which window, when it is one of several an agent holds (`day`,
        /// `monthly`, …). None for a provider's limit, whose single period is
        /// already named by the provider's own configuration.
        window: Option<String>,
    },
}

impl BlockReason {
    /// One line, for a log and for the Apps card.
    pub fn describe(&self) -> String {
        match self {
            BlockReason::PlanWindow { window, util, pct } => {
                format!("{window} window at {util:.0}% of a {pct:.0}% ceiling")
            }
            BlockReason::Spend {
                used,
                limit,
                unit,
                window,
            } => match window {
                Some(w) => format!("{used:.2} of {limit:.2} {unit} this {w}"),
                None => format!("{used:.2} of {limit:.2} {unit} this period"),
            },
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
    /// Agents over their own ceiling, keyed by agent id. Kept apart from
    /// `blocked` because the two answer different questions: that one is "this
    /// provider has spent what it was allowed", this one is "this agent has". A
    /// provider being blocked routes around it; an agent being over its own
    /// ceiling is the end of the request.
    agents_over: std::collections::HashMap<String, BlockReason>,
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

    /// Whether this agent has spent its own allowance. Read by route selection
    /// before any strategy gets a say.
    pub fn agent_blocked(&self, agent: &str) -> Option<&BlockReason> {
        self.agents_over.get(agent)
    }

    pub fn agents_over_entries(&self) -> impl Iterator<Item = (&str, &BlockReason)> {
        self.agents_over.iter().map(|(a, r)| (a.as_str(), r))
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
            agents_over: std::collections::HashMap::new(),
            tz_offset_minutes: 0,
        }
    }

    /// A snapshot with nothing blocked, on the user's clock at
    /// `tz_offset_minutes`. `Default` is UTC; this is how a test states which
    /// clock the user is on without reaching for the host's zone.
    #[cfg(test)]
    pub fn with_offset(tz_offset_minutes: i64) -> Self {
        LimitState {
            tz_offset_minutes,
            ..Default::default()
        }
    }

    /// A snapshot with exactly these agents over their own ceiling.
    #[cfg(test)]
    pub fn with_agents_over(reasons: impl IntoIterator<Item = (String, BlockReason)>) -> Self {
        LimitState {
            blocked: std::collections::HashMap::new(),
            agents_over: reasons.into_iter().collect(),
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
                        // A provider's limit has one period, named by the
                        // provider's own configuration; nothing to disambiguate.
                        window: None,
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
    // An agent's own ceiling, measured across every provider it used. Read here
    // rather than per request so the routing path stays a snapshot lookup, the
    // same reason the provider limits are computed on this tick.
    let mut agents_over = std::collections::HashMap::new();
    match store.list_agent_limits() {
        Ok(limits) => {
            for limit in limits {
                match agent_limit_usage(store, &limit) {
                    Ok(Some(pl)) if pl.used >= pl.limit => {
                        // First window to trip wins the row: which one it was
                        // matters more than the count, and an agent over two of
                        // them is over either way.
                        agents_over.entry(limit.agent.clone()).or_insert_with(|| {
                            BlockReason::Spend {
                                used: pl.used,
                                limit: pl.limit,
                                unit: pl.unit,
                                window: Some(limit.period.clone()),
                            }
                        });
                    }
                    Ok(_) => {}
                    Err(e) => tracing::warn!(
                        agent = %limit.agent,
                        period = %limit.period,
                        error = %e,
                        "agent limit not evaluated this tick; the agent stays routable"
                    ),
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "agent limits unreadable; none enforced"),
    }
    LimitState {
        blocked,
        agents_over,
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
/// It is what keeps `evaluate` free of network calls. A provider whose endpoint
/// is down keeps its previous cached answer (or none), so a flaky endpoint never
/// fabricates a block.
///
/// The queries run concurrently. Each carries its own 15-second client timeout
/// (`plan_quota::client`), so asking them one at a time put the whole batch —
/// and therefore `publish`, which waits on all of them — behind `N × 15s` when
/// N endpoints are unreachable, while requests kept routing on the stale
/// snapshot. Concurrency is safe here: `Store` is `Send + Sync` and locks per
/// call, and each provider caches under its own `app_settings` key, so two
/// queries share nothing.
///
/// Unbounded on purpose: N is the number of providers the operator configured,
/// which is single digits. A deployment with dozens would want
/// `buffer_unordered` to stop N simultaneous requests from going out at once.
pub async fn refresh_plan_reports(store: &Store) {
    let providers = match store.list_providers() {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!(error = %e, "plan-quota refresh skipped: providers unreadable");
            return;
        }
    };
    let due = providers
        .iter()
        .filter(|p| p.enabled && p.plan_query.is_some());
    // Awaiting the whole batch is the point: `evaluate` below rebuilds the
    // snapshot from the store, so it has to run after every write has landed,
    // and `publish` has to be called exactly once per tick.
    futures_util::future::join_all(due.map(|p| async move {
        if let Err(e) = crate::plan_quota::get_plan_quota_report(store, &p.id, false).await {
            tracing::debug!(provider = %p.id, error = %e, "plan quota refresh failed");
        }
    }))
    .await;
}

/// How often the limits are re-evaluated. The plan half rides a 5-minute
/// upstream cache, so this is about noticing a spending limit promptly rather
/// than about refresh cost.
pub const LIMIT_INTERVAL: StdDuration = StdDuration::from_secs(30);

/// Re-evaluate and publish, forever. The one thing in the gateway that looks at
/// the world on a schedule rather than because a request arrived.
pub async fn run(state: Arc<GatewayState>, interval: StdDuration) {
    loop {
        // No blocking island: the quota queries are awaited on the runtime like
        // everything else the daemon does, and `evaluate` is store reads only.
        // (This was a `spawn_blocking` because the refresh used a blocking HTTP
        // client — the one place the daemon had to leave its own runtime to do
        // network I/O.)
        refresh_plan_reports(&state.store).await;
        publish(&state, evaluate(&state.store));
        tokio::time::sleep(interval).await;
    }
}

/// Install a fresh evaluation, logging only the transitions — the line that
/// answers "why did my request start failing?" without a per-tick heartbeat.
///
/// Shared with the reload path on purpose: an edit is the most likely cause of a
/// transition, and a refresh that reported nothing would leave the one change the
/// user just made as the only silent one.
pub fn publish(state: &GatewayState, next: LimitState) {
    let prev = state.limits();
    let current = state.set_limits(next);
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
    // An agent's own ceiling has no "around it" to describe: the request ends.
    for (agent, reason) in current.agents_over_entries() {
        if prev.agent_blocked(agent).is_none() {
            tracing::warn!(
                agent = %agent,
                reason = %reason.describe(),
                "agent is over its own limit; requests will be refused"
            );
        }
    }
    for (agent, _) in prev.agents_over_entries() {
        if current.agent_blocked(agent).is_none() {
            tracing::info!(agent = %agent, "agent is back under its own limit");
        }
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
        crate::plan_quota::cache_write(
            &s,
            "glm-1",
            &plan_report(50.0, Some("2026-09-14T10:00:00Z")),
        );
        assert!(evaluate(&s).blocked("glm-1").is_none());

        // At it: out, and for the reason claimed.
        crate::plan_quota::cache_write(
            &s,
            "glm-1",
            &plan_report(95.0, Some("2026-09-14T10:00:00Z")),
        );
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
        crate::plan_quota::cache_write(
            &s,
            "glm-1",
            &plan_report(0.0, Some("2026-09-14T15:00:00Z")),
        );
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
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
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

    fn agent_limit(
        agent: &str,
        limit: f64,
        unit: Option<&str>,
        period: &str,
    ) -> crate::store::AgentLimit {
        crate::store::AgentLimit {
            agent: agent.into(),
            period: period.into(),
            period_limit: limit,
            limit_unit: unit.map(str::to_string),
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    /// One agent's ceilings, as the screen would save them.
    fn set_limits(store: &Store, agent: &str, limits: Vec<crate::store::AgentLimit>) {
        store.replace_agent_limits(agent, &limits).unwrap();
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

    /// An agent may hold several windows at once, and being over **any** of them
    /// is being over: a day's ceiling is what stops one runaway session, a month's
    /// what stops a runaway month, and the one that trips is the one the refusal
    /// names.
    ///
    /// Each case below is over exactly one window and under the other, which is
    /// the only shape that tells "any window" apart from "the last window read".
    #[test]
    fn an_agent_is_over_when_any_of_its_windows_is() {
        let (_dir, store) = store_with_hub_rates(r#"{"USD":1.0}"#);
        store.insert_provider(&test_provider("ds-1")).unwrap();

        let day = agent_limit("claude", 10.0, Some("requests"), "day");
        let month = agent_limit("claude", 100.0, Some("requests"), "monthly");

        // Under both: nothing to say.
        for _ in 0..5 {
            record_agent_row(&store, "claude", "ds-1");
        }
        set_limits(&store, "claude", vec![day.clone(), month.clone()]);
        assert!(evaluate(&store).agent_blocked("claude").is_none());

        // Over the day, still under the month.
        for _ in 0..6 {
            record_agent_row(&store, "claude", "ds-1");
        }
        let state = evaluate(&store);
        let reason = state
            .agent_blocked("claude")
            .expect("11 requests is past a daily ceiling of 10");
        assert!(
            reason.describe().contains("this day"),
            "the refusal names the window that tripped: {}",
            reason.describe()
        );

        // Raise the day past the spend and the month becomes the tight one — so
        // the same usage is judged by a different window without moving it.
        set_limits(
            &store,
            "claude",
            vec![agent_limit("claude", 20.0, Some("requests"), "day"), month],
        );
        assert!(evaluate(&store).agent_blocked("claude").is_none());

        set_limits(
            &store,
            "claude",
            vec![
                agent_limit("claude", 20.0, Some("requests"), "day"),
                agent_limit("claude", 10.0, Some("requests"), "monthly"),
            ],
        );
        let state = evaluate(&store);
        let reason = state
            .agent_blocked("claude")
            .expect("11 is past a monthly 10");
        assert!(
            reason.describe().contains("this monthly"),
            "{}",
            reason.describe()
        );
    }

    /// One usage row for an agent, priced or not — the counting units do not need
    /// a price, and a request count is what the daily/monthly case is about.
    fn record_agent_row(store: &Store, agent: &str, provider_id: &str) {
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
                cost: None,
                cost_currency: None,
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

    /// A local endpoint that answers each request after `delay`, counting the
    /// ones it served. Each connection gets its own thread, so the stub does not
    /// serialize what it is being used to prove is concurrent.
    ///
    /// Every plan template but `zenmux` builds its URL from a constant, so
    /// pointing a provider at this instead of the internet is what `zenmux`'s
    /// configurable `quota_url` is for.
    struct SlowUpstream {
        url: String,
        hits: Arc<std::sync::atomic::AtomicUsize>,
    }

    impl SlowUpstream {
        fn start(delay: StdDuration) -> SlowUpstream {
            use std::io::{Read, Write};
            use std::sync::atomic::Ordering;

            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub");
            let port = listener.local_addr().expect("stub addr").port();
            let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let counter = hits.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    counter.fetch_add(1, Ordering::SeqCst);
                    std::thread::spawn(move || {
                        // Drain before answering: a request spanning more than
                        // one segment otherwise sees the connection reset.
                        let mut buf = [0u8; 8192];
                        let _ = stream.read(&mut buf);
                        std::thread::sleep(delay);
                        // A body the parser rejects is fine — this stub exists
                        // to be *slow*, and the round trip is the measurement.
                        let body = r#"{"success":true,"data":{}}"#;
                        let _ = write!(
                            stream,
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    });
                }
            });
            SlowUpstream {
                url: format!("http://127.0.0.1:{port}/usage"),
                hits,
            }
        }
    }

    /// A provider whose plan query runs against `stub`.
    fn provider_querying(store: &Store, id: &str, stub: &SlowUpstream) {
        let mut p = test_provider(id);
        p.api_key = Some("sk-test".into());
        p.plan_query = Some(format!(
            r#"{{"template":"zenmux","fields":{{"quota_url":"{}"}}}}"#,
            stub.url
        ));
        store.insert_provider(&p).expect("insert provider");
    }

    /// The batch is asked concurrently, so N slow endpoints cost roughly one
    /// round trip rather than N.
    ///
    /// Timing is the only way to see this from outside — the queries share no
    /// other observable side effect — so the margin is deliberately wide: three
    /// serial rounds are 3.0× the delay and the bar is 1.8×, which leaves the
    /// measured round trip room to grow on a loaded machine. Verified to fail
    /// against the serial loop (1.15s) before it was made concurrent.
    #[tokio::test]
    async fn plan_quota_queries_do_not_serialize() {
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("kiwano.db")).unwrap();
        let delay = StdDuration::from_millis(500);
        let stub = SlowUpstream::start(delay);
        for id in ["p1", "p2", "p3"] {
            provider_querying(&store, id, &stub);
        }

        let started = std::time::Instant::now();
        refresh_plan_reports(&store).await;
        let elapsed = started.elapsed();

        assert_eq!(
            stub.hits.load(Ordering::SeqCst),
            3,
            "every provider with a query was asked"
        );
        assert!(
            elapsed < delay * 3 * 3 / 5,
            "three queries took {elapsed:?}; serially they would be {:?} and the \
             concurrent round trip is {delay:?}",
            delay * 3
        );
    }

    /// The batch skips providers that are parked or have no query to run — the
    /// concurrency change must not have widened who gets asked.
    #[tokio::test]
    async fn plan_quota_refresh_skips_parked_and_queryless_providers() {
        use std::sync::atomic::Ordering;

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("kiwano.db")).unwrap();
        let stub = SlowUpstream::start(StdDuration::from_millis(0));
        provider_querying(&store, "asked", &stub);

        // A queryless provider.
        store.insert_provider(&test_provider("no-query")).unwrap();
        // A parked one that does carry a query.
        provider_querying(&store, "parked", &stub);
        let mut parked = store.get_provider("parked").unwrap().unwrap();
        parked.enabled = false;
        store.update_provider(&parked).unwrap();

        refresh_plan_reports(&store).await;

        assert_eq!(
            stub.hits.load(Ordering::SeqCst),
            1,
            "only the enabled provider with a query was asked"
        );
    }
}
