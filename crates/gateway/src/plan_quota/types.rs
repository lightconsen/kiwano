//! The wire contract of a plan quota query: the two view-models that cross
//! the Tauri IPC boundary and the `app_settings` KV cache, and the outcome a
//! template execution returns before the provider id and timestamp are
//! attached.

/// One usage window (five_hour / weekly_limit / monthly) of a plan.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PlanTierVm {
    /// five_hour | weekly_limit | monthly
    pub name: String,
    /// Percent of the window already used. Passed through unclamped
    /// (negative / >100 values are upstream's honest numbers).
    pub utilization: f64,
    pub resets_at: Option<String>,
    /// Absolute used / window cap, when the endpoint reports amounts
    /// (ZenMux USD, Volcengine AFP). Percentage-only endpoints leave
    /// both None.
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub unit: Option<String>,
}

/// Result of one plan quota query (possibly served from cache).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PlanQuotaReport {
    pub provider_id: String,
    pub template: String,
    /// false = deterministic failure; `error` carries the user-facing reason.
    pub success: bool,
    pub error: Option<String>,
    /// Plan metadata from the endpoint (Zhipu level, ZenMux tier, Volcengine plan).
    pub note: Option<String>,
    pub tiers: Vec<PlanTierVm>,
    /// Epoch millis of the original query.
    pub queried_at: i64,
    /// true when served from the 5-minute cache.
    pub cached: bool,
}

/// Outcome of a template execution, before the provider_id/timestamp
/// wrapper is attached by the caller.
pub enum QuotaOutcome {
    Ok {
        tiers: Vec<PlanTierVm>,
        note: Option<String>,
    },
    /// Deterministic failure; the message is shown as-is in the UI.
    Failed(String),
}
