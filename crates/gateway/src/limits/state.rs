//! Which providers and agents are over a limit right now.
//!
//! `LimitState`'s fields are private, and `evaluate` and `without_blocked`
//! read and write them directly — privacy is visible to the defining module
//! and its descendants, never to a sibling — so the three are one file.
//! Grouping them any other way would mean widening the fields to `pub(crate)`.

use crate::limits::plan::{window_over, PlanLimits};
use crate::limits::usage::{agent_limit_usage, client_key_limit_usage, period_limit_usage};
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
    /// Client keys over their own ceiling, keyed by the key's handle. A third
    /// map rather than a special case of the other two: an agent being over and
    /// a *client* of that agent being over are different claims, and the client
    /// one is checked first — it is the caller's own contract.
    keys_over: std::collections::HashMap<String, BlockReason>,
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

    /// Whether this client key has spent its own allowance. Keyed by the handle
    /// (`ClientKey::id`), which is what a metered row carries.
    pub fn key_blocked(&self, key_id: &str) -> Option<&BlockReason> {
        self.keys_over.get(key_id)
    }

    pub fn keys_over_entries(&self) -> impl Iterator<Item = (&str, &BlockReason)> {
        self.keys_over.iter().map(|(k, r)| (k.as_str(), r))
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
            keys_over: std::collections::HashMap::new(),
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
            keys_over: std::collections::HashMap::new(),
            tz_offset_minutes: 0,
        }
    }

    /// A snapshot with exactly these client keys over their own ceiling.
    #[cfg(test)]
    pub fn with_keys_over(reasons: impl IntoIterator<Item = (String, BlockReason)>) -> Self {
        LimitState {
            blocked: std::collections::HashMap::new(),
            agents_over: std::collections::HashMap::new(),
            keys_over: reasons.into_iter().collect(),
            tz_offset_minutes: 0,
        }
    }
}

/// Seconds until the window a block names ends, or `None` when there is nothing
/// to wait for.
///
/// Derived from [`crate::limits::period_span_secs`] — the same boundary
/// `period_start` measures against, on the same clock — so a `Retry-After`
/// cannot disagree with the limit it explains. A provider's block names no
/// window (`BlockReason::Spend { window: None }`) and an `all` window has no
/// reset at all, so both answer `None` rather than a fabricated hour.
pub fn retry_after_for(reason: &BlockReason, tz_offset_minutes: i64) -> Option<u64> {
    let window = match reason {
        // `all` is the stored spelling of "no reset" — the very thing that makes
        // it a ceiling worth having — so there is no end to wait for. It has to
        // be named here because `period_start` reads an unrecognised window as
        // monthly rather than as open-ended.
        BlockReason::Spend {
            window: Some(w), ..
        } if w != "all" => w.as_str(),
        // A plan window is a vendor's own rolling window with its own
        // `resets_at`; what we know here is only that it is over, and inventing
        // a duration for it would be worse than saying nothing.
        _ => return None,
    };
    let now = chrono::Utc::now().timestamp();
    let (_, end) = crate::limits::period_span_secs(now, Some(window), tz_offset_minutes)?;
    Some((end - now).max(1) as u64)
}

/// Whether a client key may name this model.
///
/// Case-insensitive, which is the house's model-matching convention (the
/// declared-price path keys model ids the same way). A key with no allowlist
/// allows everything — the reading every row that predates the column gets.
///
/// `model: None` with an allowlist in force is a refusal, not a wave-through: an
/// allowance that cannot be checked is not an allowance, and the alternative
/// reading would make "send a body we cannot parse" a way around the list.
pub fn check_model(
    client: &crate::store::ClientKey,
    model: Option<&str>,
) -> Result<(), crate::error::GatewayError> {
    let allowed = crate::limits::allowlist(client.model_allow.as_deref());
    if allowed.is_empty() {
        return Ok(());
    }
    let Some(model) = model else {
        return Err(crate::error::GatewayError::ModelUnverifiable {
            agent: client.agent.clone(),
        });
    };
    let wanted = model.to_ascii_lowercase();
    if allowed.iter().any(|a| a.eq_ignore_ascii_case(&wanted)) {
        return Ok(());
    }
    Err(crate::error::GatewayError::ModelNotAllowed {
        agent: client.agent.clone(),
        model: model.to_string(),
        allowed,
    })
}

