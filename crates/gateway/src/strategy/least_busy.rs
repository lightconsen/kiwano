//! `least-busy`: the breaker-available candidate with the fewest requests
//! currently in flight, priority order breaking ties.
//!
//! Roundrobin spreads by a shared clock (the weighted ring); this spreads by
//! the actual queue — the reasoning-model case where one candidate is chewing
//! a 90-second completion while another sits idle. Weight plays no part here:
//! the bindings' weights say how the *ring* divides traffic, and a strategy
//! that watches the real queue does not need a second opinion.

use crate::error::Result;
use crate::router::AgentRoute;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use super::StrategyEngine;

/// RAII seat in a provider's in-flight counter: taken when a request starts
/// toward a candidate, released on drop — so an abandoned future or an early
/// return cannot leak a count that would skew every later choice.
pub(crate) struct InflightGuard {
    counter: Arc<AtomicUsize>,
}

impl Drop for InflightGuard {
    fn drop(&mut self) {
        self.counter.fetch_sub(1, Ordering::Relaxed);
    }
}

impl StrategyEngine {
    /// Take one seat at `agent:provider_id`, creating the counter on first use.
    /// Keyed like the breakers (`agent:provider_id`) — the same pair the
    /// candidate list addresses.
    pub(crate) async fn inflight_guard(&self, agent: &str, provider_id: &str) -> InflightGuard {
        let key = format!("{agent}:{provider_id}");
        let counter = {
            let mut map = self.inflight.lock().expect("inflight map poisoned");
            Arc::clone(
                map.entry(key)
                    .or_insert_with(|| Arc::new(AtomicUsize::new(0))),
            )
        };
        counter.fetch_add(1, Ordering::Relaxed);
        InflightGuard { counter }
    }

    /// The candidate with the fewest in-flight requests among those whose
    /// breaker is closed — priority order breaks ties, and when every breaker
    /// is open the request goes to the primary anyway, on the same terms as
    /// `failover`: the gateway does not invent a failure of its own; the real
    /// upstream is the one that gets to say no.
    ///
    /// Draining first, like every sticky strategy: a conversation keeps its
    /// provider while it runs, so the upstream prompt cache it built survives.
    pub(crate) async fn select_least_busy(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Result<crate::router::UpstreamProvider> {
        if let Some(pinned) = self.drain(route, session).await {
            return Ok(pinned);
        }
        let mut best: Option<(usize, usize)> = None; // (in-flight, candidate index)
        for i in 0..route.candidates.len() {
            if !self.candidate_available(route, i).await {
                continue;
            }
            let busy = self
                .inflight
                .lock()
                .expect("inflight map poisoned")
                .get(&format!("{}:{}", route.agent, route.candidates[i].id))
                .map(|c| c.load(Ordering::Relaxed))
                .unwrap_or(0);
            if best.is_none() || busy < best.unwrap().0 {
                best = Some((busy, i));
            }
        }
        let chosen = match best {
            Some((_, i)) => route.candidates[i].clone(),
            None => {
                tracing::warn!(
                    agent = %route.agent,
                    "all candidates circuit-open; falling back to primary"
                );
                Self::primary(route)?
            }
        };
        self.assign_drained(route, session, &chosen);
        Ok(chosen)
    }

    /// The in-flight count a choice would see — a test handle for the arm.
    #[cfg(test)]
    pub(crate) async fn inflight_of(&self, agent: &str, provider_id: &str) -> usize {
        self.inflight
            .lock()
            .expect("inflight map poisoned")
            .get(&format!("{agent}:{provider_id}"))
            .map(|c| c.load(Ordering::Relaxed))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StrategyType;
    use crate::strategy::test_support::{candidate, route, store};

    #[tokio::test]
    async fn least_busy_picks_the_idle_candidate_over_the_busy_one() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::LeastBusy,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        // 'a' is the primary and would win every tie — hold three seats there.
        let guards = [
            engine.inflight_guard("claude", "a").await,
            engine.inflight_guard("claude", "a").await,
            engine.inflight_guard("claude", "a").await,
        ];
        let picked = engine
            .select(&s, &r, None, &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(picked.id, "b", "the candidate actually sitting idle wins");
        drop(guards);
        let picked = engine
            .select(&s, &r, None, &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(
            picked.id, "a",
            "with all seats released, priority order rules"
        );
    }

    #[tokio::test]
    async fn the_guard_releases_the_seat_on_drop() {
        let engine = StrategyEngine::new();
        {
            let _g = engine.inflight_guard("claude", "a").await;
            assert_eq!(engine.inflight_of("claude", "a").await, 1);
        }
        assert_eq!(engine.inflight_of("claude", "a").await, 0, "drop releases");
    }

    /// A running session keeps its provider even when another candidate is
    /// idle — the same drain rule the other sticky strategies follow, so the
    /// upstream prompt cache a conversation built survives the switch.
    #[tokio::test]
    async fn a_running_session_drains_instead_of_switching() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::LeastBusy,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        // The session ran on 'a'; 'b' now sits idle and would win a fresh pick.
        engine.assign_drained(&r, Some("s"), &r.candidates[0]);

        let picked = engine
            .select(&s, &r, Some("s"), &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(picked.id, "a", "the running session drains on 'a'");
        // An un-named request has nothing to drain and picks freely — and with
        // both seats empty, priority order rules.
        let fresh = engine
            .select(&s, &r, None, &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(fresh.id, "a");
    }

    /// Every breaker open: the gateway does not invent a failure of its own —
    /// the request goes to the primary and lets the real upstream say no.
    #[tokio::test]
    async fn all_candidates_open_falls_back_to_the_primary() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::LeastBusy,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        for p in ["a", "b"] {
            for _ in 0..4 {
                engine
                    .record(
                        "claude",
                        p,
                        crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                        false,
                    )
                    .await;
            }
        }
        let picked = engine
            .select(&s, &r, None, &crate::limits::LimitState::default())
            .await
            .unwrap();
        assert_eq!(picked.id, "a", "the primary, not an invented failure");
    }
}
