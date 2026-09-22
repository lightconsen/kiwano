//! Circuit breaker (tech.md §4.7 failover infrastructure).
//!
//! This file is derived from cc-switch (MIT License).
//! Source: src-tauri/src/proxy/circuit_breaker.rs
//! Copied on 2026-09-07.
//! Modified for Kiwano: the `log_codes` symbols and the `AppProxyConfig` conversion
//! were removed (Kiwano logs directly via tracing; thresholds come from the strategy
//! engine defaults), the rest of the state machine
//! (Closed/Open/HalfOpen, HalfOpen probe permits, hot-reload config) is preserved line by line.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;

/// Circuit breaker state
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CircuitState {
    /// Closed - operating normally
    Closed,
    /// Open - breaker tripped, rejecting requests
    Open,
    /// HalfOpen - attempting recovery, allowing limited requests through
    HalfOpen,
}

impl std::fmt::Display for CircuitState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CircuitState::Closed => write!(f, "closed"),
            CircuitState::Open => write!(f, "open"),
            CircuitState::HalfOpen => write!(f, "half_open"),
        }
    }
}

/// Circuit breaker configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CircuitBreakerConfig {
    /// Failure threshold - consecutive failures before the breaker opens
    pub failure_threshold: u32,
    /// Success threshold - successes in HalfOpen state before the breaker closes
    pub success_threshold: u32,
    /// Timeout - how long after opening before attempting HalfOpen (seconds)
    pub timeout_seconds: u64,
    /// Error rate threshold - breaker opens when the error rate exceeds this (0.0-1.0)
    pub error_rate_threshold: f64,
    /// Minimum requests - minimum sample size before computing the error rate
    pub min_requests: u32,
    /// Consecutive auth rejections before the breaker opens with the auth-failed
    /// mark — a key the provider keeps refusing is not a "failure" to count
    /// against the error rate, it is a state of its own.
    pub auth_failure_threshold: u32,
}

impl Default for CircuitBreakerConfig {
    fn default() -> Self {
        Self {
            failure_threshold: 4,
            success_threshold: 2,
            timeout_seconds: 60,
            error_rate_threshold: 0.6,
            min_requests: 10,
            auth_failure_threshold: 2,
        }
    }
}

/// What one upstream attempt came back as — the classification the breaker
/// needs to keep "the provider is broken" apart from "the provider is busy"
/// and "the key is wrong". A 429 and a 500 both read as non-success at the
/// header, but they call for different treatment: a rate limit passes on its
/// own after a short wait, while a broken provider needs the circuit opened.
#[derive(Debug, Clone, Copy)]
pub enum AttemptOutcome {
    /// 2xx — the provider served the request.
    Served,
    /// 429 — the provider is rate-limiting, and told us (or a default) how
    /// long to wait. Never counts as a failure: a busy provider is healthy.
    RateLimited { cooldown: Duration },
    /// 401 — the key the provider was given does not work. Counted on its own
    /// auth counter, which opens the breaker with the auth-failed mark.
    AuthRejected,
    /// Everything else that is not a success — transport errors, 408, 5xx.
    Failed,
}

/// Circuit breaker instance
pub struct CircuitBreaker {
    /// Current state
    state: Arc<RwLock<CircuitState>>,
    /// Consecutive failure count
    consecutive_failures: Arc<AtomicU32>,
    /// Consecutive success count (HalfOpen state)
    consecutive_successes: Arc<AtomicU32>,
    /// Total request count
    total_requests: Arc<AtomicU32>,
    /// Failed request count
    failed_requests: Arc<AtomicU32>,
    /// Time of the last opening
    last_opened_at: Arc<RwLock<Option<Instant>>>,
    /// Configuration (supports hot reload)
    config: Arc<RwLock<CircuitBreakerConfig>>,
    /// Requests already admitted in HalfOpen state (for rate limiting)
    half_open_requests: Arc<AtomicU32>,
    /// Until when the provider asked us to back off (a 429's own window, or
    /// the default). Independent of the Open/HalfOpen machinery: a provider
    /// that is merely busy is not broken, and its window passes on its own.
    rate_limited_until: Arc<RwLock<Option<Instant>>>,
    /// Consecutive auth rejections (401) since the last success
    consecutive_auth_failures: Arc<AtomicU32>,
    /// Whether the breaker is open *because the key is bad* — the mark the
    /// UI reads to say "fix the key", distinct from a provider outage.
    auth_failed: Arc<AtomicU32>,
}

