//! OpenCode provider config structures: an AI SDK package name, its options
//! (flattened, for the fields the editor does not model) and its model map.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

/// OpenCode provider settings_config structure
///
/// OpenCode uses AI SDK package names to identify provider types, which differs
/// from the config formats of other apps. Example config:
/// ```json
/// {
///   "npm": "@ai-sdk/openai-compatible",
///   "options": { "baseURL": "https://api.example.com/v1", "apiKey": "sk-xxx" },
///   "models": { "gpt-4o": { "name": "GPT-4o" } }
/// }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeProviderConfig {
    /// AI SDK package name, e.g. "@ai-sdk/openai-compatible", "@ai-sdk/anthropic"
    pub npm: String,

    /// Provider name (optional, for display)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,

    /// Provider options (API key, base URL, etc.)
    #[serde(default)]
    pub options: OpenCodeProviderOptions,

    /// Model definition map
    #[serde(default)]
    pub models: HashMap<String, OpenCodeModel>,
}

impl Default for OpenCodeProviderConfig {
    fn default() -> Self {
        Self {
            npm: "@ai-sdk/openai-compatible".to_string(),
            name: None,
            options: OpenCodeProviderOptions::default(),
            models: HashMap::new(),
        }
    }
}

/// OpenCode provider options
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenCodeProviderOptions {
    /// API base URL
    #[serde(rename = "baseURL", skip_serializing_if = "Option::is_none")]
    pub base_url: Option<String>,

    /// API key (supports env var references, e.g. "{env:API_KEY}")
    #[serde(rename = "apiKey", skip_serializing_if = "Option::is_none")]
    pub api_key: Option<String>,

    /// Custom request headers
    #[serde(skip_serializing_if = "Option::is_none")]
    pub headers: Option<HashMap<String, String>>,

    /// Extra options (timeout, setCacheKey, etc.)
    /// Uses flatten to capture all fields not explicitly defined
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

/// OpenCode model definition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpenCodeModel {
    /// Model display name
    pub name: String,

    /// Model limits (context and output token counts)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit: Option<OpenCodeModelLimit>,

    /// Extra model options (provider routing, etc.)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<HashMap<String, Value>>,

    /// Extra fields (cost, modalities, thinking, variants, etc.)
    /// Uses flatten to capture all fields not explicitly defined
    #[serde(flatten, default, skip_serializing_if = "HashMap::is_empty")]
    pub extra: HashMap<String, Value>,
}

/// OpenCode model limits
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct OpenCodeModelLimit {
    /// Context token limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<u64>,

    /// Output token limit
    #[serde(skip_serializing_if = "Option::is_none")]
    pub output: Option<u64>,
}

#[cfg(test)]
mod tests {
    use super::OpenCodeProviderConfig;

    #[test]
    fn opencode_provider_config_defaults() {
        let config = OpenCodeProviderConfig::default();
        assert_eq!(config.npm, "@ai-sdk/openai-compatible");
        assert!(config.name.is_none());
        assert!(config.models.is_empty());
        assert!(config.options.base_url.is_none());
        assert!(config.options.api_key.is_none());
        assert!(config.options.headers.is_none());
        assert!(config.options.extra.is_empty());
    }
}
