//! The provider entry itself: its on-disk fields, and the predicates the
//! vendor quirks are read through — the managed-account OAuth types, the
//! GitHub Copilot endpoint, and the Claude auth-field choice.

use crate::provider::ProviderMeta;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Provider struct
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    #[serde(rename = "settingsConfig")]
    pub settings_config: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "websiteUrl")]
    pub website_url: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "createdAt")]
    pub created_at: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "sortIndex")]
    pub sort_index: Option<usize>,
    /// Notes
    #[serde(skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    /// Provider metadata (not written to live config; stored only in ~/.cc-switch/config.json)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub meta: Option<ProviderMeta>,
    /// Icon name (e.g. "openai", "anthropic")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Icon color (hex, e.g. "#00A67E")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "iconColor")]
    pub icon_color: Option<String>,
    /// Whether the provider joins the failover queue
    #[serde(default)]
    #[serde(rename = "inFailoverQueue")]
    pub in_failover_queue: bool,
}

impl Provider {
    /// Create a provider with an existing ID
    pub fn with_id(
        id: String,
        name: String,
        settings_config: Value,
        website_url: Option<String>,
    ) -> Self {
        Self {
            id,
            name,
            settings_config,
            website_url,
            category: None,
            created_at: None,
            sort_index: None,
            notes: None,
            meta: None,
            icon: None,
            icon_color: None,
            in_failover_queue: false,
        }
    }

    pub fn is_codex_oauth(&self) -> bool {
        self.provider_type() == Some("codex_oauth")
    }

    pub fn is_xai_oauth(&self) -> bool {
        self.provider_type() == Some("xai_oauth")
    }

    pub fn is_github_copilot(&self) -> bool {
        self.provider_type() == Some("github_copilot")
            || self.claude_base_url_contains("githubcopilot.com")
    }

    pub fn uses_managed_account_auth(&self) -> bool {
        self.is_github_copilot()
            || self.is_codex_oauth()
            || self.is_xai_oauth()
            || self.claude_base_url_contains("chatgpt.com/backend-api/codex")
    }

    /// Third-party managed OAuth (xai_oauth, github_copilot, …): the real
    /// credential is injected per-request by the local proxy, so the card is
    /// keyless by design and its stored config is only an upstream snapshot.
    /// `codex_oauth` is deliberately excluded — the official ChatGPT login
    /// in auth.json IS its credential, so the `requires_openai_auth = true`
    /// fallback is its correct shape, never a legacy leftover.
    pub fn uses_proxy_injected_oauth(&self) -> bool {
        self.is_xai_oauth() || self.is_github_copilot()
    }

    /// Whether the provider form's "auth field" was explicitly set to
    /// ANTHROPIC_API_KEY. The form only persists `meta.apiKeyField` for the
    /// non-default choice, so `None` means the default ANTHROPIC_AUTH_TOKEN.
    pub fn claude_uses_api_key_field(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.api_key_field.as_deref())
            .map(|field| field.eq_ignore_ascii_case("ANTHROPIC_API_KEY"))
            .unwrap_or(false)
    }

    fn provider_type(&self) -> Option<&str> {
        self.meta.as_ref().and_then(|m| m.provider_type.as_deref())
    }

    fn claude_base_url_contains(&self, needle: &str) -> bool {
        self.settings_config
            .pointer("/env/ANTHROPIC_BASE_URL")
            .and_then(|value| value.as_str())
            .map(|base_url| base_url.contains(needle))
            .unwrap_or(false)
    }

    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .map(|m| m.codex_fast_mode_enabled())
            .unwrap_or(false)
    }

    pub fn has_usage_script_enabled(&self) -> bool {
        self.meta
            .as_ref()
            .and_then(|m| m.usage_script.as_ref())
            .map(|s| s.enabled)
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::Provider;
    use crate::provider::ProviderMeta;
    use serde_json::json;

    #[test]
    fn proxy_injected_oauth_excludes_codex_oauth() {
        let mut provider = Provider::with_id("p".to_string(), "P".to_string(), json!({}), None);
        assert!(!provider.uses_proxy_injected_oauth());

        for (provider_type, expected) in [
            ("xai_oauth", true),
            ("github_copilot", true),
            // the official ChatGPT login IS this card's credential — its
            // auth.json fallback shape must never be neutralized
            ("codex_oauth", false),
        ] {
            provider.meta = Some(ProviderMeta {
                provider_type: Some(provider_type.to_string()),
                ..ProviderMeta::default()
            });
            assert_eq!(
                provider.uses_proxy_injected_oauth(),
                expected,
                "{provider_type}"
            );
        }
    }

    #[test]
    fn provider_with_id_populates_defaults() {
        let settings_config = json!({
            "env": { "API_KEY": "test" }
        });
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            settings_config.clone(),
            Some("https://example.com".to_string()),
        );

        assert_eq!(provider.id, "provider-1");
        assert_eq!(provider.name, "Provider");
        assert_eq!(provider.settings_config, settings_config);
        assert_eq!(provider.website_url.as_deref(), Some("https://example.com"));
        assert!(provider.category.is_none());
        assert!(provider.created_at.is_none());
        assert!(provider.sort_index.is_none());
        assert!(provider.notes.is_none());
        assert!(provider.meta.is_none());
        assert!(provider.icon.is_none());
        assert!(provider.icon_color.is_none());
        assert!(!provider.in_failover_queue);
    }

    #[test]
    fn provider_managed_account_auth_detection_uses_type_or_known_endpoint() {
        let mut copilot = Provider::with_id(
            "copilot".to_string(),
            "Copilot".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://api.githubcopilot.com"
                }
            }),
            None,
        );
        assert!(copilot.is_github_copilot());
        assert!(copilot.uses_managed_account_auth());

        let mut codex = Provider::with_id(
            "codex".to_string(),
            "Codex".to_string(),
            json!({ "env": {} }),
            None,
        );
        codex.meta = Some(ProviderMeta {
            provider_type: Some("codex_oauth".to_string()),
            ..Default::default()
        });
        assert!(codex.is_codex_oauth());
        assert!(codex.uses_managed_account_auth());

        let codex_endpoint = Provider::with_id(
            "codex-endpoint".to_string(),
            "Codex Endpoint".to_string(),
            json!({
                "env": {
                    "ANTHROPIC_BASE_URL": "https://chatgpt.com/backend-api/codex"
                }
            }),
            None,
        );
        assert!(codex_endpoint.uses_managed_account_auth());

        copilot.meta = Some(ProviderMeta {
            provider_type: Some("github_copilot".to_string()),
            ..Default::default()
        });
        assert!(copilot.is_github_copilot());
    }
}
