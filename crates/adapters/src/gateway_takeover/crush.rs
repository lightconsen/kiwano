//! Crush: the `providers` table plus the `models.large` slot that selects the
//! primary coding model (`crush model large` persists there; `small` is the
//! summarization slot and stays whatever the user chose).

use serde_json::Value;

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;

// ── crush (XDG crush.json, CRUSH_GLOBAL_CONFIG not honored by the caller) ──

/// The model id a fresh config gets when nothing selectable exists yet — the
/// gateway forwards model names verbatim, so the user replaces it with a model
/// their provider serves.
const PLACEHOLDER_MODEL_ID: &str = "kiwano";

/// A `models` entry the schema requires in full: catwalk's Model type pins ten
/// required fields, and a short one makes Crush refuse the whole config. Costs
/// are zero — the gateway meters separately — and the window is a placeholder
/// the model row's real entry would override.
fn gateway_model_entry(id: &str) -> Value {
    serde_json::json!({
        "id": id,
        "name": id,
        "cost_per_1m_in": 0,
        "cost_per_1m_out": 0,
        "cost_per_1m_in_cached": 0,
        "cost_per_1m_out_cached": 0,
        "context_window": 200000,
        "default_max_tokens": 8192,
        "can_reason": true,
        "supports_attachments": false
    })
}

/// The id the large slot selects today — the model whose requests the takeover
/// is rerouting. A `kiwano-gateway` selection (a re-run) passes through
/// unchanged, which is the same id a re-run wants.
fn selected_model_id(root: &Value) -> String {
    root.get("models")
        .and_then(|m| m.get("large"))
        .and_then(|l| l.get("model"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string()
}

/// Upsert the gateway provider and point the large slot at it. The user's own
/// providers and the `small` slot survive; the rest of the document (mcp, lsp,
/// options) keeps its fields though not its layout (serde_json re-serializes;
/// restore puts the user's bytes back the way they were).
pub fn upsert_crush_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if content.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(content).map_err(|e| format!("crush.json is not valid JSON: {e}"))?
    };
    let model_id = selected_model_id(&v);
    let obj = v
        .as_object_mut()
        .ok_or("crush.json root must be a JSON object")?;

    let providers = obj
        .entry("providers".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let providers = providers
        .as_object_mut()
        .ok_or("crush.json providers must be an object")?;
    providers.insert(
        GATEWAY_PROVIDER_ID.to_string(),
        serde_json::json!({
            "name": "Kiwano Gateway",
            "type": "openai-compat",
            "base_url": base_url,
            "api_key": key,
            "models": [gateway_model_entry(&model_id)]
        }),
    );

    let slots = obj
        .entry("models".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let slots = slots
        .as_object_mut()
        .ok_or("crush.json models must be an object")?;
    slots.insert(
        "large".to_string(),
        serde_json::json!({"provider": GATEWAY_PROVIDER_ID, "model": model_id}),
    );

    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// What Crush routes to *today*: the large slot's provider, resolved to its
/// base_url and key. An `api_key` that is a `$VAR` reference cannot be
/// resolved offline, so it extracts nothing. The provider's id is a real name
/// here (Crush's own providers carry ids like `deepseek`).
pub fn read_crush_current(
    content: &str,
) -> Option<crate::gateway_takeover::readers::CurrentProvider> {
    let v: Value = serde_json::from_str(content).ok()?;
    let large = v.get("models")?.get("large")?;
    let provider_id = large.get("provider")?.as_str()?.trim().to_string();
    if provider_id.is_empty() {
        return None;
    }
    let provider = v.get("providers")?.get(&provider_id)?;
    let base_url = provider.get("base_url")?.as_str()?.trim().to_string();
    let api_key = provider.get("api_key")?.as_str()?.trim().to_string();
    if base_url.is_empty() || api_key.is_empty() || api_key.starts_with('$') {
        return None;
    }
    let name = provider
        .get("name")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(&provider_id)
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
    fn crush_upserts_the_provider_and_points_the_large_slot_at_it() {
        let original = r#"{
  "$schema": "https://charm.land/crush.json",
  "models": { "large": { "provider": "deepseek", "model": "deepseek-chat" } },
  "providers": {
    "deepseek": { "type": "openai-compat", "base_url": "https://api.deepseek.com/v1",
                  "api_key": "$DEEPSEEK_API_KEY", "models": [] }
  },
  "mcp": {}
}"#;
        let out =
            upsert_crush_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-crush-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let large = &v["models"]["large"];
        assert_eq!(large["provider"], "kiwano-gateway");
        assert_eq!(large["model"], "deepseek-chat", "the selected id is kept");
        // small never existed here; when it does, it must survive (asserted below).
        let gw = &v["providers"]["kiwano-gateway"];
        assert_eq!(gw["type"], "openai-compat");
        assert_eq!(gw["base_url"], "http://127.0.0.1:8317/v1");
        assert_eq!(gw["api_key"], "kw-ag-crush-abcd");
        assert_eq!(gw["models"][0]["id"], "deepseek-chat");
        // schema-required fields on the model entry: a short one makes Crush
        // refuse the whole config
        assert_eq!(gw["models"][0]["context_window"], 200000);
        assert_eq!(gw["models"][0]["can_reason"], true);
        // The user's own provider and unrelated sections survive.
        assert!(v["providers"]["deepseek"].is_object());
        assert_eq!(v["mcp"], serde_json::json!({}));
        assert_eq!(v["$schema"], "https://charm.land/crush.json");
    }

    #[test]
    fn crush_keeps_the_small_slot_and_replaces_ours_on_a_rerun() {
        let original = r#"{
  "models": {
    "large": { "provider": "kiwano-gateway", "model": "kiwano" },
    "small": { "provider": "deepseek", "model": "deepseek-chat" }
  },
  "providers": { "kiwano-gateway": { "type": "openai-compat",
      "base_url": "http://127.0.0.1:9999/v1", "api_key": "kw-ag-crush-old", "models": [] } }
}"#;
        let out =
            upsert_crush_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-crush-new").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(
            v["models"]["small"]["provider"], "deepseek",
            "small untouched"
        );
        assert_eq!(
            v["models"]["large"]["model"], "kiwano",
            "the id a re-run wants"
        );
        assert_eq!(
            v["providers"]["kiwano-gateway"]["api_key"],
            "kw-ag-crush-new"
        );
    }

    #[test]
    fn crush_reader_resolves_the_large_slots_provider() {
        let config = r#"{
  "models": { "large": { "provider": "deepseek", "model": "deepseek-chat" } },
  "providers": { "deepseek": { "name": "DeepSeek", "base_url": "https://api.deepseek.com/v1",
                               "api_key": "sk-ds" } }
}"#;
        let p = read_crush_current(config).unwrap();
        assert_eq!(p.name, "DeepSeek");
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.api_key, "sk-ds");

        // A $VAR key reference cannot be resolved offline: nothing to import.
        let config = r#"{
  "models": { "large": { "provider": "deepseek", "model": "deepseek-chat" } },
  "providers": { "deepseek": { "base_url": "https://api.deepseek.com/v1", "api_key": "$DEEPSEEK_API_KEY" } }
}"#;
        assert!(read_crush_current(config).is_none());

        // No large slot at all → nothing is being routed.
        assert!(read_crush_current(r#"{"providers": {}}"#).is_none());
    }
}
