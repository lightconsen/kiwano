//! Provider rows: the list the first screen reads, and the writes behind its
//! add / edit / enable / delete actions.
//!
//! Two of the five are the daemon's now (`migrate.local.md` §7 batch 1) — it
//! performs the write and re-reads its own route table, so those commands carry
//! no `State<AppState>` and no reload ping. The other three still write directly:
//! `list_providers` and `update_provider` are mixed (they need the live
//! per-machine view), and `add_provider` reads the Hub catalog, which moves with
//! the Hub half of the work (§10.9).

use tauri::State;

use crate::paths::home_dir;
use crate::state::{after_mutation, AppState};
use kiwano_core::vm;

#[tauri::command]
pub fn list_providers(state: State<AppState>) -> Result<Vec<vm::ProviderVm>, String> {
    vm::build_provider_vms(&state.store, &state.aux, &home_dir(), state.shell_vars())
}

#[tauri::command]
pub fn add_provider(
    state: State<AppState>,
    input: vm::NewProviderInput,
) -> Result<vm::ProviderVm, String> {
    let vm = vm::add_provider(&state.store, &state.aux, &input)?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
pub fn set_provider_enabled(id: String, enabled: bool) -> Result<(), String> {
    // Served by the daemon, which reloads its own route table — a provider that
    // is switched off must stop being a candidate immediately.
    kiwano_core::daemon_api::DaemonApi::connect().set_provider_enabled(&id, enabled)
}

#[tauri::command]
pub fn update_provider(
    state: State<AppState>,
    id: String,
    input: vm::NewProviderInput,
) -> Result<vm::ProviderVm, String> {
    let vm = vm::update_provider(
        &state.store,
        &state.aux,
        &home_dir(),
        &id,
        &input,
        state.shell_vars(),
    )?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
pub fn delete_provider(id: String) -> Result<bool, String> {
    // Served by the daemon: the promotion of a replacement primary is part of
    // the same operation, and it is the daemon that re-reads its route table.
    kiwano_core::daemon_api::DaemonApi::connect().delete_provider(&id)
}
