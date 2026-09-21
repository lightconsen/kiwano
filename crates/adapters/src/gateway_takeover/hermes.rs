//! Hermes: the `custom_providers` sequence (matched by `name`) and the
//! `model.provider` that selects one.

use crate::gateway_takeover::gateway::GATEWAY_PROVIDER_ID;

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
