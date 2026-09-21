//! The route commands: strategy, bind/unbind, limits, copy, and the dump.

use super::render::render_routes;
use crate::cli::{BindingCmd, RoutesCmd};
use crate::{CliError, Ctx};
use kiwano_core::vm;
use kiwanod::strategy::QuotaConfig;

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
