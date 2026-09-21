//! The store's domain types: the row structs, the two tag enums, and the
//! aggregation results the queries below hand back.
//!
//! Field order is the wire format — `Provider` is built field-by-field in
//! the Tauri app and serialized to the frontend — so nothing here may be
//! reordered.

use serde::{Deserialize, Serialize};

/// Inbound provider protocol flavor (drives data-plane dispatch, tech.md §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Anthropic,
    OpenAI,
    Gemini,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Anthropic => "anthropic",
            Protocol::OpenAI => "openai",
            Protocol::Gemini => "gemini",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(Protocol::Anthropic),
            "openai" => Some(Protocol::OpenAI),
            "gemini" => Some(Protocol::Gemini),
            _ => None,
        }
    }
}

/// Billing model of a provider (tech.md §2.4 A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Billing {
    Subscription,
    Metered,
    Unlimited,
}

impl Billing {
    pub fn as_str(self) -> &'static str {
        match self {
            Billing::Subscription => "subscription",
            Billing::Metered => "metered",
            Billing::Unlimited => "unlimited",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "subscription" => Some(Billing::Subscription),
            "metered" => Some(Billing::Metered),
            "unlimited" => Some(Billing::Unlimited),
            _ => None,
        }
    }
}

/// Agent strategy type (tech.md §4.7.1). MVP only activates `Single`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyType {
    Single,
    Failover,
    Roundrobin,
    Timewindow,
    Quota,
}

impl StrategyType {
    pub fn as_str(self) -> &'static str {
        match self {
            StrategyType::Single => "single",
            StrategyType::Failover => "failover",
            StrategyType::Roundrobin => "roundrobin",
            StrategyType::Timewindow => "timewindow",
            StrategyType::Quota => "quota",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "single" => Some(StrategyType::Single),
            "failover" => Some(StrategyType::Failover),
            "roundrobin" => Some(StrategyType::Roundrobin),
            "timewindow" => Some(StrategyType::Timewindow),
            "quota" => Some(StrategyType::Quota),
            _ => None,
        }
    }
}

/// An additional per-protocol upstream endpoint of a provider (migration v7).
/// One vendor can expose several protocol flavors (e.g. an OpenAI-compatible
/// and a native Anthropic URL); the primary one lives in `Provider.protocol`
/// + `Provider.base_url`, the rest here — keyed by protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderEndpoint {
    pub protocol: Protocol,
    pub base_url: String,
    /// Optional upstream path prefix, same semantics as `Provider.api_path`.
    pub api_path: Option<String>,
}

/// One health-probe verdict (migration v21; the prober → `provider_health`).
///
/// `status` is deliberately two values and not a scale: the probe is an
/// unauthenticated GET to the endpoint, so all it can honestly report is whether
/// something answered, and how fast. Authorization is the API key's business and
/// the gateway never asks the probe about it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider_id: String,
    /// `reachable` | `down`.
    ///
    /// Two values, and a refusal is the first of them: a 401 is the vendor
    /// answering, which is what reachability asks. Whether it *accepted* the key
    /// is a different question — `error` carries its answer to that one.
    pub status: String,
    /// The round trip it measured; NULL for a probe that got no answer.
    pub latency_ms: Option<i64>,
    pub checked_at: String,
    /// Who measured it: `probe` (the gateway's unsigned GET) or `test` (the
    /// Apps screen's own latency test, which sends a real prompt with the
    /// provider's key). The two are not the same claim — one proves something
    /// answers there, the other proves your key works — so the cell says which.
    pub source: String,
    /// The vendor's own message when the answer was not a success ("invalid API
    /// key"), or the transport error when there was no answer at all.
    pub error: Option<String>,
}

