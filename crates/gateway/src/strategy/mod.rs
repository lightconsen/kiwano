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

use serde::{Deserialize, Serialize};

use crate::error::{GatewayError, Result};
use crate::router::AgentRoute;
use crate::store::{Store, StrategyType, UsageTotals};

use circuit_breaker::{AllowResult, CircuitBreaker, CircuitBreakerConfig};

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

/// Minutes-of-day on the user's clock — the same one the quota day boundary is
/// read from (`limits::period_start`), so a window and a "today's quota" reset
/// cannot disagree about what time it is.
///
/// The offset is the stored `tz_offset_minutes` (minutes east of UTC), not the
/// daemon host's timezone. A gateway run on a UTC host for a UTC+8 user used to
/// peak and go off-peak at the wrong hours; it is fixed, so a session spanning a
/// DST transition keeps the offset it started with (`ui_tz_offset_minutes` is
/// rewritten by the app at launch).
fn local_minutes_of_day(tz_offset_minutes: i64) -> u32 {
    use chrono::Timelike;
    let local = chrono::Utc::now() + chrono::Duration::minutes(tz_offset_minutes);
    let t = local.time();
    t.hour() * 60 + t.minute()
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
/// `period` currently supports only `day` (the user's local day); other values are
/// treated as day and warned about in engine logs. The cost unit opens up once the §8 billing model lands.
///
/// Public, because this payload is authored outside the engine too — the CLI
/// writes it — and two definitions of its spelling is how a config that looks
/// accepted ends up silently ignored.
#[derive(Debug, Serialize, Deserialize)]
pub struct QuotaConfig {
    pub limit: f64,
    #[serde(default = "default_quota_unit")]
    pub unit: String,
    #[serde(default)]
    pub period: String,
}

fn default_quota_unit() -> String {
    "requests".into()
}

impl QuotaConfig {
    /// The units [`QuotaConfig::consumed`] acts on.
    pub const UNITS: [&'static str; 2] = ["requests", "tokens"];

    /// Parse and *validate* a config authored outside the engine.
    ///
    /// Stricter than the private [`QuotaConfig::parse`], which stays lenient for
    /// the read path: a config already in the database must not start failing
    /// requests just because it names a unit this build does not know. A writer,
    /// on the other hand, can act on being told — and an unknown unit silently
    /// falling back to `requests` changes what the threshold counts.
    pub fn from_json(config: &str) -> std::result::Result<QuotaConfig, String> {
        let cfg: QuotaConfig =
            serde_json::from_str(config).map_err(|e| format!("invalid quota config JSON: {e}"))?;
        // `is_finite` first: a NaN limit would compare false against everything
        // and quietly never trip the threshold.
        if !cfg.limit.is_finite() || cfg.limit <= 0.0 {
            return Err(format!(
                "quota limit must be a positive number, got {}",
                cfg.limit
            ));
        }
        if !Self::UNITS.contains(&cfg.unit.as_str()) {
            return Err(format!(
                "unknown quota unit \"{}\" (expected {})",
                cfg.unit,
                Self::UNITS.join("|")
            ));
        }
        if !cfg.period.is_empty() && cfg.period != "day" {
            return Err(format!(
                "unknown quota period \"{}\" (only day is supported)",
                cfg.period
            ));
        }
        Ok(cfg)
    }

    pub fn to_json(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| "{}".to_string())
    }

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

/// One session's assignment: which candidate, and when it was last used.
struct Sticky {
    /// The candidate's **id**, not its position. The candidate list is pruned
    /// per request (a provider over a billing limit is dropped before the
    /// strategy is asked), so a stored index means a different provider the
    /// moment anything ahead of it is pruned — and a session pinned to the wrong
    /// provider is exactly the cache loss this table exists to prevent.
    id: String,
    /// The tick of the last request that used this entry. Ordering by it is what
    /// makes the table least-recently-used rather than oldest-first.
    used: u64,
}

/// How many sessions keep their assignment. An entry is a session key and an
/// index — tens of bytes — so this is a few hundred kilobytes at worst, which
/// is cheaper than the bookkeeping it would take to size it exactly.
const STICKY_CAPACITY: usize = 4096;

/// roundrobin's session table, with a ceiling.
///
/// The ceiling is the point: a session can run for hours and there can be many
/// of them, so one entry per session, kept for the life of the process, is a map
/// that only grows. A session that is still talking is touched by every request
/// it makes, so least-recently-used eviction cannot take the slot out from under
/// a live conversation — only one that has gone quiet, which is a conversation
/// that has ended (or one that will pay a single cold prefix if it wakes up).
struct StickyTable {
    map: HashMap<String, Sticky>,
    tick: u64,
}

impl StickyTable {
    fn new() -> Self {
        StickyTable {
            map: HashMap::new(),
            tick: 0,
        }
    }

    /// The candidate this key is assigned to, if any — and marking that as the
    /// moment it was used. A read that hits is a use: this is what keeps a live
    /// session out of the eviction scan.
    ///
    /// The id comes back owned rather than borrowed: the caller has to await
    /// before it can use it, and a guard held across an await makes the future
    /// non-Send. An id is a few dozen bytes.
    fn get(&mut self, key: &str) -> Option<String> {
        self.tick += 1;
        let tick = self.tick;
        let entry = self.map.get_mut(key)?;
        entry.used = tick;
        Some(entry.id.clone())
    }

    fn insert(&mut self, key: String, id: String) {
        self.tick += 1;
        self.map.insert(
            key,
            Sticky {
                id,
                used: self.tick,
            },
        );
        self.evict_if_over();
    }

    /// Drop the quietest quarter in one pass. A quarter rather than the single
    /// oldest entry because the scan is the cost and a batch amortizes it, and a
    /// quarter rather than everything because the survivors are the ones being
    /// used — among them, almost certainly, the session that just arrived.
    fn evict_if_over(&mut self) {
        if self.map.len() <= STICKY_CAPACITY {
            return;
        }
        let mut ages: Vec<u64> = self.map.values().map(|s| s.used).collect();
        ages.sort_unstable();
        let cutoff = ages[ages.len() / 4];
        self.map.retain(|_, s| s.used > cutoff);
    }
}

/// Per-Agent strategy runtime: breaker registry + roundrobin sticky table.
pub struct StrategyEngine {
    breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    /// roundrobin: session key → candidate index (sticky to preserve the prompt
    /// cache), bounded — see [`StickyTable`].
    sticky: Mutex<StickyTable>,
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
            sticky: Mutex::new(StickyTable::new()),
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
            StrategyType::Timewindow => self.select_timewindow(usable, limits.tz_offset_minutes()),
            StrategyType::Quota => self.select_quota(store, usable, limits).await?,
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

    /// The candidate this session is already on, if it is still usable.
    ///
    /// Session-granularity stickiness is what keeps a conversation's upstream
    /// prompt cache intact, and it belongs to every strategy whose answer can
    /// change while a conversation is running: roundrobin's ring, the quota
    /// threshold, a closing time window, a provider recovering from a failure.
    ///
    /// A *new* session asks the strategy and goes wherever it says. A session
    /// already running stays put, because the alternative is rebuilding a prefix
    /// the upstream has already cached — and what the switch buys (spreading a
    /// quota, honouring a window, returning to the preferred provider) can wait
    /// for the conversation to end. The cost of waiting is bounded by the
    /// conversation; the cost of switching is paid in this turn's tokens.
    ///
    /// What drain does *not* do is spend past a ceiling. The candidate list this
    /// looks in has already had the providers over a billing limit pruned out of
    /// it, and a breaker that has opened fails the availability check below —
    /// so a pin holds only while the provider is genuinely usable, and a hard
    /// limit still wins over a conversation in flight.
    async fn pinned(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
    ) -> Option<crate::router::UpstreamProvider> {
        let key = sticky_key(&route.agent, session);
        // The lock only guards the table read (a guard must not be held across
        // await, or the future is not Send).
        let id = self.sticky.lock().expect("sticky map poisoned").get(&key)?;
        let idx = route.candidates.iter().position(|c| c.id == id)?;
        if self.candidate_available(route, idx).await {
            return Some(route.candidates[idx].clone());
        }
        None
    }

    /// Record where a session was sent, so its next request can stay there.
    fn assign(
        &self,
        route: &AgentRoute,
        session: Option<&str>,
        provider: &crate::router::UpstreamProvider,
    ) {
        self.sticky
            .lock()
            .expect("sticky map poisoned")
            .insert(sticky_key(&route.agent, session), provider.id.clone());
    }

    /// failover: take the first breaker-available candidate in priority order; when all are open,
    /// fall back to the primary (the request fails at the real upstream, not gateway-side — cc-switch queue semantics).
    async fn select_failover(&self, route: &AgentRoute) -> Result<crate::router::UpstreamProvider> {
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

    /// timewindow: match the user's local time windows in candidate order
    /// (overnight supported); falls back to the primary on no match.
    fn select_timewindow(
        &self,
        route: &AgentRoute,
        tz_offset_minutes: i64,
    ) -> crate::router::UpstreamProvider {
        let now_min = local_minutes_of_day(tz_offset_minutes);
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
        limits: &crate::limits::LimitState,
    ) -> Result<crate::router::UpstreamProvider> {
        let Some(cfg) = QuotaConfig::parse(route.config.as_deref()) else {
            tracing::warn!(
                agent = %route.agent,
                "quota strategy without valid config; degrading to primary"
            );
            return Self::primary(route);
        };
        let primary = &route.candidates[0];
        // The user's day, from the snapshot — the same boundary the provider
        // billing limits use. It used to be UTC's, which for a UTC+8 user
        // resets "today's quota" at 08:00 and cannot be reasoned about without
        // knowing that.
        let (since, _) = crate::limits::period_start(
            chrono::Utc::now().timestamp(),
            Some("day"),
            limits.tz_offset_minutes(),
        );
        let totals = store.usage_totals_for_provider(&primary.id, since.as_deref())?;
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

    /// Every breaker's current state, keyed `agent:provider_id` — what the
    /// metrics endpoint reports so an operator can see which candidate the
    /// breakers have taken out. Read-only; nothing here admits or records.
    pub async fn breaker_snapshot(&self) -> Vec<(String, circuit_breaker::CircuitState)> {
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
            snapshot.push((key, breaker.get_state().await));
        }
        snapshot.sort_by(|a, b| a.0.cmp(&b.0));
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
    /// matching [`StrategyEngine::allow`] — it releases the probe permit.
    pub async fn record(
        &self,
        agent: &str,
        provider_id: &str,
        success: bool,
        used_half_open_permit: bool,
    ) {
        let b = self.breaker(agent, provider_id).await;
        if success {
            b.record_success(used_half_open_permit).await;
        } else {
            b.record_failure(used_half_open_permit).await;
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
            catalog_id: None,
            protocol: Protocol::Anthropic,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some(format!("sk-{id}")),
            extra_keys: Vec::new(),
            weight,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
            timeout_secs: None,
            retries: None,
            headers: None,
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
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
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
            engine.record("claude", "a", false, false).await;
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
            engine.record("claude", "b", false, false).await;
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
        engine2.record("claude", "a", false, false).await;
        engine2.record("claude", "a", true, false).await;
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
            engine.record("claude", "a", false, false).await;
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
            engine.record("claude", "a", false, false).await;
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
            .record("claude", "a", true, probe.used_half_open_permit)
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
            .record("claude", "a", false, after.used_half_open_permit)
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
            engine.record("claude", &sticky, false, false).await;
        }
        let reassigned = engine
            .select(&s, &r, Some("s"), &crate::limits::LimitState::default())
            .await
            .unwrap()
            .id;
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
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "b"
        );

        // No window contains the current time (e.g. a narrow window a minute ahead) → back to primary
        let (s2, e2) = narrow_future_window();
        let r2 = route(
            StrategyType::Timewindow,
            vec![candidate("a", 1, None), candidate("b", 1, Some((s2, e2)))],
        );
        assert_eq!(
            engine
                .select(&s, &r2, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );
    }

    /// Build an [start,end) window guaranteed not to contain the current local
    /// time (now+2 to now+3 minutes). The offset is the one the callers pass —
    /// `LimitState::default()`, i.e. UTC — so the window is built from the same
    /// clock the engine reads and the test does not depend on the host's zone.
    fn narrow_future_window() -> (&'static str, &'static str) {
        // Clock drift does not affect the assertions: the window holds only two marks within
        // the coming minute, so as long as the test finishes within the same minute, now < start always holds.
        let now_min = local_minutes_of_day(0);
        let s = now_min + 2;
        let e = now_min + 3;
        // Midnight wraparound remains a valid window; format as static HH:MM strings
        let fmt = |m: u32| {
            let m = m % (24 * 60);
            format!("{:02}:{:02}", m / 60, m % 60)
        };
        (
            Box::leak(fmt(s).into_boxed_str()),
            Box::leak(fmt(e).into_boxed_str()),
        )
    }

    #[tokio::test]
    async fn timewindow_supports_overnight_window() {
        let now_min = local_minutes_of_day(0);
        // Overnight window [23:00, 06:00]: now after 23:00 or before 06:00 must match
        let late = now_min >= 23 * 60;
        let early = now_min <= 6 * 60;
        assert_eq!(in_window(now_min, "23:00", "06:00"), late || early);
    }

    /// A ±30-minute window around the user's clock at `offset`, leaked to satisfy
    /// `candidate`'s `&'static str`. Wide enough that the minute ticking over
    /// mid-test cannot move `now` out of it.
    fn window_around(offset: i64) -> (&'static str, &'static str) {
        let now = local_minutes_of_day(offset);
        let fmt = |m: u32| {
            let m = m % (24 * 60);
            format!("{:02}:{:02}", m / 60, m % 60)
        };
        (
            Box::leak(fmt(now + 1440 - 30).into_boxed_str()),
            Box::leak(fmt(now + 30).into_boxed_str()),
        )
    }

    /// The window is matched against the stored offset, not the daemon's host
    /// zone: one instant, two offsets 12 hours apart, opposite picks. Before the
    /// offset was threaded through, both halves read `chrono::Local` and this
    /// could not be stated at all.
    #[tokio::test]
    async fn timewindow_reads_the_stored_offset_not_the_host_zone() {
        let engine = StrategyEngine::new();
        let s = store();
        let (start, end) = window_around(480);
        let r = route(
            StrategyType::Timewindow,
            vec![
                candidate("a", 1, None),
                candidate("b", 1, Some((start, end))),
            ],
        );

        // The window was built on the UTC+8 user's clock → it is the one serving.
        let at_utc8 = engine
            .select(&s, &r, None, &crate::limits::LimitState::with_offset(480))
            .await
            .unwrap();
        assert_eq!(at_utc8.id, "b");

        // The same instant read at UTC-4 is 12 hours away from that window.
        let at_utc_minus4 = engine
            .select(&s, &r, None, &crate::limits::LimitState::with_offset(-240))
            .await
            .unwrap();
        assert_eq!(at_utc_minus4.id, "a");
    }

    /// A state with exactly one provider over an amount limit.
    fn blocked(id: &str) -> crate::limits::LimitState {
        crate::limits::LimitState::from_reasons([(
            id.to_string(),
            crate::limits::BlockReason::Spend {
                used: 31.0,
                limit: 30.0,
                unit: "CNY".into(),
                window: None,
            },
        )])
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
        engine.record("claude", "b", false, false).await;
        engine.record("claude", "b", false, false).await;
        engine.record("claude", "b", false, false).await;
        engine.record("claude", "b", false, false).await;
        engine.record("claude", "b", false, false).await;

        let picked = engine.select(&s, &r, None, &blocked("a")).await.unwrap();
        assert_eq!(picked.id, "b", "the fallback must not reach for 'a'");
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

    #[tokio::test]
    async fn the_quota_day_is_the_users_day() {
        let s = store();
        let engine = StrategyEngine::new();
        // The offset the rest of the product runs on, from the same blob.
        s.set_app_setting("ui", r#"{"tz_offset_minutes":480}"#)
            .unwrap();
        s.insert_provider(&crate::store::Provider {
            id: "a".into(),
            name: "a".into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
            billing: crate::store::Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        })
        .unwrap();

        let mut r = route(
            StrategyType::Quota,
            vec![candidate("a", 1, None), candidate("b", 1, None)],
        );
        r.config = Some(r#"{"limit": 5, "unit": "requests"}"#.into());

        // A row the two boundaries date differently — and which direction that
        // is depends on the hour. While the local date still matches UTC's, the
        // local day's early hours are the previous UTC day, so the user's rule
        // counts a row UTC's does not. Once the local date has rolled over,
        // it is UTC's morning that is local yesterday, and the user's rule
        // counts one fewer. Either way this row is the difference.
        let now = chrono::Utc::now().timestamp();
        let utc_day = now.div_euclid(86_400);
        let local_day = (now + 480 * 60).div_euclid(86_400);
        let (row_ts, expected) = if local_day == utc_day {
            (local_day * 86_400 - 480 * 60 + 3_600, "b") // 01:00 local, UTC yesterday
        } else {
            (utc_day * 86_400 + 12 * 3_600, "a") // UTC noon, local yesterday
        };
        for i in 0..5 {
            let mut row = usage_row("a");
            row.ts = chrono::DateTime::from_timestamp(row_ts + i, 0)
                .expect("a valid instant")
                .to_rfc3339();
            s.record_usage(&row).unwrap();
        }
        assert!(
            crate::limits::period_start(row_ts, Some("day"), 0)
                .0
                .as_deref()
                != crate::limits::period_start(row_ts, Some("day"), 480)
                    .0
                    .as_deref(),
            "the row must be one the two boundaries date differently, or this proves nothing"
        );

        let limits = crate::limits::evaluate(&s);
        assert_eq!(
            limits.tz_offset_minutes(),
            480,
            "the snapshot carries the offset"
        );
        assert_eq!(
            engine.select(&s, &r, None, &limits).await.unwrap().id,
            expected,
            "the quota's day is the user's day, not UTC's"
        );
    }

    /// The threshold decides where a conversation *starts*. One that is already
    /// running finishes where it is — the switch is for the next one.
    ///
    /// This is the difference the drain makes: the same usage state used to move
    /// the whole agent, mid-conversation for whoever was talking, and the
    /// conversation paid for it by rebuilding a prefix its provider had already
    /// cached and charged for once.
    /// The window decides where a conversation starts; closing it does not pull
    /// a running one across.
    #[tokio::test]
    async fn quota_over_limit_sinks_to_backup() {
        let s = store();
        s.insert_provider(&crate::store::Provider {
            id: "a".into(),
            name: "a".into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: Protocol::Anthropic,
            base_url: "https://a.example.com".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: None,
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
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
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "a"
        );

        // Primary records 5 rows → over the limit, sink to backup
        for _ in 0..5 {
            s.record_usage(&usage_row("a")).unwrap();
        }
        assert_eq!(
            engine
                .select(&s, &r, None, &crate::limits::LimitState::default())
                .await
                .unwrap()
                .id,
            "b"
        );
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

    /// The read path stays lenient — a row already in the database must not
    /// start failing requests — while the write path rejects what the engine
    /// would silently reinterpret.
    #[test]
    fn quota_config_validation_is_stricter_than_parsing() {
        // Defaults fill in, and the round trip survives.
        let cfg = QuotaConfig::from_json(r#"{"limit": 100}"#).unwrap();
        assert_eq!(cfg.unit, "requests");
        assert_eq!(QuotaConfig::from_json(&cfg.to_json()).unwrap().limit, 100.0);

        // Lenient: parse accepts a unit it will fall back from.
        assert!(QuotaConfig::parse(Some(r#"{"limit":1,"unit":"cost"}"#)).is_some());
        // Strict: a writer is told instead of silently counting something else.
        let err = QuotaConfig::from_json(r#"{"limit":1,"unit":"cost"}"#).unwrap_err();
        assert!(err.contains("cost"), "{err}");
        assert!(err.contains("requests|tokens"), "{err}");

        for bad in [
            r#"{"limit":0}"#,
            r#"{"limit":-5}"#,
            r#"{"unit":"tokens"}"#,
            "not json",
            r#"{"limit":10,"period":"week"}"#,
        ] {
            assert!(
                QuotaConfig::from_json(bad).is_err(),
                "should have been rejected: {bad}"
            );
        }

        // The period the engine does support stays accepted.
        assert!(QuotaConfig::from_json(r#"{"limit":10,"period":"day"}"#).is_ok());
    }
}
