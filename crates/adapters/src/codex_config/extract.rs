//! Reading a value back out of a Codex config text: the API key (`auth.json`
//! first, the config's bearer token second) and the upstream base URL.
//!
//! The read side of `repairs` and `plan` — nothing here writes.

use crate::codex_config::is_custom_codex_model_provider_id;
use crate::codex_config::provider_id::active_codex_model_provider_id;
use serde_json::Value;
use toml_edit::DocumentMut;

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
    use crate::codex_config::test_support::valid_config_text;

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
}
