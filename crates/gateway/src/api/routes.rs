//! Agent routing: which provider serves an agent, in what order the candidates
//! are tried, and the per-agent limits and strategy that ride along.
//!
//! Moved here from `kiwano_core::vm::routes` when the daemon started serving it
//! (see this module's parent). The bodies are unchanged except for the error
//! type — an [`ApiError`] carries the kind the server picks a status from, so
//! the server never has to re-derive what the function already decided.
//!
//! [`ApiError`]: kiwano_api::error::ApiError

use crate::store::{now_rfc3339, AgentLimit, Binding, Store, Strategy, StrategyType};
use kiwano_api::error::ApiError;
use kiwano_api::logo::{logo_char, palette_color};
use kiwano_api::routes::{AgentLimitVm, AgentRouteVm, BindingVm};
use std::collections::HashMap;

// ── Agent strategy views (tech.md §4.7: strategy types + candidate ordering) ──

/// One stored ceiling as the screen sees it.
fn limit_vm(l: AgentLimit) -> AgentLimitVm {
    AgentLimitVm {
        period: l.period,
        period_limit: l.period_limit,
        limit_unit: l.limit_unit,
    }
}

/// One row per Agent (only agents with bindings); strategy defaults to single.
pub fn build_agent_routes(store: &Store) -> Result<Vec<AgentRouteVm>, ApiError> {
    let providers: HashMap<String, crate::store::Provider> = store
        .list_providers()
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();

    let mut routes = Vec::new();
    for agent in store.bound_agents().map_err(ApiError::failed)? {
        let strategy = store
            .get_strategy(&agent)
            .map_err(ApiError::failed)?
            .unwrap_or(Strategy {
                agent: agent.clone(),
                kind: StrategyType::Single,
                config: None,
            });
        let bindings = store
            .bindings_for_agent(&agent)
            .map_err(ApiError::failed)?
            .into_iter()
            .map(|b| {
                let name = providers
                    .get(&b.provider_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| b.provider_id.clone());
                BindingVm {
                    logo_char: logo_char(&name),
                    logo_color: palette_color(&name).to_string(),
                    provider_name: name,
                    provider_id: b.provider_id,
                    priority: b.priority,
                    weight: b.weight,
                    win_start: b.win_start,
                    win_end: b.win_end,
                    enabled: b.enabled,
                }
            })
            .collect();
        routes.push(AgentRouteVm {
            strategy: strategy.kind.as_str().to_string(),
            config: strategy.config,
            limits: store
                .agent_limits_for(&agent)
                .map_err(ApiError::failed)?
                .into_iter()
                .map(limit_vm)
                .collect(),
            agent,
            bindings,
        });
    }
    Ok(routes)
}

