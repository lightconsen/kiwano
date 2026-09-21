//! The provider read model: `ProviderVm` and everything that fills it in —
//! health, usage, limits, declared prices, and the badges a route contributes.
//!
//! Owns the billing vocabulary too (`billing_to_db`/`billing_to_ui`): the write
//! paths in `provider_edit` import from here, never the other way round.

use crate::detect::ShellVars;
use crate::vm::catalog::{catalog_snapshot, CatalogEntryVm};
use crate::vm::fmt::{logo_char, palette_color};
use crate::vm::settings::tz_offset;
use crate::vm::time::{in_window, local_day_start, local_minutes_now, rfc3339, unix_now};
use crate::vm::{e2s, Aux};
use kiwanod::store::{Billing, Binding, Provider, Store, Strategy, StrategyType, UsageTotals};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::Path;

/// Whether the quota config puts `provider_id` over threshold for the current
/// UTC day — counted exactly like the gateway's select_quota (requests, or
/// input+output tokens; cache reads excluded). No/invalid config → under.
fn quota_over_threshold(store: &Store, aux: &Aux, config: Option<&str>, provider_id: &str) -> bool {
    #[derive(Deserialize)]
    struct QuotaCfg {
        limit: f64,
        #[serde(default = "default_quota_unit")]
        unit: String,
    }
    fn default_quota_unit() -> String {
        "requests".into()
    }
    let Some(cfg) = config.and_then(|c| serde_json::from_str::<QuotaCfg>(c).ok()) else {
        return false;
    };
    let since = local_day_start(tz_offset(aux), unix_now());
    let Ok(t) = store.usage_totals_for_provider(provider_id, Some(&since)) else {
        return false;
    };
    let consumed = if cfg.unit == "tokens" {
        (t.input_tokens + t.output_tokens) as f64
    } else {
        t.requests as f64
    };
    consumed >= cfg.limit
}

// ── VM types (serde field names mirror src/api/types.ts verbatim) ──

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

// ── Billing mapping (UI plan/payg/unl ↔ DB subscription/metered/unlimited) ──

/// Map a UI billing tag onto the store vocabulary. Unrecognized tags are an
/// error, never a silent `Metered` fallback (an unknown tag would otherwise
/// persist as a wrong billing mode and mis-shape the quota columns).
pub(crate) fn billing_to_db(ui: &str) -> Result<Billing, String> {
    match ui {
        "plan" => Ok(Billing::Subscription),
        "unl" => Ok(Billing::Unlimited),
        "payg" => Ok(Billing::Metered),
        // Known, and still refused: the catalog says this vendor charges two
        // ways, and a local row holds one. A distinct message because "unknown
        // billing" would send whoever reads it looking for a broken catalog.
        "both" => {
            Err("billing \"both\" must be resolved to plan or payg before saving".to_string())
        }
        other => Err(format!(
            "unknown billing \"{other}\" (expected plan|payg|unl)"
        )),
    }
}

pub fn billing_to_ui(db: Billing) -> &'static str {
    match db {
        Billing::Subscription => "plan",
        Billing::Unlimited => "unl",
        Billing::Metered => "payg",
    }
}

// ── Sparkline normalization: y coords in the 80×14 viewBox, 1..13 ──

fn normalize_spark(values: &[i64]) -> Option<Vec<f64>> {
    let max = values.iter().max().copied()?;
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .map(|v| {
                if max == 0 {
                    12.0
                } else {
                    (12.0 - 10.0 * (*v as f64 / max as f64)).clamp(2.0, 12.0)
                }
            })
            .collect(),
    )
}

// ── Provider view assembly ──

