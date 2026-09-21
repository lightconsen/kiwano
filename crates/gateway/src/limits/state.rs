//! Which providers and agents are over a limit right now.
//!
//! `LimitState`'s fields are private, and `evaluate` and `without_blocked`
//! read and write them directly — privacy is visible to the defining module
//! and its descendants, never to a sibling — so the three are one file.
//! Grouping them any other way would mean widening the fields to `pub(crate)`.

use crate::limits::plan::{window_over, PlanLimits};
use crate::limits::usage::{agent_limit_usage, period_limit_usage};
use crate::store::Store;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::test_support::{
        agent_limit, set_limits, store_with_hub_rates, store_with_spend, test_provider,
    };

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
}
