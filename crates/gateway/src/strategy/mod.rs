//! Strategy engine (tech.md §4.7): per request, select 1 Provider from the Agent's ordered candidate set.
//!
//! Strategies (agent_strategies.type):
//! - `single` — fixed primary (MVP semantics, equivalent to the current switch flow)
//! - `failover` — take the first "breaker-available" candidate in priority order; when all are open, fall back to the primary
//! - `roundrobin` — session-granularity stickiness + weighted rotation (a session keeps its
//!   Provider to preserve the upstream prompt cache; new sessions pick by weight; reassigned when the sticky candidate goes unavailable)
//! - `timewindow` — match the binding's local `HH:MM` window (supports crossing
//!   midnight); falls back to the primary when nothing matches
//! - `quota` — when the primary's same-day usage (requests/tokens, read from usage
//!   aggregates) exceeds the threshold in the strategy config, sink to backups (failover semantics)
//!
//! After each real upstream attempt, the forward layer calls [`StrategyEngine::record`]
//! to feed the breaker back (key = `agent:provider_id`, tech.md §4.7.3).

pub mod circuit_breaker;
pub mod prober;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use serde::Deserialize;

use crate::error::{GatewayError, Result};
use crate::router::AgentRoute;
use crate::store::{Store, StrategyType, UsageTotals};

use circuit_breaker::{CircuitBreaker, CircuitBreakerConfig};

/// Sticky-table key for cross-session rotation: `agent \0 session` (\0 avoids concatenation ambiguity).
fn sticky_key(agent: &str, session: Option<&str>) -> String {
    format!("{agent}\u{0}{}", session.unwrap_or(""))
}

/// Breaker registry key (tech.md §4.7.3): `agent:provider_id`.
fn breaker_key(agent: &str, provider_id: &str) -> String {
    format!("{agent}:{provider_id}")
}

/// Parse "HH:MM" into minutes-of-day.
fn hhmm_minutes(s: &str) -> Option<u32> {
    let (h, m) = s.trim().split_once(':')?;
    let h: u32 = h.parse().ok()?;
    let m: u32 = m.parse().ok()?;
    if h < 24 && m < 60 {
        Some(h * 60 + m)
    } else {
        None
    }
}

/// Local time "HH:MM" (peak/off-peak windows use the local timezone).
fn local_hhmm() -> String {
    chrono::Local::now().format("%H:%M").to_string()
}

/// Whether now falls inside [start, end] (start > end means an overnight window).
fn in_window(now_min: u32, start: &str, end: &str) -> bool {
    match (hhmm_minutes(start), hhmm_minutes(end)) {
        (Some(s), Some(e)) => {
            if s <= e {
                now_min >= s && now_min <= e
            } else {
                now_min >= s || now_min <= e
            }
        }
        _ => false,
    }
}

/// Config payload of the quota strategy (agent_strategies.config JSON).
///
/// `period` currently supports only `day` (UTC calendar day); other values are treated
/// as day and warned about in engine logs. The cost unit opens up once the §8 billing model lands.
#[derive(Debug, Deserialize)]
struct QuotaConfig {
    limit: f64,
    #[serde(default = "default_quota_unit")]
    unit: String,
    #[serde(default)]
    #[allow(dead_code)] // declared for explicit semantics; currently only day
    period: String,
}

fn default_quota_unit() -> String {
    "requests".into()
}

impl QuotaConfig {
    fn parse(config: Option<&str>) -> Option<QuotaConfig> {
        let cfg: QuotaConfig = serde_json::from_str(config?).ok()?;
        if cfg.unit != "requests" && cfg.unit != "tokens" {
            tracing::warn!(unit = %cfg.unit, "quota unit not supported; falling back to requests");
        }
        if cfg.period != "day" && !cfg.period.is_empty() {
            tracing::warn!(period = %cfg.period, "quota period not supported; using day");
        }
        Some(cfg)
    }
}

