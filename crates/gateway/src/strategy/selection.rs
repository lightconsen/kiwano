//! The dispatch: one request in, the ordered candidates it may be served by out.
//!
//! `plan` is the only place a `StrategyType` becomes a provider, and the only
//! place the agent's own ceiling and the pruned candidate list are applied. Each
//! arm it dispatches to lives with the state it reads; what stays here is the
//! part that holds for every strategy — the head `select` returns, and the
//! priority-order tail a request that fails mid-flight is replayed against.

use crate::error::{GatewayError, Result};
use crate::router::AgentRoute;
use crate::store::{Store, StrategyType};

use super::StrategyEngine;

impl StrategyEngine {
    /// Single strategy / fallback: the primary.
    pub(crate) fn primary(route: &AgentRoute) -> Result<crate::router::UpstreamProvider> {
        route
            .candidates
            .first()
            .cloned()
            .ok_or_else(|| GatewayError::NoBinding(route.agent.clone()))
    }

    /// Select a Provider for one request according to the strategy.
    pub async fn select(
        &self,
        store: &Store,
        route: &AgentRoute,
        session: Option<&str>,
        limits: &crate::limits::LimitState,
    ) -> Result<crate::router::UpstreamProvider> {
        Ok(self
            .plan(store, route, session, limits)
            .await?
            .into_iter()
            .next()
            .expect("a plan always names at least one provider"))
    }

    /// The ordered candidates one request may be served by, best first.
    ///
    /// `select` is this list's head. The tail is what a request that fails
    /// mid-flight is replayed against, so the client never sees a failure the
    /// gateway could have absorbed by asking somebody else.
    ///
    /// Order after the head is plain priority order, deliberately not a second
    /// strategy decision: the strategy has had its say about who goes first,
    /// and a tail that re-weighted the rest would make "which provider served
    /// this request" unanswerable from the config.
    pub async fn plan(
        &self,
        store: &Store,
        route: &AgentRoute,
        session: Option<&str>,
        limits: &crate::limits::LimitState,
    ) -> Result<Vec<crate::router::UpstreamProvider>> {
        if route.candidates.is_empty() {
            return Err(GatewayError::NoBinding(route.agent.clone()));
        }
        // The agent's own ceiling, before any strategy has a say — and before the
        // `single` branch below, which is the point: `single` opts out of every
        // *provider* ceiling on purpose (one provider, no fallback), but choosing
        // it says nothing about how much the agent may spend. A limit that
        // `single` could slip past would be no limit at all, since `single` is
        // the default.
        if let Some(reason) = limits.agent_blocked(&route.agent) {
            return Err(GatewayError::AgentOverLimit {
                agent: route.agent.clone(),
                reason: reason.describe(),
            });
        }
        // `single` is "this one provider, no failover" — the backup is bound
        // but deliberately not a fallback. A limit does not promote it: the one
        // provider is unusable, which is a failure rather than a reason to
        // switch. So `single` opts out of the pruning below and answers for its
        // own provider alone, which also makes its plan one entry long: only
        // choosing a strategy that offers a fallback buys one.
        if matches!(route.strategy, StrategyType::Single) {
            if let Some(reason) = limits.blocked(&route.candidates[0].id) {
                return Err(GatewayError::AllOverLimit {
                    agent: route.agent.clone(),
                    reasons: format!("{} ({})", route.candidates[0].name, reason.describe()),
                });
            }
            return Ok(vec![Self::primary(route)?]);
        }
        // Every other strategy: providers over a billing limit are not
        // candidates at all. Pruning the list here, rather than checking inside
        // `candidate_available`, is what makes it hold for all of them —
        // `candidate_available` is consulted by failover/roundrobin/quota only,
        // while `timewindow` indexes `candidates` directly — and so do the
        // no-backup fallbacks (`Self::primary`, `weighted_next`), which would
        // otherwise serve a provider the user has already spent out.
        let pruned = crate::limits::without_blocked(route, limits);
        let usable = pruned.as_ref().unwrap_or(route);
        if usable.candidates.is_empty() {
            // Name what is blocked and by how much, so the failure says what to
            // do about it rather than just refusing.
            let reasons = route
                .candidates
                .iter()
                .filter_map(|c| {
                    limits
                        .blocked(&c.id)
                        .map(|r| format!("{} ({})", c.name, r.describe()))
                })
                .collect::<Vec<_>>()
                .join("; ");
            return Err(GatewayError::AllOverLimit {
                agent: route.agent.clone(),
                reasons,
            });
        }
        let first = match usable.strategy {
            StrategyType::Single => Self::primary(usable)?,
            StrategyType::Failover => self.select_failover(usable).await?,
            StrategyType::Roundrobin => self.select_roundrobin(usable, session).await?,
            StrategyType::Timewindow => {
                self.select_timewindow(usable, session, limits.tz_offset_minutes())
                    .await?
            }
            StrategyType::Quota => self.select_quota(store, usable, limits, session).await?,
            StrategyType::LeastBusy => self.select_least_busy(usable, session).await?,
        };
        let mut plan = Vec::with_capacity(usable.candidates.len());
        plan.push(first.clone());
        for c in &usable.candidates {
            if c.id != first.id {
                plan.push(c.clone());
            }
        }
        Ok(plan)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::strategy::test_support::{blocked, candidate, route, store};

    #[tokio::test]
    async fn single_always_returns_primary() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Single,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        for _ in 0..3 {
            assert_eq!(
                engine
                    .select(&s, &r, None, &crate::limits::LimitState::default())
                    .await
                    .unwrap()
                    .id,
                "a"
            );
        }
    }

