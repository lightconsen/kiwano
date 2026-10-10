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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where `latency_ms` came from: `traffic` (this provider's own requests in
    /// the window) or `probe` (the gateway's reachability check). Absent when
    /// there is no number, and the two must not be read as one: one is a real
    /// round trip with the user's key, the other is an unsigned hello.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// When the probe ran, for the cell's tooltip. `probe` only: a traffic
    /// average covers a window rather than an instant.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    /// What the endpoint said when it said no — the vendor's own message for a
    /// refused key, the transport error when nothing answered. Its presence is
    /// what makes the cell read as "the key" rather than as "no answer": a 401
    /// is the vendor responding, which reachability alone cannot tell apart from
    /// silence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub logo_border: bool,
    /// The catalog entry this provider was added from, when it came from the
    /// shelf. The dialog needs it to reach the entry again: the entry is the
    /// authority on the endpoints the provider answers on and on the currency it
    /// bills in, and a stored row can be missing both.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<ProviderEndpointVm>,
    /// The OpenAI wire this provider is declared as serving, when it is not
    /// `both` (migration v30). Absent is the pass-through reading, which is every
    /// provider that predates the column — so the field is a *restriction* being
    /// reported, not a setting being echoed, and a caller that does not show it
    /// shows the common case correctly.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_wire: Option<String>,
    pub billing: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_price: Option<String>,
    /// Raw limit unit (requests | wan_tokens | ISO currency) for edit prefill.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_unit: Option<String>,
    /// The model the add/edit form collected as this provider's default, for
    /// edit prefill. Absent when never set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fallback_agents: Vec<String>,
    pub is_current: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status_badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents_note: Option<String>,
    pub health: HealthVm,
    pub usage: Option<UsageVm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced: Option<ProviderAdvancedVm>,
    /// Token-plan quota query JSON (edit prefill); None = not configured.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_query: Option<serde_json::Value>,
    /// Plan-mode percent limits JSON `{"five_hour":20,"weekly":60}` (edit
    /// prefill); None = not set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_limits: Option<serde_json::Value>,
    /// The prices the user declared for this provider:
    /// `{"currency":"CNY","models":[…]}`. Absent = none declared, so its
    /// requests are priced from the Hub's table (or recorded unpriced when the
    /// Hub knows nothing about the model either). Edit prefill: the form that
    /// collected them is the only one that can correct them.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
///
/// `default` on both because the **stored** column is written by the daemon's
/// `plan_limits_json`, which omits a window rather than writing it as null — so
/// `{"five_hour":80}` is what a provider with only a five-hour ceiling carries.
/// Without this the daemon could not read back its own column, which is what a
/// patch needs in order to merge window by window.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlanLimitsInput {
    #[serde(default)]
    pub five_hour: Option<f64>,
    #[serde(default)]
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
    /// Which OpenAI wire this endpoint serves: `chat`, `responses`, or `both`
    /// (migration v30). Absent means `both`, which is every provider added
    /// before the field existed and the reading that changes nothing: whatever
    /// arrives is passed through, and the vendor's own 404 is what tells a user
    /// they declared it wrong.
    ///
    /// Only meaningful for an OpenAI-protocol provider; a value sent with any
    /// other protocol is refused rather than stored, because a column nobody
    /// reads is one nobody can be told about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_wire: Option<String>,
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
// ── The edit's request body ──
//
// Separate from `NewProviderInput` because the two speak differently, and the
// difference is the point: the add form speaks about **every** field, because
// it is what the user just filled in, and an edit speaks about **the ones it
// names**. One type cannot hold both — the same argument
// `NewProviderInput::agents` already makes for `Option<Vec<_>>` over `Vec<_>`:
// "a plain `Vec` cannot tell 'no agents' from 'not speaking about agents'".
//
// That is what lets a client without a form edit a provider at all. The CLI has
// none: it read the row back, rebuilt a whole input from it, and sent that, so
// editing needed a database read it was not otherwise entitled to
// (`migrate.local.md` §14.1). A client with a form has the values in hand; one
// without should be able to say only what changed.

/// The limit fields an edit may change — each absent one keeps what is stored.
///
/// Field-granular rather than a snapshot, because the flags are:
/// `providers edit --limit 50` names no billing mode and must not disturb one,
/// and `--plan-limit-5h 80` must leave the weekly window alone.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct BillingConfigPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_value: Option<f64>,
    /// A currency code, or `""` to clear it — the same spelling
    /// `update_agent_binding` uses for its window, because a JSON `null` and an
    /// absent key are the same thing to serde.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub limit_unit: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset_period: Option<String>,
    /// Merged window by window, so naming one keeps the other.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_limits: Option<PlanLimitsInput>,
}

