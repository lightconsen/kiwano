// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/gemini_config.rs
// Copied on 2026-09-07. Modified for Kiwano (the global settings-override hook
// is stubbed out; a per-call override directory will be wired by the gateway).

use crate::config::{get_home_dir, write_text_file};
use crate::error::AppError;
use serde_json::Value;
use std::collections::HashMap;
use std::fs;
use std::path::PathBuf;

/// Kiwano: stub for the removed `crate::settings::get_gemini_override_dir`
/// global hook. Always `None` until the gateway wires per-call overrides.
fn get_gemini_override_dir() -> Option<PathBuf> {
    None
}

/// Get the Gemini config directory path (supports a settings override)
pub fn get_gemini_dir() -> PathBuf {
    if let Some(custom) = get_gemini_override_dir() {
        return custom;
    }

    get_home_dir().join(".gemini")
}

/// Get the Gemini .env file path
pub fn get_gemini_env_path() -> PathBuf {
    get_gemini_dir().join(".env")
}

/// Parse .env file content into key-value pairs
///
/// This function parses .env files leniently, skipping invalid lines.
/// For strict validation, use `parse_env_file_strict`.
pub fn parse_env_file(content: &str) -> HashMap<String, String> {
    let mut map = HashMap::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Parse KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim().to_string();
            let value = value.trim().to_string();

            // Validate the key (non-empty, letters/digits/underscores only)
            if !key.is_empty() && key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                map.insert(key, value);
            }
        }
    }

    map
}

/// Parse .env file content strictly, returning detailed error information.
///
/// Unlike `parse_env_file`, this function returns an error on invalid lines,
/// including the line number and a detailed message.
///
/// # Errors
///
/// Returns an `AppError` in these cases:
/// - The line does not contain an `=` separator
/// - The key is empty or contains invalid characters
/// - The key does not follow environment variable naming conventions
///
/// # Use cases
///
/// Reserved for future strict-validation scenarios; the runtime currently uses
/// the lenient `parse_env_file`. Suitable for:
/// - Config import validation
/// - Strict mode for CLI tools
/// - Diagnosing config file errors
///
/// Fully covered by tests and ready to use.
#[allow(dead_code)]
pub fn parse_env_file_strict(content: &str) -> Result<HashMap<String, String>, AppError> {
    let mut map = HashMap::new();

    for (line_num, line) in content.lines().enumerate() {
        let line = line.trim();
        let line_number = line_num + 1; // line numbers are 1-based

        // Skip empty lines and comments
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        // Check for an =
        if !line.contains('=') {
            return Err(AppError::localized(
                "gemini.env.parse_error.no_equals",
                format!("Gemini .env 文件格式错误（第 {line_number} 行）：缺少 '=' 分隔符\n行内容: {line}"),
                format!("Invalid Gemini .env format (line {line_number}): missing '=' separator\nLine: {line}"),
            ));
        }

        // Parse KEY=VALUE
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            // Validate the key is not empty
            if key.is_empty() {
                return Err(AppError::localized(
                    "gemini.env.parse_error.empty_key",
                    format!("Gemini .env 文件格式错误（第 {line_number} 行）：环境变量名不能为空\n行内容: {line}"),
                    format!("Invalid Gemini .env format (line {line_number}): variable name cannot be empty\nLine: {line}"),
                ));
            }

            // Validate the key contains only letters, digits, and underscores
            if !key.chars().all(|c| c.is_alphanumeric() || c == '_') {
                return Err(AppError::localized(
                    "gemini.env.parse_error.invalid_key",
                    format!("Gemini .env 文件格式错误（第 {line_number} 行）：环境变量名只能包含字母、数字和下划线\n变量名: {key}"),
                    format!("Invalid Gemini .env format (line {line_number}): variable name can only contain letters, numbers, and underscores\nVariable: {key}"),
                ));
            }

            map.insert(key.to_string(), value.to_string());
        }
    }

    Ok(map)
}

/// Serialize key-value pairs into .env format
pub fn serialize_env_file(map: &HashMap<String, String>) -> String {
    let mut lines = Vec::new();

    // Sort keys for stable output
    let mut keys: Vec<_> = map.keys().collect();
    keys.sort();

    for key in keys {
        if let Some(value) = map.get(key) {
            lines.push(format!("{key}={value}"));
        }
    }

    lines.join("\n")
}

