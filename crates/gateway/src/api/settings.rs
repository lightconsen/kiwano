//! The settings blob: read whole, patched in part.
//!
//! Moved here from `kiwano_core::vm::settings` when the daemon took over the
//! read and the write (`migrate.local.md` §10.16). The patch's gateway-facing
//! mirrors (the log config, the compat shim, the DLP mode, the stream timeouts)
//! are what the daemon re-reads on its own route-table reload — which is the
//! whole of the `/reload` ping the app's `update_settings` used to send.
//!
//! **The takeovers and the custom agents are not here.** Those describe *this
//! machine's* agents and their config files, and the client layers them onto the
//! blob the daemon returns (`build_settings_with_home` stays in `kiwano-core`).
//! A patch through the API therefore never names them — which is what the
//! `takeovers` key skip enforces in code rather than in prose.

use crate::store::Store;
use kiwano_api::error::ApiError;
use kiwano_api::settings::SettingsVm;

pub fn ui_settings(store: &Store) -> Result<SettingsVm, ApiError> {
    let mut s: SettingsVm = store
        .settings_json()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();

    // Heal the one value known to be a bygone default. An endpoint the user
    // chose — even a broken one — is left exactly as it is.
    if s.hub_url == "https://hub.kiwano.cc/catalog.json" {
        s.hub_url = kiwano_api::settings::default_hub_url();
    }
    Ok(s)
}

pub fn update_settings(store: &Store, patch: &serde_json::Value) -> Result<(), ApiError> {
    let mut merged = store
        .settings_json()
        .unwrap_or_else(|| serde_json::to_value(SettingsVm::default()).expect("default settings"));
    if let (Some(obj), Some(p)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            // takeovers are managed through set_agent_takeover, not this patch
            if k == "takeovers" {
                continue;
            }
            // preferred currency: normalize + validate the ISO code
            if k == "preferred_currency" {
                let Some(code) = v.as_str() else { continue };
                let code = code.trim().to_ascii_uppercase();
                if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Err(ApiError::invalid(format!("invalid currency code: {code}")));
                }
                obj.insert(k.clone(), serde_json::Value::String(code));
                continue;
            }
            obj.insert(k.clone(), v.clone());
        }
    }
    store
        .save_settings_json(&merged)
        .map_err(ApiError::failed)?;
    // Request-log capture config lives in the shared gateway_settings table:
    // the sidecar reads it at startup and on /reload, so keep both copies in
    // sync whenever the UI patches one of these keys.
    if patch.get("request_logs").is_some()
        || patch.get("log_retention_days").is_some()
        || patch.get("log_max_body_bytes").is_some()
    {
        let mut cfg = store.load_log_config().unwrap_or_default();
        if let Some(v) = patch.get("request_logs").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        // Zero is how the patch says "keep every row": the field is a count of
        // days, and the only count that means *no* retention is none of them.
        // Inside the config the two states stay distinct — `None` is a
        // decision, zero would be a bug — but a JSON patch has one number to
        // say it with, and `0` is the number every other time limit here uses.
        if let Some(v) = patch.get("log_retention_days").and_then(|v| v.as_u64()) {
            if let Ok(days) = u32::try_from(v) {
                cfg.retain_days = (days > 0).then_some(days);
            }
        }
        if let Some(v) = patch.get("log_max_body_bytes").and_then(|v| v.as_u64()) {
            if let Ok(bytes) = usize::try_from(v) {
                cfg.max_body_bytes = (bytes > 0).then_some(bytes);
            }
        }
        store.save_log_config(&cfg).map_err(ApiError::failed)?;
    }
    // The compat shim's switch moves the same way: one gateway_settings key
    // the sidecar reads at startup and on /reload.
    if patch.get("compat_shim").is_some() {
        let mut cfg = store.load_compat_shim_config().unwrap_or_default();
        if let Some(v) = patch.get("compat_shim").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        store
            .save_compat_shim_config(&cfg)
            .map_err(ApiError::failed)?;
    }
    // The DLP mode moves the same way: one gateway_settings key the sidecar
    // reads at startup and on /reload. A mode rather than a flag, so the third
    // answer (block) does not need a new storage shape — and a value this build
    // does not recognize leaves the gateway on the mode it already had rather
    // than guessing at one.
    if patch.get("dlp_mode").is_some() {
        let mut cfg = store.load_dlp_config().unwrap_or_default();
        if let Some(mode) = patch
            .get("dlp_mode")
            .and_then(|v| v.as_str())
            .and_then(crate::store::DlpMode::parse_str)
        {
            cfg.mode = mode;
        }
        store.save_dlp_config(&cfg).map_err(ApiError::failed)?;
    }
    // Same contract for the streaming timeouts: the sidecar reads them at
    // startup and on /reload, so the two copies move together.
    if patch.get("stream_first_byte_secs").is_some() || patch.get("stream_idle_secs").is_some() {
        let mut cfg = store.load_stream_timeouts().unwrap_or_default();
        if let Some(v) = patch.get("stream_first_byte_secs").and_then(|v| v.as_u64()) {
            cfg.first_byte_secs = v;
        }
        if let Some(v) = patch.get("stream_idle_secs").and_then(|v| v.as_u64()) {
            cfg.idle_secs = v;
        }
        store.save_stream_timeouts(&cfg).map_err(ApiError::failed)?;
    }
    Ok(())
}
