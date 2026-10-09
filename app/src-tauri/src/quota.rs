//! Plan quota. The reader itself lives in the gateway crate — the same code
//! enforces the ceiling, so the display and the block cannot disagree — and
//! this is the command wrapper over it.

// ── Plan quota (the reader itself lives in the gateway crate: the same code
//    enforces the ceiling, so the display and the block cannot disagree) ──

#[tauri::command]
pub fn get_plan_quota(
    provider_id: String,
    force: Option<bool>,
) -> Result<kiwanod::plan_quota::PlanQuotaReport, String> {
    // Served by the daemon: the same reader enforces the ceiling on the data
    // plane, so the display and the block cannot disagree.
    kiwano_core::daemon_api::DaemonApi::connect()
        .get_plan_quota(&provider_id, force.unwrap_or(false))
}