/// Read the Gemini .env file
pub fn read_gemini_env() -> Result<HashMap<String, String>, AppError> {
    let path = get_gemini_env_path();

    if !path.exists() {
        return Ok(HashMap::new());
    }

    let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;

    Ok(parse_env_file(&content))
}

/// Remove lines from raw .env content by matching both key and value, keeping
/// everything else verbatim.
///
/// This deliberately avoids the `parse_env_file` -> `serialize_env_file`
/// round-trip: those two functions drop comments, blank lines, unrecognized
/// lines, and duplicate definitions, and reorder the whole file by key. That
/// is fine for a full projection (the file is being rewritten anyway), but for
/// **targeted** cleanup it would silently delete the user's hand-written
/// content too.
///
/// Matching is by value rather than key alone: only the leaked copy is
/// removed, while lines the user wrote themselves with the same key but a
/// different value are kept. When a key has duplicate definitions, only the
/// matching line is deleted and the previously shadowed line takes effect
/// again — exactly the desired outcome, since the shadowing entry is the leak.
///
/// Returns `None` when no line matched, so the caller can skip writing to disk.
pub fn remove_env_entries_preserving_layout(
    content: &str,
    doomed: &HashMap<String, String>,
) -> Option<String> {
    let mut removed = false;
    let mut kept: Vec<&str> = Vec::new();

    for line in content.split('\n') {
        let trimmed = line.trim();
        let hit = !trimmed.is_empty()
            && !trimmed.starts_with('#')
            && trimmed.split_once('=').is_some_and(|(key, value)| {
                doomed
                    .get(key.trim())
                    .is_some_and(|doomed_value| doomed_value == value.trim())
            });

        if hit {
            removed = true;
        } else {
            kept.push(line);
        }
    }

    removed.then(|| kept.join("\n"))
}

/// Targetedly remove lines from `~/.gemini/.env` whose key=value match exactly; returns whether the file actually changed
pub fn remove_gemini_env_entries(doomed: &HashMap<String, String>) -> Result<bool, AppError> {
    let path = get_gemini_env_path();
    if !path.exists() {
        return Ok(false);
    }

    let content = fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))?;
    match remove_env_entries_preserving_layout(&content, doomed) {
        Some(cleaned) => {
            write_gemini_env_text_atomic(&cleaned)?;
            Ok(true)
        }
        None => Ok(false),
    }
}

/// Write the Gemini .env file (atomic)
pub fn write_gemini_env_atomic(map: &HashMap<String, String>) -> Result<(), AppError> {
    write_gemini_env_text_atomic(&serialize_env_file(map))
}

/// Write the Gemini .env file (atomic, content written verbatim)
///
/// Shares directory/file permission handling with `write_gemini_env_atomic`;
/// the only difference is that the content skips `serialize_env_file`
/// normalization — used by the order-preserving targeted removal.
pub fn write_gemini_env_text_atomic(content: &str) -> Result<(), AppError> {
    let path = get_gemini_env_path();

    // Ensure the directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;

        // Set directory permissions to 700 (owner read/write/execute only)
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(parent)
                .map_err(|e| AppError::io(parent, e))?
                .permissions();
            perms.set_mode(0o700);
            fs::set_permissions(parent, perms).map_err(|e| AppError::io(parent, e))?;
        }
    }

    write_text_file(&path, content)?;

    // Set file permissions to 600 (owner read/write only)
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(&path)
            .map_err(|e| AppError::io(&path, e))?
            .permissions();
        perms.set_mode(0o600);
        fs::set_permissions(&path, perms).map_err(|e| AppError::io(&path, e))?;
    }

    Ok(())
}

/// Convert .env format into Provider.settings_config (JSON Value)
pub fn env_to_json(env_map: &HashMap<String, String>) -> Value {
    let mut json_map = serde_json::Map::new();

    for (key, value) in env_map {
        json_map.insert(key.clone(), Value::String(value.clone()));
    }

    serde_json::json!({ "env": json_map })
}

