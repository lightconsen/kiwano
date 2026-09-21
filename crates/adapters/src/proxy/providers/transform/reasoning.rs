//! Which upstream model takes a `reasoning_effort`, and which effort to ask for.
//!
//! `is_openai_o_series` and `supports_reasoning_effort` are predicates over a
//! model name; `resolve_reasoning_effort` reads an Anthropic request body and
//! answers what OpenAI's `reasoning_effort` should be. A leaf: none of the three
//! reads a converted request.

use serde_json::Value;

/// Detect OpenAI o-series reasoning models (o1, o3, o4-mini, etc.)
/// These models require `max_completion_tokens` instead of `max_tokens`.
pub fn is_openai_o_series(model: &str) -> bool {
    model.len() > 1
        && model.starts_with('o')
        && model.as_bytes().get(1).is_some_and(|b| b.is_ascii_digit())
}

/// Detect Responses-compatible models that support reasoning effort.
///
/// Supported families:
/// - o-series: o1, o3, o4-mini, etc.
/// - GPT-5+: gpt-5, gpt-5.1, gpt-5.4, gpt-5-codex, etc.
/// - xAI Grok Build models. `grok-4.5` is the current documented Grok Build
///   model; retain the previous `grok-build-*` family for saved providers.
pub fn supports_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_lowercase();
    is_openai_o_series(&normalized)
        || normalized
            .strip_prefix("gpt-")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_digit() && c >= '5')
        || normalized == "grok-4.5"
        || normalized.starts_with("grok-4.5-")
        || normalized.starts_with("grok-build-")
}

/// Resolve the appropriate OpenAI `reasoning_effort` from an Anthropic request body.
///
/// Priority:
/// 1. Explicit `output_config.effort` — preserves the user's intent directly.
///    `low`/`medium`/`high` map 1:1; `max` maps to `xhigh`
///    (supported by mainstream GPT models). Unknown values are ignored.
/// 2. Fallback: `thinking.type` + `budget_tokens`:
///    - `adaptive` → `xhigh` (adaptive = maximum reasoning effort)
///    - `enabled` with budget → `low` (<4 000) / `medium` (4 000–15 999) / `high` (≥16 000)
///    - `enabled` without budget → `high` (conservative default)
///    - `disabled` / absent → `None`
pub fn resolve_reasoning_effort(body: &Value) -> Option<&'static str> {
    // --- Priority 1: explicit output_config.effort ---
    if let Some(effort) = body
        .pointer("/output_config/effort")
        .and_then(|v| v.as_str())
    {
        return match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "max" => Some("xhigh"), // OpenAI xhigh = maximum reasoning effort
            _ => None,              // unknown value — do not inject
        };
    }

    // --- Priority 2: thinking.type + budget_tokens fallback ---
    let thinking = body.get("thinking")?;
    match thinking.get("type").and_then(|t| t.as_str()) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = thinking.get("budget_tokens").and_then(|b| b.as_u64());
            match budget {
                Some(b) if b < 4_000 => Some("low"),
                Some(b) if b < 16_000 => Some("medium"),
                Some(_) => Some("high"),
                None => Some("high"), // enabled but no budget — assume strong reasoning
            }
        }
        _ => None, // disabled or missing
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn test_is_openai_o_series() {
        assert!(is_openai_o_series("o1"));
        assert!(is_openai_o_series("o1-preview"));
        assert!(is_openai_o_series("o1-mini"));
        assert!(is_openai_o_series("o3"));
        assert!(is_openai_o_series("o3-mini"));
        assert!(is_openai_o_series("o4-mini"));
        assert!(!is_openai_o_series("gpt-4o"));
        assert!(!is_openai_o_series("openai-gpt"));
        assert!(!is_openai_o_series("o"));
        assert!(!is_openai_o_series(""));
    }

    #[test]
    fn test_supports_reasoning_effort() {
        assert!(supports_reasoning_effort("o1"));
        assert!(supports_reasoning_effort("o3-mini"));
        assert!(supports_reasoning_effort("gpt-5"));
        assert!(supports_reasoning_effort("gpt-5.4"));
        assert!(supports_reasoning_effort("gpt-5-codex"));
        assert!(supports_reasoning_effort("grok-4.5"));
        assert!(supports_reasoning_effort("grok-build-0.1"));
        assert!(!supports_reasoning_effort("gpt-4o"));
        assert!(!supports_reasoning_effort("claude-sonnet-4-6"));
    }

    // ── resolve_reasoning_effort unit tests ──

    #[test]
    fn test_output_config_low_maps_to_reasoning_effort_low() {
        let body = json!({"output_config": {"effort": "low"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_output_config_medium_maps_to_reasoning_effort_medium() {
        let body = json!({"output_config": {"effort": "medium"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("medium"));
    }

    #[test]
    fn test_output_config_high_maps_to_reasoning_effort_high() {
        let body = json!({"output_config": {"effort": "high"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_output_config_max_maps_to_reasoning_effort_xhigh() {
        let body = json!({"output_config": {"effort": "max"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("xhigh"));
    }

    #[test]
    fn test_output_config_takes_priority_over_thinking() {
        // Even with thinking.adaptive present, explicit effort wins
        let body = json!({
            "output_config": {"effort": "low"},
            "thinking": {"type": "adaptive"}
        });
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_output_config_unknown_value_no_reasoning_effort() {
        let body = json!({"output_config": {"effort": "turbo"}});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }

    #[test]
    fn test_thinking_enabled_small_budget_maps_low() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}});
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_thinking_enabled_medium_budget_maps_medium() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 8000}});
        assert_eq!(resolve_reasoning_effort(&body), Some("medium"));
    }

    #[test]
    fn test_thinking_enabled_large_budget_maps_high() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 32000}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_thinking_enabled_without_budget_maps_high() {
        let body = json!({"thinking": {"type": "enabled"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_thinking_adaptive_maps_xhigh() {
        let body = json!({"thinking": {"type": "adaptive"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("xhigh"));
    }

    #[test]
    fn test_thinking_disabled_no_reasoning_effort() {
        let body = json!({"thinking": {"type": "disabled"}});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }

    #[test]
    fn test_no_thinking_field_no_reasoning_effort() {
        let body = json!({"messages": [{"role": "user", "content": "Hello"}]});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }
}
