//! The session list: the gateway's traffic and the agents' own files, merged.
//!
//! The merge is the daemon's (`kiwanod::api::sessions`) — this is the transport,
//! like every other command in this directory. Two clients read it: `kiwano
//! sessions` and the app's Dashboard, which shows it as the second tab of the
//! request-log card.

use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

/// Sessions, newest first, optionally narrowed by agent, project and window.
///
/// The window has two spellings and the caller picks one: `days` is the rolling
/// span a command line means by "the last week", `since` is an instant this side
/// computed from the dashboard's **local-day** window. The app sends `since`,
/// because the figures above the card are measured in local days and a list two
/// panels below it must not be measured in something else; the CLI sends `days`.
#[tauri::command(async)]
pub fn list_sessions(
    agent: Option<String>,
    project: Option<String>,
    days: Option<i64>,
    since: Option<String>,
    page: i64,
    page_size: i64,
) -> Result<vm::SessionListVm, String> {
    DaemonApi::connect().sessions(
        agent.as_deref(),
        project.as_deref(),
        days,
        since.as_deref(),
        page,
        page_size,
    )
}

/// One session by id, or `null` when neither ledger has it.
///
/// The detail page's read: it replaces the component that held the row, so it
/// cannot be handed one — and a refresh or a pasted link has to render it too.
#[tauri::command(async)]
pub fn get_session(id: String) -> Result<Option<vm::SessionVm>, String> {
    DaemonApi::connect().session(&id)
}
