//! The commands.
//!
//! Each is thin on purpose: resolve what it needs, call into `kiwano_core`, and
//! render. The behaviour lives in the core crate, so the app and the CLI cannot
//! drift into writing different rows for the same operation.

use std::collections::BTreeMap;

use kiwano_core::detect;
use kiwano_core::sidecar;
use kiwano_core::vm;
use kiwano_core::{import, pricing, share, sync};
use kiwanod::store::{Provider, RequestLogFilter, StrategyType, UsageTotals};
use kiwanod::strategy::QuotaConfig;

use crate::cli::{
    AddArgs, AgentsCmd, BindingCmd, CatalogCmd, ConfigCmd, DashboardArgs, EditArgs, ForwardArgs,
    GatewayCmd, ImportCmd, KeysCmd, LogFilterArgs, LogsCmd, ProbeCmd, ProvidersCmd, RoutesCmd,
    SettingsCmd, UsageArgs,
};
use crate::output::{ellipsize, render_table};
use crate::{CliError, Ctx, EXIT_NEGATIVE, EXIT_OK};

// ── status / reload ─────────────────────────────────────────────────────────

pub fn status(ctx: &mut Ctx) -> Result<i32, CliError> {
    let admin = ctx.admin.describe();
    let footer = footer_stats(ctx);
    match sidecar::status_for(&ctx.admin, ctx.token.as_deref()) {
        Some(mut v) => {
            // Additive rather than reshaped: the rest of this object is the
            // admin plane's report passed through, and a consumer parsing it
            // should keep working.
            if let (Some(object), Some(footer)) = (v.as_object_mut(), footer.as_ref()) {
                if let Ok(value) = serde_json::to_value(footer) {
                    object.insert("footer".to_string(), value);
                }
            }
            let text = render_status(&v, &admin, footer.as_ref());
            ctx.out.emit(&v, || text);
            Ok(EXIT_OK)
        }
        // Gateway down is an answer, not a failure: the store is still readable
        // and on a server that is exactly what you want to see. Exit 1 is how a
        // script tells the two apart.
        None => {
            ctx.out
                .line(format!("gateway: not running (admin {admin})"));
            // Only report on a store that is already there. Opening one creates
            // it as a side effect, and `status` is the first thing anyone runs
            // on a machine that has neither — it should not be the command that
            // leaves a database behind.
            if ctx.db.is_file() {
                let metrics = ctx.store()?.metrics().map_err(runtime)?;
                ctx.out.line(format!(
                    "store: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
                    metrics.providers,
                    metrics.bindings,
                    metrics.placeholder_keys,
                    metrics.usage_rows
                ));
            }
            if let Some(footer) = &footer {
                ctx.out.line(render_footer(footer));
            }
            Ok(EXIT_NEGATIVE)
        }
    }
}

/// Today's totals — what the app's status bar shows.
///
/// Returns `None` rather than forcing an answer when there is no database yet:
/// opening the store creates the file as a side effect, and `status` is the
/// first thing anyone runs on a machine that has neither, so it must stay a
/// read-only question.
fn footer_stats(ctx: &Ctx) -> Option<vm::FooterStatsVm> {
    if !ctx.db.is_file() {
        return None;
    }
    let (store, aux) = (ctx.store().ok()?, ctx.aux().ok()?);
    vm::build_footer_stats(store, aux, env!("CARGO_PKG_VERSION")).ok()
}

fn render_footer(footer: &vm::FooterStatsVm) -> String {
    format!(
        "today: {} requests · {} tokens · hub {} · v{}",
        footer.today_requests,
        vm::fmt_tokens(footer.today_tokens),
        if footer.hub_synced {
            "synced"
        } else {
            "not synced"
        },
        footer.version
    )
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
        ProvidersCmd::Quota { provider_id, force } => providers_quota(ctx, provider_id, *force),
    }
}

