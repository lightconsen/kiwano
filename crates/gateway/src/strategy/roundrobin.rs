//! `roundrobin`: session stickiness over a weighted ring.
//!
//! A session already running keeps its provider (see `sticky`); a new one picks
//! from the ring, which is the candidate weights expanded by their gcd so that a
//! 3:1 route alternates in the ratio asked for without any per-request
//! bookkeeping. The cursor lives on the engine, so the ring keeps advancing
//! across new sessions rather than restarting at the head.

use crate::error::Result;
use crate::router::AgentRoute;

use super::StrategyEngine;

impl StrategyEngine {
    /// roundrobin: session-granularity stickiness + weighting. Reassigns when the sticky candidate is unavailable (breaker open).
    pub(crate) async fn select_roundrobin(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Result<crate::router::UpstreamProvider> {
        if let Some(pinned) = self.pinned(route, session).await {
            return Ok(pinned);
        }
        let idx = self.weighted_next(route).await?;
        let chosen = route.candidates[idx].clone();
        // Recorded whether or not the request named a session: a client that
        // names none shares the table's `agent\0` slot, and pinning it is what
        // keeps such an agent's traffic on one provider instead of rotating
        // every request through a cold prefix.
        self.assign(route, session, &chosen);
        Ok(chosen)
    }

    /// Weighted ring: next available candidate index (weights expanded into a ring by gcd, the lock
    /// only guards cursor reads/writes and never crosses await); returns 0 (primary fallback) when none is available.
    pub(crate) async fn weighted_next(&self, route: &AgentRoute) -> Result<usize> {
        fn gcd(a: i64, b: i64) -> i64 {
            if b == 0 {
                a
            } else {
                gcd(b, a % b)
            }
        }
        let weights: Vec<i64> = route.candidates.iter().map(|c| c.weight.max(1)).collect();
        let g = weights.iter().copied().fold(0, gcd).max(1);
        let ring: Vec<usize> = weights
            .iter()
            .enumerate()
            .flat_map(|(i, w)| std::iter::repeat_n(i, (*w / g) as usize))
            .collect();
        let ring_len = ring.len().max(1) as u64;
        for _ in 0..ring_len {
            let idx = {
                let mut cursor = self.cursor.lock().expect("cursor poisoned");
                let idx = ring[(*cursor % ring_len) as usize];
                *cursor += 1;
                idx
            };
            if self.candidate_available(route, idx).await {
                return Ok(idx);
            }
        }
        Ok(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::StrategyType;
    use crate::strategy::sticky::STICKY_CAPACITY;
    use crate::strategy::test_support::{candidate, route, store};

    #[tokio::test]
    async fn roundrobin_is_sticky_per_session_and_weighted_across() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Roundrobin,
            vec![candidate("a", 3, None), candidate("b", 1, None)],
        );

        // Same session stays sticky
        let first = engine
            .select(
                &s,
                &r,
                Some("sess-1"),
                &crate::limits::LimitState::default(),
            )
            .await
            .unwrap()
            .id;
        for _ in 0..5 {
            assert_eq!(
                engine
                    .select(
                        &s,
                        &r,
                        Some("sess-1"),
                        &crate::limits::LimitState::default()
                    )
                    .await
                    .unwrap()
                    .id,
                first
            );
        }

        // New sessions advance on the weighted ring: with weights 3:1, 4 new sessions should pick a 3 times, b once
        let mut counts = std::collections::HashMap::new();
        for i in 0..4 {
            let id = engine
                .select(
                    &s,
                    &r,
                    Some(&format!("sess-{i}")),
                    &crate::limits::LimitState::default(),
                )
                .await
                .unwrap()
                .id;
            *counts.entry(id).or_insert(0) += 1;
        }
        assert_eq!(counts.get("a"), Some(&3));
        assert_eq!(counts.get("b"), Some(&1));
    }

    /// The sticky table is a cache, and a cache with no ceiling is a leak: one
    /// entry per session, held for the life of the process. It keeps the
    /// sessions that are still talking and forgets the quiet ones.
    #[tokio::test]
    async fn roundrobin_keeps_the_table_bounded_without_dropping_a_live_session() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Roundrobin,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        let limits = crate::limits::LimitState::default();

        // One conversation that keeps talking…
        let hot = engine
            .select(&s, &r, Some("hot"), &limits)
            .await
            .unwrap()
            .id;
        // …while enough others come and go to push the table past its ceiling.
        for i in 0..(STICKY_CAPACITY + 512) {
            engine
                .select(&s, &r, Some(&format!("cold-{i}")), &limits)
                .await
                .unwrap();
            // Every so often the hot session speaks again, which is what keeps
            // it recent in the table.
            if i % 64 == 0 {
                assert_eq!(
                    engine
                        .select(&s, &r, Some("hot"), &limits)
                        .await
                        .unwrap()
                        .id,
                    hot,
                    "a session that is still talking keeps its provider"
                );
            }
        }

        let held = engine.sticky.lock().expect("sticky map poisoned").map.len();
        assert!(
            held <= STICKY_CAPACITY,
            "the table grew past its ceiling: {held}"
        );
        assert_eq!(
            engine
                .select(&s, &r, Some("hot"), &limits)
                .await
                .unwrap()
                .id,
            hot,
            "and it survived the eviction of everything around it"
        );
    }

    #[tokio::test]
    async fn roundrobin_reassigns_when_sticky_candidate_opens() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Roundrobin,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        let sticky = engine
            .select(&s, &r, Some("s"), &crate::limits::LimitState::default())
            .await
            .unwrap()
            .id;

        // Sticky candidate trips its breaker → same session is reassigned to another
        for _ in 0..4 {
            engine
                .record(
                    "claude",
                    &sticky,
                    crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                    false,
                )
                .await;
        }
        let reassigned = engine
            .select(&s, &r, Some("s"), &crate::limits::LimitState::default())
            .await
            .unwrap()
            .id;
        assert_ne!(reassigned, sticky);
    }
}
