//! The request-log audit trail: the paged list, one entry in full, the clear —
//! plus the one action about the log *files* rather than the rows, revealing the
//! directory in the OS file manager.
//!
//! Five of the seven are the daemon's now (`migrate.local.md` §7 batch 1): the
//! reads and the clear, and the two credential-banner calls, whose ack marker
//! sits in the same `app_settings` KV the daemon already writes its own settings
//! to. None of them affects routing, so none sends a reload ping.
//!
//! The export is **split** now (`migrate.local.md` §10.17): the daemon says what
//! the rows are and how they are spelled, and this side writes them to the path
//! the user picked in a save dialog. `open_log_folder` stays whole — it is a
//! statement about *this machine*, which the daemon will never serve.

use kiwanod::store::RequestLogFilter;
use tauri::AppHandle;

use crate::paths::db_path;
use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

// ── Request logs (full data-plane audit trail, migration V5) ──

#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn list_request_logs(
    page: i64,
    page_size: i64,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<vm::RequestLogListVm, String> {
    DaemonApi::connect().list_request_logs(
        page,
        page_size,
        RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
            // The log screen has no session filter of its own yet; the field is
            // here because the filter is one shape for every caller.
            session_id: None,
        },
    )
}

/// Writes the filtered log slice to `path` as CSV. The frontend picks the path
/// from the dialog plugin first — the same split as `export_config` — and this
/// is `async` because a full export is a lot of formatting to block a thread on.
///
/// The bodies are in the file. Capture records them and there is no switch
/// between the two, so a full export reads `request_bodies` — the trade the
/// export makes for a file that can actually be looked at.
#[tauri::command(async)]
pub fn export_request_logs(
    path: String,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<vm::RequestLogExportVm, String> {
    // The rows and their spelling are the daemon's; the **file** is this side's,
    // because the path came from the user's save dialog.
    let (csv, rows_written, truncated) = kiwano_core::daemon_api::DaemonApi::connect()
        .export_request_logs(RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
            session_id: None,
        })?;
    std::fs::write(&path, &csv).map_err(|e| format!("cannot write {path}: {e}"))?;
    Ok(vm::RequestLogExportVm {
        rows_written,
        truncated,
    })
}

#[tauri::command(async)]
pub fn get_request_log(id: i64) -> Result<Option<vm::RequestLogDetailVm>, String> {
    DaemonApi::connect().get_request_log(id)
}

#[tauri::command(async)]
pub fn clear_request_logs() -> Result<(), String> {
    DaemonApi::connect().clear_request_logs()
}

// ── Credential-watch banner ──

/// The newest credential finding the user has not acknowledged — the banner's
/// poll. Read-only on purpose: only an explicit dismiss/click acks, so the
/// banner survives across polls and restarts until then.
#[tauri::command(async)]
pub fn check_credential_finding() -> Result<Option<kiwanod::store::RequestLogEntry>, String> {
    DaemonApi::connect().check_credential_finding()
}

/// Banner dismissed or clicked: acknowledge that log id. A newer finding
/// re-raises the banner.
#[tauri::command(async)]
pub fn ack_credential_finding(id: i64) -> Result<(), String> {
    DaemonApi::connect().ack_credential_finding(id)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
// ── Logs ──

/// Reveal the log directory in the OS file manager. The path follows the
/// database, which only this side knows, so the UI asks rather than guesses.
#[tauri::command]
pub fn open_log_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = kiwanod::logging::log_dir(&db_path());
    // It may not exist yet — nothing has been logged before the directory is
    // made, and an empty folder explains itself better than an error does.
    let _ = std::fs::create_dir_all(&dir);
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}
