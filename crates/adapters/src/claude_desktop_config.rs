// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/claude_desktop_config.rs
// Copied on 2026-09-08. Modified for Kiwano: only the gateway-profile subset was
// ported (profile constants, build_gateway_profile, model-route safety checks,
// deploymentMode write, _meta.json upsert) and reshaped as pure
// content→content transforms for the takeover pipeline (read → rewrite in
// memory → backup → atomic write, see src-tauri/src/takeover.rs). The
// Database-backed status/CRUD, MCP/prompts management, official-provider
// restore, Windows path probing and snapshot/rollback plumbing were dropped:
// takeover serializes all file access and disable = byte-verbatim restore.

//! Claude Desktop gateway takeover transforms (macOS `Claude-3p` config
//! library).
//!
//! Claude Desktop's third-party mode is driven by two files per install:
//! `claude_desktop_config.json` (whose `deploymentMode: "3p"` flips the app to
//! third-party inference) and a configLibrary profile declaring a gateway
//! provider (`inferenceProvider: "gateway"`) plus the model menu. Takeover
//! writes the gateway profile pointing at the local gateway and registers it
//! in `_meta.json` as the applied profile. Model ids are forwarded verbatim by
//! the gateway, so the profile ships the Claude-safe route ids the Desktop app
//! recognizes as its model menu.

use serde_json::{json, Map, Value};

/// Fixed profile id of the Kiwano gateway profile in the Claude-3p
/// configLibrary (same sentinel uuid shape as cc-switch's).
pub const PROFILE_ID: &str = "00000000-0000-4000-8000-000000157210";

/// Display name of the gateway profile in Claude Desktop's profile picker.
pub const PROFILE_NAME: &str = "Kiwano Gateway";

/// Route-id prefixes Claude Desktop's model menu recognizes (the bare and
/// `anthropic/`-namespaced `claude-` forms).
pub const CLAUDE_ROUTE_PREFIX: &str = "claude-";
pub const ANTHROPIC_CLAUDE_ROUTE_PREFIX: &str = "anthropic/claude-";

/// Claude Code's `[1M]` context marker; the Desktop schema rejects it, so it
/// must never leak into profile entries.
const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// Safe route ids written into the profile's `inferenceModels` menu. These are
/// menu labels only — the gateway forwards them verbatim to the bound
/// Anthropic-family provider. Order matches cc-switch's DEFAULT_PROXY_ROUTES.
const DEFAULT_ROUTE_IDS: &[&str] = &[
    "claude-sonnet-5",
    "claude-opus-5",
    "claude-haiku-4-5",
    "claude-fable-5",
];

/// Whether `model` is a Claude-safe route id Claude Desktop's fail-all
/// validator accepts (`claude-{sonnet,opus,haiku,fable}-<tail>`, optionally
/// `anthropic/`-prefixed). Degenerate tails (e.g. `claude-sonnet-`) are
/// rejected — Claude Desktop 1.12603.1+ fails the whole model group otherwise.
pub fn is_claude_safe_model_id(model: &str) -> bool {
    let normalized = model.trim().to_ascii_lowercase();
    if normalized.contains(ONE_M_CONTEXT_MARKER) {
        return false;
    }

    let Some(route_tail) = normalized
        .strip_prefix(ANTHROPIC_CLAUDE_ROUTE_PREFIX)
        .or_else(|| normalized.strip_prefix(CLAUDE_ROUTE_PREFIX))
    else {
        return false;
    };

    ["sonnet-", "opus-", "haiku-", "fable-"]
        .iter()
        .any(|prefix| {
            route_tail
                .strip_prefix(prefix)
                .is_some_and(|rest| !rest.is_empty())
        })
}

fn parse_jsonc(content: &str, label: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    json5::from_str(content).map_err(|e| format!("{label} is not valid JSON/JSONC: {e}"))
}

fn require_object(v: Value, label: &str) -> Result<Map<String, Value>, String> {
    v.as_object()
        .cloned()
        .ok_or(format!("{label} root must be a JSON object"))
}