/// The quota rings the app draws, from the same reader — which matters because
/// the same code backs the gateway's own limit enforcement, so the display and
/// the block cannot disagree.
fn providers_quota(ctx: &mut Ctx, provider_id: &str, force: bool) -> Result<(), CliError> {
    let report = {
        let store = ctx.store()?;
        kiwanod::plan_quota::get_plan_quota_report(store, provider_id, force)?
    };
    // A deterministic failure (bad credentials, unknown template) comes back as
    // `success: false` rather than as an Err, and reporting it as success would
    // be a lie about what the endpoint said.
    if !report.success {
        return Err(runtime(format!(
            "quota query failed for {provider_id}: {}",
            report.error.as_deref().unwrap_or("no reason given")
        )));
    }
    let text = render_quota(&report);
    ctx.out.emit(&report, || text);
    Ok(())
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
    store: &kiwanod::store::Store,
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

    // Checked against the *effective* billing: an edit that names no billing
    // inherits the stored one, so `--plan-limit-5h` is legitimate on a provider
    // that is already a plan.
    check_plan_limits(&billing, &args.forward)?;

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
            plan_limits: plan_limits_input(&args.forward, Some(current)),
        },
        agents,
        // The flag list is authoritative when given; otherwise the stored
        // endpoints are carried over, because `update_provider` rewrites the
        // whole set and an empty list would drop them.
        endpoints: if args.forward.endpoint_extra.is_empty() {
            current
                .endpoints
                .iter()
                .map(|e| vm::NewEndpointInput {
                    protocol: e.protocol.as_str().to_string(),
                    endpoint: e.base_url.clone(),
                })
                .collect()
        } else {
            parse_extra_endpoints(&args.forward.endpoint_extra)?
        },
        advanced: advanced_input(&args.forward, Some(current), args.no_headers)?,
        plan_query: plan_query_input(&args.forward, args.clear_plan_query)?,
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

// ── logs ────────────────────────────────────────────────────────────────────

pub fn logs(cmd: &LogsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        LogsCmd::List(args) => {
            if args.page < 1 {
                return Err(CliError::usage("--page is 1-based"));
            }
            if args.page_size < 1 {
                return Err(CliError::usage("--page-size must be positive"));
            }
            let filter = log_filter(&args.filter)?;
            let list = {
                let store = ctx.store()?;
                vm::list_request_logs(store, args.page, args.page_size, filter)?
            };
            let text = render_logs(&list);
            ctx.out.emit(&list, || text);
            Ok(())
        }
        LogsCmd::Show { id } => {
            let detail = {
                let store = ctx.store()?;
                vm::get_request_log(store, *id)?
            };
            match detail {
                Some(detail) => {
                    let text = render_log_detail(&detail);
                    ctx.out.emit(&detail, || text);
                    Ok(())
                }
                // Pruned rather than missing: the retention window drops old
                // rows, so the id genuinely existed once.
                None => Err(runtime(format!(
                    "no request {id} (it may have been pruned by the retention window)"
                ))),
            }
        }
        LogsCmd::Export(args) => {
            let filter = log_filter(&args.filter)?;
            let path = args.out.to_string_lossy();
            let report = {
                let store = ctx.store()?;
                vm::export_request_logs_csv(store, &path, filter, args.include_bodies)?
            };
            if report.truncated {
                ctx.out.note(format!(
                    "note: the result exceeded the export cap; {} rows written and the \
                     file is short of the full set",
                    report.rows_written
                ));
            }
            let text = format!(
                "wrote {} rows to {}",
                report.rows_written,
                args.out.display()
            );
            ctx.out.emit(&report, || text);
            Ok(())
        }
        LogsCmd::Clear { yes } => {
            if !*yes {
                return Err(CliError::usage(
                    "this deletes every logged request; re-run with --yes to confirm",
                ));
            }
            {
                let store = ctx.store()?;
                vm::clear_request_logs(store)?;
            }
            ctx.out.line("cleared the request log");
            Ok(())
        }
        LogsCmd::Dir => {
            let dir = kiwanod::logging::log_dir(&ctx.db);
            ctx.out.line(dir.display().to_string());
            Ok(())
        }
    }
}

fn log_filter(args: &LogFilterArgs) -> Result<RequestLogFilter<'_>, CliError> {
    // The column carries a CHECK constraint, so an unknown value would fail
    // deep in SQLite with a message about a constraint rather than about the
    // flag the user typed.
    let status = match args.status.as_deref() {
        None => None,
        Some(s @ ("ok" | "error")) => Some(s),
        Some(other) => {
            return Err(CliError::usage(format!(
                "invalid --status: {other} (ok|error)"
            )))
        }
    };
    Ok(RequestLogFilter {
        agent: args.agent.as_deref(),
        provider_id: args.provider.as_deref(),
        status,
        from: args.from.as_deref(),
        to: args.to.as_deref(),
    })
}

