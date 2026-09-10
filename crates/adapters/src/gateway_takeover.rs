// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/{opencode_config,openclaw_config,hermes_config,pi_config}.rs
// Copied on 2026-09-08. Modified for Kiwano: only the provider-entry write +
// selection subsets were ported, reshaped as pure content→content transforms
// for the takeover pipeline (read → rewrite in memory → backup → atomic write,
// see src-tauri/src/takeover.rs). The DB-backed provider CRUD, CAS revision
// checks and 0600-permission plumbing were dropped because the takeover
// pipeline is the sole writer and serializes file access itself.

//! Gateway takeover transforms for additive-mode agents (opencode, openclaw,
//! hermes, pi).
//!
//! Unlike the exclusive-switch agents (claude/codex/gemini/grokbuild) whose
//! config holds one provider slot, these CLIs keep multi-provider configs, so
//! "takeover" means: upsert a `kiwano-gateway` provider entry pointing at the
//! local gateway and select it — every pre-existing provider entry survives.
//! The gateway forwards model names verbatim, so selections keep the user's
//! existing model id and only swap the provider prefix.

use serde_json::{json, Map, Value};

/// Provider id used in every additive config for the local gateway entry.
pub const GATEWAY_PROVIDER_ID: &str = "kiwano-gateway";

const GATEWAY_LABEL: &str = "Kiwano Gateway";

/// The provider an additive agent currently routes to, extracted from its
/// config (used by the first-takeover import in src-tauri/src/creds.rs).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CurrentProvider {
    /// Provider entry key in the agent's config (opencode/openclaw/pi key,
    /// hermes custom-provider name) — doubles as a friendly display name.
    pub name: String,
    pub base_url: String,
    pub api_key: String,
}

fn parse_jsonc(content: &str, label: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    json5::from_str(content).map_err(|e| format!("{label} is not valid JSON/JSONC: {e}"))
}

fn require_object(v: Value, label: &str) -> Result<Map<String, Value>, String> {
    v.as_object()
        .cloned()
        .ok_or(format!("{label} root must be a JSON object"))
}

// ── opencode (~/.config/opencode/opencode.json, JSONC) ──

/// Upsert the gateway provider (`npm: @ai-sdk/openai-compatible`) and rewrite
/// the top-level `model: "<provider>/<model>"` selector to route through the
/// gateway, keeping the user's model id. A missing or non-slash-form `model`
/// key is left alone (the user picks a gateway model in the OpenCode UI).
pub fn upsert_opencode_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "opencode.json")?;
    let mut obj = require_object(root, "opencode.json")?;

    // provider section: normalize to an object before inserting the entry
    if !obj.get("provider").is_some_and(Value::is_object) {
        obj.insert("provider".into(), json!({}));
    }
    let entry = json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": GATEWAY_LABEL,
        "options": { "baseURL": base_url, "apiKey": key },
        "models": {},
    });
    obj["provider"][GATEWAY_PROVIDER_ID] = entry;

    // selection: "<old-provider>/<model>" → "kiwano-gateway/<model>"
    if let Some(model_id) = obj
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/'))
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
    {
        obj["provider"][GATEWAY_PROVIDER_ID]["models"][&model_id] = json!({});
        obj["model"] = json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}"));
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

// ── openclaw (~/.openclaw/openclaw.json, JSONC) ──

/// Upsert the gateway provider under `models.providers` and point
/// `agents.defaults.model.primary` at `kiwano-gateway/<model>` (keeping the
/// user's model id; a missing selector is left alone).
pub fn upsert_openclaw_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "openclaw.json")?;
    let mut obj = require_object(root, "openclaw.json")?;

    if !obj.get("models").is_some_and(Value::is_object) {
        obj.insert("models".into(), json!({}));
    }
    if !obj["models"].get("providers").is_some_and(Value::is_object) {
        obj["models"]["providers"] = json!({});
    }
    let entry = json!({
        "baseUrl": base_url,
        "apiKey": key,
        "api": "openai-completions",
        "models": [],
    });
    obj["models"]["providers"][GATEWAY_PROVIDER_ID] = entry;

    if let Some(primary) = obj
        .get("agents")
        .and_then(|a| a.get("defaults"))
        .and_then(|d| d.get("model"))
        .and_then(|m| m.get("primary"))
        .and_then(Value::as_str)
    {
        if let Some((_, model_id)) = primary.split_once('/') {
            if !model_id.is_empty() {
                obj["agents"]["defaults"]["model"]["primary"] =
                    json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}"));
            }
        }
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

// ── current-provider readers (first-takeover import; see src-tauri/creds.rs) ──

