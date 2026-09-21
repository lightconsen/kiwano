//! Config sharing, and the one-time import from cc-switch. The Settings section
//! that called the export/import pair was removed on 2026-09-10; the note on
//! the banner below is why they stay registered.

use tauri::State;

use crate::paths::home_dir;
use crate::state::{after_mutation, AppState};
use kiwano_core::{import, share};

// ── Config sharing (spec §4.1 P1: export/import of one-click scheme JSON) ──
//
// Registered but unreachable from the UI: the Settings section that called them
// was removed on 2026-09-10 (80eb138), because its actions "didn't match the
// roadmap for config sharing" — the roadmap being community sharing, not a local
// file. They stay (the CLI's `config export|import` goes through the same
// `kiwano-core::share`, so the capability is live; only this app-side entry is
// missing) and the decision not to rebuild it is recorded in `api/types.ts`.

/// The frontend picks the target path via the dialog plugin first; this writes the file (file IO → async).
/// Credentials are omitted unless `include_keys` is set (local backup only).
#[tauri::command(async)]
pub fn export_config(
    state: State<AppState>,
    path: String,
    include_keys: Option<bool>,
) -> Result<usize, String> {
    share::export_config_to_file(&state.store, &path, include_keys.unwrap_or(false))
}

#[tauri::command(async)]
pub fn import_config(state: State<AppState>, path: String) -> Result<share::ImportReport, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let report = share::import_config(&state.store, &json)?;
    after_mutation(&state);
    Ok(report)
}

#[tauri::command]
pub fn import_cc_switch(state: State<AppState>) -> import::ImportReportVm {
    // The same home every other read uses: cc-switch's files are the user's,
    // and a `HOME` that is not the profile directory would look for them in a
    // place that does not have them.
    let home = home_dir().join(".cc-switch");
    let report = import::run_import(
        &state.store,
        Some(&home.join("cc-switch.db")),
        Some(&home.join("config.json")),
    );
    if report.imported > 0 {
        after_mutation(&state);
    }
    report
}