/// The bound agents whose config actually points at the gateway *right now*.
///
/// Turning a takeover off drops that agent's route (`set_agent_takeover`), so
/// what is left here is the odd row: an agent whose config was reverted behind
/// Kiwano's back by another tool, a store written by a build that kept dormant
/// routes, a binding an import landed on an agent that was never taken over.
/// In every one of them the agent's traffic goes to its own provider rather than
/// to this gateway, so a provider must not read as bound to it — or, worse, as
/// *in use* by it, which is what the list claimed for every binding row.
///
/// Recognition is by the live file (`kw-ag-<agent>-…` in the config the
/// takeover wrote), never by the `placeholder_keys` table: a key row can
/// outlive its rewrite. The takeover panel reads the same files and counts
/// *more* agents than this on purpose — it also accepts a restorable backup,
/// which is a claim about being able to undo a takeover, not about traffic
/// arriving here.
pub(crate) fn live_bound_agents(
    store: &Store,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<String>, String> {
    let custom: Vec<String> = store
        .list_custom_agents()
        .map_err(e2s)?
        .into_iter()
        .map(|a| a.id)
        .collect();
    let bound = store.bound_agents().map_err(e2s)?;
    Ok(bound
        .into_iter()
        // A user-defined agent routes as long as it exists: it has no config
        // file for the evidence to be missing from, and asking for one would
        // report its providers as unbound — the falsehood this whole helper was
        // added to remove, wearing a new hat.
        .filter(|agent| {
            custom.contains(agent)
                || crate::takeover::live_placeholder_key(agent, home, vars).is_some()
        })
        .collect())
}

/// Provider rows for the Apps screen. `home` roots the agent config files the
/// takeover state is read from, so a caller that knows its own tree (the CLI's
/// `--home`) does not have to settle for `$HOME`.
pub fn build_provider_vms(
    store: &Store,
    aux: &Aux,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<ProviderVm>, String> {
    let providers = store.list_providers().map_err(e2s)?;
    if providers.is_empty() {
        return Ok(Vec::new());
    }

    // What each provider bills in, resolved per row below.
    let catalog_entries = catalog_snapshot(aux).entries;

    let now = unix_now();
    let since7 = rfc3339(now - 7 * 86_400);
    // The window the prober scopes itself by: a provider with a request in it is
    // one whose latency this build can already show, so the probe skips it and
    // the row shows the traffic number instead. The two must agree, or a row
    // would show a probe while the prober considered it spoken for.
    let since_day = rfc3339(now - 86_400);

    // Only agents that route through this gateway have a say in the badges:
    // everything below — the agent column, "In use", the agent-count note —
    // is a claim about live traffic, and a dormant route carries none.
    let live: Vec<String> = live_bound_agents(store, home, vars)?;

    let primary = primary_by_agent(store, &live)?;
    let (bindings_by_agent, strategy_by_agent) = bindings_and_strategies(store, &primary);
    let maps = serving_maps(store, aux, &live, &providers)?;
    let health_by_id = load_health(store)?;
    let usage_by_id = load_usage(store, &since7)?;

    let ctx = ProviderViewCtx {
        store,
        aux,
        catalog_entries: &catalog_entries,
        since_day: &since_day,
        since7: &since7,
        health_by_id: &health_by_id,
        usage_by_id: &usage_by_id,
    };
    Ok(providers
        .into_iter()
        .map(|p| {
            let badges = badge_set(&p, &primary, &bindings_by_agent, &strategy_by_agent, &maps);
            provider_vm(&ctx, p, badges)
        })
        .collect())
}

/// agent → primary provider id, over the agents a request would actually route.
fn primary_by_agent(store: &Store, live: &[String]) -> Result<HashMap<String, String>, String> {
    let mut primary: HashMap<String, String> = HashMap::new();
    for agent in live.iter() {
        if let Some(id) = store.primary_provider_id(agent).map_err(e2s)? {
            primary.insert(agent.clone(), id);
        }
    }
    Ok(primary)
}

/// agent → bindings (to read priorities for the backup #N badges) and the active
/// strategy kind (to classify non-head candidates in `badge_set`).
///
/// A read that fails here is skipped rather than propagated: these two only
/// decorate a row. The serving pass re-reads both with `?`, because there the
/// answer decides what the gateway would serve and silence would be a lie.
fn bindings_and_strategies(
    store: &Store,
    primary: &HashMap<String, String>,
) -> (HashMap<String, Vec<Binding>>, HashMap<String, StrategyType>) {
    let mut bindings_by_agent: HashMap<String, Vec<Binding>> = HashMap::new();
    let mut strategy_by_agent: HashMap<String, StrategyType> = HashMap::new();
    for agent in primary.keys() {
        if let Ok(bs) = store.bindings_for_agent(agent) {
            bindings_by_agent.insert(agent.clone(), bs);
        }
        if let Ok(Some(st)) = store.get_strategy(agent) {
            strategy_by_agent.insert(agent.clone(), st.kind);
        }
    }
    (bindings_by_agent, strategy_by_agent)
}

/// Per agent, which providers would serve a request issued right now.
struct ServingMaps {
    /// Agents whose badge reads "In use".
    serving: HashMap<String, HashSet<String>>,
    /// The quota strategy's configured first backup, while that agent's primary
    /// is over its threshold.
    fallback: HashMap<String, HashSet<String>>,
}

/// agent → provider ids that would serve a request issued right now under the
/// active strategy (the "In use" badge). Mirrors the gateway's strategy
/// selection; its runtime state (breaker health, roundrobin sticky sessions) is
/// process-local and invisible here, so those two degrade to the deterministic
/// first choice / full rotation.
fn serving_maps(
    store: &Store,
    aux: &Aux,
    live: &[String],
    providers: &[Provider],
) -> Result<ServingMaps, String> {
    // Providers the user parked: their bindings stay (the route is their intent,
    // and re-enabling restores it), but the gateway's route table drops them
    // (`RouteTable::load` checks `p.enabled`), so nothing may read as served.
    let parked: HashSet<&str> = providers
        .iter()
        .filter(|p| !p.enabled)
        .map(|p| p.id.as_str())
        .collect();
    let mut maps = ServingMaps {
        serving: HashMap::new(),
        fallback: HashMap::new(),
    };
    for agent in live.iter() {
        let enabled: Vec<Binding> = store
            .bindings_for_agent(agent)
            .map_err(e2s)?
            .into_iter()
            .filter(|b| b.enabled && !parked.contains(b.provider_id.as_str()))
            .collect();
        let Some(head) = enabled.first().map(|b| b.provider_id.clone()) else {
            continue;
        };
        let strategy = store
            .get_strategy(agent.as_str())
            .map_err(e2s)?
            .unwrap_or(Strategy {
                agent: agent.clone(),
                kind: StrategyType::Single,
                config: None,
            });
        let ids: HashSet<String> = match strategy.kind {
            // every candidate takes rotation turns → all of them serve
            StrategyType::Roundrobin => enabled.iter().map(|b| b.provider_id.clone()).collect(),
            // the candidate whose local window matches now; none → the head
            StrategyType::Timewindow => {
                let now = local_minutes_now(tz_offset(aux));
                let hit = enabled.iter().find(|b| {
                    matches!(
                        (b.win_start.as_deref(), b.win_end.as_deref()),
                        (Some(s), Some(e)) if in_window(now, s, e)
                    )
                });
                HashSet::from([hit.map(|b| b.provider_id.clone()).unwrap_or(head)])
            }
            // under threshold → the primary serves; over → the primary does not
            // serve and the first backup is only *first in line*, so it lands in
            // `fallback` rather than `serving` — the gateway's breakers decide
            // which backup (if any) actually takes a request, and the UI must
            // not claim one it has not observed. "First in line" is not "in use",
            // and the gateway/CLI reader that says "In use" here would guess
            // wrong. The All tab badges it distinctly; agent tabs likewise.
            StrategyType::Quota => {
                let over = quota_over_threshold(store, aux, strategy.config.as_deref(), &head);
                if over {
                    if let Some(backup) = enabled.get(1) {
                        maps.fallback
                            .insert(agent.clone(), HashSet::from([backup.provider_id.clone()]));
                    }
                    HashSet::new()
                } else {
                    HashSet::from([head])
                }
            }
            // single / failover: the head (failover degradation is breaker runtime)
            _ => HashSet::from([head]),
        };
        maps.serving.insert(agent.clone(), ids);
    }
    Ok(maps)
}

/// provider → its last probe verdict, read whole: the rows below all come from
/// one pass over the table rather than a lookup each.
fn load_health(store: &Store) -> Result<HashMap<String, kiwanod::store::ProviderHealth>, String> {
    Ok(store
        .list_provider_health()
        .map_err(e2s)?
        .into_iter()
        .map(|h| (h.provider_id.clone(), h))
        .collect())
}

/// provider → 7d usage totals.
fn load_usage(store: &Store, since7: &str) -> Result<HashMap<String, UsageTotals>, String> {
    let mut usage_by_id: HashMap<String, UsageTotals> = HashMap::new();
    for pu in store
        .usage_by_provider(None, None, Some(since7))
        .map_err(e2s)?
    {
        usage_by_id.insert(pu.provider_id, pu.totals);
    }
    Ok(usage_by_id)
}

/// The badges one provider row carries, derived from the routes that bound it.
struct BadgeSet {
    agents: Vec<String>,
    serving_agents: Vec<String>,
    fallback_agents: Vec<String>,
    backup_for_any: bool,
}

fn badge_set(
    p: &Provider,
    primary: &HashMap<String, String>,
    bindings_by_agent: &HashMap<String, Vec<Binding>>,
    strategy_by_agent: &HashMap<String, StrategyType>,
    maps: &ServingMaps,
) -> BadgeSet {
    let mut agents: Vec<String> = Vec::new();
    let mut serving_agents: Vec<String> = Vec::new();
    let mut fallback_agents: Vec<String> = Vec::new();
    let mut backup_for_any = false;
    for agent in primary.keys() {
        let is_bound = bindings_by_agent
            .get(agent)
            .is_some_and(|bs| bs.iter().any(|b| b.provider_id == p.id));
        if !is_bound {
            continue;
        }
        agents.push(agent.clone());
        if maps
            .serving
            .get(agent)
            .is_some_and(|ids| ids.contains(&p.id))
        {
            serving_agents.push(agent.clone());
        }
        if maps
            .fallback
            .get(agent)
            .is_some_and(|ids| ids.contains(&p.id))
        {
            fallback_agents.push(agent.clone());
        }
        if primary.get(agent).map(String::as_str) == Some(&p.id) {
            continue;
        }
        // Non-head. Whether that marks the provider as a failover-queue
        // member (the Agent-column note) depends on the strategy: a
        // roundrobin tail takes rotation turns and a windowed
        // timewindow tail serves its own window — neither queues. A
        // windowless timewindow tail is never picked at all, and
        // single/failover/quota tails queue. No "Standby" badge here:
        // next to "In use" it read as a contradiction.
        let binding = bindings_by_agent
            .get(agent)
            .and_then(|bs| bs.iter().find(|b| b.provider_id == p.id));
        let windowed = binding.is_some_and(|b| b.win_start.is_some() && b.win_end.is_some());
        let standby = match strategy_by_agent.get(agent) {
            Some(StrategyType::Roundrobin) => false,
            Some(StrategyType::Timewindow) => !windowed,
            _ => true,
        };
        if standby {
            backup_for_any = true;
        }
    }
    // These three vectors are built by walking a `HashMap`, so the sorts are the
    // only thing making a row's output deterministic. `is_current` follows them:
    // it is a claim about the sorted list, not about insertion order.
    agents.sort();
    serving_agents.sort();
    fallback_agents.sort();
    BadgeSet {
        agents,
        serving_agents,
        fallback_agents,
        backup_for_any,
    }
}

/// What every row of `build_provider_vms` reads, resolved once for the page.
struct ProviderViewCtx<'a> {
    store: &'a Store,
    aux: &'a Aux,
    catalog_entries: &'a [CatalogEntryVm],
    since_day: &'a str,
    since7: &'a str,
    health_by_id: &'a HashMap<String, kiwanod::store::ProviderHealth>,
    usage_by_id: &'a HashMap<String, UsageTotals>,
}

/// One provider row: the route badges from `badge_set`, the two reads from the
/// context, and nothing else.
fn provider_vm(ctx: &ProviderViewCtx, p: Provider, badges: BadgeSet) -> ProviderVm {
    let is_current = !badges.serving_agents.is_empty();
    let note = if badges.backup_for_any {
        Some("Failover queue".to_string())
    } else if !badges.agents.is_empty() {
        Some(format!("{} agent(s)", badges.agents.len()))
    } else {
        None
    };

    let health = health_vm(ctx.aux, &p, ctx.since_day, ctx.health_by_id.get(&p.id));
    let usage = usage_vm(
        ctx.store,
        ctx.aux,
        &p,
        ctx.usage_by_id.get(&p.id),
        ctx.since7,
    );

    ProviderVm {
        id: p.id.clone(),
        name: p.name.clone(),
        logo_char: logo_char(&p.name),
        logo_color: palette_color(&p.name).to_string(),
        logo_border: false,
        catalog_id: p.catalog_id.clone(),
        currency: provider_currency(&p, ctx.catalog_entries),
        endpoint: display_endpoint(&p),
        protocol: p.protocol.as_str().to_string(),
        endpoint_note: endpoint_note(&p),
        endpoints: vm_endpoints(&p),
        billing: billing_to_ui(p.billing).to_string(),
        plan_price: kiwanod::plan_quota::plan_monthly_price(p.plan_query.as_deref()),
        limit_unit: p.limit_unit.clone(),
        model_default: p.model_default.clone(),
        plan_limits: p
            .plan_limits
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        prices: p
            .prices
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
        enabled: p.enabled,
        agents: badges.agents,
        serving_agents: badges.serving_agents,
        fallback_agents: badges.fallback_agents,
        is_current,
        status_badge: None,
        agents_note: note,
        health,
        usage,
        advanced: advanced_vm(&p),
        plan_query: p
            .plan_query
            .as_deref()
            .and_then(|s| serde_json::from_str(s).ok()),
    }
}

fn display_base(base_url: &str, api_path: &Option<String>) -> String {
    let stripped = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match api_path {
        Some(path) if !path.is_empty() => format!("{stripped}{path}"),
        _ => stripped.to_string(),
    }
}

pub(crate) fn display_endpoint(p: &Provider) -> String {
    display_base(&p.base_url, &p.api_path)
}

fn protocol_label(p: kiwanod::store::Protocol) -> &'static str {
    match p {
        kiwanod::store::Protocol::OpenAI => "OpenAI-compatible",
        kiwanod::store::Protocol::Anthropic => "Anthropic",
        kiwanod::store::Protocol::Gemini => "Gemini",
    }
}

