//! Self-update (docs/upgrade.md): check + download-and-install against the
//! GitHub Releases manifest (`latest.json`, endpoint from tauri.conf.json).
//! Progress streams to the UI over `update-progress` events; install then
//! relaunches the app. The gateway daemon outlives the GUI (lib.rs bottom
//! comment) and is adopted back on the next launch.

use serde::Serialize;
use tauri::{AppHandle, Emitter};
use tauri_plugin_updater::UpdaterExt;

/// A pending update discovered on the release channel.
#[derive(Serialize, Clone)]
pub struct UpdateInfoVm {
    pub version: String,
    pub notes: Option<String>,
    /// Release publish time, unix seconds (None when the manifest omits it).
    pub pub_date: Option<i64>,
}

/// Download progress payload emitted as `update-progress`.
#[derive(Serialize, Clone)]
pub struct UpdateProgressVm {
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Check GitHub Releases for a newer version; `None` means up to date.
/// Network IO → async command thread (lib.rs convention).
#[tauri::command]
pub async fn check_app_update(app: AppHandle) -> Result<Option<UpdateInfoVm>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let Some(update) = updater.check().await.map_err(|e| e.to_string())? else {
        return Ok(None);
    };
    Ok(Some(UpdateInfoVm {
        version: update.version,
        notes: update.body,
        pub_date: update.date.map(|d| d.unix_timestamp()),
    }))
}

/// Download, verify and install the pending update, then relaunch. Progress
/// is emitted as `update-progress` events for the Settings progress bar.
#[tauri::command]
pub async fn download_and_install_app_update(app: AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "no update available".to_string())?;

    let handle = app.clone();
    update
        .download_and_install(
            move |downloaded, total| {
                let _ = handle.emit(
                    "update-progress",
                    UpdateProgressVm {
                        downloaded: downloaded as u64,
                        total,
                    },
                );
            },
            || {},
        )
        .await
        .map_err(|e| e.to_string())?;

    relaunch(&app);
    Ok(())
}

/// Relaunch the current executable, then exit. macOS/Linux: the updater
/// replaces the bundle/AppImage in place, so re-spawning our own exe path
/// launches the new build (Windows already relaunches inside the installer).
fn relaunch(app: &AppHandle) {
    if let Ok(exe) = std::env::current_exe() {
        let _ = std::process::Command::new(exe).spawn();
    }
    app.exit(0);
}
