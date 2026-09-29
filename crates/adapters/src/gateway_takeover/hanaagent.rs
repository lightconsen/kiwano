//! HanaAgent (OpenHanako): the gateway provider in the global
//! `provider-catalog.json`, plus the `api.provider` each agent's `config.yaml`
//! points at it.
//!
//! Two files, two jobs, and the split is the app's own: the *catalog* holds
//! provider definitions (endpoint, key, protocol, the model ids that provider
//! serves) and is global, while each agent (persona) has its own `config.yaml`
//! whose `api.provider` selects one of them. The catalogue is the one that
//! carries the endpoint: the runtime builds each model entry with the
//! provider's `base_url` (core/provider-registry.ts), so pointing the selection
//! at our entry is the whole of the rerouting.
//!
//! The catalog is owner-only (the app writes it through `writeSecretFileSync`),
//! which is why the takeover's `atomic_write_private` is not incidental here.
//! Its `meta.deletedProviders` list is another thing the app keeps: an id on it
//! stays hidden even after it is defined, so a takeover has to take our id back
//! off that list — otherwise it writes an entry the app refuses to show.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

// ── hanaagent ($HANA_HOME/provider-catalog.json + agents/*/config.yaml) ──

/// The catalog file the app reads provider definitions from. Its predecessor
/// (`added-models.yaml`) is migrated on start-up and deliberately not written
/// here: a takeover should not race that migration.
pub const HANA_CATALOG_FILE: &str = "provider-catalog.json";

/// Where the per-agent configs live, under the same root.
pub const HANA_AGENTS_DIR: &str = "agents";

/// The protocol string the app writes for an OpenAI-compatible endpoint.
const HANA_API: &str = "openai-completions";

/// The model ids one agent's `config.yaml` selects — chat, then the two tool
/// roles. They have to be declared in the provider entry, because the app
/// resolves the selected id against the provider's model list; a provider that
/// declares nothing would leave the agent with a selection it cannot resolve.
///
/// The tool roles are included even when `utility_api` names another provider:
/// an extra id in the list is inert there, a missing one is not.
pub fn hana_declared_models(config: &str) -> Vec<String> {
    let Ok(yaml) = serde_yaml::from_str::<serde_yaml::Value>(config) else {
        return Vec::new();
    };
    let Some(models) = yaml.get("models").and_then(|m| m.as_mapping()) else {
        return Vec::new();
    };
    let mut out: Vec<String> = Vec::new();
    for role in ["chat", "utility", "utility_large"] {
        let Some(id) = models
            .get(serde_yaml::Value::String(role.into()))
            .and_then(|v| v.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty())
        else {
            continue;
        };
        if !out.iter().any(|existing| existing == id) {
            out.push(id.to_string());
        }
    }
    out
}

