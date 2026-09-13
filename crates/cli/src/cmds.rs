//! The commands.
//!
//! Each is thin on purpose: resolve what it needs, call into `kiwano_core`, and
//! render. The behaviour lives in the core crate, so the app and the CLI cannot
//! drift into writing different rows for the same operation.

use std::collections::BTreeMap;

use kiwano_core::detect;
use kiwano_core::sidecar;
use kiwano_core::vm;
use kiwano_gateway::store::{Provider, StrategyType, UsageTotals};
use kiwano_gateway::strategy::QuotaConfig;

use crate::cli::{
    AddArgs, AgentsCmd, BindingCmd, EditArgs, KeysCmd, ProbeCmd, ProvidersCmd, RoutesCmd, UsageArgs,
};
use crate::output::{ellipsize, render_table};
use crate::{CliError, Ctx, EXIT_NEGATIVE, EXIT_OK};

// ── status / reload ─────────────────────────────────────────────────────────

pub fn status(ctx: &mut Ctx) -> Result<i32, CliError> {
    let admin = ctx.admin.describe();
    match sidecar::status_for(&ctx.admin, ctx.token.as_deref()) {
        Some(v) => {
            let text = render_status(&v, &admin);
            ctx.out.emit(&v, || text);
            Ok(EXIT_OK)
        }
        // Gateway down is an answer, not a failure: the store is still readable
        // and on a server that is exactly what you want to see. Exit 1 is how a
        // script tells the two apart.
        None => {
            let metrics = ctx.store()?.metrics().map_err(runtime)?;
            ctx.out
                .line(format!("gateway: not running (admin {admin})"));
            ctx.out.line(format!(
                "store: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
                metrics.providers, metrics.bindings, metrics.placeholder_keys, metrics.usage_rows
            ));
            Ok(EXIT_NEGATIVE)
        }
    }
}

pub fn reload(ctx: &mut Ctx) -> Result<i32, CliError> {
    match sidecar::reload(&ctx.admin, ctx.token.as_deref()) {
        Some(v) if v["ok"].as_bool() == Some(true) => {
            ctx.out.line(format!(
                "reloaded: {} agents routed",
                v["agents_routed"].as_u64().unwrap_or(0)
            ));
            Ok(EXIT_OK)
        }
        Some(v) => Err(runtime(format!(
            "gateway refused the reload: {}",
            v["error"].as_str().unwrap_or("no reason given")
        ))),
        None => Err(runtime(format!(
            "gateway not reachable on {}",
            ctx.admin.describe()
        ))),
    }
}

// ── providers ───────────────────────────────────────────────────────────────

pub fn providers(cmd: &ProvidersCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ProvidersCmd::List { agent } => providers_list(ctx, agent.as_deref()),
        ProvidersCmd::Add(args) => providers_add(args, ctx),
        ProvidersCmd::Use { provider_id, agent } => providers_use(ctx, provider_id, agent),
        ProvidersCmd::Remove { provider_id } => providers_remove(ctx, provider_id),
        ProvidersCmd::Edit(args) => providers_edit(args, ctx),
        ProvidersCmd::Enable { provider_id } => providers_enable(ctx, provider_id),
        ProvidersCmd::Probe(cmd) => providers_probe(cmd, ctx),
    }
}

fn providers_list(ctx: &mut Ctx, agent: Option<&str>) -> Result<(), CliError> {
    let mut vms = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::build_provider_vms(store, aux)?
    };
    if let Some(agent) = agent {
        vms.retain(|p| p.agents.iter().any(|a| a == agent));
    }
    let text = render_providers(&vms);
    ctx.out.emit(&vms, || text);
    Ok(())
}

fn providers_add(args: &AddArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let input = new_provider_input(args)?;
    let created = {
        let store = ctx.store()?;
        vm::add_provider(store, &input)?
    };
    let text = format!("added {} ({})", created.id, created.name);
    ctx.out.emit(&created, || text);
    ctx.after_mutation();
    Ok(())
}