/// A configured upstream provider.
///
/// `api_key` holds the upstream credential in the clear. A system keychain is not
/// on the roadmap for it — see `harden_permissions` for the reason, which is a
/// conflict with the daemon rather than an unimplemented feature — so what stands
/// in the way of a leak is file permissions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    /// The Hub catalog entry this provider was added from, when it came from
    /// the shelf (migration v12). NULL for a hand-added provider, and for one
    /// added before the column existed — both price at the general rate.
    ///
    /// Kept apart from `id` on purpose: `vm::add_provider` names a row
    /// `<slug>-<hex>`, so `Kimi (Moonshot)` is `kimi-moonshot-4f2a1c` while the
    /// catalog calls it `kimi`, and the price table is keyed by the latter.
    #[serde(default)]
    pub catalog_id: Option<String>,
    pub protocol: Protocol,
    pub base_url: String,
    /// Optional upstream path prefix, e.g. `/anthropic` for compatible endpoints.
    pub api_path: Option<String>,
    /// Additional per-protocol endpoints (migration v7): when an inbound
    /// request's protocol matches one of these, the gateway forwards natively
    /// to its URL instead of erroring or converting. Shared `api_key` pool.
    #[serde(default)]
    pub endpoints: Vec<ProviderEndpoint>,
    pub api_key: Option<String>,
    /// The model the add/edit form collected as this provider's default
    /// (migration v14). NULL = not set, which is every provider added before the
    /// column existed. Remembered, not consulted: nothing reads it when routing
    /// a request — the model comes from the request itself.
    #[serde(default)]
    pub model_default: Option<String>,
    pub billing: Billing,
    /// User-entered spending/period cap used for the ring percentage estimate.
    pub period_limit: Option<f64>,
    /// Unit of `period_limit`: requests | wan_tokens | <3-letter currency> (NULL
    /// reads as requests). `default` keeps v1 export files deserializable
    /// (config share, share.rs).
    #[serde(default)]
    pub limit_unit: Option<String>,
    /// Token-plan quota query as JSON `{"template":"kimi","fields":{...}}`
    /// (migration v9). NULL = no plan quota configured.
    #[serde(default)]
    pub plan_query: Option<String>,
    /// Plan-mode percent limits as JSON `{"five_hour":20,"weekly":60}`
    /// (migration v10): utilization ceilings over the vendor's rolling 5h /
    /// weekly windows. NULL = no percent limit set.
    #[serde(default)]
    pub plan_limits: Option<String>,
    /// Per-provider cap on the wait for upstream response headers, seconds
    /// (migration v8). NULL = gateway defaults (10s connect / 300s read).
    #[serde(default)]
    pub timeout_secs: Option<i64>,
    /// Same-provider re-attempts before the strategy layer moves on (v8).
    #[serde(default)]
    pub retries: Option<i64>,
    /// Custom request headers as a JSON object {"Name":"value"} (v8); merged
    /// after credential injection, so they can override the defaults.
    #[serde(default)]
    pub headers: Option<String>,
    /// The prices the user declared for this provider, as JSON
    /// `{"currency":"CNY","models":[…]}` (migration v20). NULL = none declared,
    /// which is every provider the Hub prices and the only state one added from
    /// the shelf can be in.
    ///
    /// Read with `model_pricing::DeclaredPrices::parse`, never by hand. The
    /// gateway keys these rows to the provider's own `id` and consults them
    /// *before* its catalog entry's prices, because they are the user's own
    /// statement about what this provider charges — and, for a hand-added
    /// provider, the only statement there is.
    #[serde(default)]
    pub prices: Option<String>,
    pub reset_period: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Per-agent strategy row (tech.md §4.7.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    pub agent: String,
    pub kind: StrategyType,
    /// JSON payload: roundrobin weights, timewindow timezone, quota thresholds, ...
    pub config: Option<String>,
}

/// Candidate binding of a provider for an agent (tech.md §4.7.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub agent: String,
    pub provider_id: String,
    /// 0 = primary, 1/2 = backup #1/#2 (failover order).
    pub priority: i64,
    pub weight: i64,
    pub win_start: Option<String>,
    pub win_end: Option<String>,
    pub enabled: bool,
}

/// Placeholder key `kw-ag-<agent>-<rand>` mapped to an agent (tech.md §4.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceholderKey {
    pub key: String,
    pub agent: String,
    pub created_at: String,
}

/// An agent the user defined (migration v16): a name for a route, and nothing
/// else. It has no config file, so nothing about it is derived from disk — the
/// row is the whole of it, plus the key the gateway attributes its traffic by.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CustomAgent {
    /// Derived from the label at creation and never changed afterwards.
    pub id: String,
    pub label: String,
    /// Free text: what this route is for. None when the user said nothing.
    pub note: Option<String>,
    /// The protocol this agent's clients speak (`"openai"`, `"anthropic"`,
    /// `"gemini"`), or None when the user said nothing — which is every agent
    /// defined before the column existed, and a different statement from any of
    /// the three words. A *label*: nothing routes by it yet (migration v24).
    pub protocol: Option<String>,
    pub created_at: String,
}