// ── dashboard / alerts ──────────────────────────────────────────────────────

pub fn dashboard(args: &DashboardArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let window = match args.window.as_str() {
        w @ ("today" | "7d" | "30d") => w,
        other => {
            return Err(CliError::usage(format!(
                "invalid --window: {other} (today|7d|30d)"
            )))
        }
    };
    let data = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::build_dashboard(
            store,
            aux,
            window,
            args.provider.as_deref(),
            args.agent.as_deref(),
        )?
    };
    let text = render_dashboard(&data, window);
    ctx.out.emit(&data, || text);
    Ok(())
}

/// Alerts are read-only unless `--mark-notified`, because the dedup key they
/// consult is the same one the app uses to decide whether to raise a desktop
/// notification. A cron poll consuming it would silence the alert the user was
/// waiting for.
pub fn alerts(mark_notified: bool, ctx: &mut Ctx) -> Result<(), CliError> {
    let alerts = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::check_usage_alerts(store, aux, mark_notified)?
    };
    if alerts.is_empty() {
        // Not an error: "nothing is over budget" is the answer to the question.
        ctx.out.note("no provider is over its allowance");
    }
    let text = render_alerts(&alerts);
    ctx.out.emit(&alerts, || text);
    Ok(())
}

// ── gateway daemon ──────────────────────────────────────────────────────────

pub fn gateway(cmd: &GatewayCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        GatewayCmd::Start => {
            let status = sidecar::status_for(&ctx.admin, ctx.token.as_deref());
            match sidecar::startup_action(status.as_ref(), env!("CARGO_PKG_VERSION")) {
                sidecar::StartupAction::Adopt => {
                    ctx.out.line(format!(
                        "gateway already running (admin {})",
                        ctx.admin.describe()
                    ));
                }
                sidecar::StartupAction::Restart => {
                    let child = sidecar::restart(&ctx.admin).map_err(runtime)?;
                    ctx.out
                        .line(format!("replaced the running gateway (pid {})", child.id()));
                }
                sidecar::StartupAction::Spawn => {
                    let child = sidecar::spawn().map_err(runtime)?;
                    ctx.out
                        .line(format!("started gateway (pid {})", child.id()));
                }
            }
            Ok(())
        }
        GatewayCmd::Stop => {
            if !sidecar::request_shutdown(&ctx.admin) {
                return Err(runtime(format!(
                    "the gateway would not stop (admin {}); stop it by hand if it is still running",
                    ctx.admin.describe()
                )));
            }
            ctx.out.line("gateway stopped");
            Ok(())
        }
        GatewayCmd::Restart => {
            let child = sidecar::restart(&ctx.admin).map_err(runtime)?;
            ctx.out
                .line(format!("gateway restarted (pid {})", child.id()));
            Ok(())
        }
    }
}

// ── settings / config / catalog / import ────────────────────────────────────

pub fn settings(cmd: &SettingsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        SettingsCmd::Get => {
            let settings = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                // The home-taking form: `build_settings` would resolve $HOME
                // itself, ignoring --home and reporting on the wrong tree.
                vm::build_settings_with_home(store, aux, &ctx.home)?
            };
            let text = render_settings(&settings);
            ctx.out.emit(&settings, || text);
            Ok(())
        }
        SettingsCmd::Set { keys, patch } => {
            let patch = settings_patch(keys, patch.as_deref())?;
            let settings = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                vm::update_settings(store, aux, &patch)?
            };
            // Only some settings change routing; reloading for the rest would
            // be noise on every `settings set`.
            if touches_routing(&patch) {
                ctx.after_mutation();
            }
            let text = render_settings(&settings);
            ctx.out.emit(&settings, || text);
            Ok(())
        }
    }
}

