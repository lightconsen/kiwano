//! VM types (serde field names mirror src/api/types.ts verbatim).
//!
//! The whole wire contract in one file. Declaration order is JSON key order —
//! `kiwano-core` does not enable `preserve_order` — and `app/src/api/types.ts`
//! mirrors these structs field for field, so a field moved here is a field
//! moved in the payload.

use serde::Serialize;

#[derive(Serialize)]
pub struct HealthVm {
    /// `ok` | `idle` | `off` | `error` — what the dot is drawn from.
    pub state: String,
    pub latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where `latency_ms` came from: `traffic` (this provider's own requests in
    /// the window) or `probe` (the gateway's reachability check). Absent when
    /// there is no number, and the two must not be read as one: one is a real
    /// round trip with the user's key, the other is an unsigned hello.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// When the probe ran, for the cell's tooltip. `probe` only: a traffic
    /// average covers a window rather than an instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    /// What the endpoint said when it said no — the vendor's own message for a
    /// refused key, the transport error when nothing answered. Its presence is
    /// what makes the cell read as "the key" rather than as "no answer": a 401
    /// is the vendor responding, which reachability alone cannot tell apart from
    /// silence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Serialize)]
pub struct QuotaVm {
    pub used: f64,
    pub limit: f64,
    pub unit: String,
    pub resets_at: Option<String>,
}

#[derive(Serialize)]
pub struct UsageVm {
    pub requests: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    /// Needed for the cache hit rate's denominator: input-side tokens are
    /// new input plus cache reads plus cache writes, and a rate that forgets
    /// the writes overstates every hit.
    pub cache_creation_tokens: i64,
    pub output_tokens: i64,
    pub cost: Option<f64>,
    /// Currency of `cost` — the provider's own, never converted. Absent when
    /// no usage row carried a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_currency: Option<String>,
    pub latency_ms: Option<i64>,
    pub quota: Option<QuotaVm>,
    pub spark: Option<Vec<f64>>,
}

/// Advanced forwarding settings echoed back to the modal for edit prefill.
#[derive(Serialize, Clone)]
pub struct ProviderAdvancedVm {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Serialize)]
pub struct ProviderVm {
    pub id: String,
    pub name: String,
    pub logo_char: String,
    pub logo_color: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub logo_border: bool,
    /// The catalog entry this provider was added from, when it came from the
    /// shelf. The dialog needs it to reach the entry again: the entry is the
    /// authority on the endpoints the provider answers on and on the currency it
    /// bills in, and a stored row can be missing both.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
    /// The currency this provider's figures are denominated in — what the user
    /// declared, else the catalog entry's, else USD. The spending-limit picker
    /// reads it: a limit is written in one of the currencies its agent's
    /// providers actually bill in, not in the display currency, which is a
    /// preference about reading numbers rather than about which money is spent.
    pub currency: String,
    pub endpoint: String,
    pub protocol: String,
    pub endpoint_note: String,
    /// Additional per-protocol endpoints (primary excluded).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<ProviderEndpointVm>,
    pub billing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_price: Option<String>,
    /// Raw limit unit (requests | wan_tokens | ISO currency) for edit prefill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_unit: Option<String>,
    /// The model the add/edit form collected as this provider's default, for
    /// edit prefill. Absent when never set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_default: Option<String>,
    pub enabled: bool,
    pub agents: Vec<String>,
    /// Agents this provider would serve a request for right now (per-agent
    /// slice of the strategy serving map). The All tab badges the collapsed
    /// `is_current`; an agent tab badges membership here instead, so a
    /// provider serving another agent does not read as in-use locally.
    pub serving_agents: Vec<String>,
    /// Agents for which this provider is the quota strategy's configured first
    /// backup while that agent's primary is over its threshold. Not "in use":
    /// which backup actually serves depends on the gateway's breakers at
    /// request time, so the UI badges this as first-in-line instead.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallback_agents: Vec<String>,
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_badge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_note: Option<String>,
    pub health: HealthVm,
    pub usage: Option<UsageVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<ProviderAdvancedVm>,
    /// Token-plan quota query JSON (edit prefill); None = not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_query: Option<serde_json::Value>,
    /// Plan-mode percent limits JSON `{"five_hour":20,"weekly":60}` (edit
    /// prefill); None = not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_limits: Option<serde_json::Value>,
    /// The prices the user declared for this provider:
    /// `{"currency":"CNY","models":[…]}`. Absent = none declared, so its
    /// requests are priced from the Hub's table (or recorded unpriced when the
    /// Hub knows nothing about the model either). Edit prefill: the form that
    /// collected them is the only one that can correct them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prices: Option<serde_json::Value>,
}

/// An additional per-protocol endpoint of a provider (migration v7): the
/// gateway forwards natively here when an inbound request speaks `protocol`.
#[derive(Serialize)]
pub struct ProviderEndpointVm {
    pub protocol: String,
    pub endpoint: String,
}