/// Extract .env format from Provider.settings_config (JSON Value)
pub fn json_to_env(settings: &Value) -> Result<HashMap<String, String>, AppError> {
    let mut env_map = HashMap::new();

    if let Some(env_obj) = settings.get("env").and_then(|v| v.as_object()) {
        for (key, value) in env_obj {
            if let Some(val_str) = value.as_str() {
                env_map.insert(key.clone(), val_str.to_string());
            }
        }
    }

    Ok(env_map)
}

/// Validate the basic structure of a Gemini config.
///
/// This function only checks the basic config shape and does not require
/// GEMINI_API_KEY, so users can create a provider config first and fill in
/// the API key later.
///
/// API key validation happens when switching providers (via
/// `validate_gemini_settings_strict`).
pub fn validate_gemini_settings(settings: &Value) -> Result<(), AppError> {
    // Validate only the basic structure; GEMINI_API_KEY is not required
    // If an env field exists, validate that it is an object
    if let Some(env) = settings.get("env") {
        if !env.is_object() {
            return Err(AppError::localized(
                "gemini.validation.invalid_env",
                "Gemini 配置格式错误: env 必须是对象",
                "Gemini config invalid: env must be an object",
            ));
        }
    }

    // If a config field exists, validate that it is an object or null
    if let Some(config) = settings.get("config") {
        if !(config.is_object() || config.is_null()) {
            return Err(AppError::localized(
                "gemini.validation.invalid_config",
                "Gemini 配置格式错误: config 必须是对象",
                "Gemini config invalid: config must be an object",
            ));
        }
    }

    Ok(())
}

/// Validate a Gemini config strictly (required fields enforced)
///
/// Used when switching providers to ensure the config contains all required
/// fields. For providers that need an API key (e.g. PackyCode), the
/// GEMINI_API_KEY field is validated.
pub fn validate_gemini_settings_strict(settings: &Value) -> Result<(), AppError> {
    // Run the basic shape validation first (including env/config types)
    validate_gemini_settings(settings)?;

    let env_map = json_to_env(settings)?;

    // An empty env means OAuth (e.g. official Google); skip validation
    if env_map.is_empty() {
        return Ok(());
    }

    // If env is non-empty, check the required GEMINI_API_KEY field
    if !env_map.contains_key("GEMINI_API_KEY") {
        return Err(AppError::localized(
            "gemini.validation.missing_api_key",
            "Gemini 配置缺少必需字段: GEMINI_API_KEY",
            "Gemini config missing required field: GEMINI_API_KEY",
        ));
    }

    Ok(())
}

/// Get the Gemini settings.json file path
///
/// Returns `~/.gemini/settings.json` (a sibling of the `.env` file)
pub fn get_gemini_settings_path() -> PathBuf {
    get_gemini_dir().join("settings.json")
}

/// Update the security.auth.selectedType field in the Gemini directory's settings.json
///
/// This function:
/// 1. Reads the existing settings.json (if present)
/// 2. Updates only the `security.auth.selectedType` field, keeping all other fields
/// 3. Writes the file atomically
///
/// # Arguments
/// - `selected_type`: the selectedType value to set (e.g. "gemini-api-key" or "oauth-personal")
fn update_selected_type(selected_type: &str) -> Result<(), AppError> {
    let settings_path = get_gemini_settings_path();

    // Ensure the directory exists
    if let Some(parent) = settings_path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    // Read the existing settings.json (if present)
    let mut settings_content = if settings_path.exists() {
        let content =
            fs::read_to_string(&settings_path).map_err(|e| AppError::io(&settings_path, e))?;
        serde_json::from_str::<Value>(&content).unwrap_or_else(|_| serde_json::json!({}))
    } else {
        serde_json::json!({})
    };

    // Update only the security.auth.selectedType field
    if let Some(obj) = settings_content.as_object_mut() {
        let security = obj
            .entry("security")
            .or_insert_with(|| serde_json::json!({}));

        if let Some(security_obj) = security.as_object_mut() {
            let auth = security_obj
                .entry("auth")
                .or_insert_with(|| serde_json::json!({}));

            if let Some(auth_obj) = auth.as_object_mut() {
                auth_obj.insert(
                    "selectedType".to_string(),
                    Value::String(selected_type.to_string()),
                );
            }
        }
    }

    // Write the file
    crate::config::write_json_file(&settings_path, &settings_content)?;

    Ok(())
}