/// `--key k=v` pairs plus an optional `--patch` object, merged.
///
/// Values are parsed as JSON when they parse, so `false` is a boolean and `30`
/// a number; anything else stays the string that was typed. That is what makes
/// `settings set --key cost_alert=false` work without a typed flag per setting.
fn settings_patch(pairs: &[String], patch: Option<&str>) -> Result<serde_json::Value, CliError> {
    let mut merged = match patch {
        Some(raw) => serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|e| CliError::usage(format!("--patch is not valid JSON: {e}")))?,
        None => serde_json::json!({}),
    };
    let object = merged
        .as_object_mut()
        .ok_or_else(|| CliError::usage("--patch must be a JSON object"))?;

    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| CliError::usage(format!("--key expects KEY=VALUE, got {pair:?}")))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(CliError::usage(format!(
                "--key has an empty name: {pair:?}"
            )));
        }
        let value = match serde_json::from_str::<serde_json::Value>(value) {
            Ok(parsed) => parsed,
            Err(_) => serde_json::Value::String(value.to_string()),
        };
        object.insert(key.to_string(), value);
    }
    Ok(merged)
}

/// Whether a settings patch can change what the gateway routes.
///
/// The GUI reloads on any settings change; for a scripted `settings set` that
/// would mean an admin round-trip per key, most of which cannot affect routing
/// at all. The set here is the conservative one: anything that could plausibly
/// reach the route table or the limits evaluation.
fn touches_routing(patch: &serde_json::Value) -> bool {
    const ROUTING_KEYS: [&str; 4] = [
        "auto_failover",
        "gateway_listen",
        "request_logs",
        "log_retention_days",
    ];
    match patch.as_object() {
        Some(map) => map.keys().any(|k| ROUTING_KEYS.contains(&k.as_str())),
        None => false,
    }
}

pub fn config(cmd: &ConfigCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Export { out, include_keys } => {
            let path = out.to_string_lossy();
            let count = {
                let store = ctx.store()?;
                share::export_config_to_file(store, &path, *include_keys)?
            };
            let text = format!("wrote {count} providers to {path}");
            ctx.out.emit(
                &serde_json::json!({ "path": path, "providers": count }),
                || text,
            );
            if *include_keys {
                ctx.out
                    .note("note: the file contains provider API keys — it is written owner-only");
            }
            Ok(())
        }
        ConfigCmd::Import { file } => {
            let path = file.to_string_lossy();
            let json = std::fs::read_to_string(file)
                .map_err(|e| runtime(format!("cannot read {path}: {e}")))?;
            let report = {
                let store = ctx.store()?;
                share::import_config(store, &json)?
            };
            let text = format!(
                "added {} providers, kept {}, applied {} routes",
                report.providers_added, report.providers_kept, report.routes_applied
            );
            ctx.out.emit(&report, || text);
            ctx.after_mutation();
            Ok(())
        }
    }
}

pub fn catalog(cmd: &CatalogCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        CatalogCmd::List { tag, search } => {
            let mut catalog = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                vm::load_catalog(store, aux)
            };
            if let Some(tag) = tag {
                catalog.entries.retain(|e| e.tag == *tag);
            }
            if let Some(search) = search {
                let needle = search.to_lowercase();
                catalog
                    .entries
                    .retain(|e| e.name.to_lowercase().contains(&needle));
            }
            let text = render_catalog(&catalog);
            ctx.out.emit(&catalog, || text);
            Ok(())
        }
        CatalogCmd::Sync => {
            let hub_url = {
                let aux = ctx.aux()?;
                vm::ui_settings(aux).hub_url
            };
            let report = {
                let aux = ctx.aux()?;
                sync::sync_from_hub(aux, &hub_url)?
            };
            let text = if report.unchanged {
                format!(
                    "already current ({} entries, synced {})",
                    report.fetched, report.synced_at
                )
            } else {
                format!("synced {} entries from {hub_url}", report.fetched)
            };
            ctx.out.emit(&report, || text);
            // A price refresh only reaches cost recording through a reload.
            ctx.after_mutation();
            Ok(())
        }
        CatalogCmd::Currency => {
            let meta = {
                let aux = ctx.aux()?;
                pricing::currency_meta(aux)?
            };
            let text = format!(
                "preferred {} · {} currencies",
                meta.preferred,
                meta.currencies.len()
            );
            ctx.out.emit(&meta, || text);
            Ok(())
        }
    }
}