fn providers_use(ctx: &mut Ctx, provider_id: &str, agent: &str) -> Result<(), CliError> {
    {
        let store = ctx.store()?;
        if store.get_provider(provider_id).map_err(runtime)?.is_none() {
            return Err(runtime(format!("provider not found: {provider_id}")));
        }
        // `use` means "switch this agent to this provider", which is a
        // single-strategy statement — so it forces the strategy rather than
        // only reordering candidates under whatever was configured.
        store
            .upsert_strategy(agent, StrategyType::Single, None)
            .map_err(runtime)?;
        vm::bind_as_primary(store, agent, provider_id)?;
    }
    ctx.out.line(format!("{agent} -> {provider_id} (primary)"));
    ctx.after_mutation();
    Ok(())
}

fn providers_remove(ctx: &mut Ctx, provider_id: &str) -> Result<(), CliError> {
    let removed = {
        let store = ctx.store()?;
        vm::delete_provider(store, provider_id)?
    };
    if !removed {
        return Err(runtime(format!("provider not found: {provider_id}")));
    }
    ctx.out.line(format!("removed {provider_id}"));
    ctx.after_mutation();
    Ok(())
}

/// Change a provider in place.
///
/// The id is stable across the edit, which is the reason this exists at all:
/// bindings, rotating keys and usage rows all reference it, so remove-and-add
/// is not the same operation.
fn providers_edit(args: &EditArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let input = {
        let store = ctx.store()?;
        let current = store
            .get_provider(&args.provider_id)
            .map_err(runtime)?
            .ok_or_else(|| runtime(format!("provider not found: {}", args.provider_id)))?;
        edit_input(args, &current, store)?
    };
    let updated = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::update_provider(store, aux, &args.provider_id, &input)?
    };
    let text = format!("updated {} ({})", updated.id, updated.name);
    ctx.out.emit(&updated, || text);
    ctx.after_mutation();
    Ok(())
}

/// Rebuild a full `NewProviderInput` from the stored row plus whatever flags
/// were given.
///
/// `update_provider` treats name, endpoint, protocol, billing and the endpoint
/// list as authoritative rather than patch-shaped, so an edit that only sent the
/// changed fields would blank the rest — including a plan provider's percent
/// limits. Everything is therefore carried over explicitly, and only the flags
/// present in `args` override.
fn edit_input(
    args: &EditArgs,
    current: &Provider,
    store: &kiwano_gateway::store::Store,
) -> Result<vm::NewProviderInput, CliError> {
    let billing = match &args.billing {
        Some(raw) => parse_billing(raw)?.to_string(),
        None => vm::billing_to_ui(current.billing).to_string(),
    };
    let protocol = match &args.protocol {
        Some(raw) => parse_protocol(raw)?.to_string(),
        None => current.protocol.as_str().to_string(),
    };
    let agents = if args.no_bind {
        Vec::new()
    } else if args.bind.is_empty() {
        agents_bound_to(store, &args.provider_id)?
    } else {
        args.bind.clone()
    };

    let limit_value = args.limit.or(current.period_limit);
    Ok(vm::NewProviderInput {
        name: args.name.clone().unwrap_or_else(|| current.name.clone()),
        // An empty key means "keep the stored one" in update_provider, so this
        // is safe to leave blank when the flag is absent.
        api_key: args.key.clone().unwrap_or_default(),
        endpoint: args
            .endpoint
            .clone()
            .unwrap_or_else(|| current.base_url.clone()),
        protocol,
        model_default: String::new(),
        billing,
        billing_config: vm::BillingConfigInput {
            limit_value,
            limit_unit: args.unit.clone().or_else(|| current.limit_unit.clone()),
            reset_period: args.reset.clone().or_else(|| current.reset_period.clone()),
            plan_limits: plan_limits_input(current.plan_limits.as_deref()),
        },
        agents,
        endpoints: current
            .endpoints
            .iter()
            .map(|e| vm::NewEndpointInput {
                protocol: e.protocol.as_str().to_string(),
                endpoint: e.base_url.clone(),
            })
            .collect(),
        // Absent = keep, which is exactly what an edit that says nothing about
        // advanced settings wants.
        advanced: None,
        plan_query: None,
    })
}

