//! Where Codex keeps its live files and how they are written: the two paths
//! under `~/.codex`, the per-provider copies a provider switch writes, and the
//! atomic write plus TOML validation that switch goes through.
//!
//! A leaf: the paths resolve from the home directory the process sees, so a
//! test's injected home is the only root they read.

use crate::config::{
    atomic_write, delete_file, get_home_dir, sanitize_provider_name, write_json_file,
    write_text_file,
};
use crate::error::AppError;
use serde_json::Value;

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_config::test_support::valid_config_text;
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
    fn codex_provider_paths_are_suffixed_per_provider() {
        let (auth, config) = get_codex_provider_paths("p1", Some("My Provider"));
        // sanitize_provider_name only replaces filesystem-forbidden
        // characters and lowercases — spaces are preserved verbatim.
        assert!(auth.ends_with("auth-my provider.json"));
        assert!(config.ends_with("config-my provider.toml"));
    }
}
