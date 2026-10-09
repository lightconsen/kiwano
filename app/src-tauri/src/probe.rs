//! Endpoint probes: latency, protocol-aware reachability, one prompt round trip
//! against a stored provider, and the model-name list the Default model picker
//! fills from.
//!
//! Three of the four are the daemon's now (`migrate.local.md` §10.15): they are
//! network I/O, and the health verdict the provider test writes is the Status
//! column's, which the daemon owns. `test_latency` stays a plain function — a
//! bare TCP connect needs no daemon, and the CLI calls it too.

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
    protocol: String,
    endpoint: String,
    api_key: Option<String>,
    provider_id: Option<String>,
) -> Result<sidecar::ProbeReport, String> {
    // Served by the daemon: the health verdict it writes is the Status column's,
    // which the daemon owns.
    kiwano_core::daemon_api::DaemonApi::connect()
        .probe(
            &protocol,
            &endpoint,
            api_key.as_deref(),
            provider_id.as_deref(),
            "endpoint",
        )
        .await
        .and_then(|v| serde_json::from_value(v).map_err(|e| e.to_string()))
}

/// One prompt round trip against a provider, for the Apps screen's Test button.
///
/// The provider is tested as it is stored: its own endpoint, its own key, and —
/// when it has no default model — the model its catalog entry prices. Async:
/// the round trip is an HTTP call, and a blocking client panics on the runtime.
#[tauri::command]
pub async fn test_provider_latency(id: String) -> Result<vm::PromptLatencyVm, String> {
    // Served by the daemon: the verdict it writes is the health row the Status
    // column reads, which the daemon owns.
    kiwano_core::daemon_api::DaemonApi::connect()
        .test_provider_latency(&id)
        .await
}

/// Live model-name list for the Default model picker (requires an API key:
/// cloud providers reject anonymous /models calls). Same blank-key rule as the
/// probe: a typed key wins, and the stored one stands in for an edit.
#[tauri::command]
pub async fn list_models(
    protocol: String,
    endpoint: String,
    api_key: String,
    provider_id: Option<String>,
) -> Result<Vec<String>, String> {
    // Served by the daemon. A blank key is filled in with the stored one — for
    // that provider's own endpoints only, on the daemon's side now.
    let value = kiwano_core::daemon_api::DaemonApi::connect()
        .probe(
            &protocol,
            &endpoint,
            Some(api_key.as_str()).filter(|k| !k.trim().is_empty()),
            provider_id.as_deref(),
            "models",
        )
        .await?;
    serde_json::from_value(value).map_err(|e| e.to_string())
}
