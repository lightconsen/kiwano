//! Agent takeover: pointing an agent's own config at the local gateway, and
//! putting it back.
//!
//! **The last command to move, and the one that splits.** §5's first constraint
//! forbids paths in the daemon's interface — an agent's config is the user's own
//! file — so the file half is this side's, always. The store half (the imported
//! provider, the binding, the placeholder key) is the daemon's, and it goes over
//! the wire: the app must not write the shared database itself.
//!
//! §8's ordering — **state, then files, then the applied mark** — is why this
//! calls one function rather than assembling the phases here: the sequence is
//! the part that must not be written twice.

use tauri::State;

use crate::paths::home_dir;
use crate::state::AppState;
use kiwano_core::daemon_api::DaemonApi;
use kiwano_core::vm;

#[tauri::command(async)]
pub fn set_agent_takeover(
    state: State<AppState>,
    agent: String,
    enabled: bool,
) -> Result<(), String> {
    let api = DaemonApi::connect();
    // Where an agent on **this** machine reaches the gateway. Composed from the
    // daemon's own report, so a gateway on another machine — or on a
    // non-standard port — has its agents pointed at the right place
    // (`migrate.local.md` §10.30).
    let gateway = api
        .gateway_base()
        .unwrap_or_else(|_| format!("http://127.0.0.1:{}", state.data_port));
    vm::set_agent_takeover(
        &state.store,
        &state.aux,
        &agent,
        enabled,
        &gateway,
        &home_dir(),
        state.shell_vars(),
        // The store half goes to the daemon, which re-reads its own route table
        // after the import — so this command sends no reload ping. It was the
        // last one that did.
        vm::takeover::StateHalf::Via(&api),
    )
}
