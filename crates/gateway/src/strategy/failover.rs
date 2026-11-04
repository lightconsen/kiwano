//! `failover`: the first candidate, in priority order.
//!
//! There is no circuit breaker to route around any more, so the head is simply
//! the first binding. The tail of the plan is what makes this a failover
//! strategy: a request that fails mid-flight is replayed against the next
//! candidate by the data plane (see `selection::plan`).

use crate::error::Result;
use crate::router::AgentRoute;

use super::StrategyEngine;

impl StrategyEngine {
    /// failover: the primary in priority order. The rest of the plan (built in
    /// `selection::plan`) is the replay order.
    pub(crate) async fn select_failover(
        &self,
        route: &AgentRoute,
    ) -> Result<crate::router::UpstreamProvider> {
        Self::primary(route)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StrategyType;
    use crate::strategy::test_support::{blocked, candidate, route, store};

    /// Nothing is remembered about a failed provider: the next request goes to
    /// the head again, rather than around it.
    #[tokio::test]
    async fn a_provider_that_failed_is_still_the_head() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Failover,
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

    /// A billing limit still sinks to the backup — that is a ceiling the user
    /// set, not a failure the gateway observed.
    #[tokio::test]
    async fn a_no_backup_fallback_cannot_resurrect_a_blocked_provider() {
        let s = store();
        let engine = StrategyEngine::new();
        // 'a' is over its limit, so only 'b' is a candidate.
        let r = route(
            StrategyType::Failover,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        let picked = engine.select(&s, &r, None, &blocked("a")).await.unwrap();
        assert_eq!(picked.id, "b", "the fallback must not reach for 'a'");
    }
}
