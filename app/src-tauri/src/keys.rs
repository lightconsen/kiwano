//! Multi-key rotation: the extra keys a provider can be called with. The
//! gateway's key pool reads them, so a write reloads the daemon.

use tauri::State;

use crate::state::{after_mutation, AppState};
use kiwano_core::vm;

// ── Multi-key rotation (spec §4.1 P1) ──

/// A provider's rotation keys (the ones besides the primary key).
#[tauri::command]
pub fn list_api_keys(
    state: State<AppState>,
    provider_id: String,
) -> Result<Vec<vm::ApiKeyVm>, String> {
    vm::list_api_keys(&state.store, &provider_id)
}

/// Append a rotation key; triggers /reload so the gateway key pool picks it up immediately.
#[tauri::command]
pub fn add_api_key(
    state: State<AppState>,
    provider_id: String,
    api_key: String,
    label: Option<String>,
) -> Result<vm::ApiKeyVm, String> {
    let vm = vm::add_api_key(&state.store, &provider_id, &api_key, label.as_deref())?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
pub fn delete_api_key(state: State<AppState>, id: i64) -> Result<bool, String> {
    let ok = vm::delete_api_key(&state.store, id)?;
    after_mutation(&state);
    Ok(ok)
}