/// `{"five_hour":20,"weekly":60}` (the stored shape) back into the input type.
fn plan_limits_input(raw: Option<&str>) -> Option<vm::PlanLimitsInput> {
    let v: serde_json::Value = serde_json::from_str(raw?).ok()?;
    let number = |key: &str| v.get(key).and_then(|x| x.as_f64());
    Some(vm::PlanLimitsInput {
        five_hour: number("five_hour"),
        weekly: number("weekly"),
    })
}

fn providers_enable(ctx: &mut Ctx, provider_id: &str) -> Result<(), CliError> {
    let agents = {
        let store = ctx.store()?;
        let bound = agents_bound_to(store, provider_id)?;
        if bound.is_empty() {
            return Err(runtime(format!(
                "provider {provider_id} is not bound to any agent; use `routes binding add` first"
            )));
        }
        vm::enable_provider(store, provider_id)?;
        bound
    };
    ctx.out.line(format!(
        "{provider_id}: now the primary for {}",
        agents.join(", ")
    ));
    ctx.after_mutation();
    Ok(())
}

fn providers_probe(cmd: &ProbeCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ProbeCmd::Latency { endpoint } => {
            let latency_ms = sidecar::measure_latency(endpoint)?;
            let payload = serde_json::json!({ "endpoint": endpoint, "latency_ms": latency_ms });
            let text = format!("{latency_ms} ms");
            ctx.out.emit(&payload, || text);
            Ok(())
        }
        ProbeCmd::Endpoint {
            protocol,
            endpoint,
            key,
        } => {
            // The probe speaks HTTP; the CLI does not, so drive the future on
            // the shared runtime rather than making every command async.
            let report =
                kiwano_core::block_on(sidecar::probe_endpoint(protocol, endpoint, key.as_deref()))?;
            let text = render_probe(&report);
            ctx.out.emit(&report, || text);
            Ok(())
        }
        ProbeCmd::Models {
            protocol,
            endpoint,
            key,
        } => {
            let names = kiwano_core::block_on(sidecar::fetch_model_names(protocol, endpoint, key))?;
            let text = names.join("\n");
            ctx.out.emit(&names, || text);
            Ok(())
        }
    }
}

// ── routes ──────────────────────────────────────────────────────────────────

pub fn routes(cmd: &RoutesCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        RoutesCmd::List => {
            let routes = {
                let store = ctx.store()?;
                vm::build_agent_routes(store)?
            };
            let text = render_routes(&routes);
            ctx.out.emit(&routes, || text);
            Ok(())
        }
        RoutesCmd::Strategy {
            agent,
            kind,
            limit,
            unit,
        } => {
            let config = strategy_config(kind, *limit, unit)?;
            {
                let store = ctx.store()?;
                vm::set_agent_strategy(store, agent, kind, config.as_deref())?;
            }
            let detail = config.map(|c| format!(" ({c})")).unwrap_or_default();
            ctx.out.line(format!("{agent}: strategy {kind}{detail}"));
            ctx.after_mutation();
            Ok(())
        }
        RoutesCmd::Reorder {
            agent,
            provider_ids,
        } => {
            {
                let store = ctx.store()?;
                vm::reorder_agent_bindings(store, agent, provider_ids)?;
            }
            ctx.out.line(format!(
                "{agent}: {} candidates in the given order",
                provider_ids.len()
            ));
            ctx.after_mutation();
            Ok(())
        }
        RoutesCmd::Apply { from, to } => {
            {
                let store = ctx.store()?;
                vm::apply_agent_route(store, to, from)?;
            }
            ctx.out.line(format!("{to}: route copied from {from}"));
            ctx.after_mutation();
            Ok(())
        }
        RoutesCmd::Binding(cmd) => binding(cmd, ctx),
    }
}

/// The strategy payload, or `None` for strategies that carry none.
///
/// Quota is validated by the engine's own parser rather than by constructing the
/// struct here, so a config this writes cannot be one the engine would silently
/// reinterpret.
fn strategy_config(kind: &str, limit: Option<f64>, unit: &str) -> Result<Option<String>, CliError> {
    if kind != "quota" {
        if limit.is_some() {
            return Err(CliError::usage(format!(
                "--limit only applies to the quota strategy, not {kind}"
            )));
        }
        return Ok(None);
    }
    let limit = limit.ok_or_else(|| CliError::usage("the quota strategy requires --limit"))?;
    let json = QuotaConfig {
        limit,
        unit: unit.to_string(),
        period: "day".to_string(),
    }
    .to_json();
    QuotaConfig::from_json(&json).map_err(CliError::usage)?;
    Ok(Some(json))
}

