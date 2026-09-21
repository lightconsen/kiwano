//! Claude Desktop 3P write mode, and the Claude-safe model routes exposed to
//! it in local-routing mode.

use serde::{Deserialize, Serialize};

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
