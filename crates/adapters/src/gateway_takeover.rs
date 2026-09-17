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
//! Unlike the exclusive-switch agents (claude/codex/grokbuild) whose
//! config holds one provider slot, these CLIs keep multi-provider configs, so
//! "takeover" means: upsert a `kiwano-gateway` provider entry pointing at the
//! local gateway and select it — every pre-existing provider entry survives.
//! The gateway forwards model names verbatim, so selections keep the user's
//! existing model id and only swap the provider prefix.

use serde_json::{json, Map, Value};
use toml_edit::{value, DocumentMut};

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

/// The vendor mark on every entry this family of transforms writes. It is how
/// a second takeover finds the row it wrote the first time, so enabling twice
/// updates one entry instead of taking over another.
pub const GATEWAY_VENDOR: &str = "Kiwano";

fn is_our_entry(entry: &Value) -> bool {
    entry.get("vendor").and_then(Value::as_str) == Some(GATEWAY_VENDOR)
}

/// The id an entry needs when it is created from nothing. The gateway forwards
/// model names verbatim, so this is a placeholder the user replaces with a
/// model their provider serves — only reachable when the agent's config held
/// no entry to copy from.
const PLACEHOLDER_MODEL_ID: &str = "kiwano";

/// The entry to take over in a model-list config: ours from a previous
/// takeover, else the user's first, else none (the list is empty).
///
/// The first entry rather than all of them: WorkBuddy and CodeBuddy name their
/// provider by URL inside each model row and have no provider-prefix dimension
/// to swap (unlike OpenCode's `provider/model` selector), so *adding* an entry
/// would leave two rows with one id and an ambiguous pick. Taking over the row
/// keeps one id, one meaning.
fn takeover_target_index(entries: &[Value]) -> Option<usize> {
    entries
        .iter()
        .position(is_our_entry)
        .or(if entries.is_empty() { None } else { Some(0) })
}

/// Write our URL and key onto an entry, keeping everything else — the id above
/// all, because that is the model name the request goes upstream with.
fn point_entry_at_gateway(entry: &mut Value, url: &str, key: &str) -> Result<(), String> {
    let obj = entry
        .as_object_mut()
        .ok_or("model entries must be JSON objects")?;
    obj.insert("vendor".into(), json!(GATEWAY_VENDOR));
    obj.insert("url".into(), json!(url));
    obj.insert("apiKey".into(), json!(key));
    Ok(())
}

fn new_entry(model_id: &str, url: &str, key: &str) -> Value {
    json!({
        "id": model_id,
        "name": GATEWAY_LABEL,
        "vendor": GATEWAY_VENDOR,
        "url": url,
        "apiKey": key,
        "supportsToolCall": true,
        "supportsImages": false,
    })
}

// ── workbuddy (~/.workbuddy/models.json, a bare JSON array) ──

/// Take over WorkBuddy's model list.
///
/// The file is a **bare array** — the shape its GUI writes. The published docs
/// show an object instead, which is what the CLI embedded in the app parses;
/// the GUI is what reads this file, so the array is the shape to write.
///
/// `url` is a full endpoint (WorkBuddy appends nothing), so the caller passes
/// `…/v1/chat/completions`.
pub fn upsert_workbuddy_gateway(content: &str, url: &str, key: &str) -> Result<String, String> {
    let mut entries: Vec<Value> = if content.trim().is_empty() {
        Vec::new()
    } else {
        serde_json::from_str(content)
            .map_err(|e| format!("models.json is not a JSON array: {e}"))?
    };

    match takeover_target_index(&entries) {
        Some(i) => point_entry_at_gateway(&mut entries[i], url, key)?,
        None => entries.push(new_entry(PLACEHOLDER_MODEL_ID, url, key)),
    }

    serde_json::to_string_pretty(&Value::Array(entries)).map_err(|e| e.to_string())
}

// ── codebuddy (~/.codebuddy/models.json, an object with a `models` array) ──

