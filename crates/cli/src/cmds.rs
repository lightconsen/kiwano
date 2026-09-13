//! The commands.
//!
//! Each is thin on purpose: resolve what it needs, call into `kiwano_core`, and
//! render. The behaviour lives in the core crate, so the app and the CLI cannot
//! drift into writing different rows for the same operation.

use std::collections::BTreeMap;

use kiwano_core::detect;
use kiwano_core::sidecar;
use kiwano_core::vm;
use kiwano_gateway::store::{StrategyType, UsageTotals};

use crate::cli::{AddArgs, AgentsCmd, KeysCmd, ProvidersCmd, UsageArgs};
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
