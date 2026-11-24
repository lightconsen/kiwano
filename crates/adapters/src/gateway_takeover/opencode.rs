//! OpenCode: the gateway entry (an OpenAI-compatible provider) and the
//! top-level `model` selector that names it.
//!
//! The transform itself is shared with Kilo Code, OpenCode's fork, which kept
//! the schema — see [`super::openai_compatible`]. What is OpenCode's own here is
//! the file name, which is also the label its parse errors carry.

use crate::gateway_takeover::openai_compatible::upsert_openai_compatible_provider;

// ── opencode (~/.config/opencode/opencode.json, JSONC) ──

/// The file OpenCode reads its global config from. Only one file: OpenCode's
/// own config-path resolution has no candidate list — a missing `opencode.json`
/// is a config with nothing in it, and the takeover creates it.
pub(crate) const OPENCODE_CONFIG_FILE: &str = "opencode.json";

/// Upsert the gateway provider (`npm: @ai-sdk/openai-compatible`) and rewrite
/// the top-level `model: "<provider>/<model>"` selector to route through the
/// gateway, keeping the user's model id. A missing or non-slash-form `model`
/// key is left alone (the user picks a gateway model in the OpenCode UI).
pub fn upsert_opencode_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    upsert_openai_compatible_provider(content, OPENCODE_CONFIG_FILE, base_url, key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gateway_takeover::GATEWAY_PROVIDER_ID;
    use serde_json::Value;

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
