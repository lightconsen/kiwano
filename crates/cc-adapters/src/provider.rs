// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/provider.rs
// Copied on 2026-09-07. Modified for Kiwano (inlined the `crate::settings::
// CustomEndpoint` struct, and removed `resolve_usage_credentials` which
// depended on out-of-scope modules `app_config`/`pi_config`/`codex_config`).

use http::header::{HeaderValue, InvalidHeaderValue};
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;

// SSOT mode: provider copy files are no longer written

/// Custom endpoint entry (stored deduplicated by URL).
///
/// Kiwano: inlined from cc-switch `crate::settings::CustomEndpoint` so this
/// crate does not depend on the settings module.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomEndpoint {
    pub url: String,
    pub added_at: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub last_used: Option<i64>,
}

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

/// Provider manager
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProviderManager {
    pub providers: IndexMap<String, Provider>,
    pub current: String,
}

/// Usage query script config
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageScript {
    pub enabled: bool,
    pub language: String,
    pub code: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timeout: Option<u64>,
    /// API key dedicated to usage queries (used by the generic template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "apiKey")]
    pub api_key: Option<String>,
    /// Base URL dedicated to usage queries (used by the generic and NewAPI templates)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "baseUrl")]
    pub base_url: Option<String>,
    /// Access token (for endpoints that require login; used by the NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessToken")]
    pub access_token: Option<String>,
    /// User ID (for endpoints that need a user identifier; used by the NewAPI template)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "userId")]
    pub user_id: Option<String>,
    /// Template type (lets the backend decide validation rules)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "templateType")]
    pub template_type: Option<String>,
    /// Auto-query interval (minutes; 0 disables auto queries)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "autoQueryInterval")]
    pub auto_query_interval: Option<u64>,
    /// Coding Plan provider identifier (e.g. "kimi", "zhipu", "minimax")
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "codingPlanProvider")]
    pub coding_plan_provider: Option<String>,
    /// Volcano Ark control-plane OpenAPI AccessKey ID (signs usage queries; separate credential from the inference key)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "accessKeyId")]
    pub access_key_id: Option<String>,
    /// Volcano Ark control-plane OpenAPI SecretAccessKey (same as above)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "secretAccessKey")]
    pub secret_access_key: Option<String>,
    /// Zhipu Team Plan organization ID (usage query header bigmodel-organization)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamOrganizationId")]
    pub team_organization_id: Option<String>,
    /// Zhipu Team Plan project ID (usage query header bigmodel-project)
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "teamProjectId")]
    pub team_project_id: Option<String>,
}

/// Usage data
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageData {
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "planName")]
    pub plan_name: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub extra: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "isValid")]
    pub is_valid: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    #[serde(rename = "invalidMessage")]
    pub invalid_message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub total: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub used: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub remaining: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub unit: Option<String>,
}

/// Usage query result (supports multiple plans)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UsageResult {
    pub success: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Vec<UsageData>>, // supports returning multiple plans
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

/// Auth binding source
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "snake_case")]
pub enum AuthBindingSource {
    /// Read credentials from the provider's own config (default)
    #[default]
    ProviderConfig,
    /// Use managed-account auth (e.g. GitHub Copilot OAuth)
    ManagedAccount,
}

/// Generic auth binding
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct AuthBinding {
    /// Auth source
    #[serde(default)]
    pub source: AuthBindingSource,
    /// Managed auth provider identifier (e.g. github_copilot)
    #[serde(rename = "authProvider", skip_serializing_if = "Option::is_none")]
    pub auth_provider: Option<String>,
    /// Managed account ID; empty means follow the auth provider's default account
    #[serde(rename = "accountId", skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
}

/// Claude Desktop 3P write mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ClaudeDesktopMode {
    Direct,
    Proxy,
}

/// Claude-safe model routes exposed to Desktop in local-routing mode.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ClaudeDesktopModelRoute {
    /// Real upstream model name; kept internal to CC Switch, never written to the Claude Desktop profile.
    pub model: String,
    /// Display name for the Claude Desktop model menu; written to the profile's `labelOverride`.
    #[serde(rename = "labelOverride", skip_serializing_if = "Option::is_none")]
    pub label_override: Option<String>,
    /// 1M-context capability marker recognized by Claude Desktop 3P.
    #[serde(rename = "supports1m", skip_serializing_if = "Option::is_none")]
    pub supports_1m: Option<bool>,
}

/// Reasoning capability description for Codex Responses -> Chat Completions.
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq, Eq)]
pub struct CodexChatReasoningConfig {
    #[serde(rename = "supportsThinking", skip_serializing_if = "Option::is_none")]
    pub supports_thinking: Option<bool>,
    #[serde(rename = "supportsEffort", skip_serializing_if = "Option::is_none")]
    pub supports_effort: Option<bool>,
    #[serde(rename = "thinkingParam", skip_serializing_if = "Option::is_none")]
    pub thinking_param: Option<String>,
    #[serde(rename = "effortParam", skip_serializing_if = "Option::is_none")]
    pub effort_param: Option<String>,
    #[serde(rename = "effortValueMode", skip_serializing_if = "Option::is_none")]
    pub effort_value_mode: Option<String>,
    /// Declarative field: marks where upstream reasoning is returned
    /// (reasoning_content / reasoning / reasoning_details / think_tags). The
    /// response-side `extract_reasoning_field_text` currently extracts by
    /// trying fields exhaustively and does not read this field; it is kept as
    /// documentation and reserved for future per-format dispatch (e.g. think_tags).
    #[serde(rename = "outputFormat", skip_serializing_if = "Option::is_none")]
    pub output_format: Option<String>,
    /// Runtime field (not persisted, not part of meta): the valid effort tiers
    /// the current request's model declares platform-side, looked up by resolve
    /// per request model from the provider `settings_config.modelCatalog`
    /// `reasoningLevels` (declared per model, see #6228). Only the "zen" value
    /// mapping consumes it: Some → clamp to a valid tier; None → omit the
    /// effort field (model not listed, or a toggle-style model).
    #[serde(skip)]
    pub effort_levels: Option<Vec<String>>,
}

/// Local proxy request overrides applied after route/protocol transforms.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct LocalProxyRequestOverrides {
    #[serde(default, skip_serializing_if = "HashMap::is_empty")]
    pub headers: HashMap<String, String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<serde_json::Value>,
}

impl LocalProxyRequestOverrides {
    pub fn is_empty(&self) -> bool {
        self.headers.is_empty() && self.body.is_none()
    }
}

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

impl ProviderManager {
    /// Get all providers
    pub fn get_all_providers(&self) -> &IndexMap<String, Provider> {
        &self.providers
    }
}

// ============================================================================
// Universal Provider - config shared across apps
// ============================================================================

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

// ============================================================================
// OpenCode provider config structures
// ============================================================================

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
    use super::{
        ClaudeModelConfig, CodexModelConfig, GeminiModelConfig, LocalProxyRequestOverrides,
        OpenCodeProviderConfig, Provider, ProviderManager, ProviderMeta, UniversalProvider,
    };
    use serde_json::json;
    use std::collections::HashMap;

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

    #[test]
    fn provider_manager_get_all_providers_returns_map() {
        let mut manager = ProviderManager::default();
        let provider = Provider::with_id(
            "provider-1".to_string(),
            "Provider".to_string(),
            json!({ "env": {} }),
            None,
        );
        manager.providers.insert("provider-1".to_string(), provider);

        assert_eq!(manager.get_all_providers().len(), 1);
        assert!(manager.get_all_providers().contains_key("provider-1"));
    }

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