/// Circuit breaker admission result
///
/// `used_half_open_permit` indicates whether this admission consumed a HalfOpen probe permit.
/// Callers should pass the value back to `record_success` / `record_failure` after the request finishes so the permit is released correctly.
#[derive(Debug, Clone, Copy)]
pub struct AllowResult {
    pub allowed: bool,
    pub used_half_open_permit: bool,
}

impl CircuitBreaker {
    /// Creates a new circuit breaker
    pub fn new(config: CircuitBreakerConfig) -> Self {
        Self {
            state: Arc::new(RwLock::new(CircuitState::Closed)),
            consecutive_failures: Arc::new(AtomicU32::new(0)),
            consecutive_successes: Arc::new(AtomicU32::new(0)),
            total_requests: Arc::new(AtomicU32::new(0)),
            failed_requests: Arc::new(AtomicU32::new(0)),
            last_opened_at: Arc::new(RwLock::new(None)),
            config: Arc::new(RwLock::new(config)),
            half_open_requests: Arc::new(AtomicU32::new(0)),
            rate_limited_until: Arc::new(RwLock::new(None)),
            consecutive_auth_failures: Arc::new(AtomicU32::new(0)),
            auth_failed: Arc::new(AtomicU32::new(0)),
        }
    }

    /// Updates the circuit breaker configuration (hot reload, does not reset state)
    pub async fn update_config(&self, new_config: CircuitBreakerConfig) {
        *self.config.write().await = new_config;
    }

    /// Check whether the current Provider is "eligible for the candidate chain"
    ///
    /// This method never consumes a HalfOpen probe permit; it is only the
    /// "availability check" used during route selection:
    /// - Closed / HalfOpen: available (returns true)
    /// - Open: once the timeout elapses, switch to HalfOpen and return true; otherwise false
    ///
    /// Note: before actually sending a request you must still call `allow_request()` to
    /// acquire a HalfOpen probe permit and release it afterwards via `record_success()` / `record_failure()`.
    pub async fn is_available(&self) -> bool {
        // A provider that asked for a pause is skipped for exactly as long as
        // it asked — independent of the Open/HalfOpen machinery, which answers
        // "is it broken" while this answers "is it asking for a moment".
        if self.rate_limited().await {
            return false;
        }
        let state = *self.state.read().await;
        let config = self.config.read().await;

        match state {
            CircuitState::Closed | CircuitState::HalfOpen => true,
            CircuitState::Open => {
                if let Some(opened_at) = *self.last_opened_at.read().await {
                    if opened_at.elapsed().as_secs() >= config.timeout_seconds {
                        drop(config); // Release the read lock before transitioning state
                        tracing::info!("circuit breaker: open → half_open (timeout recovery)");
                        self.transition_to_half_open().await;
                        return true;
                    }
                }
                false
            }
        }
    }

