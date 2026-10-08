//! Agent routes: how an agent chooses among its providers, which providers it
//! has and in what order, what parameters each carries — plus the user-defined
//! agents that get a route of their own.
//!
//! The route and binding commands are **served by the daemon** (`migrate.local.md`
//! §7 batch 1), which is the process that holds the route table: it re-reads
//! that table itself after the write, so the `after_mutation` ping this module
//! used to end with is gone. A command that no longer touches the store also no
//! longer takes `State<AppState>` — the signature says so.
//!
//! The three custom-agent commands still write directly and still reload; they
//! are the other half of this module and move next.

use tauri::State;

use crate::state::{after_mutation, AppState};
use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

// ── Agent strategies (tech.md §4.7: strategy types / candidate ordering) ──

#[tauri::command]
pub fn get_agent_routes() -> Result<Vec<vm::AgentRouteVm>, String> {
    DaemonApi::connect().list_agent_routes()
}

#[tauri::command]
pub fn update_agent_strategy(
    agent: String,
    strategy: String,
    config: Option<String>,
) -> Result<(), String> {
    DaemonApi::connect().set_agent_strategy(&agent, &strategy, config.as_deref())
}

/// Set or clear one agent's spend ceilings — the whole set at once, one window per
/// entry. An empty list clears them.
#[tauri::command]
pub fn set_agent_limits(agent: String, limits: Vec<vm::AgentLimitVm>) -> Result<(), String> {
    DaemonApi::connect().set_agent_limits(&agent, &limits)
}

#[tauri::command]
pub fn reorder_agent_bindings(agent: String, provider_ids: Vec<String>) -> Result<(), String> {
    DaemonApi::connect().reorder_agent_bindings(&agent, &provider_ids)
}

/// Patch one binding's strategy parameters (weight / local time window).
#[tauri::command]
pub fn update_agent_binding(
    agent: String,
    provider_id: String,
    weight: Option<i64>,
    win_start: Option<String>,
    win_end: Option<String>,
) -> Result<(), String> {
    DaemonApi::connect().update_agent_binding(
        &agent,
        &provider_id,
        weight,
        win_start.as_deref(),
        win_end.as_deref(),
    )
}

/// Bind a provider to an agent (appended at the queue tail) and unbind it.
#[tauri::command]
pub fn add_agent_binding(agent: String, provider_id: String) -> Result<(), String> {
    DaemonApi::connect().add_agent_binding(&agent, &provider_id)
}

#[tauri::command]
pub fn remove_agent_binding(agent: String, provider_id: String) -> Result<(), String> {
    DaemonApi::connect().remove_agent_binding(&agent, &provider_id)
}

/// Define a user-defined agent: a named route with its own placeholder key.
/// Nothing on disk changes — there is no config here to rewrite — so the only
/// work is the row, the key and the default strategy.
#[tauri::command]
pub fn add_custom_agent(
    state: State<AppState>,
    label: String,
    note: Option<String>,
    protocol: Option<String>,
) -> Result<vm::CustomAgentVm, String> {
    let created = vm::add_custom_agent(&state.store, &label, note.as_deref(), protocol.as_deref())?;
    after_mutation(&state);
    Ok(created)
}

/// Rename a user-defined agent. Its id, route and key are untouched — only the
/// name its user reads changes.
#[tauri::command]
pub fn update_custom_agent(
    state: State<AppState>,
    id: String,
    label: String,
    note: Option<String>,
    protocol: Option<String>,
) -> Result<vm::CustomAgentVm, String> {
    let updated = vm::update_custom_agent(
        &state.store,
        &id,
        &label,
        note.as_deref(),
        protocol.as_deref(),
    )?;
    after_mutation(&state);
    Ok(updated)
}

/// Delete a user-defined agent along with its route and its key. Its usage and
/// request logs stay, so the Dashboard keeps accounting for what ran.
#[tauri::command]
pub fn remove_custom_agent(state: State<AppState>, id: String) -> Result<(), String> {
    vm::remove_custom_agent(&state.store, &id)?;
    after_mutation(&state);
    Ok(())
}

/// Copy another agent's whole route (strategy + ordered candidates) onto
/// this one, replacing whatever it had.
#[tauri::command]
pub fn apply_agent_route(target: String, source: String) -> Result<(), String> {
    DaemonApi::connect().apply_agent_route(&target, &source)
}
