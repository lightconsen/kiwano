//! Provider rows: the list the first screen reads, and the writes behind its
//! add / edit / enable / delete actions.
//!
//! Four of the five are the daemon's now — it performs each write and re-reads
//! its own route table, so they carry no reload ping. `update_provider` is the
//! one that is **split**: its write is the daemon's and its view is assembled
//! here, because the assembly reads *this machine* (`migrate.local.md` §5's
//! fifth constraint). `list_providers` is that same assembly with nothing to
//! write, so it is a client command by the same rule.

use tauri::State;

use crate::paths::home_dir;
use crate::state::AppState;
use kiwano_core::vm;

#[tauri::command(async)]
pub fn list_providers(state: State<AppState>) -> Result<Vec<vm::ProviderVm>, String> {
    // **The one screen whose assembly needs a machine fact.** Which agents route
    // through the gateway is read out of their own config files, so the client
    // computes that and the daemon builds the rows around it
    // (`migrate.local.md` §5 #2, §10.21) — which is what lets this work when the
    // daemon is on another machine.
    let live = vm::providers::live_bound_agents(&state.store, &home_dir(), state.shell_vars())?;
    kiwano_core::daemon_api::DaemonApi::connect().provider_view(&live)
}

#[tauri::command(async)]
pub fn add_provider(input: vm::NewProviderInput) -> Result<vm::ProviderVm, String> {
    // Served by the daemon. **This command mints the id**, which is what makes
    // a retried add idempotent: an id the daemon has already seen is answered
    // with the provider that exists (`migrate.local.md` §6.1). The daemon
    // re-reads its own route table, so the new candidate is picked up at once.
    let id = vm::mint_provider_id(&input.name);
    kiwano_core::daemon_api::DaemonApi::connect().add_provider(&id, &input)
}

#[tauri::command(async)]
pub fn set_provider_enabled(id: String, enabled: bool) -> Result<(), String> {
    // Served by the daemon, which reloads its own route table — a provider that
    // is switched off must stop being a candidate immediately.
    kiwano_core::daemon_api::DaemonApi::connect().set_provider_enabled(&id, enabled)
}

#[tauri::command(async)]
pub fn update_provider(
    state: State<AppState>,
    id: String,
    input: vm::NewProviderInput,
) -> Result<vm::ProviderVm, String> {
    // The write is the daemon's; the **view** is assembled here, because it is
    // the half that reads this machine (which agents actually route through the
    // gateway) — `migrate.local.md` §5's fifth constraint. The daemon re-reads
    // its own route table, so there is no reload ping to send.
    kiwano_core::daemon_api::DaemonApi::connect().update_provider(&id, &input)?;
    let vms = vm::build_provider_vms(&state.store, &state.aux, &home_dir(), state.shell_vars())?;
    vms.into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "provider vanished after update".to_string())
}

#[tauri::command(async)]
pub fn delete_provider(id: String) -> Result<bool, String> {
    // Served by the daemon: the promotion of a replacement primary is part of
    // the same operation, and it is the daemon that re-reads its route table.
    kiwano_core::daemon_api::DaemonApi::connect().delete_provider(&id)
}