    /// Check whether a request is allowed through
    ///
    /// Deliberately does **not** consult the rate-limit pause: that belongs to
    /// route selection ([`Self::is_available`]), which decides where a request
    /// goes. By the time admission runs, the request has already been pointed
    /// at this provider — by a retry that is waiting out exactly this pause, or
    /// by a failover that chose it after others failed — and blocking it here
    /// would kill the one retry the 429 just scheduled.
    pub async fn allow_request(&self) -> AllowResult {
        let state = *self.state.read().await;

        match state {
            CircuitState::Closed => AllowResult {
                allowed: true,
                used_half_open_permit: false,
            },
            CircuitState::Open => {
                let config = self.config.read().await;
                // Check whether to attempt HalfOpen
                if let Some(opened_at) = *self.last_opened_at.read().await {
                    if opened_at.elapsed().as_secs() >= config.timeout_seconds {
                        drop(config); // Release the read lock before transitioning state
                        tracing::info!("circuit breaker: open → half_open (timeout recovery)");
                        self.transition_to_half_open().await;

                        // After the transition, decide from the current state whether a HalfOpen probe permit is needed
                        let current_state = *self.state.read().await;
                        return match current_state {
                            CircuitState::Closed => AllowResult {
                                allowed: true,
                                used_half_open_permit: false,
                            },
                            CircuitState::HalfOpen => self.allow_half_open_probe(),
                            CircuitState::Open => AllowResult {
                                allowed: false,
                                used_half_open_permit: false,
                            },
                        };
                    }
                }

                AllowResult {
                    allowed: false,
                    used_half_open_permit: false,
                }
            }
            CircuitState::HalfOpen => self.allow_half_open_probe(),
        }
    }

    /// Record a success
    pub async fn record_success(&self, used_half_open_permit: bool) {
        let state = *self.state.read().await;
        let config = self.config.read().await;

        if used_half_open_permit {
            self.release_half_open_permit();
        }

        // A served request settles everything short of the Open state itself:
        // the rate-limit window is over (it was answered through or after it),
        // and a key that works is not a bad key.
        *self.rate_limited_until.write().await = None;
        self.consecutive_auth_failures.store(0, Ordering::SeqCst);
        self.auth_failed.store(0, Ordering::SeqCst);

        // Reset the failure counter
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.total_requests.fetch_add(1, Ordering::SeqCst);

        if state == CircuitState::HalfOpen {
            let successes = self.consecutive_successes.fetch_add(1, Ordering::SeqCst) + 1;

            if successes >= config.success_threshold {
                drop(config); // Release the read lock before transitioning state
                tracing::info!("circuit breaker: half_open → closed (recovered)");
                self.transition_to_closed().await;
            }
        }
    }

    /// Record a rate-limit answer (429). The provider is healthy and said so:
    /// it asked for a pause of its own naming (its Retry-After, or a short
    /// default) — so this touches neither the consecutive-failure count nor
    /// the error-rate sample, which exist to answer "is this provider broken".
    /// The pause is checked by [`Self::is_available`] / [`Self::allow_request`]
    /// and clears itself when it expires or a request gets through.
    pub async fn record_rate_limited(&self, used_half_open_permit: bool, cooldown: Duration) {
        if used_half_open_permit {
            self.release_half_open_permit();
        }
        *self.rate_limited_until.write().await = Some(Instant::now() + cooldown);
        tracing::info!(
            cooldown_secs = cooldown.as_secs(),
            "circuit breaker: rate limited; backing off"
        );
    }

    /// Record an auth rejection (401). The key is wrong in a way no amount of
    /// waiting fixes — only a new key does — so these are counted on their own
    /// counter and open the breaker with the auth-failed mark, which the UI
    /// reads as "fix the key". They do not feed the failure counters: an
    /// expired key says nothing about whether the provider is up.
    ///
    /// Returns whether this rejection is the one that reached the threshold —
    /// the moment the caller should surface "key invalid" where users look.
    pub async fn record_auth_rejected(&self, used_half_open_permit: bool) -> bool {
        let config = self.config.read().await;

        if used_half_open_permit {
            self.release_half_open_permit();
        }

        let failures = self
            .consecutive_auth_failures
            .fetch_add(1, Ordering::SeqCst)
            + 1;
        // Reset the success counter — an auth failure is not a recovery step.
        self.consecutive_successes.store(0, Ordering::SeqCst);

        if failures >= config.auth_failure_threshold {
            tracing::warn!(
                failures,
                "circuit breaker: consecutive auth rejections → open (key invalid)"
            );
            self.auth_failed.store(1, Ordering::SeqCst);
            drop(config);
            self.transition_to_open().await;
            return true;
        }
        false
    }

