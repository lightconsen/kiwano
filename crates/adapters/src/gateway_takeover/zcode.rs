//! ZCode (Z.ai): the native provider registry shared between the CLI and
//! Desktop — `providerRules` entries plus a `defaultModelSelection` that picks
//! the model new sessions start on.
//!
//! The schema is the ZCode runtime's own (provider.example.json): a rule
//! carries the access (`type: "api-key"` with a plaintext key), the wire
//! protocol (`api.type` — the takeover speaks `openai-chat-completions`), and
//! the personal model ids it serves. Model references are `providerId/modelId`
//! resolved against the rules, so the takeover adds its rule and points the
//! selection at it — the user's own rules stay untouched.

use serde_json::Value;

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};

// ── zcode (~/.zcode/v2/provider_config.json, exact-file env honored) ──

/// The id the selection gets when nothing selectable exists yet — the gateway
/// forwards model names verbatim, so the user replaces it with a model their
/// provider serves.
///
/// (gateway.rs's `PLACEHOLDER_MODEL_ID` is the same string; this alias keeps
/// the zcode-specific doc attached.)
const FRESH_MODEL_ID: &str = PLACEHOLDER_MODEL_ID;

/// The id `defaultModelSelection` names today — the model whose requests the
/// takeover is rerouting. A selection already pointing at us (a re-run) is
/// not a model name, so a fresh placeholder is picked instead.
fn selected_model_id(root: &Value) -> String {
    let sel = &root["config"]["defaultModelSelection"];
    let (provider, model) = (
        sel.get("providerId").and_then(|v| v.as_str()),
        sel.get("modelId").and_then(|v| v.as_str()),
    );
    match (provider, model) {
        (Some(p), Some(m)) if p != GATEWAY_PROVIDER_ID && !m.trim().is_empty() => {
            m.trim().to_string()
        }
        _ => FRESH_MODEL_ID.to_string(),
    }
}

/// The rule we file our entry under: the example's own shape, filled with the
/// gateway's endpoint. `visibility`/`enabled` make it show up in `/model` and
/// the Desktop editor; `modelOrder` keeps our single model first in its list.
fn gateway_rule(model_id: &str, base_url: &str, key: &str) -> Value {
    serde_json::json!({
        "providerId": GATEWAY_PROVIDER_ID,
        "providerName": GATEWAY_LABEL,
        "enabled": true,
        "config": {
            "group": "standard-personal",
            "access": { "type": "api-key", "apiKey": key },
            "api": {
                "type": "openai-chat-completions",
                "baseUrl": base_url,
                "headers": {}
            },
            "personalModelIds": [model_id],
            "modelOrder": [model_id],
            "visibility": "visible"
        }
    })
}