impl QuotaConfig {
    fn consumed(&self, t: &UsageTotals) -> f64 {
        match self.unit.as_str() {
            "tokens" => (t.input_tokens + t.output_tokens) as f64,
            _ => t.requests as f64,
        }
    }
}

/// Start-of-day (UTC) timestamp, compatible with lexicographic comparison against the usage table's RFC3339 ts.
fn today_start_utc() -> String {
    format!("{}T00:00:00Z", chrono::Utc::now().format("%Y-%m-%d"))
}

/// Per-Agent strategy runtime: breaker registry + roundrobin sticky table.
pub struct StrategyEngine {
    breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    /// roundrobin: session key → candidate index (sticky to preserve the prompt cache).
    sticky: Mutex<HashMap<String, usize>>,
    /// Cursor advanced on the weighted ring as new sessions join.
    cursor: Mutex<u64>,
}

impl Default for StrategyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StrategyEngine {
    pub fn new() -> Self {
        StrategyEngine {
            breakers: Mutex::new(HashMap::new()),
            sticky: Mutex::new(HashMap::new()),
            cursor: Mutex::new(0),
        }
    }

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

    async fn candidate_available(&self, route: &AgentRoute, idx: usize) -> bool {
        let c = &route.candidates[idx];
        self.breaker(&route.agent, &c.id).await.is_available().await
    }

    /// Single strategy / fallback: the primary.
    fn primary(route: &AgentRoute) -> Result<crate::router::UpstreamProvider> {
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
    ) -> Result<crate::router::UpstreamProvider> {
        if route.candidates.is_empty() {
            return Err(GatewayError::NoBinding(route.agent.clone()));
        }
        match route.strategy {
            StrategyType::Single => Self::primary(route),
            StrategyType::Failover => self.select_failover(route).await,
            StrategyType::Roundrobin => self.select_roundrobin(route, session).await,
            StrategyType::Timewindow => Ok(self.select_timewindow(route)),
            StrategyType::Quota => self.select_quota(store, route).await,
        }
    }

