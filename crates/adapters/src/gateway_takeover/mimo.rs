//! MiMo Code: the gateway entry (an OpenAI-compatible custom provider) and the
//! top-level `model: "custom/<model>"` selector that names it.
//!
//! MiMo's schema pins the provider id to `custom` (the doc's own quick-start
//! names it that), so — unlike the other additive agents — ours must land under
//! `provider.custom` rather than `kiwano-gateway`. The user's own `custom`
//! entry, if they had one, is what a takeover replaces; restore puts their
//! bytes back either way.

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use crate::gateway_takeover::readers::CurrentProvider;
use serde_json::{json, Value};

/// The provider id MiMo's `model` selector expects for an OpenAI-compatible
/// custom provider (its own docs: `"model": "custom/MODEL_NAME"`).
const MIMO_CUSTOM_PROVIDER: &str = "custom";

// ── mimo (~/.config/mimocode/mimocode.jsonc, JSONC) ──

/// Upsert the gateway provider (`npm: @ai-sdk/openai-compatible`) and rewrite
/// the top-level `model: "custom/<model>"` selector to route through the
/// gateway, keeping the user's model id. A missing or non-`custom/` `model`
/// key leaves the selector alone (the user picks a gateway model in the MiMo
/// UI); the provider entry is added either way.
pub fn upsert_mimo_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "mimocode.jsonc")?;
    let mut obj = require_object(root, "mimocode.jsonc")?;

    // provider section: normalize to an object before inserting the entry
    object_field(&mut obj, "mimocode.jsonc", "provider");
    let entry = json!({
        "name": "Kiwano Gateway",
        "npm": "@ai-sdk/openai-compatible",
        "only_configured_models": true,
        "models": {},
        "options": { "baseURL": base_url, "apiKey": key },
    });
    obj["provider"][MIMO_CUSTOM_PROVIDER] = entry;

    // selection: "custom/<model>" → "custom/<model>" against the gateway, the
    // model id carried upstream verbatim. Anything else — the agent's own
    // cloud models or an empty selector — keeps the user's value.
    if let Some(model_id) = obj
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/'))
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
    {
        obj["provider"][MIMO_CUSTOM_PROVIDER]["models"][&model_id] = json!({
            "name": model_id.as_str(),
        });
        obj["model"] = json!(format!("{MIMO_CUSTOM_PROVIDER}/{model_id}"));
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// What MiMo routes to *today* — the `custom` provider the `model` selector
/// names — for the first-takeover import. Returns None when unset, non-slash,
/// or already the gateway (a takeover that ran once must not re-import our
/// own loopback endpoint).
pub fn read_mimo_current(content: &str) -> Option<CurrentProvider> {
    let obj = parse_jsonc(content, "mimocode.jsonc").ok()?;
    let obj = require_object(obj, "mimocode.jsonc").ok()?;
    let id = obj.get("model")?.as_str()?.split_once('/')?.0.to_string();
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let entry = obj.get("provider")?.get(&id)?;
    let base_url = entry.get("options")?.get("baseURL")?.as_str()?.to_string();
    let api_key = entry.get("options")?.get("apiKey")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: id,
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mimo_upserts_the_custom_provider_and_rewrites_the_model_selector() {
        let original = r#"{
  // comment survives the parse (the write-back re-serializes)
  "$schema": "https://mimo.xiaomi.com/mimocode/config.json",
  "model": "custom/deepseek-chat",
  "provider": {
    "custom": {
      "name": "Custom",
      "npm": "@ai-sdk/openai-compatible",
      "only_configured_models": true,
      "models": { "deepseek-chat": { "name": "deepseek-chat" } },
      "options": { "baseURL": "https://example.com/v1", "apiKey": "sk-old" }
    }
  },
}"#;
        let out =
            upsert_mimo_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-mimo-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "custom/deepseek-chat");
        let entry = &v["provider"][MIMO_CUSTOM_PROVIDER];
        assert_eq!(entry["options"]["baseURL"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["options"]["apiKey"], "kw-ag-mimo-abcd");
        assert_eq!(entry["models"]["deepseek-chat"]["name"], "deepseek-chat");
        assert_eq!(entry["only_configured_models"], true);
    }

    #[test]
    fn mimo_any_slash_model_selector_is_routed_through_the_gateway() {
        // A model the user picked off MiMo's own cloud list is still a model
        // the gateway forwards verbatim — same rule as opencode's
        // `<provider>/<model>` rewrite: any slash-form selector routes, the id
        // travels upstream unchanged.
        let out = upsert_mimo_gateway(
            r#"{"model":"xiaomi/deepseek-v4","provider":{}}"#,
            "http://127.0.0.1:8317/v1",
            "kw-ag-mimo-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "custom/deepseek-v4");
        assert_eq!(
            v["provider"][MIMO_CUSTOM_PROVIDER]["models"]["deepseek-v4"]["name"],
            "deepseek-v4"
        );
    }

    #[test]
    fn mimo_bare_model_name_is_left_alone() {
        // No provider prefix: not a choice the gateway can inherit — the user
        // picks a gateway model in the MiMo UI instead. The entry still lands.
        let out = upsert_mimo_gateway(
            r#"{"model":"deepseek-v4","provider":{}}"#,
            "http://127.0.0.1:8317/v1",
            "kw-ag-mimo-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "deepseek-v4", "not ours to change");
        assert!(
            v["provider"][MIMO_CUSTOM_PROVIDER].is_object(),
            "entry is still added"
        );
    }

    #[test]
    fn mimo_empty_config_creates_the_entry_without_a_selector() {
        let out = upsert_mimo_gateway("", "http://127.0.0.1:8317/v1", "kw-ag-mimo-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["provider"][MIMO_CUSTOM_PROVIDER]["options"]["apiKey"],
            "kw-ag-mimo-abcd"
        );
        assert!(v.get("model").is_none(), "nothing to select from");
    }
}
