//! The current-provider readers: what an agent routes to *today*, for the
//! first-takeover import (`crates/core/src/creds.rs`) rather than for the
//! takeover, which never calls one. They are filed together by consumer,
//! not by agent; cline's reader is the exception and stays with cline,
//! because it reshapes the endpoint it reads.

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;
use crate::gateway_takeover::json::{parse_jsonc, require_object};

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

#[cfg(test)]
mod tests {
    use super::*;

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