/// opencode: the top-level `model: "<provider>/<model>"` selector's provider
/// entry. Returns None when unset, non-slash form, or already the gateway.
pub fn read_opencode_current(content: &str) -> Option<CurrentProvider> {
    let obj = parse_jsonc(content, "opencode.json").ok()?;
    let obj = require_object(obj, "opencode.json").ok()?;
    let id = obj.get("model")?.as_str()?.split_once('/')?.0.to_string();
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let entry = obj.get("provider")?.get(&id)?;
    let base_url = entry.get("options")?.get("baseURL")?.as_str()?.to_string();
    let api_key = entry.get("options")?.get("apiKey")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: id,
        base_url,
        api_key,
    })
}

/// openclaw: `agents.defaults.model.primary`'s provider entry.
pub fn read_openclaw_current(content: &str) -> Option<CurrentProvider> {
    let obj = parse_jsonc(content, "openclaw.json").ok()?;
    let obj = require_object(obj, "openclaw.json").ok()?;
    let id = obj
        .get("agents")?
        .get("defaults")?
        .get("model")?
        .get("primary")?
        .as_str()?
        .split_once('/')?
        .0
        .to_string();
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let entry = obj.get("models")?.get("providers")?.get(&id)?;
    let base_url = entry.get("baseUrl")?.as_str()?.to_string();
    let api_key = entry.get("apiKey")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: id,
        base_url,
        api_key,
    })
}

/// hermes: the `model.provider` name's entry in `custom_providers`.
pub fn read_hermes_current(content: &str) -> Option<CurrentProvider> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(content).ok()?;
    let root = yaml.as_mapping()?;
    let model_key = serde_yaml::Value::String("model".into());
    let provider_key = serde_yaml::Value::String("provider".into());
    let providers_key = serde_yaml::Value::String("custom_providers".into());
    let provider = root
        .get(&model_key)?
        .get(&provider_key)?
        .as_str()?
        .to_string();
    if provider.is_empty() || provider == GATEWAY_PROVIDER_ID {
        return None;
    }
    let providers = root.get(&providers_key)?.as_sequence()?;
    let entry = providers
        .iter()
        .find(|p| p.get("name").and_then(|n| n.as_str()) == Some(provider.as_str()))?;
    let base_url = entry.get("base_url")?.as_str()?.to_string();
    let api_key = entry.get("api_key")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: provider,
        base_url,
        api_key,
    })
}

/// pi: `settings.json` `defaultProvider`'s entry in `models.json`.
pub fn read_pi_current(models_content: &str, settings_content: &str) -> Option<CurrentProvider> {
    let settings = parse_jsonc(settings_content, "settings.json").ok()?;
    let settings = require_object(settings, "settings.json").ok()?;
    let id = settings.get("defaultProvider")?.as_str()?.to_string();
    if id.is_empty() || id == GATEWAY_PROVIDER_ID {
        return None;
    }
    let models = parse_jsonc(models_content, "models.json").ok()?;
    let models = require_object(models, "models.json").ok()?;
    let entry = models.get("providers")?.get(&id)?;
    let base_url = entry.get("baseUrl")?.as_str()?.to_string();
    let api_key = entry.get("apiKey")?.as_str()?.to_string();
    Some(CurrentProvider {
        name: id,
        base_url,
        api_key,
    })
}

// ── hermes (~/.hermes/config.yaml, HERMES_HOME honored by the caller) ──

/// Upsert the gateway entry into the `custom_providers:` sequence (matched by
/// `name`) and set `model.provider` to the gateway. `model.default` is
/// intentionally preserved — apply_switch_defaults (cc-switch) only overwrites
/// it when the provider declares models, and the gateway entry is
/// model-agnostic. Untouched top-level sections survive the round-trip.
pub fn upsert_hermes_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let yaml: serde_yaml::Value = if content.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("config.yaml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yaml root must be a YAML mapping")?;

    let name_key = serde_yaml::Value::String("custom_providers".into());
    let mut providers: Vec<serde_yaml::Value> = root
        .get(&name_key)
        .and_then(|v| v.as_sequence())
        .cloned()
        .unwrap_or_default();

    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String(GATEWAY_PROVIDER_ID.into()),
    );
    entry.insert(
        serde_yaml::Value::String("base_url".into()),
        serde_yaml::Value::String(base_url.into()),
    );
    entry.insert(
        serde_yaml::Value::String("api_key".into()),
        serde_yaml::Value::String(key.into()),
    );
    let entry = serde_yaml::Value::Mapping(entry);

    match providers
        .iter()
        .position(|p| p.get("name").and_then(|n| n.as_str()) == Some(GATEWAY_PROVIDER_ID))
    {
        Some(i) => providers[i] = entry,
        None => providers.push(entry),
    }
    root.insert(name_key, serde_yaml::Value::Sequence(providers));

    // model.provider = kiwano-gateway (create the section when absent)
    let model_key = serde_yaml::Value::String("model".into());
    if !root.get(&model_key).is_some_and(|v| v.is_mapping()) {
        root.insert(
            model_key.clone(),
            serde_yaml::Value::Mapping(Default::default()),
        );
    }
    if let Some(model) = root.get_mut(&model_key).and_then(|v| v.as_mapping_mut()) {
        model.insert(
            serde_yaml::Value::String("provider".into()),
            serde_yaml::Value::String(GATEWAY_PROVIDER_ID.into()),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root))
        .map_err(|e| format!("failed to serialize config.yaml: {e}"))
}

