//! Provider rows: the list the first screen reads, and the writes behind its
//! add / edit / enable / delete actions. Every write ends with a reload ping.

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
pub fn set_provider_enabled(
    state: State<AppState>,
    id: String,
    enabled: bool,
) -> Result<(), String> {
    vm::set_provider_enabled(&state.store, &id, enabled)?;
    after_mutation(&state);
    Ok(())
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
pub fn delete_provider(state: State<AppState>, id: String) -> Result<bool, String> {
    let ok = vm::delete_provider(&state.store, &id)?;
    after_mutation(&state);
    Ok(ok)
}