    /// Candidate ids in plan order.
    fn ids(plan: &[crate::router::UpstreamProvider]) -> Vec<&str> {
        plan.iter().map(|c| c.id.as_str()).collect()
    }

    // The plan's head is what `select` returns; its tail is the replay order a
    // failed request walks. Order after the head is priority, not a second
    // strategy decision — and `single` has no tail at all, because its backup is
    // bound rather than a fallback.
    #[tokio::test]
    async fn the_plan_lists_the_replay_order_and_single_has_no_tail() {
        let engine = StrategyEngine::new();
        let s = store();
        let limits = crate::limits::LimitState::default();

        let three = route(
            StrategyType::Failover,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, None),
                candidate("c", 1, None),
            ],
        );
        assert_eq!(
            ids(&engine.plan(&s, &three, None, &limits).await.unwrap()),
            vec!["a", "b", "c"]
        );

        // Open the head's breaker: the plan starts at the next available
        // candidate, and still names every candidate exactly once.
        for _ in 0..4 {
            engine
                .record(
                    "claude",
                    "a",
                    crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                    false,
                )
                .await;
        }
        assert_eq!(
            ids(&engine.plan(&s, &three, None, &limits).await.unwrap()),
            vec!["b", "a", "c"],
            "the fallback order stays priority order, with the pick moved to the front"
        );

        // Bound but not a fallback: `single` answers with one provider, so the
        // data plane has nothing to replay against.
        let one = route(
            StrategyType::Single,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        assert_eq!(
            ids(&engine.plan(&s, &one, None, &limits).await.unwrap()),
            vec!["a"]
        );
    }

    // The ceiling `single` cannot slip past. `single` deliberately opts out of the
    // provider-level ceilings — one provider, no fallback — but that is a
    // statement about failover, not about how much the agent may spend. It is also
    // the default strategy, so a limit it ignored would be no limit at all.
    #[tokio::test]
    async fn an_agent_over_its_own_limit_is_refused_under_every_strategy() {
        let engine = StrategyEngine::new();
        let s = store();
        let over = crate::limits::LimitState::with_agents_over([(
            "claude".to_string(),
            crate::limits::BlockReason::Spend {
                used: 12.0,
                limit: 10.0,
                unit: "CNY".into(),
                window: None,
            },
        )]);

        for strategy in [
            StrategyType::Single,
            StrategyType::Failover,
            StrategyType::Roundrobin,
        ] {
            let r = route(
                strategy,
                vec![candidate("a", 1, None), candidate("b", 1, None)],
            );
            let err = engine.plan(&s, &r, None, &over).await.unwrap_err();
            assert!(
                matches!(err, GatewayError::AgentOverLimit { .. }),
                "{strategy:?} let an over-limit agent through: {err}"
            );
        }

        // And under the ceiling it routes exactly as before, so what gates is the
        // limit being reached, not the existence of a limit.
        let under = crate::limits::LimitState::default();
        let r = route(StrategyType::Single, vec![candidate("a", 1, None)]);
        assert_eq!(
            ids(&engine.plan(&s, &r, None, &under).await.unwrap()),
            vec!["a"]
        );
    }

    #[tokio::test]
    async fn every_strategy_routes_around_a_blocked_provider() {
        let s = store();
        let engine = StrategyEngine::new();
        let limits = blocked("a");
        // `candidate_available` only guards failover/roundrobin/quota, and
        // every strategy has a "fall back to the primary" branch; pruning the
        // list is what covers them. `single` is absent on purpose — it is the
        // one strategy that does not fail over, and answers for itself.
        for strategy in [
            StrategyType::Failover,
            StrategyType::Roundrobin,
            StrategyType::Timewindow,
            StrategyType::Quota,
        ] {
            let r = route(
                strategy,
                vec![candidate("a", 1, None), candidate("b", 1, None)],
            );
            let picked = engine.select(&s, &r, None, &limits).await.unwrap();
            assert_eq!(picked.id, "b", "{strategy:?} served a blocked provider");
        }
    }

    #[tokio::test]
    async fn a_single_strategy_fails_rather_than_promoting_the_backup() {
        let s = store();
        let engine = StrategyEngine::new();
        // P1 is the primary and P2 is bound, but `single` means P1 only — the
        // backup is not a fallback, and a limit does not make it one.
        let r = route(
            StrategyType::Single,
            vec![candidate("p1", 1, None), candidate("p2", 1, None)],
        );
        let err = engine
            .select(&s, &r, None, &blocked("p1"))
            .await
            .unwrap_err();
        assert!(
            matches!(err, GatewayError::AllOverLimit { .. }),
            "expected the hard wall, got {err}"
        );
        assert!(
            err.to_string().contains("p1"),
            "and it names the one it refuses"
        );

        // Same route, same limit state, but failover: this one does move over.
        let r2 = route(
            StrategyType::Failover,
            vec![candidate("p1", 1, None), candidate("p2", 1, None)],
        );
        assert_eq!(
            engine
                .select(&s, &r2, None, &blocked("p1"))
                .await
                .unwrap()
                .id,
            "p2"
        );
    }

    #[tokio::test]
    async fn all_candidates_blocked_is_a_hard_wall() {
        let s = store();
        let engine = StrategyEngine::new();
        let r = route(
            StrategyType::Single,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        let limits = crate::limits::LimitState::from_reasons([
            (
                "a".to_string(),
                crate::limits::BlockReason::Spend {
                    used: 31.0,
                    limit: 30.0,
                    unit: "CNY".into(),
                    window: None,
                },
            ),
            (
                "b".to_string(),
                crate::limits::BlockReason::Spend {
                    used: 11.0,
                    limit: 10.0,
                    unit: "CNY".into(),
                    window: None,
                },
            ),
        ]);

        let err = engine.select(&s, &r, None, &limits).await.unwrap_err();
        let msg = err.to_string();
        assert!(
            msg.contains("over its limit") && msg.contains("31.00 of 30.00"),
            "the failure should say which limit was hit: {msg}"
        );
        // Distinct from NoBinding: something is bound, it just must not be spent on.
        assert!(
            matches!(err, GatewayError::AllOverLimit { .. }),
            "expected AllOverLimit, got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_single_strategy_with_an_unblocked_provider_is_untouched() {
        let s = store();
        let engine = StrategyEngine::new();
        let r = route(StrategyType::Single, vec![candidate("a", 1, None)]);
        // Nothing blocked anywhere: the pruning path must not alter behaviour.
        let picked = engine
            .select(&s, &r, None, &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(picked.id, "a");
    }
}
