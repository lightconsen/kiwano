//! Provider-side wire types: what the Apps screen, the CLI and (after the
//! migration) the daemon's API all describe a provider with.
//!
//! These lived in `kiwano-core` until the API needed them: `crates/gateway`
//! does not depend on core, so the daemon could not name the types it is
//! supposed to serve. Moving them here makes them visible to both sides —
//! `kiwano-api` depends on nothing local, only serde.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QuotaVm {
    pub used: f64,
    pub limit: f64,
    pub unit: String,
    pub resets_at: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderAdvancedVm {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    pub headers: std::collections::BTreeMap<String, String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderEndpointVm {
    pub protocol: String,
    pub endpoint: String,
}

// ── The add/edit form's request body ──
//
// Moved here when the daemon took over `add_provider` (`migrate.local.md`
// §10.13). They derive **both** directions, which they did not before: as
// request types the app reads them from the webview and then *sends* them, and
// a type that can only be deserialized cannot be written by the side that has
// one. That was a recorded finding of the contract skeleton; this closes it.
//
// `ProviderPricesInput` came from `vm::limits` rather than `provider_edit`,
// because the prices a user declares are a limit's subject — the two travelled
// together before and there is no reason to separate them now.

/// Plan-mode percent limits (modal form): utilization ceilings over the
/// vendor's rolling 5h / weekly windows. Both optional; both absent = none.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanLimitsInput {
    pub five_hour: Option<f64>,
    pub weekly: Option<f64>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BillingConfigInput {
    pub limit_value: Option<f64>,
    #[allow(dead_code)]
    pub limit_unit: Option<String>,
    pub reset_period: Option<String>,
    pub plan_limits: Option<PlanLimitsInput>,
}
/// Per-provider advanced forwarding settings (timeout / retries / custom
/// headers), edited in the provider modal's Advanced section. Custom header
/// names/values are sanitized before they reach the gateway.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdvancedInput {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    /// Header name → value; serialized to a JSON object column.
    pub headers: Option<std::collections::BTreeMap<String, String>>,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewEndpointInput {
    pub protocol: String,
    pub endpoint: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NewProviderInput {
    pub name: String,
    #[allow(dead_code)]
    pub api_key: String,
    pub endpoint: String,
    pub protocol: String,
    /// The model the form collected as this provider's default.
    ///
    /// It was carried here for a long time and read by nothing — stored in no
    /// column and returned by no view, so reopening a provider always showed an
    /// empty box. Persisted since v14 (`providers.model_default`), which is what
    /// lets the edit dialog show what the add dialog asked for.
    ///
    /// Remembered rather than consulted: nothing picks a model from it when
    /// routing, because the model a request uses is the one the agent sent. Empty
    /// stores `NULL`.
    pub model_default: String,
    pub billing: String,
    pub billing_config: BillingConfigInput,
    /// Agents to bind this provider to.
    ///
    /// Expected when adding — a new provider nothing serves is a dead row — and
    /// **absent when editing**, where the bindings belong to the Apps screen's
    /// agent tabs. The distinction has to be in the type: a plain `Vec` cannot
    /// tell "no agents" from "not speaking about agents", and the difference is
    /// whether an edit leaves the bindings alone or unbinds the lot.
    #[serde(default)]
    pub agents: Option<Vec<String>>,
    /// Additional per-protocol endpoints; unknown protocol strings are
    /// skipped (defaulting one to openai could collide with the primary).
    #[serde(default)]
    pub endpoints: Vec<NewEndpointInput>,
    /// Advanced forwarding settings. Absent in an update = keep existing
    /// (mirrors the empty-api_key semantics); a present object is an
    /// authoritative snapshot whose null fields clear values.
    #[serde(default)]
    pub advanced: Option<AdvancedInput>,
    /// Token-plan quota query `{"template":"kimi","fields":{...}}`. Absent in
    /// an update = keep existing; null clears; a present object replaces.
    #[serde(default)]
    pub plan_query: Option<serde_json::Value>,
    /// The prices the user declared for this provider. Absent in an update =
    /// keep what is stored (same semantics as an empty `api_key`); a present
    /// bundle is an authoritative snapshot, so an empty model list clears the
    /// column — which is what switching a provider off pay-as-you-go does.
    #[serde(default)]
    pub prices: Option<ProviderPricesInput>,
    /// The Hub catalog entry this provider is being added from, when the add
    /// came from the shelf. Prices are published per catalog entry, and a local
    /// row's own id is `<slug>-<hex>`, so this is what lets a forwarded request
    /// be costed at its provider's own rate rather than the general one.
    ///
    /// Absent (the hand-added form, and every edit) means "no catalog entry":
    /// on add the provider prices at the general rate, and on update the stored
    /// value is kept — an edit must not silently unlink the provider from its
    /// price row.
    #[serde(default)]
    pub catalog_id: Option<String>,
}
/// The prices a user declared for a provider, as the modal form sends them: one
/// currency for every figure, one row per model.
///
/// Asked for on a pay-as-you-go provider the form is the user's own (a hand-added
/// one, or an edit) because the Hub prices the models of *its* catalog entries —
/// a provider that names none has no published price to be costed at, so the user
/// is the only one who can say what it charges. These figures are also what its
/// spending limit is measured against.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPricesInput {
    /// ISO code the figures are denominated in. The form fills it from the same
    /// picker the spending limit uses: both are about what this provider bills.
    pub currency: String,
    #[serde(default)]
    pub models: Vec<ProviderPriceInput>,
}
/// One model's declared rates, per million tokens, as the user typed them.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderPriceInput {
    pub model_id: String,
    /// Kept as text: every rate in the price table is a TEXT decimal, and
    /// re-printing one from the parsed f64 would rewrite what the user wrote.
    pub input: String,
    pub output: String,
    /// Blank is zero — "this vendor charges nothing for that bucket", which is
    /// what the form's hint says. Charging the input rate instead would invent a
    /// charge and overstate the spend a limit is measured against.
    #[serde(default)]
    pub cache_read: Option<String>,
    #[serde(default)]
    pub cache_creation: Option<String>,
}

/// Result of a Hub sync, as the Settings screen shows it.
///
/// Moved here from `kiwano-core` when the daemon took over the sync
/// (`migrate.local.md` §10.14): the daemon produces it, so it is a wire type
/// rather than a view model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SyncReportVm {
    pub fetched: i64,
    pub synced_at: String,
    pub hub_url: String,
    /// Conditional sync: the manifest sha256 matched the cached catalog, so
    /// catalog.json was not re-downloaded. `synced_at` still refreshed — the
    /// app confirmed it is current, which is what the footer badge claims.
    pub unchanged: bool,
    /// Version of the price table now cached; None when the Hub offers no
    /// pricing (unreachable, malformed, or absent — the previous cache, if any,
    /// stands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<i64>,
    /// The pricing half was already current, so models.json was not fetched.
    pub pricing_unchanged: bool,
}
