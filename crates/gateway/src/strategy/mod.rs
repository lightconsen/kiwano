//! Strategy engine (tech.md §4.7): per request, select 1 Provider from the Agent's ordered candidate set.
//!
//! Strategies (agent_strategies.type):
//! - `single` — fixed primary (MVP semantics, equivalent to the current switch flow)
//! - `failover` — the first candidate in priority order
//! - `roundrobin` — session-granularity stickiness + weighted rotation (a session keeps its
//!   Provider to preserve the upstream prompt cache; new sessions pick by weight)
//! - `timewindow` — match the binding's local `HH:MM` window (supports crossing
//!   midnight); falls back to the primary when nothing matches
//! - `quota` — when the primary's same-day usage (requests/tokens, read from usage
//!   aggregates) exceeds the threshold in the strategy config, sink to backups (failover semantics)
//! - `least-busy` — the candidate with the fewest requests currently in flight
//!   (sessions drain first, like the other sticky strategies); priority order breaks ties
//!
//! There is no failure circuit breaker: a provider is a candidate whenever its
//! binding is enabled and it is not over a billing limit. A failing attempt costs
//! one round trip and is retried or failed over by the data plane; nothing is
//! remembered about it afterwards, so a provider that recovers is used again on
//! the very next request without waiting out a clock or pressing a reset.
//!
//! The module is split by strategy and by the state each one reads. Every `pub`
//! item keeps the path it had when this was one file (`strategy::StrategyEngine`,
//! `strategy::QuotaConfig`): the facade below re-exports it, because `crates/cli`
//! and the rest of this crate still name it that way.
//!
//! `StrategyEngine` itself is defined here rather than in a submodule, because
//! every arm in the tree adds an `impl StrategyEngine` block to it and the
//! fields they reach through are `pub(crate)` — the same shape `store::Store`
//! has. `selection` owns the dispatch (`plan` names the head and the replay
//! tail), and each strategy arm lives with the state it reads: `timewindow` from
//! its own clock, `quota` from its config payload, `least_busy` from the
//! in-flight counter, and the sticky table every draining strategy pins through
//! is `sticky`.

pub mod failover;
pub mod least_busy;
pub mod prober;
pub mod quota;
pub mod roundrobin;
pub mod selection;
pub mod sticky;
pub mod timewindow;

// ── the public surface, re-exported so every `strategy::x` path still resolves ──

pub use quota::QuotaConfig;

use std::collections::HashMap;
use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex};

use sticky::StickyTable;

/// Per-Agent strategy runtime: the roundrobin sticky table and the least-busy
/// in-flight counters.
pub struct StrategyEngine {
    /// roundrobin: session key → candidate index (sticky to preserve the prompt
    /// cache), bounded — see [`StickyTable`].
    pub(crate) sticky: Mutex<StickyTable>,
    /// Cursor advanced on the weighted ring as new sessions join.
    pub(crate) cursor: Mutex<u64>,
    /// least-busy: requests currently in flight, per `agent:provider_id` — the
    /// gauge a new request's head choice reads. RAII-guarded (see
    /// `inflight_guard`), so an abandoned future cannot leak a count.
    pub(crate) inflight: Mutex<HashMap<String, Arc<AtomicUsize>>>,
}

impl Default for StrategyEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl StrategyEngine {
    pub fn new() -> Self {
        StrategyEngine {
            sticky: Mutex::new(StickyTable::new()),
            cursor: Mutex::new(0),
            inflight: Mutex::new(HashMap::new()),
        }
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::router::{AgentRoute, UpstreamProvider};
    use crate::store::{Protocol, Store, StrategyType};

    pub(crate) fn candidate(id: &str, weight: i64, win: Option<(&str, &str)>) -> UpstreamProvider {
        UpstreamProvider {
            id: id.into(),
            name: format!("prov-{id}"),
            catalog_id: None,
            protocol: Protocol::Anthropic,
            openai_wire: Default::default(),
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            endpoints: Vec::new(),
            endpoint_models: Vec::new(),
            api_key: Some(format!("sk-{id}")),
            extra_keys: Vec::new(),
            weight,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
            timeout_secs: None,
            retries: None,
            headers: None,
            proxy: None,
        }
    }

    pub(crate) fn route(strategy: StrategyType, candidates: Vec<UpstreamProvider>) -> AgentRoute {
        AgentRoute {
            agent: "claude".into(),
            strategy,
            config: None,
            candidates,
        }
    }

    pub(crate) fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// A state with exactly one provider over an amount limit.
    pub(crate) fn blocked(id: &str) -> crate::limits::LimitState {
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
}
