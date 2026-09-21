//! OpenCode: the gateway entry (an OpenAI-compatible provider) and the
//! top-level `model` selector that names it.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

// ── opencode (~/.config/opencode/opencode.json, JSONC) ──

/// Upsert the gateway provider (`npm: @ai-sdk/openai-compatible`) and rewrite
/// the top-level `model: "<provider>/<model>"` selector to route through the
/// gateway, keeping the user's model id. A missing or non-slash-form `model`
/// key is left alone (the user picks a gateway model in the OpenCode UI).
pub fn upsert_opencode_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "opencode.json")?;
    let mut obj = require_object(root, "opencode.json")?;

    // provider section: normalize to an object before inserting the entry
    object_field(&mut obj, "opencode.json", "provider");
    let entry = json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": GATEWAY_LABEL,
        "options": { "baseURL": base_url, "apiKey": key },
        "models": {},
    });
    obj["provider"][GATEWAY_PROVIDER_ID] = entry;

    // selection: "<old-provider>/<model>" → "kiwano-gateway/<model>"
    if let Some(model_id) = obj
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/'))
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
    {
        obj["provider"][GATEWAY_PROVIDER_ID]["models"][&model_id] = json!({});
        obj["model"] = json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}"));
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn opencode_upserts_entry_and_rewrites_model_prefix() {
        let original = r#"{
  // user comment survives parse (formatting does not)
  "$schema": "https://opencode.ai/config.json",
  "theme": "dark",
  "model": "deepseek/deepseek-chat",
  "provider": {
    "deepseek": { "npm": "@ai-sdk/openai", "options": { "apiKey": "sk-old" } }
  },
}"#;
        let out =
            upsert_opencode_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-opencode-abcd")
                .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["theme"], "dark");
        assert_eq!(v["model"], "kiwano-gateway/deepseek-chat");
        let entry = &v["provider"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["npm"], "@ai-sdk/openai-compatible");
        assert_eq!(entry["options"]["baseURL"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["options"]["apiKey"], "kw-ag-opencode-abcd");
        assert!(entry["models"]["deepseek-chat"].is_object());
        // pre-existing providers survive (additive semantics)
        assert_eq!(v["provider"]["deepseek"]["options"]["apiKey"], "sk-old");
    }

    #[test]
    fn opencode_without_model_selector_only_adds_entry() {
        let out = upsert_opencode_gateway("{}", "http://127.0.0.1:8317/v1", "kw").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert!(v["provider"][GATEWAY_PROVIDER_ID].is_object());
        assert!(v.get("model").is_none());
    }
}
