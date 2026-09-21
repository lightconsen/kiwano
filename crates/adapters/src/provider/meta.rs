//! Provider metadata: the `ProviderMeta` hub, its accessors, and the
//! provider-level User-Agent parser its accessor is the single reader of.
//!
//! The hub is the one module here that imports its field types — the leaves
//! `custom_endpoint`, `auth`, `claude_desktop`, `usage`, `codex_reasoning` and
//! `local_proxy` — because metadata is where those shapes are stored.

use crate::provider::{
    AuthBinding, AuthBindingSource, ClaudeDesktopMode, ClaudeDesktopModelRoute,
    CodexChatReasoningConfig, CustomEndpoint, LocalProxyRequestOverrides, UsageScript,
};
use http::header::{HeaderValue, InvalidHeaderValue};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Provider metadata
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderMeta {
    /// Custom endpoint list (deduplicated by URL)
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub custom_endpoints: HashMap<String, CustomEndpoint>,
    /// Whether to apply the common config snippet when writing live config
    #[serde(
        rename = "commonConfigEnabled",
        skip_serializing_if = "Option::is_none"
    )]
    pub common_config_enabled: Option<bool>,
    /// Claude Desktop 3P write mode: direct or proxy (reserved)
    #[serde(rename = "claudeDesktopMode", skip_serializing_if = "Option::is_none")]
    pub claude_desktop_mode: Option<ClaudeDesktopMode>,
    /// Model route map for Claude Desktop proxy mode: Claude-safe route -> upstream model.
    #[serde(
        default,
        rename = "claudeDesktopModelRoutes",
        skip_serializing_if = "HashMap::is_empty"
    )]
    pub claude_desktop_model_routes: HashMap<String, ClaudeDesktopModelRoute>,
    /// Usage query script config
    #[serde(skip_serializing_if = "Option::is_none")]
    pub usage_script: Option<UsageScript>,
    /// Endpoint management: auto-select the best endpoint after speed testing
    #[serde(rename = "endpointAutoSelect", skip_serializing_if = "Option::is_none")]
    pub endpoint_auto_select: Option<bool>,
    /// Partner flag (frontend uses isPartner; field name kept in sync)
    #[serde(rename = "isPartner", skip_serializing_if = "Option::is_none")]
    pub is_partner: Option<bool>,
    /// Partner promotion key, used to identify special providers such as PackyCode
    #[serde(
        rename = "partnerPromotionKey",
        skip_serializing_if = "Option::is_none"
    )]
    pub partner_promotion_key: Option<String>,
    /// Cost multiplier (used to compute actual cost)
    #[serde(rename = "costMultiplier", skip_serializing_if = "Option::is_none")]
    pub cost_multiplier: Option<String>,
    /// Pricing model source (response/request)
    #[serde(rename = "pricingModelSource", skip_serializing_if = "Option::is_none")]
    pub pricing_model_source: Option<String>,
    /// Daily spend limit (USD)
    #[serde(rename = "limitDailyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_daily_usd: Option<String>,
    /// Monthly spend limit (USD)
    #[serde(rename = "limitMonthlyUsd", skip_serializing_if = "Option::is_none")]
    pub limit_monthly_usd: Option<String>,
    /// Claude API format (Claude providers only)
    /// - "anthropic": native Anthropic Messages API, passed through as-is
    /// - "openai_chat": OpenAI Chat Completions format, requires conversion
    /// - "openai_responses": OpenAI Responses API format, requires conversion
    #[serde(rename = "apiFormat", skip_serializing_if = "Option::is_none")]
    pub api_format: Option<String>,
    /// Generic auth binding (provider_config / managed_account)
    ///
    /// New code should write only this field; githubAccountId remains readable for compatibility.
    #[serde(rename = "authBinding", skip_serializing_if = "Option::is_none")]
    pub auth_binding: Option<AuthBinding>,
    /// Claude auth field name ("ANTHROPIC_AUTH_TOKEN" or "ANTHROPIC_API_KEY")
    #[serde(rename = "apiKeyField", skip_serializing_if = "Option::is_none")]
    pub api_key_field: Option<String>,
    /// Whether base_url is a complete API endpoint (no endpoint path appended)
    #[serde(rename = "isFullUrl", skip_serializing_if = "Option::is_none")]
    pub is_full_url: Option<bool>,
    /// Prompt cache key for OpenAI Responses-compatible endpoints.
    /// When set, injected into converted Responses requests to improve cache hit rate.
    /// If not set, Claude -> Responses conversions use a client-provided session/thread
    /// identity when available; generated session IDs are not sent upstream.
    #[serde(rename = "promptCacheKey", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_key: Option<String>,
    /// Session-based prompt-cache routing for Codex Responses -> Chat conversions.
    /// "auto" enables known-compatible upstreams; "enabled" / "disabled" are overrides.
    #[serde(rename = "promptCacheRouting", skip_serializing_if = "Option::is_none")]
    pub prompt_cache_routing: Option<String>,
    /// Codex OAuth FAST mode: inject `service_tier = "priority"` for ChatGPT Codex requests.
    #[serde(rename = "codexFastMode", skip_serializing_if = "Option::is_none")]
    pub codex_fast_mode: Option<bool>,
    /// Codex Responses -> Chat Completions reasoning capability metadata.
    #[serde(rename = "codexChatReasoning", skip_serializing_if = "Option::is_none")]
    pub codex_chat_reasoning: Option<CodexChatReasoningConfig>,
    /// Codex → Anthropic path: whether to emulate the Claude Code client
    /// (User-Agent / anthropic-beta / x-app + injecting the Claude Code system
    /// prompt first line). Disabled by default; only an explicit `true` enables it.
    #[serde(
        rename = "impersonateClaudeCode",
        skip_serializing_if = "Option::is_none"
    )]
    pub impersonate_claude_code: Option<bool>,
    /// Codex → Anthropic path: override the Anthropic `max_tokens` (output ceiling).
    ///
    /// Codex does not forward its `model_max_output_tokens` in the Responses
    /// request body, so without this the path falls back to a conservative
    /// default (8192), which truncates long or thinking-heavy responses
    /// (`stop_reason=max_tokens`). When set (>0), this value is injected as the
    /// request's `max_output_tokens` before conversion, taking precedence over
    /// both any request-supplied value and the default. Kept per-provider on
    /// purpose: a global large default would hard-400 on low-output-ceiling
    /// models/gateways (and that error is non-retryable).
    #[serde(rename = "maxOutputTokens", skip_serializing_if = "Option::is_none")]
    pub max_output_tokens: Option<u64>,
    /// Custom User-Agent for local proxy routing.
    #[serde(rename = "customUserAgent", skip_serializing_if = "Option::is_none")]
    pub custom_user_agent: Option<String>,
    /// Local proxy request overrides applied to the transformed upstream request.
    #[serde(
        rename = "localProxyRequestOverrides",
        skip_serializing_if = "Option::is_none"
    )]
    pub local_proxy_request_overrides: Option<LocalProxyRequestOverrides>,
    /// In additive-apply mode, whether this provider has been written to live config.
    /// `None` means legacy data/unknown state; `Some(false)` means it explicitly
    /// exists only in the database.
    #[serde(rename = "liveConfigManaged", skip_serializing_if = "Option::is_none")]
    pub live_config_managed: Option<bool>,
    /// Provider type identifier (used for special-provider detection)
    /// - "github_copilot": GitHub Copilot provider
    #[serde(rename = "providerType", skip_serializing_if = "Option::is_none")]
    pub provider_type: Option<String>,
    /// GitHub Copilot linked account ID (github_copilot providers only)
    /// Supports multiple accounts by linking to a specific GitHub account
    #[serde(rename = "githubAccountId", skip_serializing_if = "Option::is_none")]
    pub github_account_id: Option<String>,
}