// ── pi (~/.pi/agent/models.json + settings.json, JSONC) ──

/// Upsert the gateway provider into `providers` of Pi's models.json. The
/// entry declares an empty model list: Pi keeps its own `defaultModel` and the
/// gateway forwards model names verbatim (documented deviation from
/// cc-switch, which manages models per provider row).
pub fn upsert_pi_models_gateway(
    content: &str,
    base_url: &str,
    key: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "models.json")?;
    let mut obj = require_object(root, "models.json")?;

    if !obj.get("providers").is_some_and(Value::is_object) {
        obj.insert("providers".into(), json!({}));
    }
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
    }

    #[test]
    fn hermes_appends_provider_and_sets_model_provider() {
        let original = "model:\n  default: anthropic/claude-opus-4-8\n  provider: openrouter\nagent:\n  max_turns: 50\ncustom_providers:\n  - name: openrouter\n    base_url: https://openrouter.ai/api/v1\n    api_key: sk-or\n";
        let out =
            upsert_hermes_gateway(original, "http://127.0.0.1:8317", "kw-ag-hermes-abcd").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"]["provider"], "kiwano-gateway");
        assert_eq!(v["model"]["default"], "anthropic/claude-opus-4-8"); // preserved
        assert_eq!(v["agent"]["max_turns"], 50); // untouched section survives
        let providers = v["custom_providers"].as_sequence().unwrap();
        assert_eq!(providers.len(), 2);
        let gw = providers
            .iter()
            .find(|p| p["name"] == "kiwano-gateway")
            .unwrap();
        assert_eq!(gw["base_url"], "http://127.0.0.1:8317");
        assert_eq!(gw["api_key"], "kw-ag-hermes-abcd");
        assert_eq!(providers[0]["name"], "openrouter"); // existing entry kept
    }

    #[test]
    fn hermes_empty_config_creates_sections() {
        let out = upsert_hermes_gateway("", "http://127.0.0.1:8317", "kw").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"]["provider"], "kiwano-gateway");
        assert_eq!(v["custom_providers"][0]["name"], "kiwano-gateway");
    }

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

    #[test]
    fn current_provider_readers_extract_selections() {
        let opencode = r#"{ "model": "deepseek/deepseek-chat", "provider": { "deepseek": { "options": { "baseURL": "https://api.deepseek.com/v1", "apiKey": "sk-ds" } } } }"#;
        let cur = read_opencode_current(opencode).unwrap();
        assert_eq!(cur.name, "deepseek");
        assert_eq!(cur.base_url, "https://api.deepseek.com/v1");
        assert_eq!(cur.api_key, "sk-ds");
        // no selector / gateway-selected / missing entry → None
        assert!(read_opencode_current("{}").is_none());
        assert!(read_opencode_current(r#"{"model":"kiwano-gateway/m"}"#).is_none());
        assert!(read_opencode_current(r#"{"model":"ghost/m"}"#).is_none());

        let openclaw = r#"{ models: { providers: { openrouter: { baseUrl: 'https://openrouter.ai/api/v1', apiKey: 'sk-or' } } }, agents: { defaults: { model: { primary: 'openrouter/claude' } } } }"#;
        let cur = read_openclaw_current(openclaw).unwrap();
        assert_eq!(cur.name, "openrouter");
        assert_eq!(cur.base_url, "https://openrouter.ai/api/v1");
        assert_eq!(cur.api_key, "sk-or");

        let hermes = "model:\n  default: m1\n  provider: openrouter\ncustom_providers:\n  - name: openrouter\n    base_url: https://openrouter.ai/api/v1\n    api_key: sk-or\n";
        let cur = read_hermes_current(hermes).unwrap();
        assert_eq!(cur.name, "openrouter");
        assert_eq!(cur.api_key, "sk-or");
        // gateway-selected → None
        assert!(read_hermes_current("model:\n  provider: kiwano-gateway\n").is_none());

        let cur = read_pi_current(
            r#"{"providers":{"pi-ai":{"baseUrl":"https://pi.ai/api","apiKey":"sk-pi"}}}"#,
            r#"{"defaultProvider":"pi-ai"}"#,
        )
        .unwrap();
        assert_eq!(cur.name, "pi-ai");
        assert_eq!(cur.base_url, "https://pi.ai/api");
        assert_eq!(cur.api_key, "sk-pi");
        assert!(read_pi_current(r#"{"providers":{}}"#, r#"{}"#).is_none());
    }
}
