//! Agent detection: what is installed and at what version, the manual
//! directories the detector is pointed at when it cannot find an install by
//! itself, and the currency metadata that is read beside it.

use tauri::State;

use crate::paths::home_dir;
use crate::state::AppState;
use kiwano_core::{detect, pricing};

// ── Agent detection and currency metadata ──
//
// Both readers are parameterless or take only the auxiliary connection, so they
// carry no Tauri state themselves; these wrappers exist to give the frontend
// the command names it invokes.

#[tauri::command(async)]
pub fn detect_agents(state: State<'_, AppState>) -> Vec<detect::AgentDetectVm> {
    let declared = state.store.manual_agent_dirs();
    detect::detect_agents(&home_dir(), &declared)
}

#[tauri::command(async)]
pub fn probe_agent_versions(state: State<'_, AppState>) -> Vec<detect::AgentVersionVm> {
    let declared = state.store.manual_agent_dirs();
    detect::probe_agent_versions(&home_dir(), &declared)
}

/// Point the detector at a directory for a built-in agent it cannot find by
/// itself — the escape hatch for an install the walk has no way to know about.
///
/// The directory is checked before it is stored: it has to hold a runnable
/// executable with the agent's own name, and what was found comes back. What
/// that check cannot establish is identity — see `detect::verify_manual_dir` —
/// so the caller is told what ran, not that it was verified.
#[tauri::command]
pub fn set_agent_dir(
    state: State<AppState>,
    agent: String,
    dir: String,
) -> Result<detect::ManualHit, String> {
    let path = std::path::Path::new(&dir);
    let hit = detect::verify_manual_dir(&agent, path)?;
    state
        .store
        .set_manual_agent_dir(&agent, path)
        .map_err(|e| e.to_string())?;
    Ok(hit)
}

/// The same check without storing anything — what the dialog runs the moment a
/// directory is picked, so a wrong one is answered before the user commits to
/// it. The write path checks again rather than trusting this: the two are one
/// call apart now, but a client is not a reason to store an unverified path.
#[tauri::command(async)]
pub fn verify_agent_dir(agent: String, dir: String) -> Result<detect::ManualHit, String> {
    detect::verify_manual_dir(&agent, std::path::Path::new(&dir))
}

/// Everywhere the detector looks for `agent`, for the dialog that has to say
/// *where* it looked before asking the user to point at it.
///
/// The list comes from the walk itself rather than a copy of it, and each
/// directory is written the way the user would (`~` for their home), because
/// these are being read rather than resolved.
#[tauri::command(async)]
pub fn agent_search_dirs(state: State<'_, AppState>, agent: String) -> Vec<String> {
    let home = home_dir();
    let declared: Vec<std::path::PathBuf> = state
        .store
        .manual_agent_dirs()
        .remove(&agent)
        .into_iter()
        .collect();
    detect::agent_search_dirs(&agent, &home, &declared)
        .iter()
        .map(|dir| match dir.strip_prefix(&home) {
            Ok(rest) => format!("~/{}", rest.display()),
            Err(_) => dir.display().to_string(),
        })
        .collect()
}

/// Forget a declaration: the agent goes back to whatever the detector finds for
/// itself.
#[tauri::command]
pub fn clear_agent_dir(state: State<AppState>, agent: String) -> Result<(), String> {
    state
        .store
        .clear_manual_agent_dir(&agent)
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_currency_meta(state: State<AppState>) -> Result<pricing::CurrencyMetaVm, String> {
    pricing::currency_meta(&state.aux)
}