/// Upsert the gateway provider into the catalog and take our id off the
/// deleted list. `models` is the ids the agents select, newline-separated as
/// the caller gathered them; an empty list falls back to the placeholder so
/// the entry still declares something selectable.
pub fn upsert_hana_catalog(
    content: &str,
    base_url: &str,
    key: &str,
    models: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, HANA_CATALOG_FILE)?;
    let mut obj = require_object(root, HANA_CATALOG_FILE)?;

    let mut ids: Vec<String> = models
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(str::to_string)
        .collect();
    if ids.is_empty() {
        ids.push(PLACEHOLDER_MODEL_ID.to_string());
    }

    let providers = object_field(&mut obj, HANA_CATALOG_FILE, "providers");
    providers.insert(
        GATEWAY_PROVIDER_ID.into(),
        json!({
            "display_name": GATEWAY_LABEL,
            "api": HANA_API,
            "base_url": base_url,
            "api_key": key,
            "models": ids,
        }),
    );

    // An id the user once deleted stays hidden until it comes off this list —
    // writing the entry without this would be a takeover the app refuses to
    // show. Both spellings exist: the legacy file used a top-level key, the
    // catalog keeps it under `meta`.
    let drop_ours =
        |list: &mut Vec<Value>| list.retain(|v| v.as_str() != Some(GATEWAY_PROVIDER_ID));
    if let Some(list) = obj
        .get_mut("meta")
        .and_then(|m| m.as_object_mut())
        .and_then(|m| m.get_mut("deletedProviders"))
        .and_then(|v| v.as_array_mut())
    {
        drop_ours(list);
    }
    if let Some(list) = obj
        .get_mut("_deleted_providers")
        .and_then(|v| v.as_array_mut())
    {
        drop_ours(list);
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// Point one agent's `api.provider` at the gateway entry. Its model selection
/// (`models.chat` and the tool roles) is left alone — the gateway forwards
/// model names verbatim, so a takeover changes where the request goes, not
/// which model answers it.
///
/// Comments do not survive serde_yaml's re-serialization; restore puts the
/// user's bytes back the way they were.
pub fn select_hana_agent(config: &str) -> Result<String, String> {
    let yaml: serde_yaml::Value = if config.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(config).map_err(|e| format!("config.yaml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yaml root must be a YAML mapping")?;

    let api_key = serde_yaml::Value::String("api".into());
    if !root.get(&api_key).is_some_and(|v| v.is_mapping()) {
        root.insert(
            api_key.clone(),
            serde_yaml::Value::Mapping(Default::default()),
        );
    }
    if let Some(api) = root.get_mut(&api_key).and_then(|v| v.as_mapping_mut()) {
        api.insert(
            serde_yaml::Value::String("provider".into()),
            serde_yaml::Value::String(GATEWAY_PROVIDER_ID.into()),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"agent:
  name: Hanako
api:
  provider: deepseek
models:
  chat: deepseek-chat
  utility: deepseek-chat
  utility_large: deepseek-reasoner
memory:
  enabled: true
"#;

    #[test]
    fn hana_selects_the_gateway_and_keeps_the_models() {
        let out = select_hana_agent(CONFIG).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["api"]["provider"], GATEWAY_PROVIDER_ID);
        assert_eq!(v["models"]["chat"], "deepseek-chat");
        assert_eq!(v["models"]["utility_large"], "deepseek-reasoner");
        assert_eq!(v["agent"]["name"], "Hanako");
        assert_eq!(v["memory"]["enabled"], true);
    }

    #[test]
    fn hana_declared_models_reads_the_three_roles_once_each() {
        assert_eq!(
            hana_declared_models(CONFIG),
            vec!["deepseek-chat".to_string(), "deepseek-reasoner".to_string()]
        );
        assert!(hana_declared_models("models:\n  chat: \"\"\n").is_empty());
        assert!(hana_declared_models("agent:\n  name: x\n").is_empty());
    }

    #[test]
    fn hana_upserts_the_provider_and_keeps_the_other_entries() {
        let catalog = r#"{"providers":{"deepseek":{"api_key":"sk-real","base_url":"https://api.deepseek.com"}},"meta":{"migratedAt":"2026-01-01"}}"#;
        let out = upsert_hana_catalog(
            catalog,
            "http://127.0.0.1:8317/v1",
            "kw-ag-hanaagent-abcd",
            "deepseek-chat",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let entry = &v["providers"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["base_url"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["api"], HANA_API);
        assert_eq!(entry["api_key"], "kw-ag-hanaagent-abcd");
        assert_eq!(entry["models"][0], "deepseek-chat");
        assert_eq!(v["providers"]["deepseek"]["api_key"], "sk-real");
        assert_eq!(v["meta"]["migratedAt"], "2026-01-01");
    }

    /// An id on the deleted list stays hidden however well it is defined, so
    /// the upsert has to take it off — both spellings the app has used.
    #[test]
    fn hana_clears_our_id_from_the_deleted_list() {
        let catalog = r#"{"providers":{},"meta":{"deletedProviders":["kiwano-gateway","old"]},"_deleted_providers":["kiwano-gateway"]}"#;
        let out = upsert_hana_catalog(catalog, "http://127.0.0.1:8317/v1", "kw", "").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["meta"]["deletedProviders"], json!(["old"]));
        assert_eq!(v["_deleted_providers"], json!([]));
        // An empty model list still declares something selectable.
        assert_eq!(
            v["providers"][GATEWAY_PROVIDER_ID]["models"][0],
            PLACEHOLDER_MODEL_ID
        );
    }

    #[test]
    fn hana_rerun_replaces_our_entry_rather_than_adding_one() {
        let once = upsert_hana_catalog("{}", "http://127.0.0.1:8317/v1", "kw-1", "m").unwrap();
        let twice = upsert_hana_catalog(&once, "http://127.0.0.1:8317/v1", "kw-2", "m").unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["providers"].as_object().unwrap().len(), 1);
        assert_eq!(v["providers"][GATEWAY_PROVIDER_ID]["api_key"], "kw-2");
    }
}