pub fn import(cmd: &ImportCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ImportCmd::CcSwitch => {
            let root = ctx.home.join(".cc-switch");
            let report = {
                let store = ctx.store()?;
                import::run_import(
                    store,
                    Some(&root.join("cc-switch.db")),
                    Some(&root.join("config.json")),
                )
            };
            let text = format!(
                "imported {}, skipped {}{}",
                report.imported,
                report.skipped,
                if report.detail.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", report.detail.join("\n"))
                }
            );
            ctx.out.emit(&report, || text);
            if report.imported > 0 {
                ctx.after_mutation();
            }
            Ok(())
        }
    }
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
    check_plan_limits(billing, &args.forward)?;

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
            plan_limits: plan_limits_input(&args.forward, None),
        },
        agents: args.bind.clone(),
        endpoints: parse_extra_endpoints(&args.forward.endpoint_extra)?,
        // Absent, not empty: `advanced` is an authoritative snapshot when
        // present, so sending an empty object would clear settings the user
        // never mentioned. There is nothing to preserve on add, so it is only
        // built when a flag actually asks for it.
        advanced: advanced_input(&args.forward, None, false)?,
        plan_query: plan_query_input(&args.forward, false)?,
    })
}

/// Plan limits are only meaningful for a plan provider, and `vm` silently drops
/// them otherwise — the same shape of mistake as `--limit` on a plan. Refuse
/// rather than accept a flag that does nothing.
fn check_plan_limits(billing: &str, forward: &ForwardArgs) -> Result<(), CliError> {
    let asked = forward.plan_limit_5h.is_some() || forward.plan_limit_weekly.is_some();
    if asked && billing != "plan" {
        return Err(CliError::usage(
            "--plan-limit-5h/--plan-limit-weekly apply to plan providers only \
             (use --limit for payg)",
        ));
    }
    Ok(())
}

/// `Name: value` → a header map.
///
/// Split on the *first* colon, so a value may contain one — which it very often
/// does: `Authorization: Bearer abc:def`, or a URL.
fn parse_headers(raw: &[String]) -> Result<BTreeMap<String, String>, CliError> {
    let mut map = BTreeMap::new();
    for entry in raw {
        let (name, value) = entry.split_once(':').ok_or_else(|| {
            CliError::usage(format!("--header expects 'Name: value', got {entry:?}"))
        })?;
        let name = name.trim();
        if name.is_empty() {
            return Err(CliError::usage(format!(
                "--header has an empty name: {entry:?}"
            )));
        }
        map.insert(name.to_string(), value.trim().to_string());
    }
    Ok(map)
}

/// The stored `providers.headers` JSON column back into a map.
fn headers_from_column(raw: Option<&str>) -> Option<BTreeMap<String, String>> {
    serde_json::from_str(raw?).ok()
}

/// `PROTO=URL` → the additional-endpoint list.
fn parse_extra_endpoints(raw: &[String]) -> Result<Vec<vm::NewEndpointInput>, CliError> {
    raw.iter()
        .map(|entry| {
            let (protocol, endpoint) = entry.split_once('=').ok_or_else(|| {
                CliError::usage(format!("--endpoint-extra expects PROTO=URL, got {entry:?}"))
            })?;
            let protocol = parse_protocol(protocol)?;
            let endpoint = endpoint.trim();
            if endpoint.is_empty() {
                return Err(CliError::usage(format!(
                    "--endpoint-extra has an empty URL: {entry:?}"
                )));
            }
            Ok(vm::NewEndpointInput {
                protocol: protocol.to_string(),
                endpoint: endpoint.to_string(),
            })
        })
        .collect()
}

/// The `advanced` object, or `None` for "leave it alone".
///
/// `vm` recomputes all three columns from this object whenever it is present,
/// so on an edit the fields the flags did not mention are carried over from the
/// stored row. Sending a partial object would clear them.
fn advanced_input(
    forward: &ForwardArgs,
    current: Option<&Provider>,
    clear_headers: bool,
) -> Result<Option<vm::AdvancedInput>, CliError> {
    let touched = forward.timeout.is_some()
        || forward.retries.is_some()
        || !forward.headers.is_empty()
        || clear_headers;
    if !touched {
        return Ok(None);
    }
    let stored_headers = current.and_then(|p| headers_from_column(p.headers.as_deref()));
    let headers = if clear_headers {
        None
    } else if !forward.headers.is_empty() {
        Some(parse_headers(&forward.headers)?)
    } else {
        stored_headers
    };
    Ok(Some(vm::AdvancedInput {
        timeout_secs: forward
            .timeout
            .or_else(|| current.and_then(|p| p.timeout_secs)),
        retries: forward.retries.or_else(|| current.and_then(|p| p.retries)),
        headers,
    }))
}