/// Parse the provider-level custom User-Agent string (single source of truth).
///
/// The forwarder, stream check, and model-list fetch paths all share the same
/// semantics, avoiding inconsistencies like "one path sends the UA while
/// another doesn't / errors out".
///
/// Validity is judged by `http::HeaderValue::from_str` at the **byte** level
/// (`b >= 32 && b != 127 || b == '\t'`), strictly matching the frontend
/// `src/lib/userAgent.ts::isValidUserAgentHeader`:
/// - `Ok(None)`: unset or whitespace-only (empty after trim).
/// - `Ok(Some(hv))`: valid. Tabs, visible ASCII (0x20-0x7E), and any non-ASCII
///   characters (UTF-8 bytes are all >= 0x80) are valid.
/// - `Err(_)`: control characters only — 0x00-0x1F excluding `\t` (newlines
///   included) and 0x7F (DEL).
///
/// Invalid values are handled by **silently ignoring them on all three runtime
/// paths** (`.ok().flatten()`; one path must never error out while another
/// passes). The frontend shows a non-blocking hint at the input field.
/// Saving is currently **not blocked** — non-form paths such as deeplink
/// imports should be lenient, and the runtime silent-ignore is the safety net.
pub fn parse_custom_user_agent(
    raw: Option<&str>,
) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
    match raw.map(str::trim).filter(|s| !s.is_empty()) {
        Some(ua) => HeaderValue::from_str(ua).map(Some),
        None => Ok(None),
    }
}

