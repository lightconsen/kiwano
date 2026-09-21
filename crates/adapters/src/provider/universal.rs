//! Universal provider — config shared across apps: one record whose
//! enablement flags and per-app models are lowered into the Claude, Codex and
//! Gemini provider shapes.

use crate::provider::{Provider, ProviderMeta};
use serde::{Deserialize, Serialize};

/// Per-app enablement state of a universal provider
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderApps {
    #[serde(default)]
    pub claude: bool,
    #[serde(default)]
    pub codex: bool,
    #[serde(default)]
    pub gemini: bool,
}

/// Claude model config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ClaudeModelConfig {
    /// Primary model
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Default Haiku model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "haikuModel")]
    pub haiku_model: Option<String>,
    /// Default Sonnet model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sonnetModel")]
    pub sonnet_model: Option<String>,
    /// Default Opus model
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "opusModel")]
    pub opus_model: Option<String>,
}

/// Codex model config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CodexModelConfig {
    /// Model name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    /// Reasoning effort
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "reasoningEffort")]
    pub reasoning_effort: Option<String>,
}

/// Gemini model config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct GeminiModelConfig {
    /// Model name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

/// Per-app model config
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct UniversalProviderModels {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub claude: Option<ClaudeModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub codex: Option<CodexModelConfig>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub gemini: Option<GeminiModelConfig>,
}

/// Universal provider (config shared across apps)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UniversalProvider {
    /// Unique identifier
    pub id: String,
    /// Provider name
    pub name: String,
    /// Provider type (e.g. "newapi", "custom")
    #[serde(rename = "providerType")]
    pub provider_type: String,
    /// App enablement state
    pub apps: UniversalProviderApps,
    /// API base URL
    #[serde(rename = "baseUrl")]
    pub base_url: String,
    /// API key
    #[serde(rename = "apiKey")]
    pub api_key: String,
    /// Per-app model config
    #[serde(default)]
    pub models: UniversalProviderModels,
    /// Website link
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    /// Notes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Icon name
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon color
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// Metadata
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// Creation timestamp
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    /// Sort index
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
}

impl UniversalProvider {
    /// Create a new universal provider
    pub fn new(
        id: String,
        name: String,
        provider_type: String,
        base_url: String,
        api_key: String,
    ) -> Self {
        Self {
            id,
            name,
            provider_type,
            apps: UniversalProviderApps::default(),
            base_url,
            api_key,
            models: UniversalProviderModels::default(),
            website_url: None,
            notes: None,
            icon: None,
            icon_color: None,
            meta: None,
            created_at: Some(chrono::Utc::now().timestamp_millis()),
            sort_index: None,
        }
    }

    /// Build the Claude provider config
    pub fn to_claude_provider(&self) -> Option<Provider> {
        if !self.apps.claude {
            return None;
        }

        let models = self.models.claude.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "claude-sonnet-4-20250514".to_string());
        let haiku = models
            .and_then(|m| m.haiku_model.clone())
            .unwrap_or_else(|| model.clone());
        let sonnet = models
            .and_then(|m| m.sonnet_model.clone())
            .unwrap_or_else(|| model.clone());
        let opus = models
            .and_then(|m| m.opus_model.clone())
            .unwrap_or_else(|| model.clone());

        let settings_config = serde_json::json!({
            "env": {
                "ANTHROPIC_BASE_URL": self.base_url,
                "ANTHROPIC_AUTH_TOKEN": self.api_key,
                "ANTHROPIC_MODEL": model,
                "ANTHROPIC_DEFAULT_HAIKU_MODEL": haiku,
                "ANTHROPIC_DEFAULT_SONNET_MODEL": sonnet,
                "ANTHROPIC_DEFAULT_OPUS_MODEL": opus,
            }
        });