/// `{"five_hour":20,"weekly":60}` (the stored shape) back into the input type.
fn parse_plan_limits(raw: &str) -> Option<vm::PlanLimitsInput> {
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    let number = |key: &str| v.get(key).and_then(|x| x.as_f64());
    Some(vm::PlanLimitsInput {
        five_hour: number("five_hour"),
        weekly: number("weekly"),
    })
}

/// The plan-mode percent limits: flags override, the stored row fills the rest.
///
/// `vm` recomputes the column from this object, so a partial view would clear
/// the window the caller did not mention.
fn plan_limits_input(
    forward: &ForwardArgs,
    current: Option<&Provider>,
) -> Option<vm::PlanLimitsInput> {
    let stored = current
        .and_then(|p| p.plan_limits.as_deref())
        .and_then(parse_plan_limits);
    if forward.plan_limit_5h.is_none() && forward.plan_limit_weekly.is_none() {
        return stored;
    }
    let stored = stored.unwrap_or(vm::PlanLimitsInput {
        five_hour: None,
        weekly: None,
    });
    Some(vm::PlanLimitsInput {
        five_hour: forward.plan_limit_5h.or(stored.five_hour),
        weekly: forward.plan_limit_weekly.or(stored.weekly),
    })
}

/// The quota-query payload, or `None` for "keep what is stored".
///
/// A JSON `null` is how the API spells "clear this", which is not the same as
/// absent — hence the separate `--clear-plan-query` flag rather than treating an
/// empty value as a removal.
fn plan_query_input(
    forward: &ForwardArgs,
    clear: bool,
) -> Result<Option<serde_json::Value>, CliError> {
    if clear {
        return Ok(Some(serde_json::Value::Null));
    }
    match &forward.plan_query {
        None => Ok(None),
        Some(raw) => {
            let parsed: serde_json::Value = serde_json::from_str(raw)
                .map_err(|e| CliError::usage(format!("--plan-query is not valid JSON: {e}")))?;
            if !parsed.is_object() {
                return Err(CliError::usage(
                    "--plan-query must be a JSON object, e.g. '{\"template\":\"kimi\",\"fields\":{}}'",
                ));
            }
            Ok(Some(parsed))
        }
    }
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
    store: &kiwanod::store::Store,
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

fn render_status(v: &serde_json::Value, admin: &str, footer: Option<&vm::FooterStatsVm>) -> String {
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
    // Providers the gateway is refusing to route to right now, with its reason.
    // This is the operational answer to "why is nothing working" — the app shows
    // it as a dimmed row — and it is derived state that exists only while the
    // gateway runs, so `status` is the only place a shell can see it.
    if let Some(blocked) = v["blocked"].as_array() {
        if !blocked.is_empty() {
            out.push_str("\nblocked:");
            for row in blocked {
                out.push_str(&format!(
                    "\n  {:<28} {}",
                    ellipsize(row["provider_id"].as_str().unwrap_or("?"), 28),
                    row["reason"].as_str().unwrap_or("no reason given")
                ));
            }
        }
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
    if let Some(footer) = footer {
        out.push('\n');
        out.push_str(&render_footer(footer));
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

fn render_quota(report: &kiwanod::plan_quota::PlanQuotaReport) -> String {
    let mut out = format!("{} · template {}", report.provider_id, report.template);
    if let Some(note) = &report.note {
        out.push_str(&format!("\n{note}"));
    }
    if report.tiers.is_empty() {
        out.push_str("\n(no quota windows reported)");
        return out;
    }
    for tier in &report.tiers {
        out.push_str(&format!(
            "\n  {:<14} {:>6.1}% used",
            tier.name, tier.utilization
        ));
        // Only some endpoints report absolute amounts; the rest are
        // percentage-only and would print a misleading 0/0.
        if let (Some(used), Some(limit)) = (tier.used, tier.limit) {
            out.push_str(&format!(
                "  ({used}/{limit}{})",
                tier.unit
                    .as_deref()
                    .map(|u| format!(" {u}"))
                    .unwrap_or_default()
            ));
        }
        if let Some(resets) = &tier.resets_at {
            out.push_str(&format!("  resets {resets}"));
        }
    }
    if report.cached {
        out.push_str("\n(from the 5-minute cache; --force asks again)");
    }
    out
}

fn render_settings(settings: &vm::SettingsVm) -> String {
    // The shape is the app's; this is the summary a shell wants. `--json`
    // carries the full object.
    let mut out = format!(
        "hub {} · preferred currency {}",
        settings.hub_url, settings.preferred_currency
    );
    out.push_str(&format!(
        "\nrequest logs {} · retention {} days · cost alerts {}",
        if settings.request_logs { "on" } else { "off" },
        settings.log_retention_days,
        if settings.cost_alert { "on" } else { "off" }
    ));
    out.push_str(&format!(
        "\nauto failover {} · auto check update {}",
        if settings.auto_failover { "on" } else { "off" },
        if settings.auto_check_update {
            "on"
        } else {
            "off"
        }
    ));
    if settings.takeovers.is_empty() {
        out.push_str("\nno agent is taken over");
    } else {
        out.push_str("\ntakeovers:");
        for t in &settings.takeovers {
            out.push_str(&format!(
                "\n  {:<16} {}{}",
                t.agent,
                if t.enabled { "routed" } else { "not routed" },
                t.placeholder_key
                    .as_deref()
                    .map(|k| format!("  {k}"))
                    .unwrap_or_default()
            ));
        }
    }
    out
}

fn render_catalog(catalog: &vm::CatalogListVm) -> String {
    if catalog.entries.is_empty() {
        return "(no matching catalog entries)".to_string();
    }
    let head = ["ID", "NAME", "TAG", "PROTOCOL", "ENDPOINT", "ADDED"];
    let rows: Vec<Vec<String>> = catalog
        .entries
        .iter()
        .map(|e| {
            vec![
                ellipsize(&e.id, 24),
                ellipsize(&e.name, 24),
                e.tag.clone(),
                e.protocol.clone(),
                ellipsize(&e.endpoint, 36),
                if e.added { "yes" } else { "no" }.to_string(),
            ]
        })
        .collect();
    let mut out = render_table(&head, &rows);
    out.push_str(&format!(
        "\n{} of {} entries",
        catalog.entries.len(),
        catalog.total
    ));
    out
}

fn render_logs(list: &vm::RequestLogListVm) -> String {
    if list.rows.is_empty() {
        return "(no matching requests)".to_string();
    }
    let head = [
        "ID", "TIME", "AGENT", "PROVIDER", "MODEL", "ST", "MS", "IN", "OUT",
    ];
    let rows: Vec<Vec<String>> = list
        .rows
        .iter()
        .map(|r| {
            vec![
                r.id.to_string(),
                r.ts.clone(),
                r.agent.clone().unwrap_or_else(|| "-".to_string()),
                r.provider_id.clone().unwrap_or_else(|| "-".to_string()),
                ellipsize(r.model.as_deref().unwrap_or("-"), 24),
                r.status_code.to_string(),
                r.latency_ms
                    .map(|ms| ms.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                r.input_tokens.to_string(),
                r.output_tokens.to_string(),
            ]
        })
        .collect();
    let mut out = render_table(&head, &rows);
    // The total, not the page length, is what tells a caller whether to keep
    // paging.
    out.push_str(&format!("\n{} of {} matching", list.rows.len(), list.total));
    out
}

fn render_log_detail(detail: &kiwanod::store::RequestLogDetail) -> String {
    let e = &detail.entry;
    let mut out = format!(
        "#{} {} {} {}{}",
        e.id,
        e.ts,
        e.method,
        e.path,
        e.query
            .as_deref()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    );
    out.push_str(&format!(
        "\nstatus {} · {} ms{}",
        e.status_code,
        e.latency_ms.unwrap_or(0),
        if e.is_streaming { " · streaming" } else { "" }
    ));
    out.push_str(&format!(
        "\nagent {} · provider {} · model {}",
        e.agent.as_deref().unwrap_or("-"),
        e.provider_id.as_deref().unwrap_or("-"),
        e.model.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "\ntokens in {} · out {} · cache_read {} · size {}→{}",
        e.input_tokens, e.output_tokens, e.cache_read_tokens, e.request_size, e.response_size
    ));
    if let Some(kind) = &e.error_kind {
        out.push_str(&format!(
            "\nerror {kind}: {}",
            e.error_message.as_deref().unwrap_or("(no message)")
        ));
    }
    // Bodies are the reason to open one of these; they are already redacted by
    // the capture layer.
    for (label, body) in [
        ("request", &detail.request_body),
        ("response", &detail.response_body),
    ] {
        if let Some(body) = body {
            out.push_str(&format!("\n{label} body:\n{body}"));
        }
    }
    if e.truncated {
        out.push_str("\n(one or more captured bodies were truncated)");
    }
    out
}

fn render_dashboard(data: &vm::DashboardVm, window: &str) -> String {
    let mut out = format!(
        "{} · {} requests ({}%) · cost {:.4}",
        window, data.requests, data.requests_delta_pct, data.cost
    );
    out.push_str(&format!(
        "\ntokens in {} · out {} · cache_read {}",
        vm::fmt_tokens(data.input_tokens),
        vm::fmt_tokens(data.output_tokens),
        vm::fmt_tokens(data.cache_read_tokens)
    ));
    out.push_str(&format!(
        "\navg latency {} ms ({}%)",
        data.latency_ms, data.latency_delta_pct
    ));
    if !data.by_provider.is_empty() {
        out.push_str("\nby provider:");
        for p in &data.by_provider {
            out.push_str(&format!(
                "\n  {:<24} {:>6} req  {:>3}%  {:.4}",
                ellipsize(&p.name, 23),
                p.requests,
                p.pct,
                p.cost
            ));
        }
    }
    if !data.by_agent.is_empty() {
        out.push_str("\nby agent:");
        for a in &data.by_agent {
            out.push_str(&format!(
                "\n  {:<24} {:>6} req  {:>8} tokens  {:.4}",
                ellipsize(&a.label, 23),
                a.requests,
                a.tokens,
                a.cost
            ));
        }
    }
    out
}

fn render_alerts(alerts: &[vm::UsageAlertVm]) -> String {
    if alerts.is_empty() {
        return "(nothing over its allowance)".to_string();
    }
    let head = ["PROVIDER", "USED", "LIMIT", "UNIT"];
    let rows: Vec<Vec<String>> = alerts
        .iter()
        .map(|a| {
            vec![
                ellipsize(&a.provider_name, 24),
                format!("{:.2}", a.used),
                format!("{:.2}", a.limit),
                a.unit.clone(),
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

#[cfg(test)]
mod tests {
    use super::render_status;

    /// `status` is the only place a shell can see the gateway's blocked list:
    /// it is derived state that exists only while the gateway is running, so
    /// there is nothing in SQLite to read it from.
    #[test]
    fn status_text_lists_blocked_providers() {
        let report = serde_json::json!({
            "version": "0.1.8",
            "uptime_secs": 12,
            "providers": 2,
            "bindings": 1,
            "placeholder_keys": 1,
            "usage_rows": 5,
            "blocked": [
                { "provider_id": "capped-1", "reason": "1.00 of 1.00 requests this period" }
            ],
            "routes": []
        });
        let text = render_status(&report, "/tmp/admin.sock", None);
        assert!(text.contains("blocked:"), "{text}");
        assert!(text.contains("capped-1"), "{text}");
        assert!(
            text.contains("1.00 of 1.00 requests this period"),
            "the gateway's reason is the whole point: {text}"
        );
    }

    /// A healthy gateway must not carry a heading with no rows under it, and a
    /// liveness-only answer (no token) has no such key at all.
    #[test]
    fn status_text_stays_quiet_when_nothing_is_blocked() {
        let empty = serde_json::json!({
            "version": "0.1.8", "uptime_secs": 1, "blocked": []
        });
        assert!(!render_status(&empty, "s", None).contains("blocked"));

        let liveness = serde_json::json!({ "version": "0.1.8", "uptime_secs": 1 });
        assert!(!render_status(&liveness, "s", None).contains("blocked"));
    }
}
