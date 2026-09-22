//! The breaker registry, and the two questions the engine asks of it.
//!
//! `breaker` is the lazy per-`agent:provider_id` registry; the state machine it
//! hands out is the sibling `circuit_breaker`. `candidate_available` is the
//! route-selection question — may this candidate be picked at all — and `allow`
//! the admission one that has to hold before a real send, with the result fed
//! back through `record`. `breaker_snapshot` is the read-only view the metrics
//! endpoint reports.

use std::sync::Arc;

use crate::router::AgentRoute;

use super::circuit_breaker::{
    self, AllowResult, AttemptOutcome, CircuitBreaker, CircuitBreakerConfig,
};
use super::StrategyEngine;

/// A read-only view of one breaker, for the metrics and status surfaces:
/// whether the circuit is open, and whether it is open *because the key is
/// bad* — the distinction an operator acts on ("swap the key") rather than
/// one they can route around.
#[derive(Debug, Clone)]
pub struct BreakerSnapshot {
    pub key: String,
    pub state: circuit_breaker::CircuitState,
    pub auth_failed: bool,
}

/// Breaker registry key (tech.md §4.7.3): `agent:provider_id`.
fn breaker_key(agent: &str, provider_id: &str) -> String {
    format!("{agent}:{provider_id}")
}

impl StrategyEngine {
    /// Get (or lazily create) the breaker for an (agent, provider) pair.
    async fn breaker(&self, agent: &str, provider_id: &str) -> Arc<CircuitBreaker> {
        let key = breaker_key(agent, provider_id);
        let existing = self
            .breakers
            .lock()
            .expect("breaker registry poisoned")
            .get(&key)
            .cloned();
        if let Some(b) = existing {
            return b;
        }
        let b = Arc::new(CircuitBreaker::new(CircuitBreakerConfig::default()));
        self.breakers
            .lock()
            .expect("breaker registry poisoned")
            .insert(key, b.clone());
        b
    }

    pub(crate) async fn candidate_available(&self, route: &AgentRoute, idx: usize) -> bool {
        let c = &route.candidates[idx];
        self.breaker(&route.agent, &c.id).await.is_available().await
    }

    /// Every breaker's current state, keyed `agent:provider_id` — what the
    /// metrics endpoint reports so an operator can see which candidate the
    /// breakers have taken out. Read-only; nothing here admits or records.
    pub async fn breaker_snapshot(&self) -> Vec<BreakerSnapshot> {
        // Clone the handles out and drop the registry lock before awaiting: the
        // breakers have their own locks, and a std guard must not cross an await.
        let breakers: Vec<(String, Arc<CircuitBreaker>)> = self
            .breakers
            .lock()
            .expect("breaker registry poisoned")
            .iter()
            .map(|(k, b)| (k.clone(), b.clone()))
            .collect();
        let mut snapshot = Vec::with_capacity(breakers.len());
        for (key, breaker) in breakers {
            let stats = breaker.get_stats().await;
            snapshot.push(BreakerSnapshot {
                key,
                state: stats.state,
                auth_failed: stats.auth_failed,
            });
        }
        snapshot.sort_by(|a, b| a.key.cmp(&b.key));
        snapshot
    }

    /// Ask a provider's breaker whether this request may be sent.
    ///
    /// `is_available` is the *route-selection* check and deliberately never
    /// consumes anything; this is the admission check that must run before an
    /// actual send. In HalfOpen it hands out exactly one probe permit, and the
    /// permit has to come back through [`StrategyEngine::record`] — otherwise
    /// every concurrent request probes a provider that is still recovering,
    /// which is precisely what re-opens the circuit.
    pub async fn allow(&self, agent: &str, provider_id: &str) -> AllowResult {
        self.breaker(agent, provider_id).await.allow_request().await
    }

