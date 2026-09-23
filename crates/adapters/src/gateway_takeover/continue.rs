//! Continue: the `models` sequence in `config.yaml`, where each entry carries
//! its own `provider`/`model`/`apiBase`/`apiKey` and an optional `roles` list.
//!
//! Continue has no selector field in the config — the active model is picked in
//! the UI and remembered outside the file — and its documented fallback is
//! "the first model listed for the role" (core/config/selectedModels.ts). A
//! takeover therefore *unshifts* the gateway entry with `roles: [chat]`: it
//! becomes the default chat model without claiming autocomplete or embed, and
//! the user's own entries keep their positions behind it.

use serde_yaml::Value;

// ── continue (~/.continue/config.yaml, fixed home location) ──

const GATEWAY_MODEL_NAME: &str = "Kiwano Gateway";

/// The model id a fresh config gets when the user's had nothing to copy from.
const PLACEHOLDER_MODEL_ID: &str = "kiwano";

/// The model id worth forwarding: the first entry that is a chat model (no
/// `roles`, or `roles` containing `chat`). Continue's `model` field is the id
/// the provider receives, so it travels through the gateway verbatim.
fn current_model_id(models: &[Value]) -> String {
    let chat = models.iter().find(|m| {
        match m.get("roles").and_then(|r| r.as_sequence()) {
            Some(roles) => roles.iter().filter_map(|r| r.as_str()).any(|r| r == "chat"),
            // No roles list means every role, chat included.
            None => true,
        }
    });
    chat.and_then(|m| m.get("model"))
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string()
}

/// Upsert the gateway model at the head of `models` and keep the user's own
/// entries. Comments and layout do not survive (serde_yaml re-serializes);
/// restore puts the user's bytes back the way they were.
pub fn upsert_continue_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let yaml: Value = if content.trim().is_empty() {
        Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("config.yaml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yaml root must be a YAML mapping")?;

    let models_key = Value::String("models".into());
    let mut models: Vec<Value> = root
        .get(&models_key)
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();
    // A re-run replaces our entry rather than stacking a second one.
    models.retain(|m| m.get("name").and_then(|n| n.as_str()) != Some(GATEWAY_MODEL_NAME));

    let mut entry = serde_yaml::Mapping::new();
    let insert = |entry: &mut serde_yaml::Mapping, key: &str, value: Value| {
        entry.insert(Value::String(key.into()), value);
    };
    insert(&mut entry, "name", Value::String(GATEWAY_MODEL_NAME.into()));
    insert(&mut entry, "provider", Value::String("openai".into()));
    insert(
        &mut entry,
        "model",
        Value::String(current_model_id(&models)),
    );
    insert(&mut entry, "apiBase", Value::String(base_url.into()));
    insert(&mut entry, "apiKey", Value::String(key.into()));
    // Chat only: claiming every role would make the gateway the fallback for
    // autocomplete and embed models the user configured deliberately.
    insert(
        &mut entry,
        "roles",
        Value::Sequence(vec![Value::String("chat".into())]),
    );

    models.insert(0, Value::Mapping(entry));
    root.insert(models_key, Value::Sequence(models));

    serde_yaml::to_string(&Value::Mapping(root)).map_err(|e| e.to_string())
}