fn binding(cmd: &BindingCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        BindingCmd::Add { agent, provider_id } => {
            {
                let store = ctx.store()?;
                vm::add_agent_binding(store, agent, provider_id)?;
            }
            ctx.out.line(format!("{agent}: bound {provider_id}"));
            ctx.after_mutation();
        }
        BindingCmd::Remove { agent, provider_id } => {
            {
                let store = ctx.store()?;
                vm::remove_agent_binding(store, agent, provider_id)?;
            }
            ctx.out.line(format!("{agent}: unbound {provider_id}"));
            ctx.after_mutation();
        }
        BindingCmd::Set {
            agent,
            provider_id,
            weight,
            window,
            no_window,
        } => {
            let (win_start, win_end) = match (window, no_window) {
                (Some(w), _) => {
                    let (start, end) = w
                        .split_once('-')
                        .ok_or_else(|| CliError::usage("--window must look like HH:MM-HH:MM"))?;
                    check_hhmm(start)?;
                    check_hhmm(end)?;
                    (Some(start.to_string()), Some(end.to_string()))
                }
                // The two bounds clear together: a half window never matches,
                // which is also why `update_agent_binding` refuses to set one
                // without the other.
                (None, true) => (Some(String::new()), Some(String::new())),
                (None, false) => (None, None),
            };
            {
                let store = ctx.store()?;
                vm::update_agent_binding(store, agent, provider_id, *weight, win_start, win_end)?;
            }
            ctx.out.line(format!("{agent}: {provider_id} updated"));
            ctx.after_mutation();
        }
    }
    Ok(())
}

fn check_hhmm(value: &str) -> Result<(), CliError> {
    let (h, m) = value
        .split_once(':')
        .ok_or_else(|| CliError::usage(format!("invalid time {value:?} (expected HH:MM)")))?;
    let ok = h.parse::<u32>().is_ok_and(|h| h < 24) && m.parse::<u32>().is_ok_and(|m| m < 60);
    if ok {
        Ok(())
    } else {
        Err(CliError::usage(format!(
            "invalid time {value:?} (expected HH:MM)"
        )))
    }
}

// ── keys ────────────────────────────────────────────────────────────────────

pub fn keys(cmd: &KeysCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        KeysCmd::List { provider_id } => {
            let keys = {
                let store = ctx.store()?;
                vm::list_api_keys(store, provider_id)?
            };
            let text = render_keys(&keys, provider_id);
            ctx.out.emit(&keys, || text);
            Ok(())
        }
        KeysCmd::Add {
            provider_id,
            key,
            label,
        } => {
            let entry = {
                let store = ctx.store()?;
                vm::add_api_key(store, provider_id, key, label.as_deref())?
            };
            let text = format!("added key #{} ({})", entry.id, entry.masked);
            ctx.out.emit(&entry, || text);
            ctx.after_mutation();
            Ok(())
        }
        KeysCmd::Remove { key_id } => {
            let removed = {
                let store = ctx.store()?;
                vm::delete_api_key(store, *key_id)?
            };
            if !removed {
                return Err(runtime(format!("key not found: {key_id}")));
            }
            ctx.out.line(format!("removed key #{key_id}"));
            ctx.after_mutation();
            Ok(())
        }
    }
}

// ── usage ───────────────────────────────────────────────────────────────────