/// Set or clear one agent's own ceiling. An empty list clears them — an absent
/// row is the absence of a limit, which is what the gateway reads as "no
/// ceiling".
pub fn set_agent_limits(
    store: &Store,
    agent: &str,
    limits: Vec<AgentLimitVm>,
) -> Result<(), ApiError> {
    let now = now_rfc3339();
    let rows: Vec<AgentLimit> = limits
        .into_iter()
        // A window of zero is not a window of nothing: the gateway reads it as
        // "no ceiling at all" (see `limits::agent_limit_usage`), so storing one
        // would show the user a limit that does not exist. The screen sends what
        // its fields hold, including an empty or half-typed one, and this is where
        // that is dropped rather than being written down. `is_finite` first, for
        // the reason the quota config does it: a NaN compares false against
        // everything.
        .filter(|l| l.period_limit.is_finite() && l.period_limit > 0.0 && !l.period.is_empty())
        .map(|l| AgentLimit {
            agent: agent.to_string(),
            period: l.period,
            period_limit: l.period_limit,
            limit_unit: l.limit_unit,
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .collect();
    // An empty set is how the screen says "no limit"; `replace` writes it as the
    // absence of rows.
    store
        .replace_agent_limits(agent, &rows)
        .map_err(ApiError::failed)
}

/// Update an Agent's strategy type (+ optional JSON config); unknown types error.
pub fn set_agent_strategy(
    store: &Store,
    agent: &str,
    strategy: &str,
    config: Option<&str>,
) -> Result<(), ApiError> {
    let kind = StrategyType::parse_str(strategy)
        .ok_or_else(|| ApiError::invalid(format!("unknown strategy type: {strategy}")))?;
    store
        .upsert_strategy(agent, kind, config)
        .map_err(ApiError::failed)?;
    // Entering roundrobin: seed the weights as an even split of 100 (2
    // candidates → 50/50, 3 → 34/33/33, remainder to the head of the queue)
    // instead of leaving every candidate at 1, so the rotation starts balanced.
    if kind == StrategyType::Roundrobin {
        let bindings = store.bindings_for_agent(agent).map_err(ApiError::failed)?;
        let n = bindings.len();
        for (i, mut b) in bindings.into_iter().enumerate() {
            let w = ((100 / n) + if i < 100 % n { 1 } else { 0 }).max(1) as i64;
            if b.weight != w {
                b.weight = w;
                store.upsert_binding(&b).map_err(ApiError::failed)?;
            }
        }
    }
    Ok(())
}

/// Candidate reorder: given a provider_id order → rewrite priority 0..n
/// (weight/window/enabled bits preserved). Unlisted bindings stay; unknown
/// provider_ids error.
pub fn reorder_agent_bindings(
    store: &Store,
    agent: &str,
    provider_ids: &[String],
) -> Result<(), ApiError> {
    let existing: HashMap<String, Binding> = store
        .bindings_for_agent(agent)
        .map_err(ApiError::failed)?
        .into_iter()
        .map(|b| (b.provider_id.clone(), b))
        .collect();
    for (i, pid) in provider_ids.iter().enumerate() {
        let Some(mut b) = existing.get(pid).cloned() else {
            return Err(ApiError::not_found(format!(
                "provider {pid} is not bound to {agent}"
            )));
        };
        b.priority = i as i64;
        store.upsert_binding(&b).map_err(ApiError::failed)?;
    }
    Ok(())
}

/// Patch one binding's strategy parameters (weight for roundrobin, the local
/// "HH:MM" window for timewindow). Unspecified fields keep their value; a
/// binding without a window is the timewindow fallback candidate.
pub fn update_agent_binding(
    store: &Store,
    agent: &str,
    provider_id: &str,
    weight: Option<i64>,
    win_start: Option<String>,
    win_end: Option<String>,
) -> Result<(), ApiError> {
    let mut b = store
        .bindings_for_agent(agent)
        .map_err(ApiError::failed)?
        .into_iter()
        .find(|b| b.provider_id == provider_id)
        .ok_or_else(|| {
            ApiError::not_found(format!("provider {provider_id} is not bound to {agent}"))
        })?;
    if let Some(w) = weight {
        b.weight = w.max(1);
    }
    // Both bounds are set/cleared together: a half window would never match.
    if win_start.is_some() || win_end.is_some() {
        let (s, e) = (
            win_start.filter(|v| !v.is_empty()),
            win_end.filter(|v| !v.is_empty()),
        );
        match (s, e) {
            (Some(s), Some(e)) => {
                b.win_start = Some(s);
                b.win_end = Some(e);
            }
            _ => {
                b.win_start = None;
                b.win_end = None;
            }
        }
    }
    store.upsert_binding(&b).map_err(ApiError::failed)?;
    Ok(())
}

/// Bind a provider to an agent as a new candidate: appended at the tail of
/// the queue (primary keeps its place). Binding an already-bound provider is
/// a no-op so the call stays idempotent — the daemon reloads its own route
/// table after the write, so there is nothing for a caller to notify.
pub fn add_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), ApiError> {
    if store
        .get_provider(provider_id)
        .map_err(ApiError::failed)?
        .is_none()
    {
        return Err(ApiError::not_found(format!(
            "unknown provider: {provider_id}"
        )));
    }
    let existing = store.bindings_for_agent(agent).map_err(ApiError::failed)?;
    if existing.iter().any(|b| b.provider_id == provider_id) {
        return Ok(());
    }
    let next_priority = existing.iter().map(|b| b.priority).max().unwrap_or(-1) + 1;
    store
        .upsert_binding(&Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
            priority: next_priority,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .map_err(ApiError::failed)?;
    Ok(())
}

/// Remove one agent's binding of a provider (other agents keep theirs).
/// Unbinding the last candidate is allowed: the route then has zero
/// candidates and requests fail cleanly with NoBinding until re-bound.
pub fn remove_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), ApiError> {
    let removed = store
        .delete_binding(agent, provider_id)
        .map_err(ApiError::failed)?;
    if !removed {
        return Err(ApiError::not_found(format!(
            "provider {provider_id} is not bound to {agent}"
        )));
    }
    Ok(())
}

/// Copy another agent's whole route onto this one: strategy kind + config
/// plus the ordered candidate list (priority, weight, time windows). The
/// target's existing route is replaced; providers are shared, not moved —
/// the source agent keeps its own bindings. Weights come over as-is
/// (upsert_strategy directly, no roundrobin even-split reseed).
pub fn apply_agent_route(store: &Store, target: &str, source: &str) -> Result<(), ApiError> {
    if target == source {
        return Err(ApiError::invalid(
            "cannot copy an agent's route onto itself",
        ));
    }
    let strategy = store
        .get_strategy(source)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("{source} has no route to copy")))?;
    let bindings = store.bindings_for_agent(source).map_err(ApiError::failed)?;
    if bindings.is_empty() {
        return Err(ApiError::not_found(format!(
            "{source} has no candidates to copy"
        )));
    }
    store
        .upsert_strategy(target, strategy.kind, strategy.config.as_deref())
        .map_err(ApiError::failed)?;
    for b in store.bindings_for_agent(target).map_err(ApiError::failed)? {
        store
            .delete_binding(target, &b.provider_id)
            .map_err(ApiError::failed)?;
    }
    for b in bindings {
        store
            .upsert_binding(&Binding {
                agent: target.to_string(),
                ..b
            })
            .map_err(ApiError::failed)?;
    }
    Ok(())
}
