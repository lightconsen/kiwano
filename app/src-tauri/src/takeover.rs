//! Agent takeover: pointing an agent's own config at the local gateway, and
//! putting it back. The write path is `vm::set_agent_takeover` — this is the
//! command over it, plus the reload a new binding implies.

use tauri::State;

use crate::paths::home_dir;
use crate::state::{after_mutation, AppState};
use kiwano_core::vm;

#[tauri::command]
pub fn set_agent_takeover(
    state: State<AppState>,
    agent: String,
    enabled: bool,
) -> Result<(), String> {
    let data_port = state.data_port;
    vm::set_agent_takeover(
        &state.store,
        &state.aux,
        &agent,
        enabled,
        data_port,
        &home_dir(),
        state.shell_vars(),
    )?;
    // enabling may import a provider and bind it → the route table changed
    after_mutation(&state);
    Ok(())
}