    /// failover: take the first breaker-available candidate in priority order; when all are open,
    /// fall back to the primary (the request fails at the real upstream, not gateway-side — cc-switch queue semantics).
    async fn select_failover(
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

    /// roundrobin: session-granularity stickiness + weighting. Reassigns when the sticky candidate is unavailable (breaker open).
    async fn select_roundrobin(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Result<crate::router::UpstreamProvider> {
        let key = sticky_key(&route.agent, session);
        // The lock only guards the table read (a guard must not be held across await, or the future is not Send)
        let sticky_hit = match self.sticky.lock().expect("sticky map poisoned").get(&key) {
            Some(&idx) if idx < route.candidates.len() => Some(idx),
            _ => None,
        };
        if let Some(idx) = sticky_hit {
            if self.candidate_available(route, idx).await {
                return Ok(route.candidates[idx].clone());
            }
        }
        let idx = self.weighted_next(route).await?;
        self.sticky
            .lock()
            .expect("sticky map poisoned")
            .insert(key, idx);
        Ok(route.candidates[idx].clone())
    }

    /// Weighted ring: next available candidate index (weights expanded into a ring by gcd, the lock
    /// only guards cursor reads/writes and never crosses await); returns 0 (primary fallback) when none is available.
    async fn weighted_next(&self, route: &AgentRoute) -> Result<usize> {
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
            .flat_map(|(i, w)| std::iter::repeat(i).take((*w / g) as usize))
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

    /// timewindow: match local time windows in candidate order (overnight supported); falls back to the primary on no match.
    fn select_timewindow(&self, route: &AgentRoute) -> crate::router::UpstreamProvider {
        let now = local_hhmm();
        let now_min = hhmm_minutes(&now).unwrap_or(0);
        for c in &route.candidates {
            if let (Some(s), Some(e)) = (&c.win_start, &c.win_end) {
                if in_window(now_min, s, e) {
                    return c.clone();
                }
            }
        }
        route.candidates[0].clone()
    }

    /// quota: primary's same-day usage over threshold → sink to backups (failover semantics); under → primary.
    async fn select_quota(
        &self,
        store: &Store,
        route: &AgentRoute,
    ) -> Result<crate::router::UpstreamProvider> {
        let Some(cfg) = QuotaConfig::parse(route.config.as_deref()) else {
            tracing::warn!(
                agent = %route.agent,
                "quota strategy without valid config; degrading to primary"
            );
            return Self::primary(route);
        };
        let primary = &route.candidates[0];
        let totals =
            store.usage_totals_for_provider(&primary.id, Some(&today_start_utc()))?;
        if cfg.consumed(&totals) < cfg.limit {
            return Ok(primary.clone());
        }
        tracing::info!(
            agent = %route.agent,
            provider = %primary.id,
            used = cfg.consumed(&totals),
            limit = cfg.limit,
            "quota threshold reached; failing over to backup"
        );
        for i in 1..route.candidates.len() {
            if self.candidate_available(route, i).await {
                return Ok(route.candidates[i].clone());
            }
        }
        Self::primary(route)
    }

    /// Feed back the result of one real upstream attempt (called by the forward layer once per attempt).
    pub async fn record(&self, agent: &str, provider_id: &str, success: bool) {
        let b = self.breaker(agent, provider_id).await;
        if success {
            b.record_success(false).await;
        } else {
            b.record_failure(false).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router::UpstreamProvider;
    use crate::store::{now_rfc3339, Billing, Protocol, StrategyType, UsageRecord};

    fn candidate(id: &str, weight: i64, win: Option<(&str, &str)>) -> UpstreamProvider {
        UpstreamProvider {
            id: id.into(),
            name: format!("prov-{id}"),
            protocol: Protocol::Anthropic,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            api_key: Some(format!("sk-{id}")),
            extra_keys: Vec::new(),
            weight,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
        }
    }

    fn route(strategy: StrategyType, candidates: Vec<UpstreamProvider>) -> AgentRoute {
        AgentRoute {
            agent: "claude".into(),
            strategy,
            config: None,
            candidates,
        }
    }

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn usage_row(provider_id: &str) -> UsageRecord {
        UsageRecord {
            ts: now_rfc3339(),
            agent: "claude".into(),
            provider_id: provider_id.into(),
            model: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: None,
            status: "ok".into(),
        }
    }

    #[tokio::test]
    async fn single_always_returns_primary() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Single,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        for _ in 0..3 {
            assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "a");
        }
    }

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
            engine.record("claude", "a", false).await;
        }
        assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "b");

        // Backup also open → fall back to the primary (real upstream failure semantics)
        for _ in 0..4 {
            engine.record("claude", "b", false).await;
        }
        assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "a");

        // Primary recovery (timeout=0 goes HalfOpen immediately + success flips to closed)
        // Build a fresh engine to exercise the recovery path: a success record clears consecutive failures
        let engine2 = StrategyEngine::new();
        engine2.record("claude", "a", false).await;
        engine2.record("claude", "a", true).await;
        let r2 = route(
            StrategyType::Failover,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        assert_eq!(engine2.select(&s, &r2, None).await.unwrap().id, "a");
    }

