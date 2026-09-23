//! Aider: the three global fields a takeover rewrites (`openai-api-base`,
//! `openai-api-key`, `model`) — one provider slot, not a provider list.
//!
//! Aider's own docs describe exactly this shape for a custom endpoint
//! (`openai-api-base` + `openai-api-key` in `.aider.conf.yml`, with the model
//! prefixed `openai/` to force LiteLLM's OpenAI-compatible route), and GitHub
//! Copilot's hosted Claude models are the documented precedent that it works
//! for Anthropic-family models too. The takeover is therefore *exclusive*:
//! these three fields are the whole slot, and restore puts the user's bytes
//! back the way they were.

use serde_yaml::Value;

// ── aider (~/.aider.conf.yml, YAML) ──

/// Force the model through LiteLLM's OpenAI-compatible route: without the
/// `openai/` prefix, Aider hands non-OpenAI names to the litellm provider it
/// guesses from the name (anthropic/… → the Anthropic API), and the gateway's
/// OpenAI-compatible face would never be called.
const OPENAI_PREFIX: &str = "openai/";

/// The model id a fresh config gets when the user's had nothing to copy from.
/// The gateway forwards model names verbatim, so this is a placeholder the
/// user replaces with a model their provider serves.
const PLACEHOLDER_MODEL_ID: &str = "kiwano";

/// The user's model id, however they spelled it: the last segment after any
/// provider prefixes (`anthropic/claude-x` → `claude-x`,
/// `openrouter/anthropic/claude-x` → `claude-x`), bare names pass through.
fn model_id_of(model: Option<&str>) -> String {
    model
        .and_then(|m| m.rsplit('/').next())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string()
}

/// Take over Aider's config: point the OpenAI-compatible fields at the gateway
/// and select the user's model through the forced route. Comments and the rest
/// of the document do not survive (serde_yaml re-serializes); restore puts the
/// user's bytes back the way they were.
pub fn upsert_aider_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let yaml: Value = if content.trim().is_empty() {
        Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content)
            .map_err(|e| format!(".aider.conf.yml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or(".aider.conf.yml root must be a YAML mapping")?;

    let model_id = model_id_of(
        root.get(Value::String("model".into()))
            .and_then(|v| v.as_str()),
    );
    let insert = |root: &mut serde_yaml::Mapping, key: &str, value: &str| {
        root.insert(Value::String(key.into()), Value::String(value.into()));
    };
    insert(&mut root, "openai-api-base", base_url);
    insert(&mut root, "openai-api-key", key);
    insert(&mut root, "model", &format!("{OPENAI_PREFIX}{model_id}"));

    serde_yaml::to_string(&Value::Mapping(root)).map_err(|e| e.to_string())
}

/// What Aider routes to *today* — the custom endpoint its config names, for
/// the first-takeover import. Returns None unless both a base URL and a key
/// are set; `name` is deliberately None because Aider has no provider id for
/// the row to carry (host inference names the import, as it does for Cline).
pub fn read_aider_current(
    content: &str,
) -> Option<crate::gateway_takeover::readers::CurrentProvider> {
    let yaml: Value = serde_yaml::from_str(content).ok()?;
    let base_url = yaml.get("openai-api-base")?.as_str()?.trim().to_string();
    let api_key = yaml.get("openai-api-key")?.as_str()?.trim().to_string();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    Some(crate::gateway_takeover::readers::CurrentProvider {
        name: String::new(),
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn aider_overwrites_the_three_fields_and_keeps_the_model_id() {
        let original = r#"
# aider's own config: dark mode, and a Claude model from its vendor
model: anthropic/claude-sonnet-4-6
dark-mode: true
auto-commits: false
"#;
        let out =
            upsert_aider_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-aider-abcd").unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"], "openai/claude-sonnet-4-6");
        assert_eq!(v["openai-api-base"], "http://127.0.0.1:8317/v1");
        assert_eq!(v["openai-api-key"], "kw-ag-aider-abcd");
        // Unrelated settings survive the rewrite (though not the comments —
        // serde_yaml re-serializes; restore puts the user's bytes back).
        assert_eq!(v["auto-commits"], false);
        assert!(v.get("dark-mode").is_some());
    }

    #[test]
    fn aider_bare_and_prefixed_model_names_both_route_through_the_gateway() {
        let out =
            upsert_aider_gateway("model: gpt-5.2\n", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"], "openai/gpt-5.2");

        let out = upsert_aider_gateway(
            "model: openrouter/anthropic/claude-sonnet-4-6\n",
            "http://127.0.0.1:8317/v1",
            "k",
        )
        .unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            v["model"], "openai/claude-sonnet-4-6",
            "every prefix is stripped down to the model id itself"
        );
    }

    #[test]
    fn aider_empty_config_creates_the_fields_with_a_placeholder_model() {
        let out = upsert_aider_gateway("", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"], "openai/kiwano");
        assert_eq!(v["openai-api-base"], "http://127.0.0.1:8317/v1");
    }

    #[test]
    fn aider_reader_returns_the_custom_endpoint_when_both_fields_exist() {
        let config = "openai-api-base: https://api.example.com/v1\nopenai-api-key: sk-old\n";
        let p = read_aider_current(config).unwrap();
        assert_eq!(p.base_url, "https://api.example.com/v1");
        assert_eq!(p.api_key, "sk-old");
        assert!(p.name.is_empty(), "no provider id: host inference names it");

        // A config without a key is not a routed configuration.
        assert!(read_aider_current("openai-api-base: https://x/v1\n").is_none());
    }
}
