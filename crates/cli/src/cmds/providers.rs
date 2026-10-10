//! The provider commands: list, add, use, remove, edit, enable, probe.
//!
//! The `--forward` flags are not parsed here: `input` owns that, because `import`
//! builds the same `NewProviderInput` from a file.

use super::input::{
    advanced_patch, check_plan_limits, new_provider_input, parse_billing, parse_extra_endpoints,
    parse_protocol, plan_query_input,
};
use super::render::{render_probe, render_providers, render_quota};
use super::runtime;
use crate::cli::{AddArgs, EditArgs, ProbeCmd, ProvidersCmd};
use crate::{CliError, Ctx};
use kiwano_core::sidecar;
use kiwano_core::vm;

// ── providers ───────────────────────────────────────────────────────────────

pub fn providers(cmd: &ProvidersCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ProvidersCmd::List { agent } => providers_list(ctx, agent.as_deref()),
        ProvidersCmd::Add(args) => providers_add(args, ctx),
        ProvidersCmd::Use { provider_id, agent } => providers_use(ctx, provider_id, agent),
        ProvidersCmd::Remove { provider_id } => providers_remove(ctx, provider_id),
        ProvidersCmd::Edit(args) => providers_edit(args, ctx),
        ProvidersCmd::Price {
            provider_id,
            model,
            input,
            output,
            cache_read,
            cache_creation,
            currency,
            clear,
        } => providers_price(
            ctx,
            provider_id,
            PriceDeclaration {
                model,
                input: input.as_deref(),
                output: output.as_deref(),
                cache_read: cache_read.as_deref(),
                cache_creation: cache_creation.as_deref(),
                currency: currency.as_deref(),
                clear: *clear,
            },
        ),
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
    let report = ctx.api.get_plan_quota(provider_id, force)?;
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
    // `--home` decides which agent configs count as routed here, the same way it
    // decides it for `settings get` — and that evidence is all this side sends.
    let mut vms = vm::providers::provider_view(&ctx.api, &ctx.home, ctx.config_vars())?;
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
    // The currencies a limit may be written in come from the daemon, because
    // the rule is its: one this machine has no rate for is refused rather than
    // read as a request count, and only the side holding the rate table knows
    // which those are.
    //
    // Asked only when `--unit` names one: a flag that is not understood, or a
    // limit on a plan provider, is a usage error, and a usage error has to be
    // answerable without a daemon running.
    let known = match args.unit {
        Some(_) => ctx.api.limit_currencies()?,
        None => Vec::new(),
    };
    let input = new_provider_input(args, &known)?;
    // Minted here, like the app's: an id the daemon has already seen is
    // answered with the provider that exists, so a retried add writes nothing
    // (`migrate.local.md` §6.1).
    let id = vm::mint_provider_id(&input.name);
    let created = ctx.api.add_provider(&id, &input)?;
    let text = format!("added {} ({})", created.id, created.name);
    ctx.out.emit(&created, || text);
    Ok(())
}

fn providers_use(ctx: &mut Ctx, provider_id: &str, agent: &str) -> Result<(), CliError> {
    // The ref is the existence check: a provider that is not there is a 404
    // from the daemon, which is where the row lives.
    ctx.api.provider_ref(provider_id)?;
    // `use` means "switch this agent to this provider", which is a
    // single-strategy statement — so it forces the strategy rather than only
    // reordering candidates under whatever was configured.
    ctx.api.set_agent_strategy(agent, "single", None)?;
    ctx.api.bind_as_primary(agent, provider_id)?;
    ctx.out.line(format!("{agent} -> {provider_id} (primary)"));
    Ok(())
}

fn providers_remove(ctx: &mut Ctx, provider_id: &str) -> Result<(), CliError> {
    let removed = ctx.api.delete_provider(provider_id)?;
    if !removed {
        return Err(runtime(format!("provider not found: {provider_id}")));
    }
    ctx.out.line(format!("removed {provider_id}"));
    Ok(())
}

/// One `providers price` call: which model, at what rates.
struct PriceDeclaration<'a> {
    model: &'a str,
    input: Option<&'a str>,
    output: Option<&'a str>,
    cache_read: Option<&'a str>,
    cache_creation: Option<&'a str>,
    currency: Option<&'a str>,
    clear: bool,
}

