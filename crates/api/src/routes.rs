//! Agent routing wire types: which providers an agent may use, in what order,
//! and the ceilings and strategy that ride along.
//!
//! These lived in `kiwano-core` until the daemon started serving them
//! (`migrate.local.md` §10.8). The derives are the full set rather than the
//! original `Serialize`-only, because the daemon produces these and a client
//! parses them: the two directions need both halves, and a response type that
//! cannot be read back cannot be frozen in a fixture either.

use serde::{Deserialize, Serialize};

/// One candidate in an agent's route, as the Apps screen lists it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BindingVm {
    pub provider_id: String,
    pub provider_name: String,
    pub logo_char: String,
    pub logo_color: String,
    pub priority: i64,
    pub weight: i64,
    /// Local "HH:MM" window bounds (timewindow strategy); null = no window.
    pub win_start: Option<String>,
    pub win_end: Option<String>,
    pub enabled: bool,
}

/// UI projection of agent_strategies + agent_bindings.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentRouteVm {
    pub agent: String,
    /// single | failover | roundrobin | timewindow | quota
    pub strategy: String,
    /// Strategy JSON payload (quota: {"limit","unit"}; null otherwise)
    pub config: Option<String>,
    /// Candidates in ascending priority order (index 0 = primary)
    pub bindings: Vec<BindingVm>,
    /// The agent's own ceilings, one per window, empty when it has none. Not part
    /// of the strategy — they hold under every one of them — but read with the
    /// route because that is the fetch the agent's tab already makes.
    #[serde(default)]
    pub limits: Vec<AgentLimitVm>,
}

/// One window of an agent's spend ceiling, as the Apps screen edits it.
///
/// An agent holds a list of these: a day's ceiling and a month's answer different
/// questions, and being over either is being over. The screen edits the list as a
/// set, which is why it is a `Vec` at every layer rather than a struct with a
/// window per field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentLimitVm {
    /// `day` | `weekly` | `monthly` | `yearly` | `all` — the window this ceiling
    /// is measured over.
    pub period: String,
    pub period_limit: f64,
    /// `requests` (default), `wan_tokens`, or a 3-letter currency code.
    pub limit_unit: Option<String>,
}