    /// Whether the provider is inside the rate-limit pause it asked for.
    async fn rate_limited(&self) -> bool {
        let until = *self.rate_limited_until.read().await;
        match until {
            Some(at) if at > Instant::now() => true,
            // Expired: clear it so the read stays cheap for the common case.
            Some(_) => {
                *self.rate_limited_until.write().await = None;
                false
            }
            None => false,
        }
    }

    /// Record a failure
    pub async fn record_failure(&self, used_half_open_permit: bool) {
        let state = *self.state.read().await;
        let config = self.config.read().await;

        if used_half_open_permit {
            self.release_half_open_permit();
        }

        // Update counters
        let failures = self.consecutive_failures.fetch_add(1, Ordering::SeqCst) + 1;
        self.total_requests.fetch_add(1, Ordering::SeqCst);
        self.failed_requests.fetch_add(1, Ordering::SeqCst);

        // Reset the success counter
        self.consecutive_successes.store(0, Ordering::SeqCst);

        // Check whether the breaker should open
        match state {
            CircuitState::HalfOpen => {
                // Failed while HalfOpen: transition to Open immediately
                tracing::warn!("circuit breaker: half_open probe failed → open");
                drop(config);
                self.transition_to_open().await;
            }
            CircuitState::Closed => {
                // Check the consecutive failure count
                if failures >= config.failure_threshold {
                    tracing::warn!(failures, "circuit breaker: consecutive failures → open");
                    drop(config); // Release the read lock before transitioning state
                    self.transition_to_open().await;
                } else {
                    // Check the error rate
                    let total = self.total_requests.load(Ordering::SeqCst);
                    let failed = self.failed_requests.load(Ordering::SeqCst);

                    if total >= config.min_requests {
                        let error_rate = failed as f64 / total as f64;

                        if error_rate >= config.error_rate_threshold {
                            tracing::warn!(
                                error_rate = format!("{:.1}%", error_rate * 100.0),
                                "circuit breaker: error rate threshold → open"
                            );
                            drop(config); // Release the read lock before transitioning state
                            self.transition_to_open().await;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// Get the current state
    pub async fn get_state(&self) -> CircuitState {
        *self.state.read().await
    }

    /// Get statistics
    #[allow(dead_code)]
    pub async fn get_stats(&self) -> CircuitBreakerStats {
        CircuitBreakerStats {
            state: *self.state.read().await,
            consecutive_failures: self.consecutive_failures.load(Ordering::SeqCst),
            consecutive_successes: self.consecutive_successes.load(Ordering::SeqCst),
            total_requests: self.total_requests.load(Ordering::SeqCst),
            failed_requests: self.failed_requests.load(Ordering::SeqCst),
            auth_failed: self.auth_failed.load(Ordering::SeqCst) == 1,
            rate_limited: self.rate_limited().await,
        }
    }

    /// Reset the circuit breaker (manual recovery)
    #[allow(dead_code)]
    pub async fn reset(&self) {
        tracing::info!("circuit breaker: manual reset → closed");
        self.transition_to_closed().await;
    }

    fn allow_half_open_probe(&self) -> AllowResult {
        // HalfOpen rate limiting: only a limited number of requests are admitted for probing
        let max_half_open_requests = 1u32;
        let current = self.half_open_requests.fetch_add(1, Ordering::SeqCst);

        if current < max_half_open_requests {
            AllowResult {
                allowed: true,
                used_half_open_permit: true,
            }
        } else {
            // Over the limit: roll back the counter and reject the request
            self.half_open_requests.fetch_sub(1, Ordering::SeqCst);
            AllowResult {
                allowed: false,
                used_half_open_permit: false,
            }
        }
    }

    /// Releases only the HalfOpen permit, without touching health statistics.
    ///
    /// Called by `record_success` / `record_failure` when the admission that
    /// carried the permit reports back, so a HalfOpen window cannot stall on a
    /// probe that has already finished.
    pub fn release_half_open_permit(&self) {
        let mut current = self.half_open_requests.load(Ordering::SeqCst);
        loop {
            if current == 0 {
                return;
            }

            match self.half_open_requests.compare_exchange(
                current,
                current - 1,
                Ordering::SeqCst,
                Ordering::SeqCst,
            ) {
                Ok(_) => return,
                Err(actual) => current = actual,
            }
        }
    }

    /// Transition to the Open state
    async fn transition_to_open(&self) {
        *self.state.write().await = CircuitState::Open;
        *self.last_opened_at.write().await = Some(Instant::now());
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.consecutive_successes.store(0, Ordering::SeqCst);
    }

    /// Transition to the HalfOpen state
    async fn transition_to_half_open(&self) {
        let mut state = self.state.write().await;
        if *state != CircuitState::Open {
            return;
        }

        *state = CircuitState::HalfOpen;
        self.consecutive_successes.store(0, Ordering::SeqCst);
        // Reset the HalfOpen rate-limit counter
        self.half_open_requests.store(0, Ordering::SeqCst);
    }

    /// Transition to the Closed state
    async fn transition_to_closed(&self) {
        *self.state.write().await = CircuitState::Closed;
        self.consecutive_failures.store(0, Ordering::SeqCst);
        self.consecutive_successes.store(0, Ordering::SeqCst);
        self.consecutive_auth_failures.store(0, Ordering::SeqCst);
        self.auth_failed.store(0, Ordering::SeqCst);
        *self.rate_limited_until.write().await = None;
        // Reset the counters
        self.total_requests.store(0, Ordering::SeqCst);
        self.failed_requests.store(0, Ordering::SeqCst);
    }
}

/// Circuit breaker statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CircuitBreakerStats {
    pub state: CircuitState,
    pub consecutive_failures: u32,
    pub consecutive_successes: u32,
    pub total_requests: u32,
    pub failed_requests: u32,
    /// The breaker is open and the reason is a refused key, not an outage.
    pub auth_failed: bool,
    /// The provider asked for a rate-limit pause and the window is running.
    pub rate_limited: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn closed_to_open_on_consecutive_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        assert_eq!(breaker.get_state().await, CircuitState::Closed);
        assert!(breaker.allow_request().await.allowed);

        for _ in 0..3 {
            breaker.record_failure(false).await;
        }

        assert_eq!(breaker.get_state().await, CircuitState::Open);
        assert!(!breaker.allow_request().await.allowed);
    }

    #[tokio::test]
    async fn half_open_to_closed_on_successes() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            success_threshold: 2,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        breaker.record_failure(false).await;
        breaker.record_failure(false).await;
        assert_eq!(breaker.get_state().await, CircuitState::Open);

        breaker.transition_to_half_open().await;
        assert_eq!(breaker.get_state().await, CircuitState::HalfOpen);

        breaker.record_success(false).await;
        breaker.record_success(false).await;

        assert_eq!(breaker.get_state().await, CircuitState::Closed);
    }

    #[tokio::test]
    async fn half_open_transition_does_not_reset_inflight_permit() {
        let config = CircuitBreakerConfig {
            timeout_seconds: 0,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        // Enter Open; with timeout_seconds=0, allow_request immediately switches to HalfOpen and takes a probe permit
        breaker.transition_to_open().await;
        let first = breaker.allow_request().await;
        assert!(first.allowed);
        assert!(first.used_half_open_permit);
        assert_eq!(breaker.get_state().await, CircuitState::HalfOpen);

        // Simulate a "duplicate HalfOpen transition call" under concurrency; it must not reset the in-flight count
        breaker.transition_to_half_open().await;

        // With the permit still held, the second request should be rejected
        let second = breaker.allow_request().await;
        assert!(!second.allowed);
        assert!(!second.used_half_open_permit);
    }

    #[tokio::test]
    async fn manual_reset_reopens_the_gate() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            ..Default::default()
        };
        let breaker = CircuitBreaker::new(config);

        breaker.record_failure(false).await;
        breaker.record_failure(false).await;
        assert_eq!(breaker.get_state().await, CircuitState::Open);

        breaker.reset().await;
        assert_eq!(breaker.get_state().await, CircuitState::Closed);
        assert!(breaker.allow_request().await.allowed);
    }

    /// A rate limit is not a failure: repeated 429s never open the breaker,
    /// never touch the failure counters, and the provider is merely skipped
    /// for the window it named — available again the moment it passes.
    #[tokio::test]
    async fn rate_limits_set_a_pause_never_open_the_breaker() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig::default());

        for _ in 0..10 {
            breaker
                .record_rate_limited(false, Duration::from_secs(1))
                .await;
        }
        assert_eq!(breaker.get_state().await, CircuitState::Closed);
        let stats = breaker.get_stats().await;
        assert_eq!(stats.failed_requests, 0, "429s are not failures");
        assert!(stats.rate_limited, "the pause is running");
        assert!(
            !breaker.is_available().await,
            "selection skips a provider inside its pause"
        );

        // A served request ends the pause outright — the window was named for
        // a kind of traffic that has since succeeded.
        breaker.record_success(false).await;
        let stats = breaker.get_stats().await;
        assert!(!stats.rate_limited);
        assert!(breaker.is_available().await);
    }

