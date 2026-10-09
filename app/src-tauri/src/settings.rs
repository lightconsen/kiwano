//! The Settings object: read whole, patched in part. A patch that moves
//! autostart or the request-log knobs has a side effect the store cannot carry
//! out itself — the OS login item, or the daemon's log config — so both are
//! applied here.

use tauri::State;

use crate::paths::home_dir;
use crate::state::AppState;
use crate::tray::sync_autostart;
use kiwano_core::vm;

// `(async)`: the body is synchronous but now reaches the daemon, and a plain
// `#[tauri::command]` would run that round trip on the UI thread.
#[tauri::command(async)]
pub fn get_settings(state: State<AppState>) -> Result<vm::SettingsVm, String> {
    // The blob and the custom agents are the daemon's; the takeovers are read
    // out of this machine's agent configs (`migrate.local.md` §10.44). No
    // database handle either way.
    vm::settings_view(
        &kiwano_core::daemon_api::DaemonApi::connect(),
        &home_dir(),
        state.shell_vars(),
    )
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
    let api = kiwano_core::daemon_api::DaemonApi::connect();
    let mut vm: kiwano_api::settings::SettingsVm = api.update_settings(&patch)?;
    // The OS login item is a statement about *this machine* — the daemon never
    // touches it, so the app applies it here.
    if let Some(v) = patch.get("autostart").and_then(|v| v.as_bool()) {
        sync_autostart(&app, v);
    }
    // The takeovers and the custom agents are layered on from this machine's
    // agents and their config files — the daemon's blob has none of them.
    vm::layer_settings(&api, &mut vm, &home_dir(), state.shell_vars())?;
    Ok(vm)
}