        Some(Provider {
            id: format!("universal-claude-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// Build the Codex provider config
    pub fn to_codex_provider(&self) -> Option<Provider> {
        if !self.apps.codex {
            return None;
        }

        let models = self.models.codex.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gpt-4o".to_string());
        let reasoning_effort = models
            .and_then(|m| m.reasoning_effort.clone())
            .unwrap_or_else(|| "high".to_string());

        // A Codex/OpenAI base_url may be a bare origin (needs /v1 appended) or
        // carry a custom prefix (must not force the version segment)
        let base_trimmed = self.base_url.trim_end_matches('/');
        let origin_only = match base_trimmed.split_once("://") {
            Some((_scheme, rest)) => !rest.contains('/'),
            None => !base_trimmed.contains('/'),
        };
        let codex_base_url = if base_trimmed.ends_with("/v1") {
            base_trimmed.to_string()
        } else if origin_only {
            format!("{base_trimmed}/v1")
        } else {
            base_trimmed.to_string()
        };

        // Generate the Codex config.toml content
        let config_toml = format!(
            r#"model_provider = "custom"
model = "{model}"
model_reasoning_effort = "{reasoning_effort}"
disable_response_storage = true

[model_providers.custom]
name = "NewAPI"
base_url = "{codex_base_url}"
wire_api = "responses"
requires_openai_auth = true"#
        );

        let settings_config = serde_json::json!({
            "auth": {
                "OPENAI_API_KEY": self.api_key
            },
            "config": config_toml
        });

        Some(Provider {
            id: format!("universal-codex-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }

    /// Build the Gemini provider config
    pub fn to_gemini_provider(&self) -> Option<Provider> {
        if !self.apps.gemini {
            return None;
        }

        let models = self.models.gemini.as_ref();
        let model = models
            .and_then(|m| m.model.clone())
            .unwrap_or_else(|| "gemini-2.5-pro".to_string());

        let settings_config = serde_json::json!({
            "env": {
                "GOOGLE_GEMINI_BASE_URL": self.base_url,
                "GEMINI_API_KEY": self.api_key,
                "GEMINI_MODEL": model,
            }
        });

        Some(Provider {
            id: format!("universal-gemini-{}", self.id),
            name: self.name.clone(),
            settings_config,
            website_url: self.website_url.clone(),
            category: Some("aggregator".to_string()),
            created_at: self.created_at,
            sort_index: self.sort_index,
            notes: self.notes.clone(),
            meta: self.meta.clone(),
            icon: self.icon.clone(),
            icon_color: self.icon_color.clone(),
            in_failover_queue: false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::{ClaudeModelConfig, CodexModelConfig, GeminiModelConfig, UniversalProvider};

    #[test]
    fn universal_provider_to_claude_provider_uses_models() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.claude = true;
        universal.models.claude = Some(ClaudeModelConfig {
            model: Some("claude-main".to_string()),
            haiku_model: Some("claude-haiku".to_string()),
            sonnet_model: Some("claude-sonnet".to_string()),
            opus_model: Some("claude-opus".to_string()),
        });

        let provider = universal.to_claude_provider().expect("claude provider");

        assert_eq!(provider.id, "universal-claude-u1");
        assert_eq!(provider.name, "Universal");
        assert_eq!(provider.category.as_deref(), Some("aggregator"));
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-main")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_HAIKU_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-haiku")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_SONNET_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-sonnet")
        );
        assert_eq!(
            provider
                .settings_config
                .pointer("/env/ANTHROPIC_DEFAULT_OPUS_MODEL")
                .and_then(|item| item.as_str()),
            Some("claude-opus")
        );
    }

    #[test]
    fn universal_provider_to_claude_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_claude_provider().is_none());
    }

    #[test]
    fn universal_provider_to_codex_provider_appends_v1() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;
        universal.models.codex = Some(CodexModelConfig {
            model: Some("gpt-4o-mini".to_string()),
            reasoning_effort: Some("low".to_string()),
        });

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
        assert_eq!(
            provider
                .settings_config
                .pointer("/auth/OPENAI_API_KEY")
                .and_then(|item| item.as_str()),
            Some("api-key")
        );
    }

    #[test]
    fn universal_provider_to_codex_provider_keeps_v1_suffix() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com/v1".to_string(),
            "api-key".to_string(),
        );
        universal.apps.codex = true;

        let provider = universal.to_codex_provider().expect("codex provider");
        let config = provider
            .settings_config
            .get("config")
            .and_then(|item| item.as_str())
            .expect("config toml");

        assert!(config.contains("base_url = \"https://api.example.com/v1\""));
    }

    #[test]
    fn universal_provider_to_codex_provider_disabled_returns_none() {
        let universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );

        assert!(universal.to_codex_provider().is_none());
    }

    #[test]
    fn universal_provider_to_gemini_provider_defaults_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-2.5-pro")
        );
    }

    #[test]
    fn universal_provider_to_gemini_provider_uses_model() {
        let mut universal = UniversalProvider::new(
            "u1".to_string(),
            "Universal".to_string(),
            "newapi".to_string(),
            "https://api.example.com".to_string(),
            "api-key".to_string(),
        );
        universal.apps.gemini = true;
        universal.models.gemini = Some(GeminiModelConfig {
            model: Some("gemini-custom".to_string()),
        });

        let provider = universal.to_gemini_provider().expect("gemini provider");

        assert_eq!(
            provider
                .settings_config
                .pointer("/env/GEMINI_MODEL")
                .and_then(|item| item.as_str()),
            Some("gemini-custom")
        );
    }

    #[test]
    fn universal_codex_provider_origin_base_url_adds_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "custom".to_string(),
            "https://api.openai.com".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://api.openai.com/v1\""));
    }

    #[test]
    fn universal_codex_provider_custom_prefix_does_not_force_v1() {
        let mut p = UniversalProvider::new(
            "id".to_string(),
            "Test".to_string(),
            "custom".to_string(),
            "https://example.com/openai".to_string(),
            "sk-test".to_string(),
        );
        p.apps.codex = true;

        let provider = p.to_codex_provider().expect("should build codex provider");
        let toml = provider
            .settings_config
            .get("config")
            .and_then(|v| v.as_str())
            .expect("config should be a toml string");

        assert!(toml.contains("base_url = \"https://example.com/openai\""));
        assert!(!toml.contains("https://example.com/openai/v1"));
    }
}
