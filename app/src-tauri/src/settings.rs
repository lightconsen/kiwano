//! The Settings object: read whole, patched in part. A patch that moves
//! autostart or the request-log knobs has a side effect the store cannot carry
//! out itself — the OS login item, or the daemon's log config — so both are
//! applied here.

use tauri::State;

use crate::state::AppState;
use crate::tray::sync_autostart;
use kiwano_core::vm;

#[tauri::command]
pub fn get_settings(state: State<AppState>) -> Result<vm::SettingsVm, String> {
    vm::build_settings(&state.store, &state.aux, state.shell_vars())
}

#[tauri::command(async)]
pub fn update_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    patch: serde_json::Value,
) -> Result<vm::SettingsVm, String> {
    // The blob and the gateway-facing mirrors are the daemon's
    // (`migrate.local.md` §10.16): it applies the patch and re-reads its own
    // config, so this sends no `/reload`.
    let mut vm: kiwano_api::settings::SettingsVm =
        kiwano_core::daemon_api::DaemonApi::connect().update_settings(&patch)?;
    // The OS login item is a statement about *this machine* — the daemon never
    // touches it, so the app applies it here.
    if let Some(v) = patch.get("autostart").and_then(|v| v.as_bool()) {
        sync_autostart(&app, v);
    }
    // The takeovers and the custom agents are layered on from this machine's
    // agents and their config files — the daemon's blob has none of them.
    let full = vm::build_settings(&state.store, &state.aux, state.shell_vars())?;
    vm.takeovers = full.takeovers;
    vm.custom_agents = full.custom_agents;
    Ok(vm)
}
