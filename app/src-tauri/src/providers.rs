//! Provider rows: the list the first screen reads, and the writes behind its
//! add / edit / enable / delete actions.
//!
//! Four of the five are the daemon's now — it performs each write and re-reads
//! its own route table, so they carry no reload ping. `update_provider` is the
//! one that is **split**: its write is the daemon's and its view is assembled
//! here, because the assembly reads *this machine* (`migrate.local.md` §5's
//! fifth constraint). `list_providers` is that same assembly with nothing to
//! write, so it is a client command by the same rule.

use tauri::State;

use crate::paths::home_dir;
use crate::state::AppState;
use kiwano_core::vm;

#[tauri::command(async)]
pub fn list_providers(state: State<AppState>) -> Result<Vec<vm::ProviderVm>, String> {
    // **The one screen whose assembly needs a machine fact**, and the fact is
    // read out of the agents' own config files — which is all this side
    // supplies. Which of those agents count needs rows, so the daemon decides it
    // (`migrate.local.md` §5 #2, §10.44) — which is what lets this work when the
    // daemon is on another machine, and what takes `state.store` out of here.
    vm::providers::provider_view(
        &kiwano_core::daemon_api::DaemonApi::connect(),
        &home_dir(),
        state.shell_vars(),
    )
}

#[tauri::command(async)]
pub fn add_provider(input: vm::NewProviderInput) -> Result<vm::ProviderVm, String> {
    // Served by the daemon. **This command mints the id**, which is what makes
    // a retried add idempotent: an id the daemon has already seen is answered
    // with the provider that exists (`migrate.local.md` §6.1). The daemon
    // re-reads its own route table, so the new candidate is picked up at once.
    let id = vm::mint_provider_id(&input.name);
    kiwano_core::daemon_api::DaemonApi::connect().add_provider(&id, &input)
}

#[tauri::command(async)]
pub fn set_provider_enabled(id: String, enabled: bool) -> Result<(), String> {
    // Served by the daemon, which reloads its own route table — a provider that
    // is switched off must stop being a candidate immediately.
    kiwano_core::daemon_api::DaemonApi::connect().set_provider_enabled(&id, enabled)
}

#[tauri::command(async)]
pub fn update_provider(
    state: State<AppState>,
    id: String,
    // Still `input` on the wire: the webview sends `{ id, input }`, and Tauri
    // maps arguments by name. The *type* is what changed — the edit is a patch
    // now — and that is invisible to the frontend, whose JSON was always the
    // whole form.
    input: vm::ProviderPatch,
) -> Result<vm::ProviderVm, String> {
    // The write is the daemon's; the **view** is assembled here, because it is
    // the half that reads this machine (which agents actually route through the
    // gateway) — `migrate.local.md` §5's fifth constraint. The daemon re-reads
    // its own route table, so there is no reload ping to send.
    //
    // A patch now, not a whole input: the edit dialog still sends its whole form
    // (the JSON is the same shape — every field present), and the daemon applies
    // what is named and leaves the rest (`migrate.local.md` §10.38). That is
    // what lets a client *without* a form edit a provider too.
    let api = kiwano_core::daemon_api::DaemonApi::connect();
    api.update_provider(&id, &input)?;
    vm::providers::provider_view(&api, &home_dir(), state.shell_vars())?
        .into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "provider vanished after update".to_string())
}

#[tauri::command(async)]
pub fn delete_provider(id: String) -> Result<bool, String> {
    // Served by the daemon: the promotion of a replacement primary is part of
    // the same operation, and it is the daemon that re-reads its route table.
    kiwano_core::daemon_api::DaemonApi::connect().delete_provider(&id)
}
