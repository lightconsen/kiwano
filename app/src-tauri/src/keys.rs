//! Multi-key rotation: the extra keys a provider can be called with.
//!
//! The gateway's key pool is part of a provider's route, so a write has to be
//! picked up by the process that will use it — which the daemon now does for
//! itself (`migrate.local.md` §7 batch 1). This module no longer sends a reload
//! ping, and no longer takes `State<AppState>`.

use kiwano_core::vm;

// ── Multi-key rotation (spec §4.1 P1) ──

/// A provider's rotation keys (the ones besides the primary key).
#[tauri::command(async)]
pub fn list_api_keys(provider_id: String) -> Result<Vec<vm::ApiKeyVm>, String> {
    // Served by the daemon (`migrate.local.md` §7 batch 1): no `AppState`, no
    // store — the signature lost its state parameter because the command no
    // longer has any. The return type is unchanged (`vm::ApiKeyVm` is
    // `kiwano_api::keys::ApiKeyVm`, re-exported), and so is what the frontend
    // sends: `state` was injected by Tauri, never passed by the webview.
    kiwano_core::daemon_api::DaemonApi::connect().list_api_keys(&provider_id)
}

/// Append a rotation key; triggers /reload so the gateway key pool picks it up immediately.
#[tauri::command(async)]
pub fn add_api_key(
    provider_id: String,
    api_key: String,
    label: Option<String>,
) -> Result<vm::ApiKeyVm, String> {
    // Served by the daemon, which also owns the idempotency (§6.1): a retry
    // answers with the key that is already there.
    // The daemon re-reads its own route table — the key pool is part of what a
    // provider's route carries, so the second key is picked up immediately.
    kiwano_core::daemon_api::DaemonApi::connect().add_api_key(
        &provider_id,
        &api_key,
        label.as_deref(),
    )
}

#[tauri::command(async)]
pub fn delete_api_key(id: i64) -> Result<bool, String> {
    kiwano_core::daemon_api::DaemonApi::connect().delete_api_key(id)
}
