//! Self-update as the window sees it: the version in the footer, the update the
//! startup check left behind, the record step every check ends in, and the
//! button behind Settings' "Check for updates". The check itself is
//! `crate::update`.

use tauri::{AppHandle, Emitter, Manager, State};

use crate::state::AppState;
use crate::tray::refresh_tray;
use crate::update;
use kiwano_core::vm;

#[tauri::command]
pub fn get_footer_stats(
    app: AppHandle,
    state: State<AppState>,
) -> Result<vm::FooterStatsVm, String> {
    // tauri.conf.json is the single source of truth for the app version —
    // the same one the updater compares against.
    let version = format!("v{}", app.package_info().version);
    vm::build_footer_stats(&state.store, &state.aux, &version)
}

/// The update the silent startup check found, if any. The About block reads
/// this on mount so a user who just saw the "update available" notification
/// finds the same version waiting in Settings.
#[tauri::command]
pub fn get_pending_update(state: State<AppState>) -> Option<update::UpdateInfoVm> {
    state
        .pending_update
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

/// Everything that must happen when a check returns. Kept in one place so the
/// startup check and the manual one cannot drift: both record the result (the
/// update banner and About read it), refresh the tray entry, and tell an open
/// UI.
pub(crate) fn record_update_check(app: &AppHandle, info: Option<&update::UpdateInfoVm>) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.pending_update.lock() {
            // A check that finds nothing clears a previous hit: the pending
            // value always describes the latest answer, not the best one.
            *slot = info.cloned();
        }
        let port = state.data_port;
        let _ = refresh_tray(app, port, info.map(|i| i.version.as_str()));
    }
    if info.is_some() {
        let _ = app.emit("update-available", ());
    }
}

/// Settings' "Check for updates". A thin wrapper over the module function so
/// every check — startup or manual — goes through `record_update_check`.
#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<Option<update::UpdateInfoVm>, String> {
    let info = update::check(app.clone()).await?;
    record_update_check(&app, info.as_ref());
    Ok(info)
}
