//! The Dashboard read: usage and spend over a window, optionally narrowed to
//! one provider or one agent.

use tauri::State;

use crate::state::AppState;
use kiwano_core::vm;

#[tauri::command]
pub fn get_dashboard(
    state: State<AppState>,
    window: String,
    provider_id: Option<String>,
    agent: Option<String>,
) -> Result<vm::DashboardVm, String> {
    vm::build_dashboard(
        &state.store,
        &state.aux,
        &window,
        provider_id.as_deref(),
        agent.as_deref(),
    )
}