/// Declare, or drop, what a provider charges for one model.
///
/// A provider's declared prices are **one currency and a whole list of models**,
/// and the write is a snapshot rather than a patch (`ProviderPatch::prices`:
/// "a present bundle is an authoritative snapshot, so an empty model list clears
/// the column"). So this reads the stored set, folds this model into it, and
/// sends the result back whole — which is what keeps "declare a price for a
/// second model" from quietly dropping the first.
fn providers_price(
    ctx: &mut Ctx,
    provider_id: &str,
    declared: PriceDeclaration<'_>,
) -> Result<(), CliError> {
    // Through the daemon, with this machine's evidence: nothing here reads a
    // database (`migrate.local.md` §10.44).
    let providers = vm::providers::provider_view(&ctx.api, &ctx.home, ctx.config_vars())?;
    let provider = providers
        .iter()
        .find(|p| p.id == provider_id)
        .ok_or_else(|| runtime(format!("no provider `{provider_id}`")))?;

    // The view carries the stored blob as JSON, so a declaration the user made in
    // the app arrives here as the same shape this command writes. An unreadable
    // one is refused rather than treated as empty: overwriting a set nobody can
    // parse would destroy whatever it says.
    let stored: Option<vm::ProviderPricesInput> = match provider.prices.as_ref() {
        Some(v) => Some(serde_json::from_value(v.clone()).map_err(|e| {
            runtime(format!(
                "this provider's stored prices could not be read ({e}); edit them in the app \
                 rather than replacing them from here"
            ))
        })?),
        None => None,
    };
    let mut models = stored
        .as_ref()
        .map(|p| p.models.clone())
        .unwrap_or_default();

    // One currency per provider, because the stored shape has one. A new currency
    // would re-denominate every other model's rates, so it is refused rather than
    // applied to a set it does not describe.
    let currency = match (
        declared.currency,
        stored.as_ref().map(|p| p.currency.as_str()),
    ) {
        (Some(wanted), Some(stored))
            if !models.is_empty() && !wanted.eq_ignore_ascii_case(stored) =>
        {
            return Err(runtime(format!(
                "this provider's declared prices are in {stored}; declaring {wanted} would \
                 re-denominate the {} other model(s) already declared. Clear them first \
                 (`--clear <model>` each), or keep the currency.",
                models.len()
            )));
        }
        (Some(wanted), _) => wanted.trim().to_ascii_uppercase(),
        (None, Some(stored)) => stored.to_string(),
        (None, None) => provider.currency.clone(),
    };

    // Replace-or-drop this model, leaving every other declaration alone.
    models.retain(|m| m.model_id != declared.model);
    if !declared.clear {
        fn blank(v: Option<&str>) -> Option<&str> {
            v.map(str::trim).filter(|s| !s.is_empty())
        }
        models.push(vm::ProviderPriceInput {
            model_id: declared.model.to_string(),
            input: blank(declared.input).unwrap_or_default().to_string(),
            output: blank(declared.output).unwrap_or_default().to_string(),
            // Blank is zero, not "charge the input rate": the field is omitted
            // rather than defaulted to something the user did not say.
            cache_read: blank(declared.cache_read).map(str::to_string),
            cache_creation: blank(declared.cache_creation).map(str::to_string),
        });
    }

    ctx.api.update_provider(
        provider_id,
        &vm::ProviderPatch {
            prices: Some(vm::ProviderPricesInput { currency, models }),
            ..Default::default()
        },
    )?;

    let verb = if declared.clear {
        "dropped"
    } else {
        "declared"
    };
    ctx.out.line(format!(
        "{verb} a price for {} on {provider_id}",
        declared.model
    ));
    Ok(())
}

/// Change a provider in place.
///
/// The id is stable across the edit, which is the reason this exists at all:
/// bindings, rotating keys and usage rows all reference it, so remove-and-add
/// is not the same operation.
fn providers_edit(args: &EditArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    // The one read a flag-driven client cannot avoid: two of its rules are
    // stated in terms of the **stored** billing mode (`migrate.local.md`
    // §10.38), and the daemon hands back just that much rather than the row.
    let current = ctx.api.provider_ref(&args.provider_id)?;
    let patch = edit_patch(args, &current, &ctx.api)?;
    ctx.api.update_provider(&args.provider_id, &patch)?;
    // Back through the daemon for the refreshed row, with this machine's
    // evidence: nothing here reads a database (`migrate.local.md` §10.44).
    let updated = vm::providers::provider_view(&ctx.api, &ctx.home, ctx.config_vars())?
        .into_iter()
        .find(|v| v.id == args.provider_id)
        .ok_or_else(|| runtime("provider vanished after update"))?;
    let text = format!("updated {} ({})", updated.id, updated.name);
    ctx.out.emit(&updated, || text);
    Ok(())
}