fn to_pretty(obj: Map<String, Value>) -> Result<String, String> {
    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// Set `deploymentMode` in a claude_desktop_config.json (missing or non-object
/// files are normalized to `{}`, all other keys survive).
pub fn set_deployment_mode(content: &str, mode: &str) -> Result<String, String> {
    let mut obj = require_object(
        parse_jsonc(content, "claude_desktop_config.json")?,
        "claude_desktop_config.json",
    )?;
    obj.insert("deploymentMode".into(), json!(mode));
    to_pretty(obj)
}

/// Build the full gateway profile JSON: egress allow-all, disabled mode
/// chooser, bearer-auth gateway credentials and the default route-id menu.
/// The profile file is Kiwano-owned while takeover is active, so this is a
/// full-content builder rather than an upsert (disable restores the backup).
pub fn build_gateway_profile(base_url: &str, api_key: &str) -> Result<String, String> {
    let models: Vec<Value> = DEFAULT_ROUTE_IDS
        .iter()
        .map(|id| {
            json!({
                "name": id,
                "supports1m": true,
            })
        })
        .collect();
    let profile = json!({
        "coworkEgressAllowedHosts": ["*"],
        "disableDeploymentModeChooser": true,
        "inferenceGatewayApiKey": api_key,
        "inferenceGatewayAuthScheme": "bearer",
        "inferenceGatewayBaseUrl": base_url,
        "inferenceModels": models,
        "inferenceProvider": "gateway",
    });
    serde_json::to_string_pretty(&profile).map_err(|e| e.to_string())
}

/// Upsert the gateway profile entry into `_meta.json` and mark it applied:
/// `entries` gains `{id: PROFILE_ID, name: PROFILE_NAME}` (replacing any
/// stale copy) and `appliedId` is set to PROFILE_ID. Untouched keys and other
/// entries survive. (The remove/clear branch of cc-switch's write_meta is not
/// ported — Kiwano disables by restoring the original bytes verbatim.)
pub fn upsert_meta(content: &str) -> Result<String, String> {
    let mut obj = require_object(parse_jsonc(content, "_meta.json")?, "_meta.json")?;

    let mut entries = obj
        .get("entries")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    entries.retain(|entry| entry.get("id").and_then(Value::as_str) != Some(PROFILE_ID));
    entries.push(json!({ "id": PROFILE_ID, "name": PROFILE_NAME }));
    obj.insert("entries".into(), Value::Array(entries));
    obj.insert("appliedId".into(), json!(PROFILE_ID));

    to_pretty(obj)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deployment_mode_write_preserves_other_keys() {
        let original = r#"{"deploymentMode":"1p","mcpServers":{"fs":{"command":"npx"}}}"#;
        let out = set_deployment_mode(original, "3p").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["deploymentMode"], "3p");
        assert!(v["mcpServers"]["fs"]["command"] == "npx");
    }

    #[test]
    fn deployment_mode_accepts_empty_and_invalid_root() {
        let out = set_deployment_mode("", "3p").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["deploymentMode"], "3p");
    }

    #[test]
    fn gateway_profile_shape_and_route_ids() {
        let out =
            build_gateway_profile("http://127.0.0.1:8317", "kw-ag-claude-desktop-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["inferenceProvider"], "gateway");
        assert_eq!(v["inferenceGatewayBaseUrl"], "http://127.0.0.1:8317");
        assert_eq!(v["inferenceGatewayApiKey"], "kw-ag-claude-desktop-abcd");
        assert_eq!(v["inferenceGatewayAuthScheme"], "bearer");
        assert_eq!(v["coworkEgressAllowedHosts"], json!(["*"]));
        assert_eq!(v["disableDeploymentModeChooser"], true);
        let models = v["inferenceModels"].as_array().unwrap();
        assert_eq!(models.len(), 4);
        for m in models {
            let name = m["name"].as_str().unwrap();
            assert!(is_claude_safe_model_id(name));
            assert_eq!(m["supports1m"], true);
        }
        assert_eq!(models[0]["name"], "claude-sonnet-5");
    }

    #[test]
    fn safe_model_id_rules() {
        assert!(is_claude_safe_model_id("claude-sonnet-5"));
        assert!(is_claude_safe_model_id("claude-opus-4-8"));
        assert!(is_claude_safe_model_id("ANTHROPIC/claude-haiku-4-5"));
        assert!(is_claude_safe_model_id("  claude-fable-5 "));
        assert!(!is_claude_safe_model_id("claude-sonnet-")); // degenerate tail
        assert!(!is_claude_safe_model_id("claude-sonnet-5[1m]")); // 1M marker
        assert!(!is_claude_safe_model_id("gpt-5"));
        assert!(!is_claude_safe_model_id("claude-")); // no role
        assert!(!is_claude_safe_model_id("claude-mythos-1")); // non-whitelisted role
    }

    #[test]
    fn meta_upsert_replaces_stale_entry_and_marks_applied() {
        let original = r#"{"entries":[{"id":"other-profile","name":"Other"},{"id":"00000000-0000-4000-8000-000000157210","name":"Stale"}],"appliedId":"other-profile","version":3}"#;
        let out = upsert_meta(original).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["appliedId"], PROFILE_ID);
        assert_eq!(v["version"], 3); // untouched key survives
        let entries = v["entries"].as_array().unwrap();
        assert_eq!(entries.len(), 2); // other entry kept, stale copy replaced
        let ours = entries.iter().find(|e| e["id"] == PROFILE_ID).unwrap();
        assert_eq!(ours["name"], PROFILE_NAME);
    }

    #[test]
    fn meta_upsert_creates_entries_from_empty() {
        let out = upsert_meta("").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["appliedId"], PROFILE_ID);
        assert_eq!(v["entries"][0]["id"], PROFILE_ID);
    }
}