impl ProviderMeta {
    /// Whether Codex OAuth FAST mode is enabled. Off by default, because
    /// `service_tier="priority"` burns the ChatGPT subscription quota at a
    /// higher rate; users must opt in explicitly in exchange for lower latency.
    pub fn codex_fast_mode_enabled(&self) -> bool {
        self.codex_fast_mode.unwrap_or(false)
    }

    /// The validated provider-level custom User-Agent. See [`parse_custom_user_agent`].
    pub fn custom_user_agent_header(&self) -> Result<Option<HeaderValue>, InvalidHeaderValue> {
        parse_custom_user_agent(self.custom_user_agent.as_deref())
    }

    /// Resolve the account ID bound to the given managed auth provider.
    ///
    /// Newer data reads authBinding first; legacy data falls back to
    /// githubAccountId.
    pub fn managed_account_id_for(&self, auth_provider: &str) -> Option<String> {
        if let Some(binding) = self.auth_binding.as_ref() {
            if binding.source == AuthBindingSource::ManagedAccount
                && binding.auth_provider.as_deref() == Some(auth_provider)
            {
                return binding.account_id.clone();
            }
        }

        if auth_provider == "github_copilot" {
            return self.github_account_id.clone();
        }

        None
    }
}

#[cfg(test)]
mod tests {
    use super::ProviderMeta;
    use crate::provider::LocalProxyRequestOverrides;
    use serde_json::json;
    use std::collections::HashMap;

    #[test]
    fn provider_meta_serializes_pricing_model_source() {
        let meta = ProviderMeta {
            pricing_model_source: Some("response".to_string()),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert_eq!(
            value
                .get("pricingModelSource")
                .and_then(|item| item.as_str()),
            Some("response")
        );
        assert!(value.get("pricing_model_source").is_none());
    }

    #[test]
    fn provider_meta_omits_pricing_model_source_when_none() {
        let meta = ProviderMeta::default();
        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");

        assert!(value.get("pricingModelSource").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_max_output_tokens() {
        let meta = ProviderMeta {
            max_output_tokens: Some(64000),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value.get("maxOutputTokens").and_then(|v| v.as_u64()),
            Some(64000)
        );
        assert!(value.get("max_output_tokens").is_none());

        let parsed: ProviderMeta = serde_json::from_value(value).expect("deserialize ProviderMeta");
        assert_eq!(parsed.max_output_tokens, Some(64000));
    }

    #[test]
    fn provider_meta_omits_max_output_tokens_when_none() {
        let value = serde_json::to_value(ProviderMeta::default()).expect("serialize ProviderMeta");
        assert!(value.get("maxOutputTokens").is_none());
    }

    #[test]
    fn provider_meta_roundtrips_local_proxy_request_overrides() {
        let meta = ProviderMeta {
            local_proxy_request_overrides: Some(LocalProxyRequestOverrides {
                headers: HashMap::from([("X-Test".to_string(), "yes".to_string())]),
                body: Some(json!({ "temperature": 0.2 })),
            }),
            ..ProviderMeta::default()
        };

        let value = serde_json::to_value(&meta).expect("serialize ProviderMeta");
        assert_eq!(
            value["localProxyRequestOverrides"]["headers"]["X-Test"],
            "yes"
        );
        assert_eq!(
            value["localProxyRequestOverrides"]["body"]["temperature"],
            0.2
        );

        let decoded: ProviderMeta =
            serde_json::from_value(value).expect("deserialize ProviderMeta");
        let overrides = decoded.local_proxy_request_overrides.unwrap();
        assert_eq!(overrides.headers.get("X-Test"), Some(&"yes".to_string()));
        assert_eq!(overrides.body.unwrap()["temperature"], 0.2);
    }
}
