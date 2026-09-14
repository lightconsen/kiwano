// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/model_mapper.rs
// Copied on 2026-09-07. Modified for Kiwano (ONE_M_CONTEXT_MARKER now imported from model_capabilities instead of claude_desktop_config).
// Trimmed on 2026-09-15: the env-driven mapping half was removed, not wired.

//! The `[1M]` marker strip, and nothing else.
//!
//! Claude Code declares 1M-context capability by suffixing the model name with
//! `[1M]`. Upstream APIs do not accept that local marker, so it is stripped
//! before forwarding — and only that. The rest of the upstream module (mapping a
//! request's model name onto a provider-configured one, per role: haiku, sonnet,
//! opus, fable, subagent) was removed rather than wired, for two reasons.
//!
//! It read its configuration out of environment variables (`ANTHROPIC_MODEL`, …),
//! which this app has no way to set or show; and Kiwano already answers the same
//! question with a column — `providers.model_default`, editable on the provider
//! and visible in its row. Two spellings of "which model does this provider
//! actually serve" would have been one too many, so seeing the ported code
//! compile was not a reason to keep it.

use crate::model_capabilities::ONE_M_CONTEXT_MARKER;
use serde_json::Value;

/// Claude Code declares 1M-context capability via the `[1M]` suffix; upstream
/// APIs usually do not accept this local capability marker, so it must be
/// stripped before forwarding.
pub fn strip_one_m_suffix_for_upstream(model: &str) -> &str {
    let trimmed = model.trim_end();
    let marker = ONE_M_CONTEXT_MARKER.as_bytes();
    let bytes = trimmed.as_bytes();
    if bytes.len() >= marker.len()
        && bytes[bytes.len() - marker.len()..].eq_ignore_ascii_case(marker)
    {
        return trimmed[..trimmed.len() - marker.len()].trim_end();
    }
    model
}

pub fn strip_one_m_suffix_for_upstream_from_body(mut body: Value) -> Value {
    let Some(model) = body.get("model").and_then(Value::as_str) else {
        return body;
    };

    let stripped = strip_one_m_suffix_for_upstream(model);
    if stripped != model {
        log::debug!("[ModelMapper] 去除本地 1M 标记: {model} → {stripped}");
        body["model"] = serde_json::json!(stripped);
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn strips_one_m_suffix_before_upstream() {
        let body = json!({"model": "deepseek-v4-pro[1M]"});
        let result = strip_one_m_suffix_for_upstream_from_body(body);
        assert_eq!(result["model"], "deepseek-v4-pro");
    }

    #[test]
    fn keeps_model_without_one_m_suffix() {
        let body = json!({"model": "deepseek-v4-pro"});
        let result = strip_one_m_suffix_for_upstream_from_body(body);
        assert_eq!(result["model"], "deepseek-v4-pro");
    }

    /// The marker is a local convention, not something the upstream ever sees, so
    /// the comparison follows the client's own spelling rather than the case it
    /// happens to have been written in — and the separator before it, if any, goes
    /// with it.
    #[test]
    fn the_marker_is_matched_case_insensitively_and_takes_its_separator() {
        assert_eq!(strip_one_m_suffix_for_upstream("m [1m]"), "m");
        assert_eq!(strip_one_m_suffix_for_upstream("m[1M] "), "m");
        assert_eq!(strip_one_m_suffix_for_upstream("m"), "m");
        // Only a suffix: a model that merely mentions it keeps its name.
        assert_eq!(
            strip_one_m_suffix_for_upstream("[1M]-prefixed"),
            "[1M]-prefixed"
        );
    }
}