/// The patch an edit sends: the fields its flags name, and no others.
///
/// This is what used to be a full `NewProviderInput` rebuilt from the row. The
/// difference is not cosmetic — the old shape had to read the row to put back
/// everything the flags did not mention, and every field it "carried over" was
/// a place the two rules could disagree (`migrate.local.md` §10.38). What is
/// still needed from the row is the **billing mode**, because two of the rules
/// below are phrased in terms of it, and that is all `ProviderRefVm` carries.
fn edit_patch(
    args: &EditArgs,
    current: &vm::ProviderRefVm,
    api: &kiwano_core::daemon_api::DaemonApi,
) -> Result<vm::ProviderPatch, CliError> {
    // The billing the edit will have: the flag's, or the stored one.
    let billing = match &args.billing {
        Some(raw) => parse_billing(raw)?.to_string(),
        None => current.billing.clone(),
    };
    // Checked against the *effective* billing: an edit that names no billing
    // inherits the stored one, so `--plan-limit-5h` is legitimate on a provider
    // that is already a plan.
    check_plan_limits(&billing, &args.forward)?;

    // Three intents, three values: `--no-bind` unbinds everything (an
    // authoritative empty set), naming agents binds those, and saying nothing
    // leaves the bindings alone. The last one used to be emulated by re-sending
    // the set already bound, which re-promoted this provider to primary for each
    // of them and flattened their strategies.
    let agents = if args.no_bind {
        Some(Vec::new())
    } else if args.bind.is_empty() {
        None
    } else {
        Some(args.bind.clone())
    };

    // The limit fields are only sent when a flag names one — the daemon merges
    // what arrives against the row, so an edit that changes the name does not
    // speak about limits at all.
    let touched_limits = args.limit.is_some() || args.unit.is_some() || args.reset.is_some();
    let touched_windows =
        args.forward.plan_limit_5h.is_some() || args.forward.plan_limit_weekly.is_some();
    let billing_config = if touched_limits || touched_windows {
        Some(vm::BillingConfigPatch {
            limit_value: args.limit,
            // The currencies come from the daemon: the rule that a limit cannot
            // be written in one this machine cannot price is its.
            limit_unit: match args.unit.as_deref() {
                Some(unit) => checked_unit(Some(unit), &api.limit_currencies()?)?,
                None => None,
            },
            reset_period: args.reset.clone(),
            plan_limits: touched_windows.then_some(vm::PlanLimitsInput {
                five_hour: args.forward.plan_limit_5h,
                weekly: args.forward.plan_limit_weekly,
            }),
        })
    } else {
        None
    };

    Ok(vm::ProviderPatch {
        name: args.name.clone(),
        // Absent or empty keeps the stored key, so leaving the flag out is not
        // speaking about it.
        api_key: args.key.clone(),
        endpoint: args.endpoint.clone(),
        protocol: match &args.protocol {
            Some(raw) => Some(parse_protocol(raw)?.to_string()),
            None => None,
        },
        openai_wire: args.openai_wire.clone(),
        // Not a flag here, and the old shape sent `""` — which *cleared* the
        // column. Silent data loss on an unrelated edit, and exactly what
        // "absent means keep" removes.
        model_default: None,
        billing: args.billing.as_ref().map(|_| billing.clone()),
        billing_config,
        agents,
        // The flag list is the resulting set when given; otherwise the stored
        // endpoints are not spoken about.
        endpoints: if args.forward.endpoint_extra.is_empty() {
            None
        } else {
            Some(parse_extra_endpoints(&args.forward.endpoint_extra)?)
        },
        advanced: advanced_patch(&args.forward, args.no_headers)?,
        // Doubled: `None` = not speaking about it, `Some(None)` = clear —
        // which is what `--clear-plan-query` means, and why the field cannot be
        // a plain `Option<Value>` (`migrate.local.md` §10.38).
        plan_query: plan_query_input(&args.forward, args.clear_plan_query)?.map(Some),
        prices: None,
        catalog_id: None,
    })
}

/// Park a provider, or put it back. Nothing about its route changes: the row's
/// own `enabled` decides whether it may serve, and the gateway's route table
/// reads it on every reload.
fn providers_set_enabled(ctx: &mut Ctx, provider_id: &str, enabled: bool) -> Result<(), CliError> {
    // Read the two things the message needs *before* the write, so a
    // disable can still name who it affects.
    let name = ctx.api.provider_ref(provider_id)?.name;
    let agents = agents_bound_to(&ctx.api, provider_id)?;
    ctx.api.set_provider_enabled(provider_id, enabled)?;
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
/// Derived from the agent routes the daemon serves rather than from a store
/// walk: bindings are indexed by agent, so the honest shape of the question is
/// "which routes name this provider", and that is what the route list answers.
fn agents_bound_to(
    api: &kiwano_core::daemon_api::DaemonApi,
    provider_id: &str,
) -> Result<Vec<String>, CliError> {
    Ok(api
        .list_agent_routes()?
        .into_iter()
        .filter(|r| r.bindings.iter().any(|b| b.provider_id == provider_id))
        .map(|r| r.agent)
        .collect())
}
