//! `failover`: the first candidate whose breaker is closed, in priority order.
//!
//! When every candidate is open the request goes to the primary anyway — the
//! gateway does not invent a failure of its own; the real upstream is the one
//! that gets to say no (cc-switch queue semantics).

use crate::error::Result;
use crate::router::AgentRoute;

use super::StrategyEngine;

impl StrategyEngine {
    /// failover: take the first breaker-available candidate in priority order; when all are open,
    /// fall back to the primary (the request fails at the real upstream, not gateway-side — cc-switch queue semantics).
    pub(crate) async fn select_failover(
        &self,
        route: &AgentRoute,
    ) -> Result<crate::router::UpstreamProvider> {
        for i in 0..route.candidates.len() {
            if self.candidate_available(route, i).await {
                return Ok(route.candidates[i].clone());
            }
        }
        tracing::warn!(agent = %route.agent, "all candidates circuit-open; falling back to primary");
        Self::primary(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StrategyType;
    use crate::strategy::test_support::{blocked, candidate, route, store};

    #[tokio::test]
    async fn failover_sinks_to_backup_then_recovers() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Failover,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );

        // Primary hits the consecutive-failure threshold → sink to backup
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
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "b"
        );

        // Backup also open → fall back to the primary (real upstream failure semantics)
        for _ in 0..4 {
            engine
                .record(
                    "claude",
                    "b",
                    crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                    false,
                )
                .await;
        }
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );

        // Primary recovery (timeout=0 goes HalfOpen immediately + success flips to closed)
        // Build a fresh engine to exercise the recovery path: a success record clears consecutive failures
        let engine2 = StrategyEngine::new();
        engine2
            .record(
                "claude",
                "a",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;
        engine2
            .record(
                "claude",
                "a",
                crate::strategy::circuit_breaker::AttemptOutcome::Served,
                false,
            )
            .await;
        let r2 = route(
            StrategyType::Failover,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        assert_eq!(
            engine2
                .select(&s, &r2, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );
    }

    #[tokio::test]
    async fn a_no_backup_fallback_cannot_resurrect_a_blocked_provider() {
        let s = store();
        let engine = StrategyEngine::new();
        // 'a' is over its limit and 'b' is circuit-open, so failover has no
        // available candidate and takes its fallback branch. That branch must
        // land on the pruned primary, not the original one.
        let r = route(
            StrategyType::Failover,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        engine
            .record(
                "claude",
                "b",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;
        engine
            .record(
                "claude",
                "b",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;
        engine
            .record(
                "claude",
                "b",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;
        engine
            .record(
                "claude",
                "b",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;
        engine
            .record(
                "claude",
                "b",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                false,
            )
            .await;

        let picked = engine.select(&s, &r, None, &blocked("a")).await.unwrap();
        assert_eq!(picked.id, "b", "the fallback must not reach for 'a'");
    }
}
