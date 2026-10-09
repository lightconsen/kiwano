//! Cost alerts: the hits the frontend polls for so it can raise a system
//! notification. The gateway enforces the limits from its own timer, so this
//! only ever reads.

use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

/// Polled periodically by the frontend; hits not yet notified are returned so
/// the frontend can raise a system notification (the daemon's dedup prevents
/// repeats — and now remembers them, so a machine that opens the app later does
/// not re-raise what the other one already said).
///
/// Notification only: the gateway enforces the limits, from its own timer, so
/// they hold whether or not this app is running. Nothing here disables a
/// provider any more.
#[tauri::command(async)]
pub fn check_usage_alerts() -> Result<Vec<vm::UsageAlertVm>, String> {
    DaemonApi::connect().usage_alerts(true)
}
