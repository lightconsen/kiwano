//! Config sharing, and the one-time import from cc-switch. The Settings section
//! that called the export/import pair was removed on 2026-09-10; the note on
//! the banner below is why they stay registered.

use crate::paths::home_dir;

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
pub fn export_config(path: String, include_keys: Option<bool>) -> Result<usize, String> {
    // The document is the daemon's; the **file** is this side's, because the
    // path came from the user's save dialog.
    let json = kiwano_core::daemon_api::DaemonApi::connect()
        .export_config(include_keys.unwrap_or(false))?;
    share::write_config_file(&path, &json)?;
    Ok(share::provider_count(&json))
}

#[tauri::command(async)]
pub fn import_config(path: String) -> Result<share::ImportReport, String> {
    // The file is this side's; the daemon applies what it says and re-reads its
    // own route table, so there is no reload ping to send.
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    kiwano_core::daemon_api::DaemonApi::connect().import_config(&json)
}

#[tauri::command]
pub fn import_cc_switch() -> import::ImportReportVm {
    // The same home every other read uses: cc-switch's files are the user's,
    // and a `HOME` that is not the profile directory would look for them in a
    // place that does not have them.
    let home = home_dir().join(".cc-switch");
    // Read here — those files are this machine's — and let the daemon write:
    // it applies the rows and re-reads its own route table, so there is no
    // reload ping to send (`migrate.local.md` §10.19).
    let (raws, skips, mut detail) = import::read_cc_switch(
        Some(&home.join("cc-switch.db")),
        Some(&home.join("config.json")),
    );
    if raws.is_empty() && skips.is_empty() {
        detail.push("No importable CC Switch data found".into());
        return import::ImportReportVm {
            imported: 0,
            skipped: 0,
            detail,
        };
    }
    let mut report = kiwano_core::daemon_api::DaemonApi::connect()
        .import_cc_switch(&raws, &skips)
        .unwrap_or_else(|e| import::ImportReportVm {
            imported: 0,
            skipped: raws.len(),
            detail: vec![format!("import failed: {e}")],
        });
    detail.append(&mut report.detail);
    report.detail = detail;
    report
}
