//! MiniMax Code: the `custom_provider.<id>` mapping and the top-level
//! `defaultModel: "custom_provider:<id>/<model>"` selector that names it.
//!
//! Everything here is read off the shipped CLI (`@minimax-ai/code`), whose own
//! `mcode provider add` writes exactly these fields into `~/.minimax/config.yaml`
//! — including the API key in plaintext (the command reads it from the
//! environment and serializes it verbatim), so a takeover writing the same
//! shape is a takeover the tool itself accepts.

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;
use crate::gateway_takeover::readers::CurrentProvider;

/// The selector prefix for a custom-provider model. Not a slash form: the
/// provider id is `custom_provider:<id>`, and the model follows after one
/// more slash — `custom_provider:kiwano-gateway/deepseek-chat`.
const SELECTOR_PREFIX: &str = "custom_provider:";

// ── mcode (~/.minimax/config.yaml, YAML) ──

/// Upsert the gateway entry (`apiFormat: openai-completions`) and rewrite the
/// top-level `defaultModel` selector to route through the gateway, keeping the
/// user's model id. A `defaultModel` without a `<provider>/<model>` shape is
/// left alone (the user picks a gateway model in the mcode UI); the entry is
/// added either way.
pub fn upsert_mcode_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let yaml: serde_yaml::Value = if content.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("config.yaml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yaml root must be a YAML mapping")?;

    // custom_provider: a mapping of id → entry. mcode reserves a few ids for
    // its own providers ("minimax", "minimax_api", "provider",
    // "custom_provider"); `kiwano-gateway` is none of them.
    let providers_key = serde_yaml::Value::String("custom_provider".into());
    let mut providers = root
        .get(&providers_key)
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();

    let mut models = Vec::new();
    // The model the user is on: `defaultModel` is "<provider>/<model>" (the
    // official OAuth cloud models are `minimax/<id>`, custom ones
    // `custom_provider:<id>/<model>`). Its id goes upstream verbatim, so ours
    // keeps it — a takeover changes where the request goes, not which model
    // answers it.
    let selector = root
        .get(serde_yaml::Value::String("defaultModel".into()))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let model_id = selector
        .split_once('/')
        .map(|(_, id)| id.trim())
        .filter(|id| !id.is_empty())
        .unwrap_or("kiwano")
        .to_string();

    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String("Kiwano Gateway".into()),
    );
    entry.insert(
        serde_yaml::Value::String("baseUrl".into()),
        serde_yaml::Value::String(base_url.into()),
    );
    entry.insert(
        serde_yaml::Value::String("apiKey".into()),
        serde_yaml::Value::String(key.into()),
    );
    entry.insert(
        serde_yaml::Value::String("apiFormat".into()),
        serde_yaml::Value::String("openai-completions".into()),
    );
    let mut model = serde_yaml::Mapping::new();
    model.insert(
        serde_yaml::Value::String("modelId".into()),
        serde_yaml::Value::String(model_id.clone()),
    );
    models.push(serde_yaml::Value::Mapping(model));
    entry.insert(
        serde_yaml::Value::String("models".into()),
        serde_yaml::Value::Sequence(models),
    );

    providers.insert(
        serde_yaml::Value::String(GATEWAY_PROVIDER_ID.into()),
        serde_yaml::Value::Mapping(entry),
    );
    root.insert(providers_key, serde_yaml::Value::Mapping(providers));

    // Selection: point `defaultModel` at the gateway entry, model id kept.
    if selector.contains('/') {
        root.insert(
            serde_yaml::Value::String("defaultModel".into()),
            serde_yaml::Value::String(format!("{SELECTOR_PREFIX}{GATEWAY_PROVIDER_ID}/{model_id}")),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .map_err(|e| format!("failed to serialize config.yaml: {e}"))
}

/// What mcode routes to *today* — the provider the `defaultModel` selector
/// names — for the first-takeover import. Returns None when unset, prefix-less,
/// or already the gateway (a takeover that ran once must not re-import our own
/// loopback endpoint).
pub fn read_mcode_current(content: &str) -> Option<CurrentProvider> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(content).ok()?;
    let selector = yaml.get("defaultModel")?.as_str()?;
    let (provider_part, model_id) = selector.split_once('/')?;
    if model_id.trim().is_empty() {
        return None;
    }
    // The custom-provider form carries a colon (`custom_provider:<id>`); the
    // official OAuth form is a bare `minimax` with no entry of its own to read.
    let id = provider_part
        .strip_prefix(SELECTOR_PREFIX)
        .unwrap_or(provider_part);
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let entry = yaml.get("custom_provider")?.get(id)?;
    let base_url = entry.get("baseUrl")?.as_str()?.to_string();
    let api_key = entry.get("apiKey")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: id.to_string(),
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mcode_upserts_entry_and_rewrites_the_selector() {
        let original = r#"
# the provider add command writes these same fields
defaultModel: minimax/minimax-m2.5
beta:
  mcodeTools: false
custom_provider:
  openrouter:
    name: OpenRouter
    baseUrl: https://openrouter.ai/api/v1
    apiKey: sk-or
    apiFormat: openai-completions
    models:
      - modelId: deepseek-chat
"#;
        let out =
            upsert_mcode_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-mcode-abcd").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            v["defaultModel"],
            "custom_provider:kiwano-gateway/minimax-m2.5"
        );
        let gw = &v["custom_provider"]["kiwano-gateway"];
        assert_eq!(gw["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(gw["apiKey"], "kw-ag-mcode-abcd");
        assert_eq!(gw["apiFormat"], "openai-completions");
        assert_eq!(gw["models"][0]["modelId"], "minimax-m2.5");
        // The user's own custom provider and unrelated sections survive.
        assert_eq!(v["custom_provider"]["openrouter"]["apiKey"], "sk-or");
        assert_eq!(v["beta"]["mcodeTools"], false);
    }

    #[test]
    fn mcode_repeated_takeover_is_idempotent() {
        let original = r#"defaultModel: custom_provider:kiwano-gateway/deepseek-chat
custom_provider:
  kiwano-gateway:
    name: Kiwano Gateway
    baseUrl: http://127.0.0.1:8317/v1
    apiKey: kw-ag-mcode-old
    apiFormat: openai-completions
    models:
      - modelId: deepseek-chat
"#;
        let out =
            upsert_mcode_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-mcode-new").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            v["defaultModel"],
            "custom_provider:kiwano-gateway/deepseek-chat"
        );
        assert_eq!(
            v["custom_provider"]["kiwano-gateway"]["apiKey"],
            "kw-ag-mcode-new"
        );
    }

    #[test]
    fn mcode_prefixless_selector_is_left_alone() {
        let out = upsert_mcode_gateway(
            "defaultModel: deepseek-chat\ncustom_provider: {}\n",
            "http://127.0.0.1:8317/v1",
            "kw-ag-mcode-abcd",
        )
        .unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["defaultModel"], "deepseek-chat", "not ours to change");
        assert!(
            v["custom_provider"]["kiwano-gateway"].is_mapping(),
            "entry is still added"
        );
    }

    #[test]
    fn mcode_empty_config_creates_the_entry_without_a_selector() {
        let out = upsert_mcode_gateway("", "http://127.0.0.1:8317/v1", "kw-ag-mcode-abcd").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(
            v["custom_provider"]["kiwano-gateway"]["apiKey"],
            "kw-ag-mcode-abcd"
        );
        assert!(v.get("defaultModel").is_none(), "nothing to select from");
    }

    #[test]
    fn mcode_reader_reads_the_selector_named_provider() {
        let config = r#"defaultModel: custom_provider:openrouter/deepseek-chat
custom_provider:
  openrouter:
    name: OpenRouter
    baseUrl: https://openrouter.ai/api/v1
    apiKey: sk-or
"#;
        let p = read_mcode_current(config).unwrap();
        assert_eq!(p.name, "openrouter");
        assert_eq!(p.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(p.api_key, "sk-or");

        // The official OAuth shape has no entry to read.
        assert!(read_mcode_current("defaultModel: minimax/minimax-m2.5\n").is_none());
        // Ours is not re-imported.
        let taken = r#"defaultModel: custom_provider:kiwano-gateway/deepseek-chat
custom_provider:
  kiwano-gateway:
    name: Kiwano Gateway
    baseUrl: http://127.0.0.1:8317/v1
    apiKey: kw
"#;
        assert!(read_mcode_current(taken).is_none());
    }
}
