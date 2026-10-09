//! The Dashboard, the footer and the currency picker: the shapes those screens
//! are drawn from.
//!
//! Moved here from `kiwano-core` when the daemon took over assembling them
//! (`migrate.local.md` §10.21): the daemon produces them, so they are wire types
//! and both sides must be able to name them. Both directions derive for the same
//! reason — the client parses what the daemon wrote.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrendVm {
    pub date: String,
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderDistVm {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Requests attributed to this provider in the window — what `pct` is a
    /// share of, so a chart can size its segments without re-deriving them.
    pub requests: i64,
    pub pct: i64,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockedProviderVm {
    pub id: String,
    pub reason: String,
}

/// One select option of the dashboard's provider/agent filters.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilterOptionVm {
    pub id: String,
    pub label: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDistVm {
    pub agent: String,
    pub label: String,
    pub requests: i64,
    pub tokens: String,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DashboardVm {
    pub window: String,
    pub requests: i64,
    /// How this window's request count compares with the window before it, in
    /// percent. `None` when there is nothing to compare against — the "all"
    /// window has no earlier period, and an earlier period with no traffic in it
    /// has no percentage to give. Absent is not 0: a delta of zero says the two
    /// windows matched, which is a claim of its own.
    pub requests_delta_pct: Option<i64>,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
    pub latency_ms: i64,
    /// The same comparison for the average latency, and absent for the same
    /// reasons. Positive means slower than the window before it; the screen
    /// colours it as the bad direction.
    pub latency_delta_pct: Option<i64>,
    pub trend: Vec<TrendVm>,
    pub by_provider: Vec<ProviderDistVm>,
    pub by_agent: Vec<AgentDistVm>,
    /// Filter select options: providers/agents with traffic in the window,
    /// computed independent of the active filter (otherwise the option list
    /// would collapse to the current selection).
    pub filter_providers: Vec<FilterOptionVm>,
    pub filter_agents: Vec<FilterOptionVm>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FooterStatsVm {
    pub today_requests: i64,
    /// Tokens consumed today (input + output) — the footer's headline metric;
    /// cost stays out of the status bar until price tables land (P1).
    pub today_tokens: i64,
    pub hub_synced: bool,
    pub version: String,
}

/// Currency metadata for the Settings selector + UI conversion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CurrencyMetaVm {
    /// The codes the selector offers: the rate table's keys, plus the current
    /// preference if the table has no rate for it.
    pub currencies: Vec<String>,
    /// currency -> units of that currency per 1 USD (e.g. CNY: 7.1).
    pub exchange_rates: HashMap<String, f64>,
    /// The user's preferred display currency (Settings).
    pub preferred: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GatewayStatusVm {
    pub running: bool,
    pub port: u16,
    /// Providers the gateway is refusing to route, with the reason it gave.
    ///
    /// Read from the gateway rather than recomputed here: the block is decided
    /// there, and a card that worked out its own answer could disagree with the
    /// process actually turning requests away.
    pub blocked: Vec<BlockedProviderVm>,
    /// The gateway is up and writing to a *different* database than this app
    /// reads — so every number in this window is about a store the gateway is
    /// not filling. No error reports this on its own; the identity comparison
    /// does (see `crate::sidecar::database_agreement`).
    pub db_mismatch: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageAlertVm {
    pub provider_id: String,
    pub provider_name: String,
    pub used: f64,
    pub limit: f64,
    /// requests | wan_tokens | 3-letter ISO currency code
    pub unit: String,
    /// Which check raised this: `provider_limit` | `plan_window` |
    /// `cost_forecast` | `anomaly` | `agent_limit`. The first two are formatted
    /// by the frontend from the numbers; the rest carry their text in
    /// `message`.
    pub kind: String,
    /// Pre-built notification text for the feature alerts (English, the same
    /// convention as insights findings); empty for the two legacy kinds.
    pub message: String,
}