pub fn usage(args: &UsageArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    if args.days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let since = vm::rfc3339(vm::unix_now() - args.days * 86_400);
    let (totals, by_provider, names) = {
        let store = ctx.store()?;
        let totals = store
            .usage_totals(args.agent.as_deref(), None, Some(&since))
            .map_err(runtime)?;
        let by_provider = store
            .usage_by_provider(args.agent.as_deref(), None, Some(&since))
            .map_err(runtime)?;
        let names: BTreeMap<String, String> = store
            .list_providers()
            .map_err(runtime)?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        (totals, by_provider, names)
    };

    let report = UsageReport {
        days: args.days,
        agent: args.agent.clone(),
        totals,
        by_provider: by_provider
            .into_iter()
            .map(|u| {
                let name = names
                    .get(&u.provider_id)
                    .cloned()
                    .unwrap_or_else(|| u.provider_id.clone());
                (u.provider_id, name, u.totals)
            })
            .collect(),
    };
    let text = render_usage(&report);
    ctx.out.emit(&report, || text);
    Ok(())
}

/// The `--json` shape of `usage`. Field names are the CLI's own; the app has no
/// equivalent object because its usage surface is the dashboard.
#[derive(serde::Serialize)]
struct UsageReport {
    days: i64,
    agent: Option<String>,
    totals: UsageTotals,
    by_provider: Vec<(String, String, UsageTotals)>,
}

// ── agents ──────────────────────────────────────────────────────────────────

pub fn agents(cmd: &AgentsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        AgentsCmd::Detect => {
            let found = detect::detect_agents();
            let text = render_agents(&found);
            ctx.out.emit(&found, || text);
            Ok(())
        }
        AgentsCmd::Versions => {
            let versions = detect::probe_agent_versions();
            let text = render_versions(&versions);
            ctx.out.emit(&versions, || text);
            Ok(())
        }
        AgentsCmd::Takeover { agent } => set_takeover(ctx, agent, true),
        AgentsCmd::Restore { agent } => set_takeover(ctx, agent, false),
    }
}

/// Take over, or restore, one agent — the operation that makes a server
/// usable, and the one the CLI could not do at all.
fn set_takeover(ctx: &mut Ctx, agent: &str, enabled: bool) -> Result<(), CliError> {
    let (home, port) = (ctx.home.clone(), ctx.data_port);
    {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::set_agent_takeover(store, aux, agent, enabled, port, &home)?;
    }

    if enabled {
        // The placeholder key is the part that is invisible from outside, and
        // getting it wrong is the difference between a routed agent and a 401 —
        // the data plane only routes keys it minted itself. Printed as stored.
        let key = {
            let store = ctx.store()?;
            store
                .list_placeholder_keys()
                .map_err(runtime)?
                .into_iter()
                .find(|k| k.agent == agent)
                .map(|k| k.key)
        };
        ctx.out.line(format!(
            "{agent}: routed through the gateway on 127.0.0.1:{port}"
        ));
        match key {
            Some(key) => ctx.out.line(format!("placeholder key: {key}")),
            None => ctx.out.note(format!(
                "note: {agent} has no placeholder key; the gateway will refuse its requests"
            )),
        }
    } else {
        ctx.out.line(format!("{agent}: configuration restored"));
    }
    ctx.after_mutation();
    Ok(())
}

// ── input mapping ───────────────────────────────────────────────────────────

/// Turn the flags into the app's `NewProviderInput`.
fn new_provider_input(args: &AddArgs) -> Result<vm::NewProviderInput, CliError> {
    let billing = parse_billing(&args.billing)?;
    let protocol = parse_protocol(&args.protocol)?;

    // Plan providers track quota from the plan query, not from the legacy
    // number+unit+cycle columns — `vm::add_provider` writes those NULL for
    // them. Accepting the flags and dropping the value would be worse than
    // saying no.
    if billing == "plan" && (args.limit.is_some() || args.unit.is_some() || args.reset.is_some()) {
        return Err(CliError::usage(
            "--limit/--unit/--reset do not apply to plan providers; \
             their quota comes from the plan query instead",
        ));
    }
    let reset_period = match args.reset.as_deref() {
        None | Some("none") => None,
        Some(p @ ("monthly" | "weekly" | "yearly")) => Some(p.to_string()),
        Some(other) => {
            return Err(CliError::usage(format!(
                "invalid --reset: {other} (monthly|weekly|yearly|none)"
            )))
        }
    };
    if let Some(days) = args.limit {
        if days <= 0.0 {
            return Err(CliError::usage("--limit must be positive"));
        }
    }

    Ok(vm::NewProviderInput {
        name: args.name.trim().to_string(),
        api_key: args.key.clone().unwrap_or_default(),
        endpoint: args.endpoint.trim().to_string(),
        protocol: protocol.to_string(),
        model_default: String::new(),
        billing: billing.to_string(),
        billing_config: vm::BillingConfigInput {
            limit_value: args.limit,
            limit_unit: args.unit.clone(),
            reset_period,
            plan_limits: None,
        },
        agents: args.bind.clone(),
        endpoints: Vec::new(),
        // Absent, not empty: `advanced` is an authoritative snapshot when
        // present, so an empty object would clear settings the user never saw.
        advanced: None,
        plan_query: None,
    })
}

