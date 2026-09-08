// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/codex_config.rs
// Copied on 2026-09-07. Modified for Kiwano (Tier C function-level port: only
// the config path acquisition, atomic live-config write, TOML validation, and
// auth/base-url extraction were ported; the ~6k lines of managed-OAuth logic
// and the settings-override hook were deliberately not carried over).

//! Codex (`~/.codex`) config write core ported from cc-switch.
//!
//! Each ported function is marked with a `// Ported from cc-switch:` comment
//! naming its upstream origin. Functions not marked are Kiwano scaffolding.

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::config::{
    atomic_write, delete_file, get_home_dir, sanitize_provider_name, write_json_file,
    write_text_file,
};
use crate::error::AppError;

/// Reserved built-in provider IDs from OpenAI Codex's config/model-provider
/// catalog. Keep in sync with Codex `RESERVED_MODEL_PROVIDER_IDS` (0.149:
/// exactly these five; 0.148 is the same minus `amazon-bedrock-runtime`).
/// `oss` / `ollama-chat` are NOT reserved on 0.148/0.149 — both load as
/// ordinary custom tables — so listing them here would strand their bearer
/// token in the ignored top level. Mirror: providerConfigUtils.ts.
// Ported from cc-switch: src-tauri/src/codex_config.rs::CODEX_RESERVED_MODEL_PROVIDER_IDS
const CODEX_RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "amazon-bedrock-runtime",
    "openai",
    "ollama",
    "lmstudio",
];

/// Kiwano: stub for the removed `crate::settings::get_codex_override_dir`
/// global hook. Always `None` until the gateway wires per-call overrides.
fn get_codex_override_dir() -> Option<std::path::PathBuf> {
    None
}

/// Get the Codex config directory path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_config_dir
pub fn get_codex_config_dir() -> std::path::PathBuf {
    if let Some(custom) = get_codex_override_dir() {
        return custom;
    }

    get_home_dir().join(".codex")
}

/// Get the Codex auth.json path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_auth_path
pub fn get_codex_auth_path() -> std::path::PathBuf {
    get_codex_config_dir().join("auth.json")
}

/// Get the Codex config.toml path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_config_path
pub fn get_codex_config_path() -> std::path::PathBuf {
    get_codex_config_dir().join("config.toml")
}

/// Get the Codex provider config file paths
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_provider_paths
pub fn get_codex_provider_paths(
    provider_id: &str,
    provider_name: Option<&str>,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let base_name = provider_name
        .map(sanitize_provider_name)
        .unwrap_or_else(|| sanitize_provider_name(provider_id));

    let auth_path = get_codex_config_dir().join(format!("auth-{base_name}.json"));
    let config_path = get_codex_config_dir().join(format!("config-{base_name}.toml"));

    (auth_path, config_path)
}

/// Atomically write Codex's `auth.json` and `config.toml`, rolling back the
/// first step if the second fails
// Ported from cc-switch: src-tauri/src/codex_config.rs::write_codex_live_atomic
pub fn write_codex_live_atomic(
    auth: &Value,
    config_text_opt: Option<&str>,
) -> Result<(), AppError> {
    let auth_path = get_codex_auth_path();
    let config_path = get_codex_config_path();

    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    // Read the old contents for rollback
    let old_auth = if auth_path.exists() {
        Some(std::fs::read(&auth_path).map_err(|e| AppError::io(&auth_path, e))?)
    } else {
        None
    };

    // Prepare the content to write
    let cfg_text = match config_text_opt {
        Some(s) => s.to_string(),
        None => String::new(),
    };
    if !cfg_text.trim().is_empty() {
        toml::from_str::<toml::Table>(&cfg_text).map_err(|e| AppError::toml(&config_path, e))?;
    }

    // Step 1: write auth.json
    write_json_file(&auth_path, auth)?;

    // Step 2: write config.toml (roll back auth.json on failure)
    if let Err(e) = write_text_file(&config_path, &cfg_text) {
        // Roll back auth.json
        if let Some(bytes) = old_auth {
            let _ = atomic_write(&auth_path, &bytes);
        } else {
            let _ = delete_file(&auth_path);
        }
        return Err(e);
    }

    Ok(())
}

/// Read `~/.codex/config.toml`, returning an empty string if it does not exist
// Ported from cc-switch: src-tauri/src/codex_config.rs::read_codex_config_text
pub fn read_codex_config_text() -> Result<String, AppError> {
    let path = get_codex_config_path();
    if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))
    } else {
        Ok(String::new())
    }
}