/// The advanced forwarding settings an edit may change — absent keeps, present
/// sets.
///
/// A distinct type from [`AdvancedInput`] rather than the same struct under a
/// new name, because there the object **is** the resulting state (a null field
/// clears it) and here it is a list of changes. Same fields, opposite meaning
/// for "absent": one type carrying both is how a caller ends up clearing a
/// retry count it never mentioned.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AdvancedPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub retries: Option<i64>,
    /// Present replaces the whole header set, so an empty map clears it — which
    /// is what `providers edit --no-headers` sends.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub headers: Option<std::collections::BTreeMap<String, String>>,
}

/// The fields an edit may change, and only those it names.
///
/// Absent = leave it. Where "clear it" has to be expressible, the field's own
/// spelling says so: an empty `model_default`, `limit_unit` or `catalog_id`, an
/// empty `headers` map, or `plan_query: null` — which is what
/// `--clear-plan-query` sends, and why that is a flag rather than an empty
/// value.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProviderPatch {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    /// Absent or empty keeps the stored key, as it always did: a key that is
    /// not being replaced is not usually being spoken about.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoint: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub protocol: Option<String>,
    /// Which OpenAI wire the endpoint serves (migration v30). Absent keeps the
    /// stored value; the same values `NewProviderInput` takes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub openai_wire: Option<String>,
    /// Empty clears the column (`model_default` stores NULL for none).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model_default: Option<String>,
    /// `plan` | `payg` | `unl`. Changing the mode recomputes which columns carry
    /// the limits, and keeps whatever `billing_config` says — or the stored
    /// values when it says nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub billing_config: Option<BillingConfigPatch>,
    /// Present replaces the bound set (an empty one unbinds everything);
    /// absent leaves the bindings exactly as they are.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agents: Option<Vec<String>>,
    /// Present = the resulting set, so an empty list drops every extra
    /// endpoint.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub endpoints: Option<Vec<NewEndpointInput>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub advanced: Option<AdvancedPatch>,
    /// Absent keeps; `null` clears; a present object replaces.
    ///
    /// The doubled `Option` is what makes `null` and "absent" different things
    /// across the wire. serde collapses a JSON `null` into `None` for
    /// `Option<T>`, so a one-layer field cannot tell "clear the query" from
    /// "not speaking about it" — and the two must differ, because clearing is
    /// what `providers edit --clear-plan-query` means. Written as
    /// `Option<Option<_>>`: absent keeps the outer `None`, `null` arrives as
    /// `Some(None)`, an object as `Some(Some(v))`.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub plan_query: Option<Option<serde_json::Value>>,
    /// Present = a snapshot, so an empty model list clears the column.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prices: Option<ProviderPricesInput>,
    /// Absent or empty keeps the stored link — an edit must not silently
    /// unlink a provider from the price row it is costed at.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
}

/// A provider named by id, with the two fields a client needs to speak about it
/// without reading its row: what to call it, and how it is billed.
///
/// The CLI edits providers by flags, so it has no form to prefill — but two of
/// its rules are stated in terms of the **stored** state: which limits apply
/// (plan ceilings or a spend ceiling) and whether the two plan-window flags are
/// legal at all. `billing` is the whole of what those need. The rest of the row
/// stays on the daemon, which is where the patch leaves it
/// (`migrate.local.md` §10.38).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderRefVm {
    pub id: String,
    pub name: String,
    /// `plan` | `payg` | `unl` — the display spelling, not the column's.
    pub billing: String,
}

/// Deserialize a field that is present as `Some`, whatever it holds — `null`
/// included.
///
/// The other half of [`ProviderPatch::plan_query`]'s doubled `Option`: it runs
/// only when the key is there, so `None` (from `default`) keeps meaning "the
/// caller did not mention this".
fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<i64>,
    /// The pricing half was already current, so models.json was not fetched.
    pub pricing_unchanged: bool,
}