/// The app's billing vocabulary, with the older CLI's words kept as aliases.
fn parse_billing(raw: &str) -> Result<&'static str, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "plan" | "subscription" => Ok("plan"),
        "payg" | "metered" => Ok("payg"),
        "unl" | "unlimited" => Ok("unl"),
        other => Err(CliError::usage(format!(
            "invalid --billing: {other} (plan|payg|unl)"
        ))),
    }
}

fn parse_protocol(raw: &str) -> Result<&'static str, CliError> {
    match raw.trim().to_ascii_lowercase().as_str() {
        "anthropic" => Ok("anthropic"),
        "openai" => Ok("openai"),
        "gemini" => Ok("gemini"),
        other => Err(CliError::usage(format!(
            "invalid --protocol: {other} (anthropic|openai|gemini)"
        ))),
    }
}

fn runtime(e: impl std::fmt::Display) -> CliError {
    CliError::runtime(e.to_string())
}

/// Which agents a provider is currently bound to.
///
/// The store indexes bindings by agent, not by provider, so this walks the
/// bound agents. Cheap at this scale, and there is no reverse index to keep in
/// sync.
fn agents_bound_to(
    store: &kiwano_gateway::store::Store,
    provider_id: &str,
) -> Result<Vec<String>, CliError> {
    let mut bound = Vec::new();
    for agent in store.bound_agents().map_err(runtime)? {
        if store
            .bindings_for_agent(&agent)
            .map_err(runtime)?
            .iter()
            .any(|b| b.provider_id == provider_id)
        {
            bound.push(agent);
        }
    }
    Ok(bound)
}

// ── rendering ───────────────────────────────────────────────────────────────

fn render_status(v: &serde_json::Value, admin: &str) -> String {
    let mut out = format!(
        "gateway: running (v{}, uptime {}s, admin {admin})",
        v["version"].as_str().unwrap_or("?"),
        v["uptime_secs"].as_u64().unwrap_or(0)
    );
    // The token is what turns a liveness answer into the full report; without
    // it these fields are absent rather than zero, and printing "null" would
    // read as a store that has nothing in it.
    if v.get("providers").is_some() {
        out.push_str(&format!(
            "\nstore: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
            v["providers"], v["bindings"], v["placeholder_keys"], v["usage_rows"]
        ));
    } else {
        out.push_str(
            "\nstore: not reported (no gateway token — point --db at the shared database)",
        );
    }
    if let Some(routes) = v["routes"].as_array() {
        for r in routes {
            let candidates: Vec<&str> = r["candidates"]
                .as_array()
                .map(|cs| cs.iter().filter_map(|c| c["id"].as_str()).collect())
                .unwrap_or_default();
            out.push_str(&format!(
                "\n  {:<8} {:<10} primary={}",
                r["agent"].as_str().unwrap_or("?"),
                r["strategy"].as_str().unwrap_or("?"),
                r["primary_provider"].as_str().unwrap_or("-")
            ));
            if !candidates.is_empty() {
                out.push_str(&format!(" candidates={}", candidates.join(",")));
            }
        }
    }
    out
}

