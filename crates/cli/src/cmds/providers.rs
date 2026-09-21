//! The provider commands: list, add, use, remove, edit, enable, probe.
//!
//! The `--forward` flags are not parsed here: `input` owns that, because `import`
//! builds the same `NewProviderInput` from a file.

use super::input::{
    advanced_input, check_plan_limits, new_provider_input, parse_billing, parse_extra_endpoints,
    parse_protocol, plan_limits_input, plan_query_input,
};
use super::render::{render_probe, render_providers, render_quota};
use super::runtime;
use crate::cli::{AddArgs, EditArgs, ProbeCmd, ProvidersCmd};
use crate::{CliError, Ctx};
use kiwano_core::sidecar;
use kiwano_core::vm;
use kiwanod::store::{Provider, StrategyType};

// ── providers ───────────────────────────────────────────────────────────────

pub fn providers(cmd: &ProvidersCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ProvidersCmd::List { agent } => providers_list(ctx, agent.as_deref()),
        ProvidersCmd::Add(args) => providers_add(args, ctx),
        ProvidersCmd::Use { provider_id, agent } => providers_use(ctx, provider_id, agent),
        ProvidersCmd::Remove { provider_id } => providers_remove(ctx, provider_id),
        ProvidersCmd::Edit(args) => providers_edit(args, ctx),
        ProvidersCmd::Enable { provider_id } => providers_set_enabled(ctx, provider_id, true),
        ProvidersCmd::Disable { provider_id } => providers_set_enabled(ctx, provider_id, false),
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
        kiwano_core::block_on(kiwanod::plan_quota::get_plan_quota_report(
            store,
            provider_id,
            force,
        ))?
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
        // `--home` decides which agent configs count as routed here, the same
        // way it decides it for `settings get`.
        vm::build_provider_vms(store, aux, &ctx.home, ctx.config_vars())?
    };
    if let Some(agent) = agent {
        vms.retain(|p| p.agents.iter().any(|a| a == agent));
    }
    let text = render_providers(&vms);
    ctx.out.emit(&vms, || text);
    Ok(())
}

/// A `--unit` value, refused when this machine has no rate for that currency.
///
/// `vm` enforces the same rule where every writer passes — the dialog, this CLI
/// and the config importer — and this runs it here to say *which* kind of failure
/// it is: a typo in a flag is a usage error (exit 2), not a runtime one (exit 3).
pub(crate) fn checked_unit(
    unit: Option<&str>,
    known: &[String],
) -> Result<Option<String>, CliError> {
    match unit {
        Some(u) => vm::normalize_limit_unit(Some(u), true, known).map_err(CliError::usage),
        None => Ok(None),
    }
}

fn providers_add(args: &AddArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let known = vm::known_limit_currencies(ctx.store()?);
    let input = new_provider_input(args, &known)?;
    let created = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::add_provider(store, aux, &input)?
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
        edit_input(args, &current, &vm::known_limit_currencies(store))?
    };
    let updated = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::update_provider(
            store,
            aux,
            &ctx.home,
            &args.provider_id,
            &input,
            ctx.config_vars(),
        )?
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
    known: &[String],
) -> Result<vm::NewProviderInput, CliError> {
    let billing = match &args.billing {
        Some(raw) => parse_billing(raw)?.to_string(),
        None => vm::billing_to_ui(current.billing).to_string(),
    };
    let protocol = match &args.protocol {
        Some(raw) => parse_protocol(raw)?.to_string(),
        None => current.protocol.as_str().to_string(),
    };
    // Three intents, now three values rather than three vectors: `--no-bind`
    // unbinds everything (an authoritative empty set), naming agents binds those,
    // and saying nothing leaves the bindings alone. That last case used to be
    // emulated by re-sending the set already bound, which re-promoted this
    // provider to primary for each of them and flattened their strategies — the
    // side effect `None` now avoids.
    let agents = if args.no_bind {
        Some(Vec::new())
    } else if args.bind.is_empty() {
        None
    } else {
        Some(args.bind.clone())
    };

    // Checked against the *effective* billing: an edit that names no billing
    // inherits the stored one, so `--plan-limit-5h` is legitimate on a provider
    // that is already a plan.
    check_plan_limits(&billing, &args.forward)?;

    let limit_value = args.limit.or(current.period_limit);
    Ok(vm::NewProviderInput {
        // The CLI has no shelf to add from, and this column is only ever set
        // from there — the stored link is kept when the field is absent, so an
        // edit from the shell does not unlink a provider added in the app.
        catalog_id: None,
        // No flag asks for prices yet, and absent means "keep": a `providers
        // edit` from the shell leaves the ones the app collected alone rather
        // than clearing them.
        prices: None,
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
            limit_unit: checked_unit(args.unit.as_deref(), known)?
                .or_else(|| current.limit_unit.clone()),
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

/// Park a provider, or put it back. Nothing about its route changes: the row's
/// own `enabled` decides whether it may serve, and the gateway's route table
/// reads it on every reload.
fn providers_set_enabled(ctx: &mut Ctx, provider_id: &str, enabled: bool) -> Result<(), CliError> {
    let (name, agents) = {
        let store = ctx.store()?;
        let p = store
            .get_provider(provider_id)
            .map_err(runtime)?
            .ok_or_else(|| runtime(format!("provider not found: {provider_id}")))?;
        vm::set_provider_enabled(store, provider_id, enabled)?;
        (p.name, agents_bound_to(store, provider_id)?)
    };
    // What the state *means*, not just what changed — and for a disable, who it
    // means it for: the difference between "my requests stop going there" and
    // "the row is gone" is the point of this command.
    let text = if enabled {
        format!("enabled {name} ({provider_id}) — it may serve again")
    } else if agents.is_empty() {
        format!("disabled {name} ({provider_id}) — it was in no route")
    } else {
        format!(
            "disabled {name} ({provider_id}) — {} fall through to the next candidate",
            agents.join(", ")
        )
    };
    ctx.out.line(text);
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