/// What Continue routes to *today*: the first chat model that carries both an
/// `apiBase` and an `apiKey` — a config whose chat model is a vendor default
/// (no custom endpoint) extracts nothing. The entry's `name` is a display
/// name, so unlike Cline/Aider the import can carry it.
pub fn read_continue_current(
    content: &str,
) -> Option<crate::gateway_takeover::readers::CurrentProvider> {
    let yaml: Value = serde_yaml::from_str(content).ok()?;
    let models = yaml.get("models")?.as_sequence()?;
    let chat = models.iter().find(|m| match m.get("roles") {
        Some(Value::Sequence(roles)) => {
            roles.iter().filter_map(|r| r.as_str()).any(|r| r == "chat")
        }
        _ => true,
    })?;
    let base_url = chat.get("apiBase")?.as_str()?.trim().to_string();
    let api_key = chat.get("apiKey")?.as_str()?.trim().to_string();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    Some(crate::gateway_takeover::readers::CurrentProvider {
        name: chat
            .get("name")
            .and_then(|n| n.as_str())
            .unwrap_or_default()
            .trim()
            .to_string(),
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn continue_unshifts_the_gateway_model_and_keeps_the_user_entries() {
        let original = r#"
name: my config
version: 0.0.1
schema: v1
models:
  - name: DeepSeek
    provider: openai
    model: deepseek-chat
    apiBase: https://api.deepseek.com/v1
    apiKey: sk-ds
    roles: [chat, edit]
  - name: Coder
    provider: ollama
    model: qwen3-coder
    roles: [autocomplete]
"#;
        let out =
            upsert_continue_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-continue-abcd")
                .unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        let models = v["models"].as_sequence().unwrap();
        assert_eq!(models[0]["name"], "Kiwano Gateway");
        assert_eq!(models[0]["provider"], "openai");
        assert_eq!(
            models[0]["model"], "deepseek-chat",
            "the id the user's chat model used"
        );
        assert_eq!(models[0]["apiBase"], "http://127.0.0.1:8317/v1");
        assert_eq!(models[0]["apiKey"], "kw-ag-continue-abcd");
        assert_eq!(models[0]["roles"][0], "chat");
        // The user's own entries survive, in their original relative order.
        assert_eq!(models[1]["name"], "DeepSeek");
        assert_eq!(models[2]["name"], "Coder");
        assert_eq!(v["schema"], "v1"); // untouched top-level fields survive
    }

    #[test]
    fn continue_replaces_our_entry_on_a_rerun() {
        let taken = "models:\n  - name: Kiwano Gateway\n    provider: openai\n    model: old\n    apiBase: http://127.0.0.1:8317/v1\n    apiKey: kw-ag-continue-old\n    roles: [chat]\n  - name: DeepSeek\n    provider: openai\n    model: deepseek-chat\n    apiBase: https://api.deepseek.com/v1\n    apiKey: sk-ds\n";
        let out = upsert_continue_gateway(taken, "http://127.0.0.1:8317/v1", "kw-ag-continue-new")
            .unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        let models = v["models"].as_sequence().unwrap();
        assert_eq!(models.len(), 2, "no stacked gateway entries");
        assert_eq!(models[0]["apiKey"], "kw-ag-continue-new");
        // The id comes from the first *user* chat model now that ours is gone.
        assert_eq!(models[0]["model"], "deepseek-chat");
    }

    #[test]
    fn continue_empty_config_creates_a_placeholder_model() {
        let out = upsert_continue_gateway("", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["models"][0]["model"], "kiwano");
        assert_eq!(v["models"][0]["roles"][0], "chat");
    }

    #[test]
    fn continue_reader_takes_the_first_chat_model_only() {
        let config = r#"models:
  - name: DeepSeek
    provider: openai
    model: deepseek-chat
    apiBase: https://api.deepseek.com/v1
    apiKey: sk-ds
    roles: [chat, edit]
"#;
        let p = read_continue_current(config).unwrap();
        assert_eq!(p.name, "DeepSeek");
        assert_eq!(p.base_url, "https://api.deepseek.com/v1");
        assert_eq!(p.api_key, "sk-ds");

        // The first chat model is the one Continue routes chat through — a
        // later entry's endpoint is not what the user is using, so a vendor
        // default in front of a configured model extracts nothing.
        let config = r#"models:
  - name: Local
    provider: ollama
    model: qwen3
    roles: [chat]
  - name: DeepSeek
    provider: openai
    model: deepseek-chat
    apiBase: https://api.deepseek.com/v1
    apiKey: sk-ds
"#;
        assert!(
            read_continue_current(config).is_none(),
            "only the first chat model is Continue's active route"
        );
    }
}
