//! The provider screen: the rows the Apps list is built from.
//!
//! Moved here from `kiwano_core::vm::providers` (`migrate.local.md` §10.21). The
//! split is by **kind**, and it is §5's two constraints deciding it:
//!
//! - the daemon assembles the view, because the client cannot open its database
//!   when the daemon is on another machine;
//! - the client computes **which agents actually route here** — that is a fact
//!   about *this* machine, read out of the agents' own config files — and sends
//!   it as `live` (§5 #2). The daemon never touches those files.
//!
//! §5 #5 said the aggregation stays on the client. That was written when the
//! aggregation lived with the store it read and the client was the only side
//! that had one; neither is true now, and its stated reason ("it already runs
//! there") is what decided it. What §5 #5 protects is §5 #2's invariant, and
//! that is kept: no machine fact is computed here.
//!
//! `kiwano-core` re-exports all of it, so its call sites are unchanged.

use crate::store::{Binding, Provider, ProviderHealth, Store, Strategy, StrategyType, UsageTotals};
use kiwano_api::error::ApiError;
use kiwano_api::logo::{logo_char, palette_color};
use kiwano_api::providers::{HealthVm, ProviderVm, QuotaVm, UsageVm};
use serde::Deserialize;
use std::collections::{HashMap, HashSet};

