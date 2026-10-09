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
//! The whole module is served by the daemon now, so nothing here takes
//! `State<AppState>` any more: a command that no longer touches the store says
//! so in its signature.

use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

// ── Agent strategies (tech.md §4.7: strategy types / candidate ordering) ──

#[tauri::command(async)]
pub fn get_agent_routes() -> Result<Vec<vm::AgentRouteVm>, String> {
    DaemonApi::connect().list_agent_routes()
}

#[tauri::command(async)]
pub fn update_agent_strategy(
    agent: String,
    strategy: String,
    config: Option<String>,
) -> Result<(), String> {
    DaemonApi::connect().set_agent_strategy(&agent, &strategy, config.as_deref())
}

/// Set or clear one agent's spend ceilings — the whole set at once, one window per
/// entry. An empty list clears them.
#[tauri::command(async)]
pub fn set_agent_limits(agent: String, limits: Vec<vm::AgentLimitVm>) -> Result<(), String> {
    DaemonApi::connect().set_agent_limits(&agent, &limits)
}

#[tauri::command(async)]
pub fn reorder_agent_bindings(agent: String, provider_ids: Vec<String>) -> Result<(), String> {
    DaemonApi::connect().reorder_agent_bindings(&agent, &provider_ids)
}

/// Patch one binding's strategy parameters (weight / local time window).
#[tauri::command(async)]
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
#[tauri::command(async)]
pub fn add_agent_binding(agent: String, provider_id: String) -> Result<(), String> {
    DaemonApi::connect().add_agent_binding(&agent, &provider_id)
}

#[tauri::command(async)]
pub fn remove_agent_binding(agent: String, provider_id: String) -> Result<(), String> {
    DaemonApi::connect().remove_agent_binding(&agent, &provider_id)
}

/// Define a user-defined agent: a named route with its own placeholder key.
/// Nothing on disk changes — there is no config here to rewrite — so the only
/// work is the row, the key and the default strategy.
///
/// **This command mints the id**, and that is what makes a retried create
/// idempotent: nothing else in the request identifies the operation, since the
/// id carries a random suffix, and the daemon answers an id it has already seen
/// with the agent that exists (`migrate.local.md` §6.1). A label the user
/// repeats is a fresh intent here, so it gets a fresh id — and therefore its own
/// agent, which is the behaviour the Apps screen documents.
#[tauri::command(async)]
pub fn add_custom_agent(
    label: String,
    note: Option<String>,
    protocol: Option<String>,
) -> Result<vm::CustomAgentVm, String> {
    let id = vm::mint_agent_id(&label);
    DaemonApi::connect().add_custom_agent(&id, &label, note.as_deref(), protocol.as_deref())
}

/// Rename a user-defined agent. Its id, route and key are untouched — only the
/// name its user reads changes.
#[tauri::command(async)]
pub fn update_custom_agent(
    id: String,
    label: String,
    note: Option<String>,
    protocol: Option<String>,
) -> Result<vm::CustomAgentVm, String> {
    DaemonApi::connect().update_custom_agent(&id, &label, note.as_deref(), protocol.as_deref())
}

/// Delete a user-defined agent along with its route and its key. Its usage and
/// request logs stay, so the Dashboard keeps accounting for what ran.
#[tauri::command(async)]
pub fn remove_custom_agent(id: String) -> Result<(), String> {
    DaemonApi::connect().remove_custom_agent(&id)
}

/// Copy another agent's whole route (strategy + ordered candidates) onto
/// this one, replacing whatever it had.
#[tauri::command(async)]
pub fn apply_agent_route(target: String, source: String) -> Result<(), String> {
    DaemonApi::connect().apply_agent_route(&target, &source)
}