    /// Feed back the result of one real upstream attempt (called by the forward
    /// layer once per attempt). `used_half_open_permit` is the flag from the
    /// matching [`StrategyEngine::allow`] — it releases the probe permit. The
    /// outcome decides which counter the attempt feeds: a served request
    /// clears everything, a rate limit sets its own pause, a refused key
    /// counts on the auth counter, and the rest are breaker failures.
    ///
    /// The bool answer is only meaningful for `AuthRejected`: whether this
    /// rejection is the one that crossed the auth threshold — the moment the
    /// caller should surface "key invalid" where users look.
    pub async fn record(
        &self,
        agent: &str,
        provider_id: &str,
        outcome: AttemptOutcome,
        used_half_open_permit: bool,
    ) -> bool {
        let b = self.breaker(agent, provider_id).await;
        match outcome {
            AttemptOutcome::Served => {
                b.record_success(used_half_open_permit).await;
                false
            }
            AttemptOutcome::RateLimited { cooldown } => {
                b.record_rate_limited(used_half_open_permit, cooldown).await;
                false
            }
            AttemptOutcome::AuthRejected => b.record_auth_rejected(used_half_open_permit).await,
            AttemptOutcome::Failed => {
                b.record_failure(used_half_open_permit).await;
                false
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Install a breaker whose open timeout the test can actually reach: the
    /// engine hands every breaker the production default, and its 60s is not
    /// something a test can wait out.
    fn seed_breaker(engine: &StrategyEngine, agent: &str, provider_id: &str) {
        engine
            .breakers
            .lock()
            .expect("breaker registry poisoned")
            .insert(
                breaker_key(agent, provider_id),
                Arc::new(CircuitBreaker::new(CircuitBreakerConfig {
                    timeout_seconds: 0,
                    ..CircuitBreakerConfig::default()
                })),
            );
    }

    // HalfOpen admits one probe at a time, and the permit returns with the
    // record that follows it. Both halves were missing: `allow` had no caller at
    // all, so HalfOpen admitted every concurrent request at once and the provider
    // under probe took the full load — the exact failure the permit exists to
    // prevent, and the one that re-opens the circuit it had just left.
    #[tokio::test]
    async fn half_open_admits_one_probe_and_takes_it_back_on_record() {
        let engine = StrategyEngine::new();
        seed_breaker(&engine, "claude", "a");

        // Four consecutive failures open it (the default threshold).
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

        // timeout_seconds = 0, so the first admission moves Open → HalfOpen and
        // is itself the probe.
        let probe = engine.allow("claude", "a").await;
        assert!(probe.allowed, "the first probe is admitted");
        assert!(
            probe.used_half_open_permit,
            "and it is the one holding the single permit"
        );

        // Everything else waits for that probe's outcome.
        let concurrent = engine.allow("claude", "a").await;
        assert!(!concurrent.allowed, "a second probe must not be admitted");

        // Handing the flag back is what releases it.
        engine
            .record(
                "claude",
                "a",
                crate::strategy::circuit_breaker::AttemptOutcome::Served,
                probe.used_half_open_permit,
            )
            .await;
        let after = engine.allow("claude", "a").await;
        assert!(
            after.allowed,
            "the permit is free again once the probe has recorded"
        );

        // A probe that fails instead re-opens the circuit. The queue that piled
        // up behind the permit is what this protects: without the permit they
        // would all have been sent, and the recovering provider would have taken
        // the full load it had just failed under.
        engine
            .record(
                "claude",
                "a",
                crate::strategy::circuit_breaker::AttemptOutcome::Failed,
                after.used_half_open_permit,
            )
            .await;
        assert_eq!(
            engine.breaker("claude", "a").await.get_state().await,
            circuit_breaker::CircuitState::Open
        );
    }

    // Asking is not reporting. `allow` must leave health alone, or a burst of
    // refusals would itself drive the breaker outward — the one thing the
    // forward layer is careful not to do when it turns a denial into a response.
    #[tokio::test]
    async fn asking_for_admission_leaves_health_alone() {
        let engine = StrategyEngine::new();
        seed_breaker(&engine, "claude", "a");

        for _ in 0..10 {
            assert!(engine.allow("claude", "a").await.allowed);
        }

        let stats = engine.breaker("claude", "a").await.get_stats().await;
        assert_eq!(stats.total_requests, 0, "an admission is not a request");
        assert_eq!(stats.consecutive_failures, 0);
    }
}
