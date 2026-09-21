//! Plan quota. The reader itself lives in the gateway crate — the same code
//! enforces the ceiling, so the display and the block cannot disagree — and
//! this is the command wrapper over it.

use tauri::State;

use crate::state::AppState;

// ── Plan quota (the reader itself lives in the gateway crate: the same code
//    enforces the ceiling, so the display and the block cannot disagree) ──

#[tauri::command(async)]
pub async fn get_plan_quota(
    state: State<'_, AppState>,
    provider_id: String,
    force: Option<bool>,
) -> Result<kiwanod::plan_quota::PlanQuotaReport, String> {
    kiwanod::plan_quota::get_plan_quota_report(&state.store, &provider_id, force.unwrap_or(false))
        .await
}
