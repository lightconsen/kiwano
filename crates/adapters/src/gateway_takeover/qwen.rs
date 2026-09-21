//! Qwen Code: the `modelProviders.openai` entry, the `env` block its key
//! goes in, and the auth-type/model-name pair that selects both.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, PLACEHOLDER_MODEL_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

// ── qwen (~/.qwen/settings.json, JSONC) ──

/// The environment-variable name Qwen Code reads the gateway key from. Qwen
/// resolves credentials through `envKey` (a *name*, never a value), and its
/// own `env` block is a plaintext fallback inside settings.json — writing both
/// is what keeps the key out of the shell environment.
const QWEN_ENV_KEY: &str = "KIWANO_GATEWAY_KEY";

/// Take over Qwen Code's settings.
///
/// Adds a `modelProviders.openai` entry (the key names the protocol), points
/// `env` at the gateway key, and selects it via `security.auth.selectedType`
/// plus `model.name` — Qwen picks the protocol by auth type and the model by
/// name, so both have to move.
pub fn upsert_qwen_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "settings.json")?;
    let mut obj = require_object(root, "settings.json")?;

    let model_id = obj
        .get("model")
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();

    object_field(&mut obj, "settings.json", "modelProviders");
    if !obj["modelProviders"]
        .get("openai")
        .is_some_and(Value::is_array)
    {
        obj["modelProviders"]["openai"] = json!([]);
    }
    let entry = json!({
        "id": model_id,
        "name": GATEWAY_LABEL,
        "envKey": QWEN_ENV_KEY,
        "baseUrl": base_url,
    });
    let providers = obj["modelProviders"]["openai"]
        .as_array_mut()
        .expect("inserted above");
    match providers
        .iter()
        .position(|p| p.get("envKey").and_then(Value::as_str) == Some(QWEN_ENV_KEY))
    {
        Some(i) => providers[i] = entry,
        None => providers.push(entry),
    }

    // The value behind `envKey`. Qwen's own `env` block is the lowest-priority
    // source it reads, which is exactly what makes it the one to write: a shell
    // export still wins, and nothing has to touch the user's shell.
    object_field(&mut obj, "settings.json", "env");
    obj["env"][QWEN_ENV_KEY] = json!(key);

    // Selection: the protocol by auth type, the model by name.
    object_field(
        object_field(&mut obj, "settings.json", "security"),
        "settings.json",
        "security.auth",
    );
    obj["security"]["auth"]["selectedType"] = json!("openai");

    object_field(&mut obj, "settings.json", "model");
    obj["model"]["name"] = json!(model_id);

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qwen_upserts_a_provider_and_selects_it() {
        let original = r#"{
  // The user's own settings, comment included.
  "model": { "name": "qwen3-coder-plus" },
  "security": { "auth": { "selectedType": "qwen-oauth" } },
  "themes": "dark"
}"#;
        let out =
            upsert_qwen_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-qwen-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["themes"], "dark");
        assert_eq!(v["security"]["auth"]["selectedType"], "openai");
        assert_eq!(
            v["model"]["name"], "qwen3-coder-plus",
            "the model name is what goes upstream"
        );
        let p = &v["modelProviders"]["openai"][0];
        assert_eq!(p["id"], "qwen3-coder-plus");
        assert_eq!(p["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(p["envKey"], QWEN_ENV_KEY);
        assert_eq!(
            v["env"][QWEN_ENV_KEY], "kw-ag-qwen-abcd",
            "the key goes in Qwen's own env block, never the user's shell"
        );
    }

    #[test]
    fn qwen_empty_config_creates_the_sections() {
        let out = upsert_qwen_gateway("", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["modelProviders"]["openai"][0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(v["security"]["auth"]["selectedType"], "openai");
        assert_eq!(v["env"][QWEN_ENV_KEY], "k");
    }
}
