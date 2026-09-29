//! oh-my-pi (omp): `models.yml` gets the gateway provider entry and
//! `config.yml` gets the role selection that names it.
//!
//! omp is a Pi fork, so the split is Pi's (`pi.rs`): the provider table and the
//! selection live in two files, and the takeover writes both. What differs is
//! the format (YAML, not JSONC), the selection's shape — `modelRoles.default`,
//! one role among several, whose value is a whole `provider/model` reference —
//! and the auth: our gateway refuses a request with no placeholder key
//! (crates/gateway/src/router/mod.rs `route_agent`), so the entry has to make
//! omp send one. omp injects `Authorization: Bearer <resolved-key>` only when
//! the provider carries both `apiKey` and `authHeader: true` (its models.md),
//! which is why both are written here rather than a keyless `auth: none`.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};

// ── omp (~/.omp/agent/{config,models}.yml, .yaml accepted by the caller) ──

/// The gateway entry's api flavour: omp hands the base URL to an
/// OpenAI-completions client, the same shape its own custom providers declare.
const OMP_API: &str = "openai-completions";

/// The model id the selection named, without its provider prefix: the value is
/// a whole `provider/model` reference and the gateway forwards model names
/// verbatim, so only the prefix changes. A value with no `/` is a bare model id
/// and passes through as-is; a missing/empty role falls back to the placeholder.
///
/// Idempotent by construction: re-reading our own `kiwano-gateway/<id>` yields
/// `<id>` again.
pub fn read_omp_selected_model(config: &str) -> Option<String> {
    let yaml: serde_yaml::Value = serde_yaml::from_str(config).ok()?;
    let value = yaml
        .get("modelRoles")?
        .get("default")?
        .as_str()?
        .trim()
        .to_string();
    if value.is_empty() {
        return None;
    }
    Some(match value.split_once('/') {
        Some((_, id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => value,
    })
}

/// The id to write into both files: what the user was on, or the placeholder.
pub fn omp_model_id(config: &str) -> String {
    read_omp_selected_model(config).unwrap_or_else(|| PLACEHOLDER_MODEL_ID.to_string())
}

/// Point `modelRoles.default` at the gateway entry. Every other role (smol,
/// slow, …) and every other key survives — omp owns them and the gateway is
/// model-agnostic, so only the default role's provider prefix changes.
///
/// Comments do not survive serde_yaml's re-serialization; restore puts the
/// user's bytes back the way they were.
pub fn select_omp_gateway(config: &str, model_id: &str) -> Result<String, String> {
    let yaml: serde_yaml::Value = if config.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(config).map_err(|e| format!("config.yml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yml root must be a YAML mapping")?;

    let roles_key = serde_yaml::Value::String("modelRoles".into());
    if !root.get(&roles_key).is_some_and(|v| v.is_mapping()) {
        root.insert(
            roles_key.clone(),
            serde_yaml::Value::Mapping(Default::default()),
        );
    }
    if let Some(roles) = root.get_mut(&roles_key).and_then(|v| v.as_mapping_mut()) {
        roles.insert(
            serde_yaml::Value::String("default".into()),
            serde_yaml::Value::String(format!("{GATEWAY_PROVIDER_ID}/{model_id}")),
        );
    }

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).map_err(|e| e.to_string())
}

/// Upsert the gateway entry into `providers` of omp's models file. `config_yml`
/// is the *primary* config's original content: the model id it selects is the
/// one this entry has to declare, and reading it from the file rather than from
/// this entry keeps a re-run stable.
///
/// `contextWindow`, `thinking` and the rest are deliberately not written: omp
/// inherits them for an id it knows, and inventing numbers here would pin
/// settings the takeover has no way to verify.
pub fn upsert_omp_models_gateway(
    content: &str,
    config_yml: &str,
    base_url: &str,
    key: &str,
) -> Result<String, String> {
    let yaml: serde_yaml::Value = if content.trim().is_empty() {
        serde_yaml::Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("models.yml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("models.yml root must be a YAML mapping")?;

    let providers_key = serde_yaml::Value::String("providers".into());
    let mut providers = root
        .get(&providers_key)
        .and_then(|v| v.as_mapping())
        .cloned()
        .unwrap_or_default();

    let model_id = omp_model_id(config_yml);
    let mut model = serde_yaml::Mapping::new();
    model.insert(
        serde_yaml::Value::String("id".into()),
        serde_yaml::Value::String(model_id.clone()),
    );
    model.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String(model_id),
    );

    let mut entry = serde_yaml::Mapping::new();
    entry.insert(
        serde_yaml::Value::String("name".into()),
        serde_yaml::Value::String(GATEWAY_LABEL.into()),
    );
    entry.insert(
        serde_yaml::Value::String("baseUrl".into()),
        serde_yaml::Value::String(base_url.into()),
    );
    entry.insert(
        serde_yaml::Value::String("api".into()),
        serde_yaml::Value::String(OMP_API.into()),
    );
    entry.insert(
        serde_yaml::Value::String("apiKey".into()),
        serde_yaml::Value::String(key.into()),
    );
    entry.insert(
        serde_yaml::Value::String("auth".into()),
        serde_yaml::Value::String("apiKey".into()),
    );
    // Without this omp resolves the key but never sends it, and our gateway
    // answers every request 401 (router/mod.rs `route_agent`).
    entry.insert(
        serde_yaml::Value::String("authHeader".into()),
        serde_yaml::Value::Bool(true),
    );
    entry.insert(
        serde_yaml::Value::String("models".into()),
        serde_yaml::Value::Sequence(vec![serde_yaml::Value::Mapping(model)]),
    );

    providers.insert(
        serde_yaml::Value::String(GATEWAY_PROVIDER_ID.into()),
        serde_yaml::Value::Mapping(entry),
    );
    root.insert(providers_key, serde_yaml::Value::Mapping(providers));

    serde_yaml::to_string(&serde_yaml::Value::Mapping(root)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = r#"modelRoles:
  default: openrouter/deepseek-chat # main
  smol: openai/gpt-6-mini
theme: dark
"#;

    #[test]
    fn omp_selects_the_gateway_and_keeps_the_other_roles() {
        let out = select_omp_gateway(CONFIG, "deepseek-chat").unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["modelRoles"]["default"], "kiwano-gateway/deepseek-chat");
        assert_eq!(v["modelRoles"]["smol"], "openai/gpt-6-mini"); // untouched
        assert_eq!(v["theme"], "dark");
    }

    #[test]
    fn omp_model_id_strips_the_provider_prefix_and_is_stable() {
        assert_eq!(omp_model_id(CONFIG), "deepseek-chat");
        // A bare id has no prefix to strip.
        assert_eq!(omp_model_id("modelRoles:\n  default: gpt-6\n"), "gpt-6");
        // No role at all: the placeholder, which a re-run reads back unchanged.
        assert_eq!(omp_model_id("theme: dark\n"), PLACEHOLDER_MODEL_ID);
        let written = select_omp_gateway("theme: dark\n", PLACEHOLDER_MODEL_ID).unwrap();
        assert_eq!(omp_model_id(&written), PLACEHOLDER_MODEL_ID);
    }

    #[test]
    fn omp_upserts_the_models_entry_with_the_configs_model() {
        let models = "providers:\n  mine:\n    baseUrl: https://x\n    apiKey: literal\n";
        let out =
            upsert_omp_models_gateway(models, CONFIG, "http://127.0.0.1:8317/v1", "kw-ag-omp-abcd")
                .unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        let entry = &v["providers"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["api"], OMP_API);
        assert_eq!(entry["apiKey"], "kw-ag-omp-abcd");
        assert_eq!(entry["auth"], "apiKey");
        assert_eq!(entry["authHeader"], true);
        assert_eq!(entry["models"][0]["id"], "deepseek-chat");
        assert_eq!(entry["models"][0]["name"], "deepseek-chat");
        // The user's own provider survives, and nothing is invented for it.
        assert_eq!(v["providers"]["mine"]["baseUrl"], "https://x");
        assert!(entry.get("contextWindow").is_none());
    }

    #[test]
    fn omp_rerun_replaces_our_entry_rather_than_adding_one() {
        let once =
            upsert_omp_models_gateway("", CONFIG, "http://127.0.0.1:8317/v1", "kw-ag-omp-1111")
                .unwrap();
        let twice =
            upsert_omp_models_gateway(&once, CONFIG, "http://127.0.0.1:8317/v1", "kw-ag-omp-2222")
                .unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&twice).unwrap();
        assert_eq!(
            v["providers"].as_mapping().unwrap().len(),
            1,
            "one provider entry after a re-run"
        );
        assert_eq!(
            v["providers"][GATEWAY_PROVIDER_ID]["apiKey"],
            "kw-ag-omp-2222"
        );
    }
}
