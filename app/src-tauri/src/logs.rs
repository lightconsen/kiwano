//! The request-log audit trail: the paged list, one entry in full, the CSV
//! export and the clear — plus the one action about the log *files* rather than
//! the rows, revealing the directory in the OS file manager.

use kiwanod::store::RequestLogFilter;
use tauri::{AppHandle, State};

use crate::paths::db_path;
use crate::state::AppState;
use kiwano_core::vm;

// ── Request logs (full data-plane audit trail, migration V5) ──

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub fn list_request_logs(
    state: State<AppState>,
    page: i64,
    page_size: i64,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<vm::RequestLogListVm, String> {
    vm::list_request_logs(
        &state.store,
        page,
        page_size,
        RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
        },
    )
}

/// Writes the filtered log slice to `path` as CSV. The frontend picks the path
/// from the dialog plugin first — the same split as `export_config` — and this
/// is `async` because a full export is a lot of formatting to block a thread on.
/// `include_bodies` is the export dialog's choice; bodies are always captured,
/// so this is where the file decides whether to carry them. Defaults to off for
/// a caller that does not send it — the safe direction for a file that may be
/// shared.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
pub fn export_request_logs(
    state: State<AppState>,
    path: String,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
    include_bodies: Option<bool>,
) -> Result<vm::RequestLogExportVm, String> {
    vm::export_request_logs_csv(
        &state.store,
        &path,
        RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
        },
        include_bodies.unwrap_or(false),
    )
}

#[tauri::command]
pub fn get_request_log(
    state: State<AppState>,
    id: i64,
) -> Result<Option<vm::RequestLogDetailVm>, String> {
    vm::get_request_log(&state.store, id)
}

#[tauri::command]
pub fn clear_request_logs(state: State<AppState>) -> Result<(), String> {
    vm::clear_request_logs(&state.store)
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