    /// Two refused keys are two refused keys: the breaker opens with the
    /// auth-failed mark (the UI's "fix the key"), and a probe that gets
    /// through — the user swapped the key — clears the mark as it closes.
    #[tokio::test]
    async fn consecutive_auth_rejections_open_with_the_mark_and_recover() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            // The probe runs immediately in the test; production waits 60s.
            timeout_seconds: 0,
            ..Default::default()
        });
        assert!(
            !breaker.record_auth_rejected(false).await,
            "the first is a warning"
        );
        assert_eq!(breaker.get_state().await, CircuitState::Closed);

        assert!(
            breaker.record_auth_rejected(false).await,
            "the second trips"
        );
        let stats = breaker.get_stats().await;
        assert_eq!(stats.state, CircuitState::Open);
        assert!(stats.auth_failed);
        // 401s do not count as failures: an expired key says nothing about
        // whether the provider is up.
        assert_eq!(stats.failed_requests, 0);

        let probe = breaker.allow_request().await;
        assert!(probe.allowed);
        breaker.record_success(probe.used_half_open_permit).await;
        // Recovery needs the success threshold (2): the first probe success
        // leaves HalfOpen, the second closes it. The mark clears with the
        // first — the key demonstrably works.
        assert!(
            !breaker.get_stats().await.auth_failed,
            "a served probe clears the mark"
        );
        let second = breaker.allow_request().await;
        breaker.record_success(second.used_half_open_permit).await;
        let stats = breaker.get_stats().await;
        assert_eq!(stats.state, CircuitState::Closed);
        assert!(!stats.auth_failed);
    }

    /// A 401 does not march the failure counter toward the outage threshold:
    /// an expired key and a flaky provider are different problems with
    /// different fixes, and mixing them would open the breaker for the wrong
    /// reason (and describe it wrong in the UI).
    #[tokio::test]
    async fn auth_rejections_do_not_feed_the_failure_counter() {
        let breaker = CircuitBreaker::new(CircuitBreakerConfig {
            failure_threshold: 2,
            // Too high to trip in this test: the only way Open here is via the
            // *failure* counter, which is the point.
            auth_failure_threshold: 10,
            ..Default::default()
        });
        // Four auth rejections: more than the failure threshold, and still not
        // an outage — and not a single one of them a failure.
        for _ in 0..4 {
            breaker.record_auth_rejected(false).await;
        }
        assert_eq!(breaker.get_state().await, CircuitState::Closed);
        let stats = breaker.get_stats().await;
        assert_eq!(stats.failed_requests, 0, "401s are not failures");

        // Real failures trip at their own threshold, untouched by the auths.
        breaker.record_failure(false).await;
        breaker.record_failure(false).await;
        let stats = breaker.get_stats().await;
        assert_eq!(stats.state, CircuitState::Open);
        assert!(!stats.auth_failed, "opened for the outage, not the key");
    }
}