/// Take the providers a client key may not use out of a route's candidates.
///
/// The provider half of a key's contract, and a pruning exactly like
/// [`without_blocked`] rather than a check inside the selection: the same
/// reasoning applies, which is that `timewindow` and the no-backup fallbacks
/// index `candidates` directly and would otherwise serve a provider this client
/// was never granted.
///
/// `single` is the exception, and for the reason [`without_blocked`] records: it
/// means "this one provider, deliberately", so a disallowed primary is a refusal
/// rather than a promotion of the backup the user chose not to fail over to.
pub fn route_without_disallowed_providers<'r>(
    route: &'r crate::router::AgentRoute,
    client: &crate::store::ClientKey,
) -> Result<std::borrow::Cow<'r, crate::router::AgentRoute>, crate::error::GatewayError> {
    let allowed = crate::limits::allowlist(client.provider_allow.as_deref());
    if allowed.is_empty() {
        return Ok(std::borrow::Cow::Borrowed(route));
    }
    let permits = |id: &str| allowed.iter().any(|a| a.eq_ignore_ascii_case(id));
    if matches!(route.strategy, crate::store::StrategyType::Single) {
        let primary = route.candidates.first();
        if primary.is_some_and(|c| !permits(&c.id)) {
            return Err(crate::error::GatewayError::ProviderNotAllowed {
                agent: client.agent.clone(),
                allowed,
            });
        }
        return Ok(std::borrow::Cow::Borrowed(route));
    }
    let kept: Vec<_> = route
        .candidates
        .iter()
        .filter(|c| permits(&c.id))
        .cloned()
        .collect();
    if kept.len() == route.candidates.len() {
        return Ok(std::borrow::Cow::Borrowed(route));
    }
    if kept.is_empty() {
        return Err(crate::error::GatewayError::ProviderNotAllowed {
            agent: client.agent.clone(),
            allowed,
        });
    }
    Ok(std::borrow::Cow::Owned(crate::router::AgentRoute {
        candidates: kept,
        ..route.clone()
    }))
}

