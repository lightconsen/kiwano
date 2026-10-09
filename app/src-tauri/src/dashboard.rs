//! The Dashboard read: usage and spend over a window, optionally narrowed to
//! one provider or one agent.

use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

#[tauri::command(async)]
pub fn get_dashboard(
    window: String,
    provider_id: Option<String>,
    agent: Option<String>,
) -> Result<vm::DashboardVm, String> {
    // Served by the daemon: every read this screen makes is its — the usage
    // table, the price cache, the settings blob — and none of them is a fact
    // about this machine (`migrate.local.md` §10.21).
    DaemonApi::connect().dashboard(&window, provider_id.as_deref(), agent.as_deref())
}