/// Validate the syntax of non-empty TOML text
// Ported from cc-switch: src-tauri/src/codex_config.rs::validate_config_toml
pub fn validate_config_toml(text: &str) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    toml::from_str::<toml::Table>(text)
        .map(|_| ())
        .map_err(|e| AppError::toml(std::path::Path::new("config.toml"), e))
}

/// Read and validate `~/.codex/config.toml`, returning the text (possibly empty)
// Ported from cc-switch: src-tauri/src/codex_config.rs::read_and_validate_codex_config_text
pub fn read_and_validate_codex_config_text() -> Result<String, AppError> {
    let s = read_codex_config_text()?;
    validate_config_toml(&s)?;
    Ok(s)
}

/// Write only Codex `config.toml` for provider switching.
///
/// Codex login state lives in `auth.json`; provider routing, endpoint, model,
/// and provider-scoped bearer tokens live in `config.toml`. Provider switches
/// should not overwrite the user's ChatGPT login cache.
// Ported from cc-switch: src-tauri/src/codex_config.rs::write_codex_live_config_atomic
pub fn write_codex_live_config_atomic(config_text_opt: Option<&str>) -> Result<(), AppError> {
    let config_path = get_codex_config_path();
    let cfg_text = match config_text_opt {
        Some(config_text) => config_text.to_string(),
        None => String::new(),
    };

    if !cfg_text.trim().is_empty() {
        toml::from_str::<toml::Table>(&cfg_text).map_err(|e| AppError::toml(&config_path, e))?;
    }

    write_text_file(&config_path, &cfg_text)
}