fn render_providers(vms: &[vm::ProviderVm]) -> String {
    if vms.is_empty() {
        return "(no providers)".to_string();
    }
    let head = ["ID", "NAME", "PROTO", "ENDPOINT", "BILLING", "AGENTS"];
    let rows: Vec<Vec<String>> = vms
        .iter()
        .map(|p| {
            vec![
                ellipsize(&p.id, 30),
                ellipsize(&p.name, 24),
                p.protocol.clone(),
                ellipsize(&p.endpoint, 40),
                p.billing.clone(),
                p.agents
                    .iter()
                    .map(|a| {
                        // `*` marks the agent's primary, matching the app's
                        // "In use" badge.
                        if p.serving_agents.contains(a) {
                            format!("{a}*")
                        } else {
                            a.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            ]
        })
        .collect();
    render_table(&head, &rows)
}

fn render_probe(report: &sidecar::ProbeReport) -> String {
    let mut out = format!("{} ({} ms)", report.verdict, report.latency_ms);
    if let Some(status) = report.status {
        out.push_str(&format!(" · HTTP {status}"));
    }
    if !report.detail.is_empty() {
        out.push_str(&format!("\n{}", report.detail));
    }
    out
}

fn render_routes(routes: &[vm::AgentRouteVm]) -> String {
    if routes.is_empty() {
        return "(no agent has a route; take one over first)".to_string();
    }
    let mut out = String::new();
    for (i, route) in routes.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{} · {}", route.agent, route.strategy));
        if let Some(config) = &route.config {
            out.push_str(&format!(" {config}"));
        }
        for (n, b) in route.bindings.iter().enumerate() {
            let marker = if n == 0 { "→" } else { " " };
            out.push_str(&format!(
                "\n  {marker} {:<28} {}",
                ellipsize(&b.provider_id, 28),
                b.provider_name
            ));
            // Only surfaced when they mean something: a weight on a strategy
            // that ignores it, or a window that is not set, is noise.
            if route.strategy == "roundrobin" {
                out.push_str(&format!("  weight={}", b.weight));
            }
            if let (Some(start), Some(end)) = (&b.win_start, &b.win_end) {
                out.push_str(&format!("  {start}-{end}"));
            }
        }
    }
    out
}

fn render_agents(found: &[detect::AgentDetectVm]) -> String {
    if found.is_empty() {
        return "(no agents known)".to_string();
    }
    let head = ["AGENT", "INSTALLED", "PATH"];
    let rows: Vec<Vec<String>> = found
        .iter()
        .map(|a| {
            vec![
                a.agent.clone(),
                if a.installed { "yes" } else { "no" }.to_string(),
                a.path.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&head, &rows)
}

fn render_versions(versions: &[detect::AgentVersionVm]) -> String {
    let head = ["AGENT", "VERSION"];
    let rows: Vec<Vec<String>> = versions
        .iter()
        .map(|v| {
            vec![
                v.agent.clone(),
                v.version.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&head, &rows)
}

fn render_keys(keys: &[vm::ApiKeyVm], provider_id: &str) -> String {
    if keys.is_empty() {
        return format!("(no rotation keys for {provider_id})");
    }
    let head = ["ID", "KEY", "LABEL", "CREATED"];
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| {
            vec![
                k.id.to_string(),
                k.masked.clone(),
                k.label.clone().unwrap_or_else(|| "-".to_string()),
                k.created_at.clone(),
            ]
        })
        .collect();
    render_table(&head, &rows)
}

fn render_usage(report: &UsageReport) -> String {
    let mut out = format!(
        "usage ({}d{})",
        report.days,
        report
            .agent
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default()
    );
    out.push_str(&format!(
        "\nrequests {} · input {} · output {} · cache_read {}",
        report.totals.requests,
        vm::fmt_tokens(report.totals.input_tokens),
        vm::fmt_tokens(report.totals.output_tokens),
        vm::fmt_tokens(report.totals.cache_read_tokens)
    ));
    if report.by_provider.is_empty() {
        out.push_str("\n(no usage in window)");
        return out;
    }
    out.push_str("\nby provider:");
    for (provider_id, name, totals) in &report.by_provider {
        let label = if name.is_empty() { provider_id } else { name };
        out.push_str(&format!(
            "\n  {:<24} {:>6} req  in {:>7}  out {:>7}",
            ellipsize(label, 23),
            totals.requests,
            vm::fmt_tokens(totals.input_tokens),
            vm::fmt_tokens(totals.output_tokens)
        ));
    }
    out
}
