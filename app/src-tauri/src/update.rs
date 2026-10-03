//! Self-update (docs/upgrade.md): check + download-and-install against the
//! release manifest (`latest.json`, endpoints from tauri.conf.json).
//!
//! Endpoint order decides which host serves the bytes: the plugin keeps the
//! first manifest that answers and downloads from the URL baked into it, never
//! falling back afterwards. R2 is listed first because the GitHub asset URL
//! redirects to release-assets.githubusercontent.com, which stalls for users
//! who cannot reach it.
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

/// Transfer state, emitted as `update-progress`.
///
/// The steps after the download are the reason this exists. `download_and_install`
/// hands the bytes to `verify_signature` and then to `install`, which unpacks a
/// ~12 MB payload and replaces the bundle — seconds of work that emitted nothing
/// at all, so the UI's last event was the final chunk and the next thing anyone
/// saw was the app relaunching. The empty `on_download_finish` closure is
/// exactly that boundary, and the install's return is the other.
#[derive(Serialize, Clone)]
pub struct UpdateProgressVm {
    pub phase: &'static str,
    /// Bytes so far, and the total from `Content-Length` when the server sent
    /// one. Meaningless outside `downloading` — the UI draws no bar for the
    /// other phases — but carried as the final value rather than as zero, so a
    /// reader of the event stream cannot mistake it for a fresh start.
    pub downloaded: u64,
    pub total: Option<u64>,
}

/// Check the release channel for a newer version; `None` means up to date.
/// Network IO → async command thread (lib.rs convention).
///
/// Deliberately not a command: lib.rs wraps it in `check_app_update`, which
/// records the answer for the UI before returning it. Two commands of the same
/// name would collide on the macro-generated symbols.
pub async fn check(app: AppHandle) -> Result<Option<UpdateInfoVm>, String> {
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
    // The byte count the chunks report is needed by two closures and by the
    // code after the await, and each `move` takes ownership — so one clone per
    // reader. It exists only to label the later phases with the size that was
    // actually transferred.
    let seen = std::sync::Arc::new(std::sync::atomic::AtomicU64::new(0));
    let seen_for_chunk = seen.clone();
    let seen_for_finish = seen.clone();
    let finish_handle = app.clone();
    update
        .download_and_install(
            move |downloaded, total| {
                seen_for_chunk.store(downloaded as u64, std::sync::atomic::Ordering::Relaxed);
                let _ = handle.emit(
                    "update-progress",
                    UpdateProgressVm {
                        phase: "downloading",
                        downloaded: downloaded as u64,
                        total,
                    },
                );
            },
            move || {
                // The plugin calls this after the last byte and before it
                // verifies the signature and installs — the start of the part
                // that used to be silent.
                let done = seen_for_finish.load(std::sync::atomic::Ordering::Relaxed);
                let _ = finish_handle.emit(
                    "update-progress",
                    UpdateProgressVm {
                        phase: "installing",
                        downloaded: done,
                        total: Some(done),
                    },
                );
            },
        )
        .await
        .map_err(|e| e.to_string())?;

    // Installed. The relaunch is next and there is nothing left to report but
    // that, which is still better than the window disappearing unexplained.
    let done = seen.load(std::sync::atomic::Ordering::Relaxed);
    let _ = app.emit(
        "update-progress",
        UpdateProgressVm {
            phase: "restarting",
            downloaded: done,
            total: Some(done),
        },
    );
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
