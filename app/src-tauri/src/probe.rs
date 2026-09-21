//! Endpoint probes: latency, protocol-aware reachability, one prompt round trip
//! against a stored provider, and the model-name list the Default model picker
//! fills from. These are diagnostics — none writes, none reloads the daemon.

use tauri::State;

use crate::state::AppState;
use kiwano_core::{sidecar, vm};

#[tauri::command(async)]
pub fn test_latency(endpoint: String) -> Result<u64, String> {
    sidecar::measure_latency(&endpoint)
}

/// Protocol-aware probe: GET the protocol's models route (auth headers only
/// when a key is given) and classify the answer. A missing key is fine —
/// 401/403 still proves the protocol route exists. Async: the probe uses an
/// async HTTP client (a blocking one panics when dropped on the runtime).
///
/// `provider_id` is how the edit dialog's blank key gets filled in: that form
/// never holds the stored credential, so a Test on a provider whose saved
/// configuration is what you are checking would otherwise probe anonymously and
/// report `auth`. Only that provider's own endpoints qualify — see
/// `vm::stored_key_for`.
#[tauri::command]
pub async fn test_endpoint(
    state: State<'_, AppState>,
    protocol: String,
    endpoint: String,
    api_key: Option<String>,
    provider_id: Option<String>,
) -> Result<sidecar::ProbeReport, String> {
    let key = api_key
        .filter(|k| !k.trim().is_empty())
        .or_else(|| vm::stored_key_for(&state.store, provider_id.as_deref()?, &endpoint));
    sidecar::probe_endpoint(&protocol, &endpoint, key.as_deref()).await
}

/// One prompt round trip against a provider, for the Apps screen's Test button.
///
/// The provider is tested as it is stored: its own endpoint, its own key, and —
/// when it has no default model — the model its catalog entry prices. Async:
/// the round trip is an HTTP call, and a blocking client panics on the runtime.
#[tauri::command]
pub async fn test_provider_latency(
    state: State<'_, AppState>,
    id: String,
) -> Result<vm::PromptLatencyVm, String> {
    vm::test_provider_latency(&state.store, &state.aux, &id).await
}

/// Live model-name list for the Default model picker (requires an API key:
/// cloud providers reject anonymous /models calls). Same blank-key rule as the
/// probe: a typed key wins, and the stored one stands in for an edit.
#[tauri::command]
pub async fn list_models(
    state: State<'_, AppState>,
    protocol: String,
    endpoint: String,
    api_key: String,
    provider_id: Option<String>,
) -> Result<Vec<String>, String> {
    let key = if api_key.trim().is_empty() {
        provider_id
            .as_deref()
            .and_then(|id| vm::stored_key_for(&state.store, id, &endpoint))
            .ok_or_else(|| {
                format!("no stored key covers {endpoint} — enter one to fetch its models")
            })?
    } else {
        api_key
    };
    sidecar::fetch_model_names(&protocol, &endpoint, &key).await
}