pub fn extract_codex_auth_api_key(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_api_key
pub fn extract_codex_api_key(auth: Option<&Value>, config_text: Option<&str>) -> Option<String> {
    auth.and_then(extract_codex_auth_api_key)
        .or_else(|| config_text.and_then(extract_codex_experimental_bearer_token))
}

/// Extract the upstream base URL from a Codex `config.toml` string.
///
/// Prefers the active `[model_providers.<model_provider>].base_url`, falling
/// back to a top-level `base_url`. Deliberately never reads a non-active
/// `[model_providers.*]` section — the frontend `extractCodexBaseUrl`
/// (`getRecoverableBaseUrlAssignments`) excludes those too, and a leftover
/// section unrelated to the active provider must not leak into `{{baseUrl}}`.
// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_base_url
pub fn extract_codex_base_url(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;

    if let Some(active_provider) = doc.get("model_provider").and_then(|v| v.as_str()) {
        if let Some(base_url) = doc
            .get("model_providers")
            .and_then(|providers| providers.get(active_provider))
            .and_then(|provider| provider.get("base_url"))
            .and_then(|v| v.as_str())
        {
            return Some(base_url.to_string());
        }
    }

    doc.get("base_url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn active_codex_model_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::is_custom_codex_model_provider_id
pub(crate) fn is_custom_codex_model_provider_id(id: &str) -> bool {
    // Exact match, mirroring upstream: both the built-in provider lookup and
    // validate_reserved_model_provider_ids are case-sensitive, so `OpenAI`
    // etc. are legitimate custom ids whose tables must receive the token.
    // Keep in sync with the frontend list in src/utils/providerConfigUtils.ts.
    let id = id.trim();
    !id.is_empty() && !CODEX_RESERVED_MODEL_PROVIDER_IDS.contains(&id)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_experimental_bearer_token
pub fn extract_codex_experimental_bearer_token(config_text: &str) -> Option<String> {
    if !config_text.contains("experimental_bearer_token") {
        return None;
    }
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let provider_id = active_codex_model_provider_id(&doc);

    let top_level_token = || {
        doc.get("experimental_bearer_token")
            .and_then(|item| item.as_str())
    };
    let token = match provider_id.as_deref() {
        // `as_table_like` (not `as_table`): user configs may use inline tables
        // (`model_providers = { foo = {...} }`), which `as_table` rejects.
        Some(id) if is_custom_codex_model_provider_id(id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get(id))
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get("experimental_bearer_token"))
            .and_then(|item| item.as_str())
            .or_else(top_level_token),
        Some(_) => top_level_token(),
        None => top_level_token(),
    };

    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use tempfile::TempDir;

    /// Point get_home_dir()/CC_SWITCH_TEST_HOME at a temp dir for the
    /// duration of the test (env-var mutation requires serial execution).
    struct TestHomeGuard(Option<std::ffi::OsString>);

    impl TestHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::set_var("CC_SWITCH_TEST_HOME", path);
            Self(previous)
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("CC_SWITCH_TEST_HOME", v),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    fn valid_config_text() -> &'static str {
        r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
wire_api = "responses"
"#
    }

    #[test]
    #[serial]
    fn writes_valid_config_toml_and_auth_json_on_tempdir() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-codex-test" });
        write_codex_live_atomic(&auth, Some(valid_config_text())).unwrap();

        // Both live files landed under the isolated home with valid contents.
        let config_path = get_codex_config_path();
        assert_eq!(config_path, temp.path().join(".codex").join("config.toml"));
        let config_text = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(config_text, valid_config_text());

        let auth_path = get_codex_auth_path();
        let read_back: Value = crate::config::read_json_file(&auth_path).unwrap();
        assert_eq!(read_back["OPENAI_API_KEY"], "sk-codex-test");

        // Round-trip through the readers + validators.
        let text = read_and_validate_codex_config_text().unwrap();
        assert_eq!(text, valid_config_text());
    }

    #[test]
    fn validate_config_toml_accepts_valid_and_rejects_broken() {
        validate_config_toml("").unwrap();
        validate_config_toml("   \n").unwrap();
        validate_config_toml(valid_config_text()).unwrap();

        let err = validate_config_toml("not = [valid").unwrap_err();
        assert!(matches!(err, AppError::Toml { .. }));
    }

    #[test]
    #[serial]
    fn write_codex_live_atomic_rejects_invalid_toml_without_touching_auth() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let err = write_codex_live_atomic(
            &serde_json::json!({ "OPENAI_API_KEY": "sk" }),
            Some("broken = ["),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Toml { .. }));

        // The auth file must not exist: validation fails before any write.
        assert!(!get_codex_auth_path().exists());
    }

    #[test]
    #[serial]
    fn write_codex_live_config_atomic_keeps_auth_json_intact() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-login" });
        write_codex_live_atomic(&auth, Some(valid_config_text())).unwrap();

        // Provider switch rewrites only config.toml.
        write_codex_live_config_atomic(Some("model_provider = \"openai\"\n")).unwrap();
        let config = read_codex_config_text().unwrap();
        assert_eq!(config, "model_provider = \"openai\"\n");

        let auth_back: Value = crate::config::read_json_file(&get_codex_auth_path()).unwrap();
        assert_eq!(auth_back["OPENAI_API_KEY"], "sk-login");
    }

    #[test]
    fn extract_codex_base_url_prefers_active_provider_section() {
        // Top-level base_url must precede the table headers; appended after
        // them it would land inside `[model_providers.custom-1]` as a
        // duplicate key and fail to parse.
        let text = format!(
            "base_url = \"https://top-level.example.com\"\n{}",
            valid_config_text()
        );
        assert_eq!(
            extract_codex_base_url(&text).as_deref(),
            Some("https://api.example.com/v1")
        );

        // Top-level fallback when no active provider section exists. The
        // top-level key must precede any `[table]` headers, or it would be
        // parsed as a (duplicate) key inside the last table.
        assert_eq!(
            extract_codex_base_url("base_url = \"https://top-level.example.com\"\n").as_deref(),
            Some("https://top-level.example.com")
        );
        let top_level_only =
            "base_url = \"https://top-level.example.com\"\n\n[model_providers.custom-1]\nname = \"C\"\n";
        assert_eq!(
            extract_codex_base_url(top_level_only).as_deref(),
            Some("https://top-level.example.com")
        );
        assert_eq!(extract_codex_base_url(""), None);
    }

    #[test]
    fn extract_codex_api_key_reads_auth_then_bearer_fallback() {
        let auth = serde_json::json!({ "OPENAI_API_KEY": " sk-auth " });
        assert_eq!(
            extract_codex_api_key(Some(&auth), None).as_deref(),
            Some("sk-auth")
        );

        // Bearer token inside the active custom provider table.
        let config = format!(
            "{}\n[model_providers.custom-1]\nexperimental_bearer_token = \"sk-bearer\"\n",
            "model_provider = \"custom-1\"\n"
        );
        assert_eq!(
            extract_codex_api_key(None, Some(&config)).as_deref(),
            Some("sk-bearer")
        );
        assert_eq!(extract_codex_api_key(None, Some("model = \"x\"\n")), None);
    }

    #[test]
    fn codex_provider_paths_are_suffixed_per_provider() {
        let (auth, config) = get_codex_provider_paths("p1", Some("My Provider"));
        // sanitize_provider_name only replaces filesystem-forbidden
        // characters and lowercases — spaces are preserved verbatim.
        assert!(auth.ends_with("auth-my provider.json"));
        assert!(config.ends_with("config-my provider.toml"));
    }
}
