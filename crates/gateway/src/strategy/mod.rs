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
//!
//! The module is split by strategy and by the state each one reads. Every `pub`
//! item keeps the path it had when this was one file (`strategy::StrategyEngine`,
//! `strategy::QuotaConfig`): the facade below re-exports it, because `crates/cli`
//! and the rest of this crate still name it that way.
//!
//! `StrategyEngine` itself is defined here rather than in a submodule, because
//! every arm in the tree adds an `impl StrategyEngine` block to it and the three
//! fields they reach through are `pub(crate)` — the same shape `store::Store`
//! has. `selection` owns the dispatch (`plan` names the head and the replay
//! tail), `breaker_registry` owns the breakers it dispatches around, and each
//! strategy arm lives with the state it reads: `failover` and `roundrobin`
//! answer from the breakers, `timewindow` from its own clock, `quota` from its
//! config payload, and the sticky table every draining strategy pins through is
//! `sticky`.

pub mod breaker_registry;
pub mod circuit_breaker;
pub mod failover;
pub mod prober;
pub mod quota;
pub mod roundrobin;
pub mod selection;
pub mod sticky;
pub mod timewindow;

// ── the public surface, re-exported so every `strategy::x` path still resolves ──

pub use quota::QuotaConfig;

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use circuit_breaker::CircuitBreaker;
use sticky::StickyTable;

/// Per-Agent strategy runtime: breaker registry + roundrobin sticky table.
pub struct StrategyEngine {
    pub(crate) breakers: Mutex<HashMap<String, Arc<CircuitBreaker>>>,
    /// roundrobin: session key → candidate index (sticky to preserve the prompt
    /// cache), bounded — see [`StickyTable`].
    pub(crate) sticky: Mutex<StickyTable>,
    /// Cursor advanced on the weighted ring as new sessions join.
    pub(crate) cursor: Mutex<u64>,
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
