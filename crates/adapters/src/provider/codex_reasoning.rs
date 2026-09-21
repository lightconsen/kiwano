//! Reasoning capability description for a Codex Responses -> Chat
//! Completions conversion.

use serde::{Deserialize, Serialize};

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