/// Take over CodeBuddy's model list.
///
/// Same product family as WorkBuddy, different file shape: an object carrying
/// `models` plus the `availableModels` list the picker reads, and the same
/// full-endpoint `url`. The taken-over entry's id joins `availableModels` so
/// the row is selectable; an id already listed is not duplicated.
pub fn upsert_codebuddy_models_gateway(
    content: &str,
    url: &str,
    key: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "models.json")?;
    let mut obj = require_object(root, "models.json")?;
    if !obj.get("models").is_some_and(Value::is_array) {
        obj.insert("models".into(), json!([]));
    }

    let model_id = {
        let entries = obj["models"].as_array().expect("inserted above");
        let index = takeover_target_index(entries);
        let id = index
            .and_then(|i| entries[i].get("id"))
            .and_then(Value::as_str)
            .filter(|s| !s.is_empty())
            .unwrap_or(PLACEHOLDER_MODEL_ID)
            .to_string();
        let entries = obj["models"].as_array_mut().expect("inserted above");
        match index {
            Some(i) => point_entry_at_gateway(&mut entries[i], url, key)?,
            None => entries.push(new_entry(&id, url, key)),
        }
        id
    };

    // The picker reads this list, not `models`: an id the picker does not know
    // is a row the user cannot choose.
    if !obj.get("availableModels").is_some_and(Value::is_array) {
        obj.insert("availableModels".into(), json!([]));
    }
    if let Some(list) = obj["availableModels"].as_array_mut() {
        if !list.iter().any(|m| m.as_str() == Some(model_id.as_str())) {
            list.push(json!(model_id));
        }
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

// ── kimi (~/.kimi/config.toml, and ~/.kimi-code/config.toml for its successor) ──

/// Take over Kimi CLI's config.
///
/// Unlike the JSON agents this one *adds* a provider instead of taking over an
/// existing row: Kimi names its records (`[providers.<name>]`,
/// `[models.<alias>]`), so a gateway entry and the user's own can coexist with
/// no ambiguity, and selecting ours is one line (`default_model`).
///
/// `provider_type` is the protocol name the installed generation uses —
/// `openai_legacy` for the Python CLI, `openai` for its successor — and it is
/// chosen from the config path by the caller, since only the path tells the
/// generations apart.
pub fn upsert_kimi_gateway(
    content: &str,
    base_url: &str,
    key: &str,
    provider_type: &str,
) -> Result<String, String> {
    let mut doc: DocumentMut = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse()
            .map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    };

    // The model the user is on: `default_model` is `<provider>/<model>`. Its id
    // is what goes upstream verbatim, so ours keeps it — a takeover changes
    // where the request goes, not which model answers it.
    let model_id = doc
        .get("default_model")
        .and_then(|v| v.as_str())
        .and_then(|m| m.split_once('/').map(|(_, id)| id))
        .filter(|id| !id.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();
    let alias = format!("{GATEWAY_PROVIDER_ID}/{model_id}");

    doc["providers"][GATEWAY_PROVIDER_ID]["type"] = value(provider_type);
    doc["providers"][GATEWAY_PROVIDER_ID]["base_url"] = value(base_url);
    doc["providers"][GATEWAY_PROVIDER_ID]["api_key"] = value(key);

    doc["models"][&alias]["provider"] = value(GATEWAY_PROVIDER_ID);
    doc["models"][&alias]["model"] = value(model_id.as_str());
    // Capabilities drive which features Kimi offers this model, not whether the
    // request works; copying the ones the user's own model declared keeps the
    // toggles they had.
    if let Some(caps) = doc
        .get("models")
        .and_then(|m| m.get(&alias))
        .and_then(|m| m.get("capabilities"))
        .and_then(|c| c.as_array())
        .cloned()
    {
        doc["models"][&alias]["capabilities"] = value(caps);
    }

    doc["default_model"] = value(alias.as_str());

    Ok(doc.to_string())
}

// ── qwen (~/.qwen/settings.json, JSONC) ──

/// The environment-variable name Qwen Code reads the gateway key from. Qwen
/// resolves credentials through `envKey` (a *name*, never a value), and its
/// own `env` block is a plaintext fallback inside settings.json — writing both
/// is what keeps the key out of the shell environment.
const QWEN_ENV_KEY: &str = "KIWANO_GATEWAY_KEY";

/// Take over Qwen Code's settings.
///
/// Adds a `modelProviders.openai` entry (the key names the protocol), points
/// `env` at the gateway key, and selects it via `security.auth.selectedType`
/// plus `model.name` — Qwen picks the protocol by auth type and the model by
/// name, so both have to move.
pub fn upsert_qwen_gateway(content: &str, base_url: &str, key: &str) -> Result<String, String> {
    let root = parse_jsonc(content, "settings.json")?;
    let mut obj = require_object(root, "settings.json")?;

    let model_id = obj
        .get("model")
        .and_then(|m| m.get("name"))
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();

    if !obj.get("modelProviders").is_some_and(Value::is_object) {
        obj.insert("modelProviders".into(), json!({}));
    }
    if !obj["modelProviders"]
        .get("openai")
        .is_some_and(Value::is_array)
    {
        obj["modelProviders"]["openai"] = json!([]);
    }
    let entry = json!({
        "id": model_id,
        "name": GATEWAY_LABEL,
        "envKey": QWEN_ENV_KEY,
        "baseUrl": base_url,
    });
    let providers = obj["modelProviders"]["openai"]
        .as_array_mut()
        .expect("inserted above");
    match providers
        .iter()
        .position(|p| p.get("envKey").and_then(Value::as_str) == Some(QWEN_ENV_KEY))
    {
        Some(i) => providers[i] = entry,
        None => providers.push(entry),
    }

    // The value behind `envKey`. Qwen's own `env` block is the lowest-priority
    // source it reads, which is exactly what makes it the one to write: a shell
    // export still wins, and nothing has to touch the user's shell.
    if !obj.get("env").is_some_and(Value::is_object) {
        obj.insert("env".into(), json!({}));
    }
    obj["env"][QWEN_ENV_KEY] = json!(key);

    // Selection: the protocol by auth type, the model by name.
    if !obj.get("security").is_some_and(Value::is_object) {
        obj.insert("security".into(), json!({}));
    }
    if !obj["security"].get("auth").is_some_and(Value::is_object) {
        obj["security"]["auth"] = json!({});
    }
    obj["security"]["auth"]["selectedType"] = json!("openai");

    if !obj.get("model").is_some_and(Value::is_object) {
        obj.insert("model".into(), json!({}));
    }
    obj["model"]["name"] = json!(model_id);

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

    // ── workbuddy ──

    /// The shape a real install writes (a bare array — see any
    /// `~/.workbuddy/models.json`); the published docs' object form is what
    /// the CLI embedded in the app reads, not what the GUI writes.
    #[test]
    fn workbuddy_takes_over_the_first_entry_and_keeps_the_rest() {
        let original = r#"[
  { "id": "deepseek-v4-pro", "name": "DeepSeek-V4 Pro", "vendor": "DeepSeek",
    "url": "https://api.deepseek.com/chat/completions", "apiKey": "sk-old",
    "supportsToolCall": true, "supportsImages": false },
  { "id": "kimi-k2", "vendor": "Moonshot",
    "url": "https://api.moonshot.cn/v1/chat/completions", "apiKey": "sk-kimi" }
]"#;
        let out = upsert_workbuddy_gateway(
            original,
            "http://127.0.0.1:8317/v1/chat/completions",
            "kw-ag-workbuddy-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().expect("still an array");

        assert_eq!(arr.len(), 2, "the second model is left alone");
        assert_eq!(
            arr[0]["id"], "deepseek-v4-pro",
            "the model id survives — it is what goes upstream"
        );
        assert_eq!(arr[0]["vendor"], GATEWAY_VENDOR);
        assert_eq!(arr[0]["url"], "http://127.0.0.1:8317/v1/chat/completions");
        assert_eq!(arr[0]["apiKey"], "kw-ag-workbuddy-abcd");
        assert_eq!(arr[1]["vendor"], "Moonshot", "a later entry is untouched");
        assert_eq!(arr[1]["apiKey"], "sk-kimi");
    }

    #[test]
    fn workbuddy_empty_file_creates_one_entry() {
        let out =
            upsert_workbuddy_gateway("", "http://127.0.0.1:8317/v1/chat/completions", "k").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let arr = v.as_array().expect("an array");
        assert_eq!(arr.len(), 1);
        assert_eq!(arr[0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(arr[0]["supportsToolCall"], true);
    }

    /// Enabling twice updates the row the first run wrote instead of taking
    /// over another one.
    #[test]
    fn workbuddy_second_takeover_updates_its_own_entry() {
        let url = "http://127.0.0.1:8317/v1/chat/completions";
        let once = upsert_workbuddy_gateway("[]", url, "kw-ag-workbuddy-1").unwrap();
        let twice = upsert_workbuddy_gateway(&once, url, "kw-ag-workbuddy-2").unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v.as_array().unwrap().len(), 1, "not a second entry");
        assert_eq!(v[0]["apiKey"], "kw-ag-workbuddy-2");
    }

    // ── codebuddy ──

    #[test]
    fn codebuddy_takes_over_the_first_entry_and_lists_it() {
        let original = r#"{
  "models": [
    { "id": "deepseek-v3", "name": "DeepSeek V3", "vendor": "DeepSeek",
      "apiKey": "sk-old", "url": "https://api.deepseek.com/v1/chat/completions" }
  ],
  "availableModels": []
}"#;
        let out = upsert_codebuddy_models_gateway(
            original,
            "http://127.0.0.1:8317/v1/chat/completions",
            "kw-ag-codebuddy-abcd",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["models"][0]["id"], "deepseek-v3");
        assert_eq!(v["models"][0]["vendor"], GATEWAY_VENDOR);
        assert_eq!(
            v["models"][0]["url"],
            "http://127.0.0.1:8317/v1/chat/completions"
        );
        assert_eq!(
            v["availableModels"][0], "deepseek-v3",
            "the picker lists availableModels, not models"
        );
    }

    #[test]
    fn codebuddy_empty_config_creates_both_sections() {
        let out =
            upsert_codebuddy_models_gateway("", "http://127.0.0.1:8317/v1/chat/completions", "k")
                .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["models"].as_array().unwrap().len(), 1);
        assert_eq!(v["models"][0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(v["availableModels"][0], PLACEHOLDER_MODEL_ID);
    }

    // ── kimi ──

    #[test]
    fn kimi_upserts_a_provider_and_selects_it() {
        let original = r#"default_model = "kimi-code/kimi-for-coding"
default_thinking = true

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
capabilities = ["thinking", "image_in"]

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
api_key = "sk-old"

[loop_control]
max_steps_per_turn = 50
"#;
        let out = upsert_kimi_gateway(
            original,
            "http://127.0.0.1:8317/v1",
            "kw-ag-kimi-abcd",
            "openai_legacy",
        )
        .unwrap();
        let doc: DocumentMut = out.parse().expect("output is valid TOML");
        let ours = &doc["providers"][GATEWAY_PROVIDER_ID];

        assert_eq!(
            doc["default_model"].as_str(),
            Some("kiwano-gateway/kimi-for-coding")
        );
        assert_eq!(ours["type"].as_str(), Some("openai_legacy"));
        assert_eq!(ours["base_url"].as_str(), Some("http://127.0.0.1:8317/v1"));
        assert_eq!(ours["api_key"].as_str(), Some("kw-ag-kimi-abcd"));
        assert_eq!(
            doc["models"]["kiwano-gateway/kimi-for-coding"]["model"].as_str(),
            Some("kimi-for-coding"),
            "the model id is what goes upstream"
        );
        // The user's own provider and the unrelated section survive.
        assert_eq!(
            doc["providers"]["managed:kimi-code"]["api_key"].as_str(),
            Some("sk-old")
        );
        assert_eq!(
            doc["loop_control"]["max_steps_per_turn"].as_integer(),
            Some(50)
        );
    }

    #[test]
    fn kimi_empty_config_creates_the_sections() {
        let out = upsert_kimi_gateway("", "http://127.0.0.1:8317/v1", "k", "openai").unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        assert_eq!(
            doc["providers"][GATEWAY_PROVIDER_ID]["type"].as_str(),
            Some("openai")
        );
        assert_eq!(
            doc["default_model"].as_str(),
            Some("kiwano-gateway/kiwano"),
            "nothing to copy from: the placeholder id"
        );
    }

    // ── qwen ──

    #[test]
    fn qwen_upserts_a_provider_and_selects_it() {
        let original = r#"{
  // The user's own settings, comment included.
  "model": { "name": "qwen3-coder-plus" },
  "security": { "auth": { "selectedType": "qwen-oauth" } },
  "themes": "dark"
}"#;
        let out =
            upsert_qwen_gateway(original, "http://127.0.0.1:8317/v1", "kw-ag-qwen-abcd").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["themes"], "dark");
        assert_eq!(v["security"]["auth"]["selectedType"], "openai");
        assert_eq!(
            v["model"]["name"], "qwen3-coder-plus",
            "the model name is what goes upstream"
        );
        let p = &v["modelProviders"]["openai"][0];
        assert_eq!(p["id"], "qwen3-coder-plus");
        assert_eq!(p["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(p["envKey"], QWEN_ENV_KEY);
        assert_eq!(
            v["env"][QWEN_ENV_KEY], "kw-ag-qwen-abcd",
            "the key goes in Qwen's own env block, never the user's shell"
        );
    }

    #[test]
    fn qwen_empty_config_creates_the_sections() {
        let out = upsert_qwen_gateway("", "http://127.0.0.1:8317/v1", "k").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["modelProviders"]["openai"][0]["id"], PLACEHOLDER_MODEL_ID);
        assert_eq!(v["security"]["auth"]["selectedType"], "openai");
        assert_eq!(v["env"][QWEN_ENV_KEY], "k");
    }
}