/// What one agent may spend, measured over a reset period.
///
/// Deliberately a table of its own rather than a column anywhere else: a limit is
/// not part of a strategy and must hold under every one of them, including
/// `single` — which is the strategy that otherwise opts out of every ceiling in
/// the codebase, by design. Keeping it separate is what makes that a decision
/// about the *limit* rather than an accident of where it was stored.
///
/// The columns mirror a provider's billing limit on purpose (`period_limit`,
/// `limit_unit`, and the same period vocabulary): the same three units and the
/// same `period_start` boundary, so "100 requests a day" means the same thing
/// wherever it is written.
///
/// An agent may hold **several windows at once**, and that is the point of the
/// pair being the key rather than the agent: a day's ceiling is what stops one
/// runaway session, a month's is what stops a runaway month, and neither answers
/// the other's question. Being over any one of them is being over. No rows for an
/// agent means no ceiling.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentLimit {
    pub agent: String,
    /// `day` | `weekly` | `monthly` | `yearly` | `all` — the window this ceiling
    /// is measured over, and half the key. `all` rather than NULL because a NULL
    /// key is not a key: SQLite treats every NULL as distinct, so a table keyed on
    /// (agent, period) could hold any number of "no period" rows for one agent.
    pub period: String,
    pub period_limit: f64,
    /// `requests` (default), `wan_tokens`, or a 3-letter currency for a money
    /// limit. None reads as `requests`, the same way a v1 provider row does.
    pub limit_unit: Option<String>,
    pub created_at: String,
    pub updated_at: String,
}

/// One extra API key of a provider (spec §4.1 P1 multi-key rotation).
/// `providers.api_key` is the pool's primary; these rotate after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyRow {
    pub id: i64,
    pub provider_id: String,
    pub api_key: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// One metered request (tech.md §4.3 request flow, usage capture).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// RFC3339 UTC timestamp of the request.
    pub ts: String,
    pub agent: String,
    pub provider_id: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub latency_ms: Option<i64>,
    pub status: String,
    /// Computed request cost in the price entry's currency (migration v9);
    /// NULL when the model is unpriced.
    pub cost: Option<f64>,
    /// Currency of `cost` (ISO code, e.g. "USD"); NULL when cost is NULL.
    pub cost_currency: Option<String>,
    /// What the same tokens would have cost at the price row's off-peak rates
    /// (migration v13) — equal to `cost` when the model publishes no schedule,
    /// so `cost - cost_off_peak` is a sum over every priced row. NULL exactly
    /// when `cost` is NULL.
    pub cost_off_peak: Option<f64>,
}

/// A cost summed over a window, with what the same rows would have cost at
/// their off-peak rates (migration v13). The two are equal for a model with no
/// published schedule.
#[derive(Debug, Clone, PartialEq)]
pub struct CostBucket {
    pub currency: Option<String>,
    pub cost: f64,
    pub cost_off_peak: f64,
}

/// The same pair, split per provider.
#[derive(Debug, Clone, PartialEq)]
pub struct ProviderCostBucket {
    pub provider_id: String,
    pub currency: Option<String>,
    pub cost: f64,
    pub cost_off_peak: f64,
}

/// Aggregated token/request totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

impl UsageTotals {
    pub(crate) fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(UsageTotals {
            requests: row.get(0)?,
            input_tokens: row.get(1)?,
            output_tokens: row.get(2)?,
            cache_read_tokens: row.get(3)?,
            cache_creation_tokens: row.get(4)?,
        })
    }
}

/// Request-log traffic over a time window: the anomaly detector's unit of
/// evidence (`Store::traffic_stats`).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct TrafficStats {
    pub requests: u64,
    pub errors: u64,
    /// Mean over the rows that recorded one; None when none did.
    pub avg_latency_ms: Option<f64>,
}

/// Per-provider aggregation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider_id: String,
    pub totals: UsageTotals,
}

/// Per-bucket aggregation result (sparkline / dashboard, tech.md §2.4 A).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUsage {
    /// The bucket the totals belong to: `YYYY-MM-DD` from `usage_daily`,
    /// `YYYY-MM-DDTHH` from `usage_hourly`. Doubles as the sort key.
    pub day: String,
    pub totals: UsageTotals,
}