/// Write settings.json for Packycode Gemini providers
///
/// Sets the following in `~/.gemini/settings.json`:
/// ```json
/// {
///   "security": {
///     "auth": {
///       "selectedType": "gemini-api-key"
///     }
///   }
/// }
/// ```
///
/// All other fields in the file are preserved.
pub fn write_packycode_settings() -> Result<(), AppError> {
    update_selected_type("gemini-api-key")
}

/// Write settings.json for the official Google Gemini provider (OAuth mode)
///
/// Sets the following in `~/.gemini/settings.json`:
/// ```json
/// {
///   "security": {
///     "auth": {
///       "selectedType": "oauth-personal"
///     }
///   }
/// }
/// ```
///
/// All other fields in the file are preserved.
pub fn write_google_oauth_settings() -> Result<(), AppError> {
    update_selected_type("oauth-personal")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_env_file() {
        let content = r#"
# Comment line
GOOGLE_GEMINI_BASE_URL=https://example.com
GEMINI_API_KEY=sk-test123
GEMINI_MODEL=gemini-3.5-flash

# Another comment
"#;

        let map = parse_env_file(content);

        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("GOOGLE_GEMINI_BASE_URL"),
            Some(&"https://example.com".to_string())
        );
        assert_eq!(map.get("GEMINI_API_KEY"), Some(&"sk-test123".to_string()));
        assert_eq!(
            map.get("GEMINI_MODEL"),
            Some(&"gemini-3.5-flash".to_string())
        );
    }

    #[test]
    fn test_serialize_env_file() {
        let mut map = HashMap::new();
        map.insert("GEMINI_API_KEY".to_string(), "sk-test".to_string());
        map.insert("GEMINI_MODEL".to_string(), "gemini-3.5-flash".to_string());

        let content = serialize_env_file(&map);

        assert!(content.contains("GEMINI_API_KEY=sk-test"));
        assert!(content.contains("GEMINI_MODEL=gemini-3.5-flash"));
    }

    #[test]
    fn test_env_json_conversion() {
        let mut env_map = HashMap::new();
        env_map.insert("GEMINI_API_KEY".to_string(), "test-key".to_string());

        let json = env_to_json(&env_map);
        let converted = json_to_env(&json).unwrap();

        assert_eq!(
            converted.get("GEMINI_API_KEY"),
            Some(&"test-key".to_string())
        );
    }

    #[test]
    fn test_parse_env_file_strict_success() {
        // Test normal parsing in strict mode
        let content = r#"
# Comment line
GOOGLE_GEMINI_BASE_URL=https://example.com
GEMINI_API_KEY=sk-test123
GEMINI_MODEL=gemini-3.5-flash

# Another comment
"#;

        let result = parse_env_file_strict(content);
        assert!(result.is_ok());

        let map = result.unwrap();
        assert_eq!(map.len(), 3);
        assert_eq!(
            map.get("GOOGLE_GEMINI_BASE_URL"),
            Some(&"https://example.com".to_string())
        );
        assert_eq!(map.get("GEMINI_API_KEY"), Some(&"sk-test123".to_string()));
        assert_eq!(
            map.get("GEMINI_MODEL"),
            Some(&"gemini-3.5-flash".to_string())
        );
    }

    #[test]
    fn test_parse_env_file_strict_missing_equals() {
        // Test strict-mode detection of lines missing =
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
INVALID_LINE_WITHOUT_EQUALS
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("第 2 行") || err_msg.contains("line 2"));
        assert!(err_msg.contains("INVALID_LINE_WITHOUT_EQUALS"));
    }

    #[test]
    fn test_parse_env_file_strict_empty_key() {
        // Test strict-mode detection of an empty key
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
=value_without_key
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("第 2 行") || err_msg.contains("line 2"));
        assert!(err_msg.contains("empty") || err_msg.contains("空"));
    }

    #[test]
    fn test_parse_env_file_strict_invalid_key_characters() {
        // Test strict-mode detection of invalid characters (spaces, special symbols)
        let content = "GOOGLE_GEMINI_BASE_URL=https://example.com
INVALID KEY WITH SPACES=value
GEMINI_API_KEY=sk-test123";

        let result = parse_env_file_strict(content);
        assert!(result.is_err());

        let err = result.unwrap_err();
        let err_msg = format!("{err:?}");
        assert!(err_msg.contains("第 2 行") || err_msg.contains("line 2"));
        assert!(err_msg.contains("INVALID KEY WITH SPACES"));
    }

    #[test]
    fn test_parse_env_file_lax_vs_strict() {
        // Test the difference between lenient and strict modes
        let content = "VALID_KEY=value
INVALID LINE
KEY_WITH-DASH=value";

        // Lenient mode: skip invalid lines and keep parsing
        let lax_result = parse_env_file(content);
        assert_eq!(lax_result.len(), 1); // only VALID_KEY
        assert_eq!(lax_result.get("VALID_KEY"), Some(&"value".to_string()));

        // Strict mode: return an error immediately on an invalid line
        let strict_result = parse_env_file_strict(content);
        assert!(strict_result.is_err());
    }

    #[test]
    fn test_packycode_settings_structure() {
        // Verify the Packycode settings.json structure is correct
        let settings_content = serde_json::json!({
            "security": {
                "auth": {
                    "selectedType": "gemini-api-key"
                }
            }
        });

        assert_eq!(
            settings_content["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
    }

    #[test]
    fn test_packycode_settings_merge() {
        // Test merge logic: other fields should be preserved
        let mut existing_settings = serde_json::json!({
            "otherField": "should-be-kept",
            "security": {
                "otherSetting": "also-kept",
                "auth": {
                    "otherAuth": "preserved"
                }
            }
        });

        // Simulate updating selectedType
        if let Some(obj) = existing_settings.as_object_mut() {
            let security = obj
                .entry("security")
                .or_insert_with(|| serde_json::json!({}));

            if let Some(security_obj) = security.as_object_mut() {
                let auth = security_obj
                    .entry("auth")
                    .or_insert_with(|| serde_json::json!({}));

                if let Some(auth_obj) = auth.as_object_mut() {
                    auth_obj.insert(
                        "selectedType".to_string(),
                        Value::String("gemini-api-key".to_string()),
                    );
                }
            }
        }

        // Verify all fields are preserved
        assert_eq!(existing_settings["otherField"], "should-be-kept");
        assert_eq!(existing_settings["security"]["otherSetting"], "also-kept");
        assert_eq!(
            existing_settings["security"]["auth"]["otherAuth"],
            "preserved"
        );
        assert_eq!(
            existing_settings["security"]["auth"]["selectedType"],
            "gemini-api-key"
        );
    }

    #[test]
    fn test_google_oauth_settings_structure() {
        // Verify the Google OAuth settings.json structure is correct
        let settings_content = serde_json::json!({
            "security": {
                "auth": {
                    "selectedType": "oauth-personal"
                }
            }
        });

        assert_eq!(
            settings_content["security"]["auth"]["selectedType"],
            "oauth-personal"
        );
    }

    #[test]
    fn test_validate_empty_env_for_oauth() {
        // Test that an empty env (official Google OAuth) passes basic validation
        let settings = serde_json::json!({
            "env": {}
        });

        assert!(validate_gemini_settings(&settings).is_ok());
        // Strict validation should also pass (empty env means OAuth)
        assert!(validate_gemini_settings_strict(&settings).is_ok());
    }

    #[test]
    fn test_validate_env_with_api_key() {
        // Test that a config with an API key passes validation
        let settings = serde_json::json!({
            "env": {
                "GEMINI_API_KEY": "sk-test123",
                "GEMINI_MODEL": "gemini-3.5-flash"
            }
        });

        assert!(validate_gemini_settings(&settings).is_ok());
        assert!(validate_gemini_settings_strict(&settings).is_ok());
    }

    #[test]
    fn test_validate_env_without_api_key_relaxed() {
        // Test that a non-empty config missing the API key passes basic validation (user fills it in later)
        let settings = serde_json::json!({
            "env": {
                "GEMINI_MODEL": "gemini-3.5-flash"
            }
        });

        // Basic validation should pass (API key may be filled in later)
        assert!(validate_gemini_settings(&settings).is_ok());
        // Strict validation should fail (switching requires a complete config)
        assert!(validate_gemini_settings_strict(&settings).is_err());
    }

    #[test]
    fn test_validate_invalid_env_type() {
        // Test that a non-object env fails
        let settings = serde_json::json!({
            "env": "invalid_string"
        });

        assert!(validate_gemini_settings(&settings).is_err());
    }
}
