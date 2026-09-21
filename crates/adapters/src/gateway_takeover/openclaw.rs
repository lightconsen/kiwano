//! OpenClaw: `openclaw.json` (the provider entry and
//! `agents.defaults.model.primary`) and the per-agent `models.json`
//! catalogue entry, whose session-affinity flag only that file's
//! validator accepts.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Map, Value};

// ── openclaw (~/.openclaw/openclaw.json, JSONC) ──

/// The model id `agents.defaults.model.primary` selects: the part after the
/// provider prefix, or `None` when the selector is unset or not in
/// `provider/model` form. Shared by both openclaw files, which have to agree
/// on the id for the catalogue declaration below to apply to anything.
fn openclaw_primary_model_id(obj: &Map<String, Value>) -> Option<String> {
    obj.get("agents")?
        .get("defaults")?
        .get("model")?
        .get("primary")?
        .as_str()?
        .split_once('/')
        .map(|(_, id)| id.to_string())
        .filter(|id| !id.is_empty())
}

/// Upsert the gateway provider under `models.providers` and point
/// `agents.defaults.model.primary` at `kiwano-gateway/<model>` (keeping the
/// user's model id; a missing selector is left alone).
pub fn upsert_openclaw_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "openclaw.json")?;
    let mut obj = require_object(root, "openclaw.json")?;

    // Both levels ensured separately: `object_field` takes a field name, not a
    // path — nesting the calls used to insert a literal `"models.providers"`
    // key inside `models`, a field OpenClaw's schema does not know.
    let models = object_field(&mut obj, "openclaw.json", "models");
    object_field(models, "openclaw.json", "providers");
    let entry = json!({
        "baseUrl": base_url,
        "apiKey": key,
        "api": "openai-completions",
        "models": [],
    });
    obj["models"]["providers"][GATEWAY_PROVIDER_ID] = entry;

    if let Some(model_id) = openclaw_primary_model_id(&obj) {
        obj["agents"]["defaults"]["model"]["primary"] =
            json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}"));
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// Upsert the gateway provider into OpenClaw's custom model catalogue (the
/// `models.json` under its per-agent runtime directory — which one that is,
/// is `takeover_paths`'s business, not this writer's), declaring the selected
/// model with `compat.sendSessionAffinityHeaders`. That flag is what makes OpenClaw put
/// its session id on the wire (`session_id`, `x-client-request-id`,
/// `x-session-affinity`) on the chat-completions route this takeover writes;
/// without it the session id never leaves the agent and the gateway's
/// `session_hint` has nothing to read.
///
/// The flag lives here rather than in `openclaw.json` because that file's
/// schema rejects it: OpenClaw validates its main config against a strict
/// compat schema that predates the flag, and refuses to start on a config it
/// cannot validate (verified against 2026.6.9). This file's validator accepts
/// extra compat keys and merges them into the resolved model.
///
/// The model is declared in full because the catalogue's own validator
/// requires `baseUrl`/`apiKey`/`api` once models are listed, and because its
/// loader would otherwise substitute defaults (a 128k context window, a
/// 16k output cap) that silently shrink OpenClaw's budgeting: the numbers
/// written here are OpenClaw's own fallbacks for an undeclared model (200k
/// each, zero cost), so declaring it changes nothing but the affinity flag.
///
/// The id comes from `agents.defaults.model.primary` in `openclaw_json` (the
/// sibling main config — before or after its own rewrite, both keep the id).
/// With no selector to pin the declaration to, the placeholder id is written:
/// the same honest stand-in the model-list agents get, which the user
/// replaces by selecting a model and re-running the takeover.
pub fn upsert_openclaw_models_json(
    content: &str,
    openclaw_json: &str,
    base_url: &str,
    key: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "models.json")?;
    let mut obj = require_object(root, "models.json")?;

    let model_id = parse_jsonc(openclaw_json, "openclaw.json")
        .ok()
        .and_then(|cfg| require_object(cfg, "openclaw.json").ok())
        .and_then(|cfg| openclaw_primary_model_id(&cfg))
        .unwrap_or_else(|| PLACEHOLDER_MODEL_ID.to_string());

    object_field(&mut obj, "models.json", "providers");
    obj["providers"][GATEWAY_PROVIDER_ID] = json!({
        "name": GATEWAY_LABEL,
        "baseUrl": base_url,
        "apiKey": key,
        "api": "openai-completions",
        "models": [{
            "id": model_id,
            "name": model_id,
            "contextWindow": 200_000,
            "maxTokens": 200_000,
            "compat": { "sendSessionAffinityHeaders": true },
        }],
    });

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn openclaw_upserts_provider_and_selection() {
        let original = r#"{
  models: { mode: 'merge', providers: { openrouter: { baseUrl: 'https://openrouter.ai/api/v1' } } },
  agents: { defaults: { model: { primary: 'openrouter/anthropic/claude-sonnet-4.6' } } },
}"#;
        let out = upsert_openclaw_gateway(original, "http://127.0.0.1:8317", "kw-ag-openclaw-abcd")
            .unwrap();
        let v: Value = json5::from_str::<Value>(&out).unwrap();
        let entry = &v["models"]["providers"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["baseUrl"], "http://127.0.0.1:8317");
        assert_eq!(entry["apiKey"], "kw-ag-openclaw-abcd");
        assert_eq!(entry["api"], "openai-completions");
        assert_eq!(
            v["agents"]["defaults"]["model"]["primary"],
            "kiwano-gateway/anthropic/claude-sonnet-4.6"
        );
        // other providers untouched
        assert!(v["models"]["providers"]["openrouter"].is_object());
        // No literal "models.providers" key: object_field takes a field name,
        // not a path, and OpenClaw's schema does not know such a field.
        assert!(v["models"].get("models.providers").is_none());
    }

    #[test]
    fn openclaw_models_json_declares_the_selected_model() {
        let config = r#"{
  agents: { defaults: { model: { primary: 'kiwano-gateway/deepseek-chat' } } },
}"#;
        let existing = r#"{
  "providers": {
    "my-own": { "baseUrl": "https://example.com/v1", "apiKey": "sk-x", "api": "openai-completions", "models": [] }
  }
}"#;
        let out = upsert_openclaw_models_json(
            existing,
            config,
            "http://127.0.0.1:8317",
            "kw-ag-openclaw-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let entry = &v["providers"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["baseUrl"], "http://127.0.0.1:8317");
        assert_eq!(entry["apiKey"], "kw-ag-openclaw-abcd");
        assert_eq!(entry["api"], "openai-completions");
        let model = &entry["models"][0];
        assert_eq!(model["id"], "deepseek-chat");
        assert_eq!(model["compat"]["sendSessionAffinityHeaders"], true);
        assert_eq!(model["contextWindow"], 200_000);
        assert_eq!(model["maxTokens"], 200_000);
        // The user's own catalogue entry survives.
        assert!(v["providers"]["my-own"].is_object());

        // A re-run replaces our entry instead of stacking a second model.
        let again =
            upsert_openclaw_models_json(&out, config, "http://127.0.0.1:8317", "kw-2").unwrap();
        let v2: Value = serde_json::from_str(&again).unwrap();
        assert_eq!(
            v2["providers"][GATEWAY_PROVIDER_ID]["models"]
                .as_array()
                .unwrap()
                .len(),
            1
        );
        assert_eq!(v2["providers"][GATEWAY_PROVIDER_ID]["apiKey"], "kw-2");

        // No selector to pin the declaration to: the placeholder stands in.
        let fresh = upsert_openclaw_models_json("", "", "http://127.0.0.1:8317", "kw-3").unwrap();
        let v3: Value = serde_json::from_str(&fresh).unwrap();
        assert_eq!(
            v3["providers"][GATEWAY_PROVIDER_ID]["models"][0]["id"],
            "kiwano"
        );
    }
}