use super::catalog::{catalog_snapshot, CatalogEntryVm};
use super::views::{
    advanced_vm, billing_to_ui, display_endpoint, endpoint_note, provider_currency, vm_endpoints,
};
use crate::store::time::{
    in_window, local_day_start, local_minutes_now, rfc3339_from_unix, unix_now,
};
use crate::store::Billing;

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
pub fn health_vm(
    store: &Store,
    p: &Provider,
    since: &str,
    probe: Option<&crate::store::ProviderHealth>,
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
    if let Some(ms) = store.avg_latency(Some(&p.id), None, Some(since), None) {
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

/// Usage cell for one provider.
pub fn usage_vm(
    store: &Store,
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
            &store
                .provider_daily(&p.id, since7)
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
        latency_ms: store.avg_latency(Some(&p.id), None, Some(since7), None),
        quota,
        spark,
    })
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

/// The currencies a spending limit may be denominated in.
///
/// The Hub's rate table, because that is what makes the limit comparable with the
/// costs it is measured against: `convert_cost_buckets` needs a rate for both
/// sides, and a currency without one is **added** to the others at 1:1 — the bug
/// that once let a `¥50` limit mean nothing in particular.
///
/// The currency a provider's figures are denominated in.
///
/// Cost of one provider, in the currency its usage was priced in.
///
/// No conversion: a provider bills in one currency and this number is read
/// beside that provider's own limits. Should usage ever be priced in more than
/// one currency (a price-table currency change mid-period), the currency
/// carrying the most money names the total — the alternatives are folding
/// other currencies in at a rate nobody asked for, or inventing a second line
/// for a case that does not occur in practice. Unpriced rows (`None`)
/// contribute nothing, exactly as they did when the sum was converted.
pub fn provider_cost(buckets: &[(Option<String>, f64)]) -> (Option<f64>, Option<String>) {
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

/// Whether the quota config puts `provider_id` over threshold for the current
/// UTC day — counted exactly like the gateway's select_quota (requests, or
/// input+output tokens; cache reads excluded). No/invalid config → under.
fn quota_over_threshold(store: &Store, config: Option<&str>, provider_id: &str) -> bool {
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
    let since = local_day_start(store.ui_tz_offset_minutes(), unix_now());
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

// ── Provider view assembly ──

// The one machine fact travels in: which agents' own configs carry our key.
// Reading those files stays on the client (§5 #2); deciding which of them count
// needs rows, so it happens here.

/// Provider rows for the Apps screen.
pub fn build_provider_vms(store: &Store, carrying: &[String]) -> Result<Vec<ProviderVm>, ApiError> {
    let live = live_agents(store, carrying)?;
    build_rows(store, &live)
}

/// The agents that count, from the one machine fact the client supplies.
///
/// `carrying` is "these agents' own configs carry our placeholder key right
/// now" — the client's to know, because those files are this machine's and the
/// daemon never reads them (§5 #2). **Which of them count** is the daemon's,
/// because it needs rows: an agent is live when its config carries the key *and*
/// it is one this database knows about — a user-defined agent always is (it has
/// no config to lack evidence from), a built-in one when it has a route.
///
/// The intersection used to be the client's whole job (`live_bound_agents`),
/// which meant it needed the custom agents and the bindings — two shared reads —
/// to answer a question whose only *machine* half was the config files. Doing it
/// this way round leaves the client with the half it can actually see
/// (`migrate.local.md` §10.44).
fn live_agents(store: &Store, carrying: &[String]) -> Result<Vec<String>, ApiError> {
    let custom: Vec<String> = store
        .list_custom_agents()
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|a| a.id)
        .collect();
    Ok(store
        .bound_agents()
        .map_err(ApiError::failed)?
        .into_iter()
        .filter(|agent| custom.contains(agent) || carrying.contains(agent))
        .collect())
}

fn build_rows(store: &Store, live: &[String]) -> Result<Vec<ProviderVm>, ApiError> {
    let providers = store.list_providers().map_err(ApiError::failed)?;
    if providers.is_empty() {
        return Ok(Vec::new());
    }

    // What each provider bills in, resolved per row below.
    let catalog_entries = catalog_snapshot(store).entries;

    let now = unix_now();
    let since7 = rfc3339_from_unix(now - 7 * 86_400);
    // The window the prober scopes itself by: a provider with a request in it is
    // one whose latency this build can already show, so the probe skips it and
    // the row shows the traffic number instead. The two must agree, or a row
    // would show a probe while the prober considered it spoken for.
    let since_day = rfc3339_from_unix(now - 86_400);

    // Only agents that route through this gateway have a say in the badges:
    // everything below — the agent column, "In use", the agent-count note —
    // is a claim about live traffic, and a dormant route carries none.
    let primary = primary_by_agent(store, live)?;
    let (bindings_by_agent, strategy_by_agent) = bindings_and_strategies(store, &primary);
    let maps = serving_maps(store, live, &providers)?;
    let health_by_id = load_health(store)?;
    let usage_by_id = load_usage(store, &since7)?;

    let ctx = ProviderViewCtx {
        store,
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
fn primary_by_agent(store: &Store, live: &[String]) -> Result<HashMap<String, String>, ApiError> {
    let mut primary: HashMap<String, String> = HashMap::new();
    for agent in live.iter() {
        if let Some(id) = store.primary_provider_id(agent).map_err(ApiError::failed)? {
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
    live: &[String],
    providers: &[Provider],
) -> Result<ServingMaps, ApiError> {
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
            .map_err(ApiError::failed)?
            .into_iter()
            .filter(|b| b.enabled && !parked.contains(b.provider_id.as_str()))
            .collect();
        let Some(head) = enabled.first().map(|b| b.provider_id.clone()) else {
            continue;
        };
        let strategy = store
            .get_strategy(agent.as_str())
            .map_err(ApiError::failed)?
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
                let now = local_minutes_now(store.ui_tz_offset_minutes());
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
                let over = quota_over_threshold(store, strategy.config.as_deref(), &head);
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
fn load_health(store: &Store) -> Result<HashMap<String, ProviderHealth>, ApiError> {
    Ok(store
        .list_provider_health()
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|h| (h.provider_id.clone(), h))
        .collect())
}

/// provider → 7d usage totals.
fn load_usage(store: &Store, since7: &str) -> Result<HashMap<String, UsageTotals>, ApiError> {
    let mut usage_by_id: HashMap<String, UsageTotals> = HashMap::new();
    for pu in store
        .usage_by_provider(None, None, Some(since7))
        .map_err(ApiError::failed)?
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
    catalog_entries: &'a [CatalogEntryVm],
    since_day: &'a str,
    since7: &'a str,
    health_by_id: &'a HashMap<String, crate::store::ProviderHealth>,
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

    let health = health_vm(ctx.store, &p, ctx.since_day, ctx.health_by_id.get(&p.id));
    let usage = usage_vm(ctx.store, &p, ctx.usage_by_id.get(&p.id), ctx.since7);

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
        plan_price: crate::plan_quota::plan_monthly_price(p.plan_query.as_deref()),
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
