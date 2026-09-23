//! Droid (Factory): the `customModels` array in `~/.factory/settings.json`,
//! plus the top-level `model` that names the default. Custom entries carry
//! their own `baseUrl`/`apiKey` and speak the OpenAI Chat Completions shape
//! (`provider: "generic-chat-completion-api"`).

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, PLACEHOLDER_MODEL_ID};
use serde_json::Value;

// ── droid (~/.factory/settings.json, fixed location) ──

/// Upsert the gateway model into `customModels` and make it the default
/// `model`. Organization policy that forbids custom models is refused rather
/// than written around — Droid would reject the model and the takeover would
/// be a silent no-op. The rest of the document keeps its fields though not its
/// layout (serde_json re-serializes); restore puts the user's bytes back.
pub fn upsert_droid_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if content.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(content)
            .map_err(|e| format!("settings.json is not valid JSON: {e}"))?
    };
    let obj = v
        .as_object_mut()
        .ok_or("settings.json root must be a JSON object")?;

    // The org-level kill switch is the honest place to stop: with custom
    // models disallowed, Droid hides our entry and the takeover is a lie.
    if obj.get("allowCustomModels") == Some(&Value::Bool(false)) {
        return Err(
            "settings.json sets allowCustomModels to false — this machine's org policy forbids \
             custom models, so Droid would refuse the gateway entry"
                .into(),
        );
    }

    let model_id = obj
        .get("model")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != GATEWAY_LABEL)
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();

    let models = obj
        .entry("customModels".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let models = models
        .as_array_mut()
        .ok_or("settings.json customModels must be an array")?;
    models.retain(|m| m.get("displayName").and_then(|n| n.as_str()) != Some(GATEWAY_LABEL));
    models.insert(
        0,
        serde_json::json!({
            "model": model_id,
            "displayName": GATEWAY_LABEL,
            "baseUrl": base_url,
            "apiKey": key,
            "provider": "generic-chat-completion-api",
            "maxOutputTokens": 16384
        }),
    );

    obj.insert("model".into(), Value::String(model_id));

    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// What Droid routes to *today*: the `model` setting, resolved against
/// `customModels` when it names a custom entry (an official model id has no
/// endpoint to import). An `apiKey` written as a `${VAR}` reference cannot be
/// resolved offline, so it extracts nothing.
pub fn read_droid_current(
    content: &str,
) -> Option<crate::gateway_takeover::readers::CurrentProvider> {
    let v: Value = serde_json::from_str(content).ok()?;
    let model_id = v.get("model")?.as_str()?;
    let entry = v
        .get("customModels")?
        .as_array()?
        .iter()
        .find(|m| m.get("model").and_then(|x| x.as_str()) == Some(model_id))?;
    let base_url = entry.get("baseUrl")?.as_str()?.trim().to_string();
    let api_key = entry.get("apiKey")?.as_str()?.trim().to_string();
    if base_url.is_empty() || api_key.is_empty() || api_key.starts_with("${") {
        return None;
    }
    let name = entry
        .get("displayName")
        .and_then(|n| n.as_str())
        .unwrap_or_default()
        .trim()
        .to_string();
    Some(crate::gateway_takeover::readers::CurrentProvider {
        name,
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn droid_upserts_the_custom_model_and_sets_the_default() {
        let original = r#"{
  "model": "claude-sonnet-4-6",
  "reasoningEffort": "high",
  "customModels": [
    { "model": "deepseek-chat", "displayName": "My DeepSeek",
      "baseUrl": "https://api.deepseek.com/v1", "apiKey": "sk-ds",
      "provider": "generic-chat-completion-api" }
  ]
}"#;
        let out =
            upsert_droid_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-droid-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "claude-sonnet-4-6", "the default id is kept");
        let models = v["customModels"].as_array().unwrap();
        assert_eq!(models[0]["displayName"], "Kiwano Gateway");
        assert_eq!(models[0]["model"], "claude-sonnet-4-6");
        assert_eq!(models[0]["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(models[0]["apiKey"], "kw-ag-droid-abcd");
        assert_eq!(models[0]["provider"], "generic-chat-completion-api");
        // The user's own custom model survives behind ours.
        assert_eq!(models[1]["displayName"], "My DeepSeek");
        assert_eq!(v["reasoningEffort"], "high"); // untouched settings survive
    }

    #[test]
    fn droid_replaces_our_model_on_a_rerun() {
        let taken = r#"{
  "model": "Kiwano Gateway",
  "customModels": [
    { "model": "kiwano", "displayName": "Kiwano Gateway",
      "baseUrl": "http://127.0.0.1:9999/v1", "apiKey": "kw-ag-droid-old",
      "provider": "generic-chat-completion-api" }
  ]
}"#;
        let out =
            upsert_droid_gateway(taken, "http://127.0.0.1:8317/v1", "kw-ag-droid-new").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let models = v["customModels"].as_array().unwrap();
        assert_eq!(models.len(), 1, "no stacked gateway entries");
        assert_eq!(models[0]["apiKey"], "kw-ag-droid-new");
        // Our own placeholder is not "the user's model": a fresh one is picked.
        assert_eq!(models[0]["model"], "kiwano");
    }

    #[test]
    fn droid_org_policy_forbidding_custom_models_is_refused() {
        let err = upsert_droid_gateway(
            r#"{"allowCustomModels": false, "model": "claude-sonnet-4-6"}"#,
            "http://127.0.0.1:8317/v1",
            "k",
        )
        .unwrap_err();
        assert!(err.contains("org policy"), "{err}");
    }

    #[test]
    fn droid_reader_resolves_the_default_model_entry() {
        let config = r#"{
  "model": "deepseek-chat",
  "customModels": [
    { "model": "deepseek-chat", "displayName": "My DeepSeek",
      "baseUrl": "https://api.deepseek.com/v1", "apiKey": "sk-ds",
      "provider": "generic-chat-completion-api" }
  ]
}"#;
        let p = read_droid_current(config).unwrap();
        assert_eq!(p.name, "My DeepSeek");
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.api_key, "sk-ds");

        // An official default (not in customModels) extracts nothing.
        assert!(read_droid_current(r#"{"model": "claude-sonnet-4-6"}"#).is_none());
        // A ${VAR} key reference cannot be resolved offline.
        let config = r#"{
  "model": "m",
  "customModels": [{ "model": "m", "baseUrl": "https://x/v1", "apiKey": "${MY_KEY}",
                     "provider": "generic-chat-completion-api" }]
}"#;
        assert!(read_droid_current(config).is_none());
    }
}
