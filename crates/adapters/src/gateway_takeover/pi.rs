//! Pi: `models.json` gets the gateway entry and `settings.json` gets the
//! selection.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

// ── pi (~/.pi/agent/models.json + settings.json, JSONC) ──

/// Upsert the gateway provider into `providers` of Pi's models.json. The
/// entry declares an empty model list: Pi keeps its own `defaultModel` and the
/// gateway forwards model names verbatim (documented deviation from
/// cc-switch, which manages models per provider row).
///
/// Pi's own session id cannot reach the gateway on this route, and no config
/// can change that: the pi-ai chat-completions builder the takeover selects
/// has no session field at all — no `prompt_cache_key`, no `user`, no
/// metadata, no session header — while the library's responses-flavored
/// routes do carry `sessionId`. Closing the gap would mean switching the
/// agent to a protocol its upstreams may not speak. The gateway's
/// `session_hint` therefore falls back to a derived fingerprint of the
/// conversation's opening for Pi, marked `derived:` so it is not mistaken
/// for a client-stated id.
pub fn upsert_pi_models_gateway(
    content: &str,
    base_url: &str,
    key: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "models.json")?;
    let mut obj = require_object(root, "models.json")?;

    object_field(&mut obj, "models.json", "providers");
    obj["providers"][GATEWAY_PROVIDER_ID] = json!({
        "name": GATEWAY_LABEL,
        "baseUrl": base_url,
        "api": "openai-completions",
        "apiKey": key,
        "models": [],
    });

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// Select the gateway provider in Pi's settings.json (`defaultProvider`).
/// `defaultModel` is preserved — Pi owns the model choice and the gateway is
/// model-agnostic. This settings.json write is a deliberate deviation from
/// cc-switch (whose Pi integration leaves selection to the Pi UI).
pub fn select_pi_gateway(content: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "settings.json")?;
    let mut obj = require_object(root, "settings.json")?;
    obj.insert("defaultProvider".into(), json!(GATEWAY_PROVIDER_ID));
    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pi_upserts_models_entry_and_selects_provider() {
        let models = r#"{"providers": {"anthropic": {"baseUrl": "https://api.anthropic.com"}}}"#;
        let out =
            upsert_pi_models_gateway(models, "http://127.0.0.1:8317/v1", "kw-ag-pi-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let entry = &v["providers"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["api"], "openai-completions");
        assert_eq!(entry["apiKey"], "kw-ag-pi-abcd");
        assert!(v["providers"]["anthropic"].is_object()); // additive

        let settings = r#"{"defaultProvider":"anthropic","defaultModel":"claude-sonnet-4-5"}"#;
        let out = select_pi_gateway(settings).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["defaultProvider"], "kiwano-gateway");
        assert_eq!(v["defaultModel"], "claude-sonnet-4-5"); // model choice preserved
    }
}