/// A key's whole contract, checked where the key is resolved and before any
/// ceiling the agent carries.
///
/// The order is the point: a client that is out of its own budget or asking for
/// a model it was not granted never reaches the agent's accounting, so the two
/// refusals stay distinguishable in the log and the more specific one wins.
pub fn check_client_key(
    limits: &LimitState,
    client: &crate::store::ClientKey,
    model: Option<&str>,
) -> Result<(), crate::error::GatewayError> {
    if let Some(reason) = limits.key_blocked(&client.id) {
        return Err(crate::error::GatewayError::ClientOverLimit {
            agent: client.agent.clone(),
            key_id: client.id.clone(),
            reason: reason.describe(),
            retry_after_secs: retry_after_for(reason, limits.tz_offset_minutes()),
        });
    }
    check_model(client, model)
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
    // A client key's own windows, read the same way and for the same reason: the
    // request path stays a snapshot lookup. A key with no windows never appears
    // here, which is every key that predates the table.
    let mut keys_over = std::collections::HashMap::new();
    match store.list_client_key_limits() {
        Ok(limits) => {
            for limit in limits {
                match client_key_limit_usage(store, &limit) {
                    Ok(Some(pl)) if pl.used >= pl.limit => {
                        keys_over.entry(limit.key_id.clone()).or_insert_with(|| {
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
                        key_id = %limit.key_id,
                        period = %limit.period,
                        error = %e,
                        "client-key limit not evaluated this tick; the key stays usable"
                    ),
                }
            }
        }
        Err(e) => tracing::warn!(error = %e, "client-key limits unreadable; none enforced"),
    }
    LimitState {
        blocked,
        agents_over,
        keys_over,
        tz_offset_minutes,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::limits::test_support::{
        agent_limit, client_key, record_key_row, set_key_limits, set_limits, store_with_hub_rates,
        store_with_spend, test_provider,
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
                client_key_id: None,
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

    /// A client key's windows are the credential's, not the agent's: two keys of
    /// one agent spend against two budgets, and the agent's own ceiling is a
    /// third thing again.
    #[test]
    fn a_client_key_is_over_when_any_of_its_windows_is_and_its_siblings_are_not() {
        let (_dir, store) = store_with_hub_rates(r#"{"USD":1.0}"#);
        store.insert_provider(&test_provider("ds-1")).unwrap();

        let laptop = client_key(&store, "claude", "laptop");
        let desktop = client_key(&store, "claude", "desktop");
        assert_ne!(laptop.id, desktop.id, "two keys, two handles");

        set_key_limits(&store, &laptop.id, &[("day", 3.0, Some("requests"))]);
        set_key_limits(&store, &desktop.id, &[("day", 100.0, Some("requests"))]);

        for _ in 0..3 {
            record_key_row(&store, "claude", &laptop.id, "ds-1");
        }
        // At the ceiling is over it (the agent ceilings read the same way), and
        // the sibling key — whose own window is nowhere near — is untouched.
        let state = evaluate(&store);
        let reason = state
            .key_blocked(&laptop.id)
            .expect("3 requests is at a daily ceiling of 3");
        assert!(
            reason.describe().contains("this day"),
            "the refusal names the window that tripped: {}",
            reason.describe()
        );
        assert!(state.key_blocked(&desktop.id).is_none());
        assert!(state.agent_blocked("claude").is_none());

        // A key that has spent nothing is never blocked, even under a ceiling.
        let idle = client_key(&store, "codex", "one");
        set_key_limits(&store, &idle.id, &[("day", 1.0, Some("requests"))]);
        assert!(evaluate(&store).key_blocked(&idle.id).is_none());
    }

    /// The wait a 429 can name comes from the same boundary the window is
    /// measured against, and windows with no reset name none.
    #[test]
    fn retry_after_is_the_rest_of_the_window_or_nothing() {
        let day = BlockReason::Spend {
            used: 5.0,
            limit: 5.0,
            unit: "requests".into(),
            window: Some("day".into()),
        };
        let secs = retry_after_for(&day, 0).expect("a day rolls over");
        assert!((1..=86_400).contains(&secs), "{secs}");

        // A month is a month, so the wait is longer than a day's — the number is
        // derived, not a constant.
        let monthly = BlockReason::Spend {
            used: 5.0,
            limit: 5.0,
            unit: "requests".into(),
            window: Some("monthly".into()),
        };
        assert!(retry_after_for(&monthly, 0).unwrap() >= secs);

        // `all` never resets, and a provider's ceiling names no window at all.
        let all = BlockReason::Spend {
            used: 5.0,
            limit: 5.0,
            unit: "requests".into(),
            window: Some("all".into()),
        };
        assert_eq!(retry_after_for(&all, 0), None);
        let provider = BlockReason::Spend {
            used: 5.0,
            limit: 5.0,
            unit: "requests".into(),
            window: None,
        };
        assert_eq!(retry_after_for(&provider, 0), None);
    }

    /// The three refusals a credential can produce, in the order they are asked.
    #[test]
    fn a_keys_own_contract_is_checked_before_the_agents() {
        use crate::error::GatewayError;
        use crate::store::ClientKey;

        let key = |model_allow: Option<&str>| ClientKey {
            id: "ck-1".into(),
            key: "kw-ag-claude-1".into(),
            agent: "claude".into(),
            label: None,
            model_allow: model_allow.map(str::to_string),
            provider_allow: None,
            created_at: "t0".into(),
            updated_at: "t0".into(),
        };

        // Over the key's own ceiling: the wait rides the error.
        let limits = LimitState::with_keys_over([(
            "ck-1".to_string(),
            BlockReason::Spend {
                used: 5.0,
                limit: 5.0,
                unit: "requests".into(),
                window: Some("day".into()),
            },
        )]);
        match check_client_key(&limits, &key(None), Some("gpt-5.5")) {
            Err(GatewayError::ClientOverLimit {
                retry_after_secs, ..
            }) => assert!(retry_after_secs.is_some()),
            other => panic!("expected ClientOverLimit, got {other:?}"),
        }

        // No ceiling on the key, an allowlist that names the model: allowed.
        let none = LimitState::default();
        assert!(check_client_key(&none, &key(Some(r#"["gpt-5.5"]"#)), Some("GPT-5.5")).is_ok());
        // Case-insensitive, like every other model match in the tree.
        assert!(check_client_key(&none, &key(Some(r#"["gpt-5.5"]"#)), Some("gpt-5.5")).is_ok());

        // A model the list does not name, and a request that names none at all.
        assert!(matches!(
            check_client_key(&none, &key(Some(r#"["gpt-5.5"]"#)), Some("opus")),
            Err(GatewayError::ModelNotAllowed { .. })
        ));
        assert!(matches!(
            check_client_key(&none, &key(Some(r#"["gpt-5.5"]"#)), None),
            Err(GatewayError::ModelUnverifiable { .. })
        ));

        // No list is no restriction, and an unreadable one is read as absent —
        // the migration's promise for every row that predates the column.
        assert!(check_client_key(&none, &key(None), Some("anything")).is_ok());
        assert!(check_client_key(&none, &key(Some("not json")), Some("anything")).is_ok());
    }

    /// A provider allowlist prunes like a ceiling does — except for `single`,
    /// where the one provider is the strategy's whole meaning.
    #[test]
    fn a_provider_allowlist_prunes_and_single_is_refused_rather_than_promoted() {
        use crate::error::GatewayError;
        use crate::store::ClientKey;

        let key = |provider_allow: Option<&str>| ClientKey {
            id: "ck-1".into(),
            key: "kw-ag-claude-1".into(),
            agent: "claude".into(),
            label: None,
            model_allow: None,
            provider_allow: provider_allow.map(str::to_string),
            created_at: "t0".into(),
            updated_at: "t0".into(),
        };
        let route = |strategy| {
            crate::strategy::test_support::route(
                strategy,
                vec![
                    crate::strategy::test_support::candidate("a", 1, None),
                    crate::strategy::test_support::candidate("b", 1, None),
                ],
            )
        };

        // No list: the route comes back borrowed, untouched.
        let r = route(crate::store::StrategyType::Failover);
        assert!(matches!(
            route_without_disallowed_providers(&r, &key(None)),
            Ok(std::borrow::Cow::Borrowed(_))
        ));

        // A list that excludes the primary leaves the backup as the candidate.
        let pruned =
            route_without_disallowed_providers(&r, &key(Some(r#"["b"]"#))).expect("something left");
        assert_eq!(
            pruned
                .candidates
                .iter()
                .map(|c| c.id.as_str())
                .collect::<Vec<_>>(),
            vec!["b"]
        );

        // A list that excludes everything is a refusal, not an empty plan.
        assert!(matches!(
            route_without_disallowed_providers(&r, &key(Some(r#"["z"]"#))),
            Err(GatewayError::ProviderNotAllowed { .. })
        ));

        // `single` names one provider on purpose: refusing beats serving the
        // backup the user chose not to fail over to.
        let single = route(crate::store::StrategyType::Single);
        assert!(matches!(
            route_without_disallowed_providers(&single, &key(Some(r#"["b"]"#))),
            Err(GatewayError::ProviderNotAllowed { .. })
        ));
        assert!(
            route_without_disallowed_providers(&single, &key(Some(r#"["a"]"#))).is_ok(),
            "the provider it does name is still served"
        );
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
