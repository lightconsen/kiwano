//! Cost alerts: the hits the frontend polls for so it can raise a system
//! notification. The gateway enforces the limits from its own timer, so this
//! only ever reads.

use tauri::State;

use crate::state::AppState;
use kiwano_core::vm;

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

/// Polled periodically by the frontend; hits not yet notified are returned so
/// the frontend can raise a system notification (KV dedup prevents repeats).
///
/// Notification only: the gateway enforces the limits, from its own timer, so
/// they hold whether or not this app is running. Nothing here disables a
/// provider any more.
#[tauri::command]
pub fn check_usage_alerts(state: State<AppState>) -> Result<Vec<vm::UsageAlertVm>, String> {
    vm::check_usage_alerts(&state.store, &state.aux, true)
}