    #[tokio::test]
    async fn roundrobin_is_sticky_per_session_and_weighted_across() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Roundrobin,
            vec![candidate("a", 3, None), candidate("b", 1, None)],
        );

        // Same session stays sticky
        let first = engine.select(&s, &r, Some("sess-1")).await.unwrap().id;
        for _ in 0..5 {
            assert_eq!(engine.select(&s, &r, Some("sess-1")).await.unwrap().id, first);
        }

        // New sessions advance on the weighted ring: with weights 3:1, 4 new sessions should pick a 3 times, b once
        let mut counts = std::collections::HashMap::new();
        for i in 0..4 {
            let id = engine
                .select(&s, &r, Some(&format!("sess-{i}")))
                .await
                .unwrap()
                .id;
            *counts.entry(id).or_insert(0) += 1;
        }
        assert_eq!(counts.get("a"), Some(&3));
        assert_eq!(counts.get("b"), Some(&1));
    }

    #[tokio::test]
    async fn roundrobin_reassigns_when_sticky_candidate_opens() {
        let engine = StrategyEngine::new();
        let s = store();
        let r = route(
            StrategyType::Roundrobin,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        let sticky = engine.select(&s, &r, Some("s")).await.unwrap().id;

        // Sticky candidate trips its breaker → same session is reassigned to another
        for _ in 0..4 {
            engine.record("claude", &sticky, false).await;
        }
        let reassigned = engine.select(&s, &r, Some("s")).await.unwrap().id;
        assert_ne!(reassigned, sticky);
    }

    #[tokio::test]
    async fn timewindow_matches_by_priority_and_falls_back() {
        let engine = StrategyEngine::new();
        let s = store();
        // Candidate b holds an all-day window; a has none → b matches
        let r = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some(("00:00", "23:59"))),
            ],
        );
        assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "b");

        // No window contains the current time (e.g. a narrow window a minute ahead) → back to primary
        let (s2, e2) = narrow_future_window();
        let r2 = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some((s2, e2))),
            ],
        );
        assert_eq!(engine.select(&s, &r2, None).await.unwrap().id, "a");
    }

    /// Build an [start,end) window guaranteed not to contain the current local time (now+2 to now+3 minutes).
    fn narrow_future_window() -> (&'static str, &'static str) {
        // Clock drift does not affect the assertions: the window holds only two marks within
        // the coming minute, so as long as the test finishes within the same minute, now < start always holds.
        let now_min = hhmm_minutes(&local_hhmm()).unwrap();
        let s = now_min + 2;
        let e = now_min + 3;
        // Midnight wraparound remains a valid window; format as static HH:MM strings
        let fmt = |m: u32| {
            let m = m % (24 * 60);
            format!("{:02}:{:02}", m / 60, m % 60)
        };
        (Box::leak(fmt(s).into_boxed_str()), Box::leak(fmt(e).into_boxed_str()))
    }

    #[tokio::test]
    async fn timewindow_supports_overnight_window() {
        let now = local_hhmm();
        let now_min = hhmm_minutes(&now).unwrap();
        // Overnight window [23:00, 06:00]: now after 23:00 or before 06:00 must match
        let late = now_min >= 23 * 60;
        let early = now_min <= 6 * 60;
        assert_eq!(in_window(now_min, "23:00", "06:00"), late || early);
    }

    #[tokio::test]
    async fn quota_over_limit_sinks_to_backup() {
        let s = store();
        s.insert_provider(&crate::store::Provider {
            id: "a".into(),
            name: "a".into(),
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            api_key: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        })
        .unwrap();

        let engine = StrategyEngine::new();
        let mut r = route(
            StrategyType::Quota,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        r.config = Some(r#"{"limit": 5, "unit": "requests"}"#.into());

        // Under the limit → primary
        assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "a");

        // Primary records 5 rows → over the limit, sink to backup
        for _ in 0..5 {
            s.record_usage(&usage_row("a")).unwrap();
        }
        assert_eq!(engine.select(&s, &r, None).await.unwrap().id, "b");
    }

    #[test]
    fn window_parsing_and_containment() {
        assert_eq!(hhmm_minutes("09:30"), Some(570));
        assert_eq!(hhmm_minutes("24:00"), None);
        assert!(in_window(600, "09:00", "18:00"));
        assert!(!in_window(100, "09:00", "18:00"));
        // Crossing midnight
        assert!(in_window(60, "23:00", "06:00"));
        assert!(in_window(23 * 60 + 30, "23:00", "06:00"));
        assert!(!in_window(12 * 60, "23:00", "06:00"));
    }

    #[test]
    fn quota_config_parsing() {
        let cfg = QuotaConfig::parse(Some(r#"{"limit": 100}"#)).unwrap();
        assert_eq!(cfg.unit, "requests");
        assert!(QuotaConfig::parse(Some("not json")).is_none());
        assert!(QuotaConfig::parse(None).is_none());
    }
}