pub(crate) fn endpoint_note(p: &Provider) -> String {
    let mut note = protocol_label(p.protocol).to_string();
    // Additional endpoints surface in the same subtitle: "OpenAI-compatible · +Anthropic".
    for e in &p.endpoints {
        let tag = match e.protocol {
            kiwanod::store::Protocol::OpenAI => "OpenAI",
            kiwanod::store::Protocol::Anthropic => "Anthropic",
            kiwanod::store::Protocol::Gemini => "Gemini",
        };
        note.push_str(" · +");
        note.push_str(tag);
    }
    note
}

/// What the row says about a provider nobody has just asked.
///
/// The gateway used to keep a background probe's verdict here (every 30s, one
/// HTTP request per provider, written to `provider_health`), and the row showed
/// it as a green dot and a latency. That signal never routed anything — the
/// breaker, fed by real traffic, is what decides — so it was a continuous
/// background request per provider for a badge, and it is gone. An enabled
/// provider now reads as neutral rather than as healthy, which is the honest
/// answer to "is it up?" when the way to find out is to ask it: the row's Test
/// button measures one, and the request log is where failures show up.
/// What the Status column shows, and where the number came from.
///
/// Two sources, in this order, because they answer the same question with
/// different authority:
///
/// 1. **The provider's own requests** inside `since`. A round trip through the
///    gateway, with the user's key, to the model they actually route to — the
///    number is already in the usage table, so showing it costs nothing and it
///    is the most honest of the two.
/// 2. **The prober's verdict**, for a provider with nothing of its own to
///    measure (just added, or idle since yesterday). An unsigned GET: it says
///    whether something answers at that endpoint, never whether the key works.
///    The `source` field is what keeps the two apart downstream.
///
/// A parked provider answers neither question — it is out of every route, and
/// that is the fact worth showing.
pub(crate) fn health_vm(
    aux: &Aux,
    p: &Provider,
    since: &str,
    probe: Option<&kiwanod::store::ProviderHealth>,
) -> HealthVm {
    if !p.enabled {
        return HealthVm {
            state: "off".into(),
            latency_ms: None,
            note: Some("Disabled".into()),
            source: None,
            checked_at: None,
            error: None,
        };
    }
    if let Some(ms) = aux.avg_latency(Some(&p.id), None, Some(since), None) {
        return HealthVm {
            state: "ok".into(),
            latency_ms: Some(ms),
            note: None,
            source: Some("traffic".into()),
            checked_at: None,
            error: None,
        };
    }
    match probe {
        // Answered. The number is a round trip, and `source` says whose: the
        // prober's unsigned GET, or the Apps screen's own test — which sent a
        // real prompt with the provider's key, and is therefore the stronger
        // claim of the two.
        Some(h) if h.status == "reachable" => HealthVm {
            // A refusal is reachability too: the vendor answered, and what it
            // said was no. The cell reads that as the key rather than as
            // silence, which is the difference the tooltip carries.
            state: if h.error.is_some() {
                "error".into()
            } else {
                "ok".into()
            },
            latency_ms: h.latency_ms,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        Some(h) => HealthVm {
            // No answer at all. Not a latency to print but a fact to show: the
            // endpoint did not respond when it was last asked.
            state: "error".into(),
            latency_ms: None,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        // Nothing measured it yet — a provider added a moment ago, or one whose
        // first probe has not come round. Nothing to say, which is what the
        // blank cell has always meant.
        None => HealthVm {
            state: "idle".into(),
            latency_ms: None,
            note: None,
            source: None,
            checked_at: None,
            error: None,
        },
    }
}

/// The currencies a spending limit may be denominated in.
///
/// The Hub's rate table, because that is what makes the limit comparable with the
/// costs it is measured against: `convert_cost_buckets` needs a rate for both
/// sides, and a currency without one is **added** to the others at 1:1 — the bug
/// that once let a `¥50` limit mean nothing in particular.
///
/// The currency a provider's figures are denominated in.
///
/// Declared prices win: they are what cost this provider's requests, and a
/// spending limit is measured against that cost. Then the catalog entry it was
/// added from — the authority on what a provider bills in, and the field the Hub
/// added for exactly this (its own default is USD, so a matched entry carries a
/// currency either way). A provider added by hand, with no entry behind it and
/// nothing declared, falls back to USD as well: the price table's base, and the
/// only honest answer when nothing said otherwise.
pub(crate) fn provider_currency(p: &Provider, entries: &[CatalogEntryVm]) -> String {
    if let Some(c) = p
        .prices
        .as_deref()
        .and_then(|s| {
            serde_json::from_str::<kiwano_adapters::model_pricing::DeclaredPrices>(s).ok()
        })
        .map(|d| d.currency)
        .filter(|c| !c.is_empty())
    {
        return c;
    }
    p.catalog_id
        .as_deref()
        .and_then(|id| entries.iter().find(|e| e.id == id))
        .map(|e| e.currency.clone())
        .unwrap_or_else(|| "USD".to_string())
}

/// Cost of one provider, in the currency its usage was priced in.
///
/// No conversion: a provider bills in one currency and this number is read
/// beside that provider's own limits. Should usage ever be priced in more than
/// one currency (a price-table currency change mid-period), the currency
/// carrying the most money names the total — the alternatives are folding
/// other currencies in at a rate nobody asked for, or inventing a second line
/// for a case that does not occur in practice. Unpriced rows (`None`)
/// contribute nothing, exactly as they did when the sum was converted.
fn provider_cost(buckets: &[(Option<String>, f64)]) -> (Option<f64>, Option<String>) {
    let mut per_currency: HashMap<&str, f64> = HashMap::new();
    for (currency, cost) in buckets {
        let Some(currency) = currency.as_deref() else {
            continue;
        };
        *per_currency.entry(currency).or_default() += cost;
    }
    match per_currency.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((currency, total)) => (Some(total), Some(currency.to_string())),
        None => (None, None),
    }
}

/// Usage cell for one provider.
fn usage_vm(
    store: &Store,
    aux: &Aux,
    p: &Provider,
    totals: Option<&UsageTotals>,
    since7: &str,
) -> Option<UsageVm> {
    let t = totals?;
    // The cost stays in the currency this provider's usage was priced in: it
    // is read next to that provider's own limits, and converting it into the
    // user's display currency made the two disagree. Rolling several
    // providers into one number is the Dashboard's job, and converting there
    // is what the display currency is for.
    let cost_buckets = store
        .usage_cost_by_currency(None, Some(&p.id), Some(since7))
        .unwrap_or_default();
    let (cost, cost_currency) = provider_cost(&cost_buckets);
    let quota = match (p.billing, p.limit_unit.as_deref()) {
        // A subscription's period limit and a metered provider's spending cap
        // are the same arithmetic: this period's usage against a number in the
        // provider's own unit. Building it only for Subscription left the
        // pay-as-you-go branches above unreachable — the Apps list drew no ring
        // and its tooltip said "no limit set" while a limit sat in the row.
        // Unlimited has nothing to measure, so it stays None.
        (Billing::Subscription | Billing::Metered, unit) => p.period_limit.map(|limit| {
            let (used, unit) = match unit {
                Some("wan_tokens") => (
                    (t.input_tokens
                        + t.output_tokens
                        + t.cache_read_tokens
                        + t.cache_creation_tokens) as f64
                        / 10_000.0,
                    "wan_tokens",
                ),
                // A currency limit rings against the period's cost as recorded:
                // the limit is denominated in the provider's own currency (the
                // price table's), so no rate is involved. Converting would make
                // the threshold move with the exchange rate.
                Some(u) if u.len() == 3 => (cost.unwrap_or(0.0), u),
                _ => (t.requests as f64, "requests"),
            };
            QuotaVm {
                used: (used * 100.0).round() / 100.0,
                limit,
                unit: unit.to_string(),
                resets_at: None, // reset-cycle tracking lands with the quota strategy (P2)
            }
        }),
        _ => None,
    };
    let spark = match quota {
        None => normalize_spark(
            &aux.provider_daily(&p.id, since7)
                .into_iter()
                .map(|(_, v)| v)
                .collect::<Vec<_>>(),
        ),
        Some(_) => None,
    };
    Some(UsageVm {
        requests: t.requests,
        input_tokens: t.input_tokens,
        cache_read_tokens: t.cache_read_tokens,
        cache_creation_tokens: t.cache_creation_tokens,
        output_tokens: t.output_tokens,
        cost: cost.map(|c| (c * 1e6).round() / 1e6),
        cost_currency,
        latency_ms: aux.avg_latency(Some(&p.id), None, Some(since7), None),
        quota,
        spark,
    })
}

/// Parse a provider row's advanced columns back into the VM (for edit prefill).
pub(crate) fn advanced_vm(p: &Provider) -> Option<ProviderAdvancedVm> {
    if p.timeout_secs.is_none() && p.retries.is_none() && p.headers.is_none() {
        return None;
    }
    let headers = p
        .headers
        .as_deref()
        .and_then(|raw| {
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(raw).ok()
        })
        .unwrap_or_default();
    Some(ProviderAdvancedVm {
        timeout_secs: p.timeout_secs,
        retries: p.retries,
        headers,
    })
}

pub(crate) fn vm_endpoints(p: &Provider) -> Vec<ProviderEndpointVm> {
    p.endpoints
        .iter()
        .map(|e| ProviderEndpointVm {
            protocol: e.protocol.as_str().to_string(),
            endpoint: display_base(&e.base_url, &e.api_path),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::catalog::catalog_snapshot;
    use crate::vm::provider_edit::{
        add_provider, update_provider, BillingConfigInput, NewEndpointInput, NewProviderInput,
    };
    use crate::vm::routes::set_agent_strategy;
    use crate::vm::settings::tz_offset;
    use crate::vm::test_support::{
        catalog_input, linkless_aux, live_home, no_vars, provider, store, usage_row,
    };
    use crate::vm::time::{local_day_start, local_minutes_now, rfc3339, unix_now};
    use crate::vm::Aux;
    use kiwanod::store::{Billing, Binding, Store, StrategyType};

    /// Seed the plan-quota cache (Aux KV, same shape plan_quota.rs writes) so

    #[test]
    fn billing_mapping_roundtrip() {
        assert_eq!(billing_to_ui(billing_to_db("plan").unwrap()), "plan");
        assert_eq!(billing_to_ui(billing_to_db("payg").unwrap()), "payg");
        assert_eq!(billing_to_ui(billing_to_db("unl").unwrap()), "unl");
    }

    #[test]
    fn billing_to_db_rejects_unknown_tag() {
        // An unknown tag must never silently become payg/metered.
        let err = billing_to_db("per-token").unwrap_err();
        assert!(err.contains("per-token"), "{err}");
        assert!(err.contains("plan|payg|unl"), "{err}");
        assert!(billing_to_db("").is_err());
        assert!(billing_to_db("PAYG").is_err());

        // `both` is a tag we *know*, and still refuse: the catalog is telling us
        // the vendor charges two ways, and the local row holds one. The message
        // has to point at the choice rather than at the catalog — "unknown
        // billing" would send the reader looking for a data defect.
        let err = billing_to_db("both").unwrap_err();
        assert!(err.contains("resolved"), "{err}");
        assert!(err.contains("plan or payg"), "{err}");
        assert!(!err.contains("unknown"), "{err}");
    }

    #[test]
    fn provider_vm_maps_catalog_shape() {
        let s = store();
        s.insert_provider(&provider("deepseek-1", "DeepSeek", Billing::Metered))
            .unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "deepseek-1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(vms.len(), 1);
        let vm = &vms[0];
        let json = serde_json::to_value(vm).unwrap();
        assert_eq!(json["billing"], "payg");
        assert_eq!(json["logo_char"], "D");
        assert_eq!(json["is_current"], true);
        assert_eq!(json["agents"][0], "claude");
        assert_eq!(json["agents_note"], "1 agent(s)");
        assert_eq!(json["endpoint"], "deepseek-1.example.com");
        assert_eq!(json["endpoint_note"], "OpenAI-compatible");
    }

    /// A binding whose agent never took the gateway over is not a route: the
    /// provider list must leave that agent out of the agent column, out of
    /// "In use", and out of the count — the route stays in the store for the
    /// day the takeover is re-enabled, but no traffic reaches us until then.
    #[test]
    fn dormant_agent_bindings_are_not_counted() {
        let s = store();
        s.insert_provider(&provider("p1", "P One", Billing::Metered))
            .unwrap();
        for agent in ["claude", "codex"] {
            s.upsert_binding(&Binding {
                agent: agent.into(),
                provider_id: "p1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let aux = Aux::open_in_memory().unwrap();

        // claude is routed through the gateway, codex is not.
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert_eq!(vm.agents, ["claude"]);
        assert_eq!(vm.serving_agents, ["claude"]);
        assert!(vm.is_current);
        assert_eq!(vm.agents_note.as_deref(), Some("1 agent(s)"));

        // Nothing taken over at all: the provider reads as unbound rather than
        // as serving an agent that has its own config back.
        let none = live_home(&[]);
        let vms = build_provider_vms(&s, &aux, none.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert!(vm.agents.is_empty());
        assert!(vm.serving_agents.is_empty());
        assert!(!vm.is_current);
        assert_eq!(vm.agents_note, None);
    }

    #[test]
    fn backup_binding_gets_badge_and_note() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let alpha = vms.iter().find(|v| v.id == "a1").unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(alpha.is_current);
        assert_eq!(alpha.serving_agents, ["claude"]);
        assert!(!beta.is_current);
        assert!(beta.serving_agents.is_empty());
        // Standby badges are gone; the failover-queue role lives in the note
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn standby_flag_follows_strategy() {
        let s = store();
        for (id, name) in [
            ("a1", "Alpha"),
            ("b1", "Beta"),
            ("c1", "Gamma"),
            ("d1", "Delta"),
        ] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        s.upsert_strategy("codex", StrategyType::Roundrobin, None)
            .unwrap();
        s.upsert_strategy("opencode", StrategyType::Timewindow, None)
            .unwrap();
        s.upsert_strategy("hermes", StrategyType::Timewindow, None)
            .unwrap();
        let bind = |agent: &str, pid: &str, priority: i64, win: Option<(&str, &str)>| Binding {
            agent: agent.into(),
            provider_id: pid.into(),
            priority,
            weight: 1,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
            enabled: true,
        };
        // roundrobin tail: takes rotation turns → not a standby
        s.upsert_binding(&bind("codex", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("codex", "b1", 1, None)).unwrap();
        // windowed timewindow tail: serves its own window → not a standby
        s.upsert_binding(&bind("opencode", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("opencode", "c1", 1, Some(("22:00", "06:00"))))
            .unwrap();
        // windowless timewindow tail: never picked → still a standby
        s.upsert_binding(&bind("hermes", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("hermes", "d1", 1, None)).unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["codex", "opencode", "hermes"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(beta.is_current); // roundrobin serves every candidate
        assert_eq!(beta.serving_agents, ["codex"]);
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("1 agent(s)"));
        let gamma = vms.iter().find(|v| v.id == "c1").unwrap();
        assert_eq!(gamma.status_badge, None);
        let delta = vms.iter().find(|v| v.id == "d1").unwrap();
        assert!(!delta.is_current);
        assert_eq!(delta.status_badge, None);
        assert_eq!(delta.agents_note.as_deref(), Some("Failover queue"));
    }

    fn stored_base(s: &Store, id: &str) -> String {
        s.get_provider(id).unwrap().expect("provider row").base_url
    }

    /// The currency a provider's figures are denominated in — the field the
    /// agent's spending-limit picker offers as a unit, so it decides which money
    /// a ceiling can be written in.
    ///
    /// Declared prices win over the entry: a user who typed their own rates also
    /// typed what they are in, and those are the figures costing the requests the
    /// limit is measured against. Neither source means USD — the price table's
    /// base, and the only honest answer when nothing named another.
    #[test]
    fn a_provider_carries_the_currency_its_figures_are_in() {
        const CUR: &str = r#"{"total":2,"entries":[
            {"id":"cn-1","name":"CN One","tag":"third","rating":3,"billing":"payg",
             "currency":"CNY",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.cn1.example"}]},
            {"id":"us-1","name":"US One","tag":"third","rating":3,"billing":"payg",
             "currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.us1.example"}]}
        ]}"#;
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_cache(CUR, "2026-09-07T00:00:00Z").unwrap();
        let entries = catalog_snapshot(&aux).entries;

        // Added from an entry: what that entry bills in.
        let mut cn = provider("cn", "CN One", Billing::Metered);
        cn.catalog_id = Some("cn-1".into());
        assert_eq!(provider_currency(&cn, &entries), "CNY");

        // Declared prices, over an entry that says USD.
        let mut declared = provider("decl", "Declared", Billing::Metered);
        declared.catalog_id = Some("us-1".into());
        declared.prices = Some(r#"{"currency":"CNY","models":[]}"#.into());
        assert_eq!(provider_currency(&declared, &entries), "CNY");

        // Hand-added, and an id the catalog no longer carries: the same answer,
        // because neither names a currency at all.
        assert_eq!(
            provider_currency(&provider("bare", "Bare", Billing::Metered), &entries),
            "USD"
        );
        let mut stale = provider("stale", "Stale", Billing::Metered);
        stale.catalog_id = Some("gone".into());
        assert_eq!(provider_currency(&stale, &entries), "USD");
    }

    /// A stored endpoint is an absolute URL, whatever the form it was typed in.
    ///
    /// The dialog is handed `display_endpoint` — scheme stripped, `api_path`
    /// folded in — and hands it back on save, and nothing downstream adds a
    /// scheme: the gateway concatenates and `reqwest` refuses a relative URL. So
    /// opening a provider and saving it again used to leave a row that reads like
    /// a working provider and fails every request.
    #[test]
    fn a_saved_endpoint_keeps_its_scheme() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        // Typed as the dialog shows it. `absolute_endpoint` is the only thing
        // between that and the row, and this is the case that was broken.
        let mut bare = catalog_input("DeepSeek", "api.deepseek.com");
        bare.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "api.deepseek.com/anthropic".into(),
        }];
        let vm = add_provider(&s, &aux, &bare).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");
        let extra = s.get_provider(&vm.id).unwrap().unwrap().endpoints;
        assert_eq!(extra[0].base_url, "https://api.deepseek.com/anthropic");

        // …and the edit the dialog sends back is the same stripped form, which
        // is what used to strip the scheme off an already-correct row.
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &bare,
            &no_vars(),
        )
        .unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");

        // A local server is plain HTTP: guessing https there fails at the
        // handshake, before anything can say why.
        let local = catalog_input("Ollama", "localhost:11434");
        let vm = add_provider(&s, &aux, &local).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://localhost:11434");
        let loopback = catalog_input("Local", "127.0.0.1:1234");
        let vm = add_provider(&s, &aux, &loopback).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://127.0.0.1:1234");

        // An absolute URL is left exactly as it is, http and https alike.
        for (typed, stored) in [
            ("https://api.moonshot.cn", "https://api.moonshot.cn"),
            ("http://relay.internal", "http://relay.internal"),
        ] {
            let vm = add_provider(&s, &aux, &catalog_input("Typed", typed)).unwrap();
            assert_eq!(stored_base(&s, &vm.id), stored, "{typed}");
        }
    }

    #[test]
    fn provider_vm_carries_endpoints_and_note_suffix() {
        let s = store();
        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: "Qianfan".into(),
            api_key: "sk-x".into(),
            endpoint: "https://qianfan.baidubce.com/v2/tokenplan/personal".into(),
            protocol: "openai".into(),
            model_default: "qianfan-code-latest".into(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: Some(vec![]),
            endpoints: vec![NewEndpointInput {
                protocol: "anthropic".into(),
                endpoint: "https://qianfan.baidubce.com/anthropic/coding".into(),
            }],
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &linkless_aux(), &input).unwrap();
        assert_eq!(vm.endpoints.len(), 1);
        assert_eq!(vm.endpoints[0].protocol, "anthropic");
        // display_base strips the scheme (same as the primary endpoint field)
        assert_eq!(
            vm.endpoints[0].endpoint,
            "qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(vm.endpoint_note, "OpenAI-compatible · +Anthropic");
        // endpoints survive a fresh VM build from the store
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux, live_home(&[]).path(), &no_vars()).unwrap();
        let loaded = vms.iter().find(|v| v.id == vm.id).unwrap();
        assert_eq!(loaded.endpoints.len(), 1);
        assert_eq!(loaded.endpoint_note, "OpenAI-compatible · +Anthropic");
    }

    #[test]
    fn provider_cost_stays_in_its_own_currency() {
        let (cost, currency) = provider_cost(&[
            (Some("USD".into()), 1.5),
            (Some("USD".into()), 0.5),
            (None, 9.0), // unpriced row: no currency, contributes nothing
        ]);
        assert_eq!(cost, Some(2.0));
        assert_eq!(currency.as_deref(), Some("USD"));

        // Mixed currencies: the one carrying the most money names the total.
        let (cost, currency) =
            provider_cost(&[(Some("USD".into()), 1.0), (Some("CNY".into()), 40.0)]);
        assert_eq!(cost, Some(40.0));
        assert_eq!(currency.as_deref(), Some("CNY"));

        assert_eq!(provider_cost(&[]), (None, None));
        assert_eq!(provider_cost(&[(None, 3.0)]), (None, None));
    }

    #[test]
    fn a_spending_limit_shows_up_for_every_billing_that_has_one() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let since7 = local_day_start(0, now - 6 * 86_400);

        // A metered provider with a 50 CNY cap and 30 CNY of cost this period.
        let mut metered = provider("payg-1", "Payg", Billing::Metered);
        metered.period_limit = Some(50.0);
        metered.limit_unit = Some("CNY".into());
        s.insert_provider(&metered).unwrap();
        s.record_usage(&kiwanod::store::UsageRecord {
            ts: rfc3339(now - 60),
            agent: "claude".into(),
            provider_id: "payg-1".into(),
            model: Some("demo-model".into()),
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(214),
            status: "ok".into(),
            cost: Some(30.0),
            cost_currency: Some("CNY".into()),
            cost_off_peak: None,
        })
        .unwrap();
        let totals = s.usage_totals(None, Some("payg-1"), Some(&since7)).unwrap();
        let payg = usage_vm(&s, &aux, &metered, Some(&totals), &since7).unwrap();
        let q = payg.quota.expect("a payg cap is a quota too");
        assert_eq!((q.used, q.limit), (30.0, 50.0));
        assert_eq!(q.unit, "CNY", "denominated in the provider's own currency");

        // Unlimited has nothing to measure, limit or no limit.
        let mut unl = provider("unl-1", "Local", Billing::Unlimited);
        unl.period_limit = Some(50.0);
        s.insert_provider(&unl).unwrap();
        let totals = s.usage_totals(None, Some("unl-1"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &unl, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());

        // A metered provider with no cap has nothing to ring against, so the
        // card falls back to the usage trend.
        let bare = provider("payg-2", "Bare", Billing::Metered);
        s.insert_provider(&bare).unwrap();
        let totals = s.usage_totals(None, Some("payg-2"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &bare, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());
    }

    #[test]
    fn in_use_badge_follows_strategy() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Metered))
            .unwrap();
        for (pid, pr) in [("a1", 0), ("b1", 1)] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let home = live_home(&["claude"]);
        // (a1 current, b1 current, b1 fallback) — the quota-over case is where
        // the two badge claims split: the backup is first in line, not in use,
        // so it reads as fallback and neither provider reads as current.
        let badges = |s: &Store| -> (bool, bool, Vec<String>) {
            let vms = build_provider_vms(s, &aux, home.path(), &no_vars()).unwrap();
            let p = |id: &str| vms.iter().find(|x| x.id == id).unwrap();
            (
                p("a1").is_current,
                p("b1").is_current,
                p("b1").fallback_agents.clone(),
            )
        };

        // single: only the head serves
        assert_eq!(badges(&s), (true, false, vec![]));

        // roundrobin: every candidate takes rotation turns
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        assert_eq!(badges(&s), (true, true, vec![]));

        // timewindow: a window containing now moves the badge off the head.
        // Built from the same clock the view model reads — `Aux` here defaults
        // to UTC — so the case is stated without depending on the host's zone.
        let now = local_minutes_now(tz_offset(&aux));
        let hhmm = |min: u32| format!("{:02}:{:02}", min / 60 % 24, min % 60);
        // [now-30, now+30] — wraps midnight safely near the day edges
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: Some(hhmm(now + 1440 - 30)),
            win_end: Some(hhmm(now + 30)),
            enabled: true,
        })
        .unwrap();
        set_agent_strategy(&s, "claude", "timewindow", None).unwrap();
        assert_eq!(badges(&s), (false, true, vec![]));

        // timewindow: no window matching now → the fallback head serves
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            // one-minute window later today — can never contain now
            win_start: Some(hhmm(now + 60)),
            win_end: Some(hhmm(now + 60)),
            enabled: true,
        })
        .unwrap();
        assert_eq!(badges(&s), (true, false, vec![]));

        // quota: under the threshold the head serves; over it the head stops
        // serving and the first backup is badged fallback, not in use (windows
        // ignored).
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":5,"unit":"requests"}"#),
        )
        .unwrap();
        assert_eq!(badges(&s), (true, false, vec![]));
        for _ in 0..5 {
            s.record_usage(&kiwanod::store::UsageRecord {
                ts: rfc3339(unix_now()),
                agent: "claude".into(),
                provider_id: "a1".into(),
                model: None,
                input_tokens: 10,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();
        }
        // Over: neither reads as current (the gateway may still serve the
        // primary when every backup is down, which is its runtime call), and
        // b1 — the configured first backup — reads as fallback.
        assert_eq!(badges(&s), (false, false, vec!["claude".to_string()]));
    }

    /// The Status column reads two sources and must not confuse them: a
    /// provider's own round trips when it has any, the prober's unsigned GET
    /// when it does not, and neither for a parked one.
    #[test]
    fn the_status_column_shows_a_providers_own_latency_before_a_probe() {
        // The store and the aux have to share one file here: the traffic average
        // comes off the aux's connection, which in production is the same
        // database the gateway writes usage rows into.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let s = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();

        let mut parked = provider("parked", "Parked", Billing::Metered);
        parked.enabled = false;
        for p in [
            provider("busy", "Busy", Billing::Metered),
            provider("idle", "Idle", Billing::Metered),
            provider("dead", "Dead", Billing::Metered),
            provider("fresh", "Fresh", Billing::Metered),
            provider("tested", "Tested", Billing::Metered),
            provider("refused", "Refused", Billing::Metered),
            parked,
        ] {
            s.insert_provider(&p).unwrap();
        }

        // The prober has an opinion about all of them — including `busy`, whose
        // verdict is stale (it had no traffic when the probe ran). That stale row
        // is exactly what the order has to get right.
        s.upsert_provider_health("busy", "reachable", 500, "probe", None)
            .unwrap();
        s.upsert_provider_health("idle", "reachable", 12, "probe", None)
            .unwrap();
        s.upsert_provider_health("dead", "down", 0, "probe", None)
            .unwrap();
        s.upsert_provider_health("parked", "reachable", 3, "probe", None)
            .unwrap();
        // The Apps screen's own test: a real prompt with the key. One answered,
        // one refused — and a refusal is reachability with a reason, not silence.
        s.upsert_provider_health("tested", "reachable", 218, "test", None)
            .unwrap();
        s.upsert_provider_health("refused", "reachable", 60, "test", Some("invalid API key"))
            .unwrap();

        // Two requests of its own inside the window: 200ms on average.
        for ms in [180, 220] {
            let mut row = usage_row("busy");
            row.latency_ms = Some(ms);
            s.record_usage(&row).unwrap();
        }

        let vms = build_provider_vms(&s, &aux, std::path::Path::new("/tmp"), &no_vars()).unwrap();
        let health = |id: &str| &vms.iter().find(|v| v.id == id).unwrap().health;

        let busy = health("busy");
        assert_eq!(
            busy.source.as_deref(),
            Some("traffic"),
            "its own round trips outrank a verdict that predates them"
        );
        assert_eq!(
            busy.latency_ms,
            Some(200),
            "…and it is their average, not the probe's 500"
        );

        let idle = health("idle");
        assert_eq!(idle.source.as_deref(), Some("probe"));
        assert_eq!(idle.latency_ms, Some(12));
        assert!(
            idle.checked_at.is_some(),
            "a probe is dated; a traffic average is a window"
        );

        let dead = health("dead");
        assert_eq!(dead.state, "error", "an endpoint that did not answer");
        assert_eq!(dead.latency_ms, None, "no answer is not a latency");
        assert_eq!(dead.source.as_deref(), Some("probe"));

        let parked_health = health("parked");
        assert_eq!(parked_health.note.as_deref(), Some("Disabled"));
        assert_eq!(
            parked_health.latency_ms, None,
            "out of every route outranks both"
        );

        // A verdict the app took itself. The number and its provenance both
        // travel: the cell says "you tested it", not "the gateway asked".
        let tested = health("tested");
        assert_eq!(tested.source.as_deref(), Some("test"));
        assert_eq!(tested.latency_ms, Some(218));
        assert_eq!(tested.state, "ok");
        assert_eq!(tested.error, None);

        // Answered and refused: reachable, with the vendor's reason. Read as an
        // error state so the cell cannot pass it off as a working provider.
        let refused = health("refused");
        assert_eq!(refused.source.as_deref(), Some("test"));
        assert_eq!(refused.state, "error");
        assert_eq!(refused.error.as_deref(), Some("invalid API key"));
        assert_eq!(refused.latency_ms, Some(60), "it did answer, in 60ms");

        let fresh = health("fresh");
        assert_eq!(
            fresh.source, None,
            "never probed and never used: nothing to say"
        );
        assert_eq!(fresh.latency_ms, None);
        assert_eq!(fresh.state, "idle");
    }
}