/// Upsert the gateway rule and point `defaultModelSelection` at it. The
/// user's own rules, model overlays and ordering survive; the rest of the
/// document keeps its fields though not its layout (serde_json
/// re-serializes; restore puts the user's bytes back the way they were).
pub fn upsert_zcode_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if content.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(content)
            .map_err(|e| format!("provider_config.json is not valid JSON: {e}"))?
    };
    if !v.is_object() {
        return Err("provider_config.json root must be a JSON object".into());
    }
    let model_id = selected_model_id(&v);

    let obj = v.as_object_mut().unwrap();
    // A fresh file still needs the version the parser keys on.
    obj.entry("schemaVersion".to_string())
        .or_insert_with(|| serde_json::json!(1));
    let config = obj
        .entry("config".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let config = config
        .as_object_mut()
        .ok_or("provider_config.json config must be an object")?;

    let rules_obj = config
        .entry("providerConfigRules".to_string())
        .or_insert_with(|| Value::Object(Default::default()));
    let rules_obj = rules_obj
        .as_object_mut()
        .ok_or("providerConfigRules must be an object")?;
    let rules = rules_obj
        .entry("providerRules".to_string())
        .or_insert_with(|| Value::Array(Vec::new()));
    let rules = rules
        .as_array_mut()
        .ok_or("providerRules must be an array")?;
    // A re-run replaces our rule rather than stacking a second one.
    rules.retain(|r| r.get("providerId").and_then(|p| p.as_str()) != Some(GATEWAY_PROVIDER_ID));
    rules.push(gateway_rule(&model_id, base_url, key));

    // Join the display order when the user's file curates one; absent means
    // "everything in rule order", which our push already satisfies.
    if let Some(order) = config
        .get_mut("providerOrder")
        .and_then(|o| o.as_array_mut())
    {
        if !order
            .iter()
            .any(|id| id.as_str() == Some(GATEWAY_PROVIDER_ID))
        {
            order.push(Value::String(GATEWAY_PROVIDER_ID.into()));
        }
    }

    config.insert(
        "defaultModelSelection".to_string(),
        serde_json::json!({ "providerId": GATEWAY_PROVIDER_ID, "modelId": model_id }),
    );

    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// What ZCode routes to *today*: the selection's provider, resolved to its
/// rule's base URL and key. A rule that is disabled, keyless, or not
/// api-key-accessed extracts nothing.
pub fn read_zcode_current(
    content: &str,
) -> Option<crate::gateway_takeover::readers::CurrentProvider> {
    let v: Value = serde_json::from_str(content).ok()?;
    let sel = &v["config"]["defaultModelSelection"];
    let provider_id = sel.get("providerId")?.as_str()?;
    if provider_id.is_empty() {
        return None;
    }
    let rule = v["config"]["providerConfigRules"]["providerRules"]
        .as_array()?
        .iter()
        .find(|r| r.get("providerId").and_then(|p| p.as_str()) == Some(provider_id))?;
    if rule.get("enabled") == Some(&Value::Bool(false)) {
        return None;
    }
    let cfg = rule.get("config")?;
    if cfg
        .get("access")
        .and_then(|a| a.get("type"))
        .and_then(|t| t.as_str())
        != Some("api-key")
    {
        return None;
    }
    let base_url = cfg
        .get("api")
        .and_then(|a| a.get("baseUrl"))
        .and_then(|b| b.as_str())?
        .trim()
        .to_string();
    let api_key = cfg
        .get("access")
        .and_then(|a| a.get("apiKey"))
        .and_then(|k| k.as_str())?
        .trim()
        .to_string();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    let name = rule
        .get("providerName")
        .and_then(|n| n.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .unwrap_or(provider_id)
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
    fn zcode_upserts_the_rule_and_points_the_selection_at_it() {
        let original = r#"{
  "schemaVersion": 1,
  "config": {
    "providerConfigRules": { "providerRules": [
      { "providerId": "deepseek", "providerName": "DeepSeek", "enabled": true,
        "config": { "group": "standard-personal",
                    "access": { "type": "api-key", "apiKey": "sk-ds" },
                    "api": { "type": "openai-chat-completions", "baseUrl": "https://api.deepseek.com/v1" },
                    "personalModelIds": ["deepseek-chat"], "visibility": "visible" } }
    ] },
    "providerOrder": ["deepseek"],
    "defaultModelSelection": { "providerId": "deepseek", "modelId": "deepseek-chat",
                               "options": { "reasoningLevel": "high" } }
  }
}"#;
        let out =
            upsert_zcode_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-zcode-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let sel = &v["config"]["defaultModelSelection"];
        assert_eq!(sel["providerId"], "kiwano-gateway");
        assert_eq!(sel["modelId"], "deepseek-chat", "the selected id is kept");
        // The user's rule and ordering survive behind ours.
        let rules = v["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 2);
        assert_eq!(rules[0]["providerId"], "deepseek");
        let gw = &rules[1];
        assert_eq!(gw["providerId"], "kiwano-gateway");
        assert_eq!(gw["enabled"], true);
        assert_eq!(gw["config"]["api"]["type"], "openai-chat-completions");
        assert_eq!(gw["config"]["api"]["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(gw["config"]["access"]["apiKey"], "kw-ag-zcode-abcd");
        assert_eq!(gw["config"]["personalModelIds"][0], "deepseek-chat");
        // Our id joined the curated order without displacing the user's.
        let order = v["config"]["providerOrder"].as_array().unwrap();
        assert_eq!(order, &["deepseek", "kiwano-gateway"]);
    }

    #[test]
    fn zcode_replaces_our_rule_on_a_rerun() {
        let taken = r#"{
  "schemaVersion": 1,
  "config": {
    "providerConfigRules": { "providerRules": [
      { "providerId": "kiwano-gateway", "providerName": "Kiwano Gateway", "enabled": true,
        "config": { "group": "standard-personal",
                    "access": { "type": "api-key", "apiKey": "kw-ag-zcode-old" },
                    "api": { "type": "openai-chat-completions", "baseUrl": "http://127.0.0.1:9999/v1" },
                    "personalModelIds": ["kiwano"], "visibility": "visible" } }
    ] },
    "defaultModelSelection": { "providerId": "kiwano-gateway", "modelId": "kiwano" }
  }
}"#;
        let out =
            upsert_zcode_gateway(taken, "http://127.0.0.1:8317/v1", "kw-ag-zcode-new").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let rules = v["config"]["providerConfigRules"]["providerRules"]
            .as_array()
            .unwrap();
        assert_eq!(rules.len(), 1, "no stacked gateway rules");
        assert_eq!(rules[0]["config"]["access"]["apiKey"], "kw-ag-zcode-new");
        // Our own placeholder is not "the user's model": a fresh one is picked.
        assert_eq!(v["config"]["defaultModelSelection"]["modelId"], "kiwano");
    }

    #[test]
    fn zcode_empty_config_creates_a_minimal_file() {
        let out = upsert_zcode_gateway("", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["schemaVersion"], 1);
        assert_eq!(
            v["config"]["defaultModelSelection"]["modelId"], "kiwano",
            "nothing to keep on a fresh machine"
        );
    }

    #[test]
    fn zcode_reader_resolves_the_selections_provider() {
        let config = r#"{
  "config": {
    "providerConfigRules": { "providerRules": [
      { "providerId": "deepseek", "providerName": "DeepSeek", "enabled": true,
        "config": { "access": { "type": "api-key", "apiKey": "sk-ds" },
                    "api": { "type": "openai-chat-completions", "baseUrl": "https://api.deepseek.com/v1" } } }
    ] },
    "defaultModelSelection": { "providerId": "deepseek", "modelId": "deepseek-chat" }
  }
}"#;
        let p = read_zcode_current(config).unwrap();
        assert_eq!(p.name, "DeepSeek");
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.api_key, "sk-ds");

        // A disabled rule is not a routed configuration.
        let config = config.replace("\"enabled\": true", "\"enabled\": false");
        assert!(read_zcode_current(&config).is_none());
        // No selection at all → nothing is being routed.
        assert!(read_zcode_current(r#"{"config": {}}"#).is_none());
    }
}
