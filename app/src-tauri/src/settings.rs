//! The Settings object: read whole, patched in part. A patch that moves
//! autostart or the request-log knobs has a side effect the store cannot carry
//! out itself — the OS login item, or the daemon's log config — so both are
//! applied here.

use tauri::State;

use crate::state::{after_mutation, AppState};
use crate::tray::sync_autostart;
use kiwano_core::vm;

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<vm::SettingsVm, String> {
    vm::build_settings(&state.store, &state.aux, state.shell_vars())
}

#[tauri::command]
pub fn update_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    patch: serde_json::Value,
) -> Result<vm::SettingsVm, String> {
    let vm = vm::update_settings(&state.store, &state.aux, &patch, state.shell_vars())?;
    // apply autostart changes to the OS login items immediately
    if let Some(v) = patch.get("autostart").and_then(|v| v.as_bool()) {
        sync_autostart(&app, v);
    }
    // The gateway re-reads the log config on /reload; ping it when the
    // request-log settings changed so the toggle applies without a restart.
    if patch.get("request_logs").is_some()
        || patch.get("log_retention_days").is_some()
        || patch.get("log_max_body_bytes").is_some()
        || patch.get("compat_shim").is_some()
        || patch.get("stream_first_byte_secs").is_some()
        || patch.get("stream_idle_secs").is_some()
    {
        after_mutation(&state);
    }
    Ok(vm)
}
