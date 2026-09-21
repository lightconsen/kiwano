//! The route state `build_provider_vms` reads and the rows it assembles: which
//! agents actually route through this gateway, which providers would serve each
//! of them right now, and the badges a provider row carries.

use super::billing::billing_to_ui;
use super::display::{display_endpoint, endpoint_note, vm_endpoints};
use super::health::health_vm;
use super::money::provider_currency;
use super::types::{ProviderAdvancedVm, ProviderVm};
use super::usage::usage_vm;
use crate::detect::ShellVars;
use crate::vm::catalog::{catalog_snapshot, CatalogEntryVm};
use crate::vm::fmt::{logo_char, palette_color};
use crate::vm::settings::tz_offset;
use crate::vm::time::{in_window, local_day_start, local_minutes_now, rfc3339, unix_now};
use crate::vm::{e2s, Aux};
use kiwanod::store::{Binding, Provider, Store, Strategy, StrategyType, UsageTotals};
use serde::Deserialize;
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
fn live_bound_agents(store: &Store, home: &Path, vars: &ShellVars) -> Result<Vec<String>, String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::routes::set_agent_strategy;
    use crate::vm::settings::tz_offset;
    use crate::vm::test_support::{live_home, no_vars, provider, store};
    use kiwanod::store::Billing;

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
}
