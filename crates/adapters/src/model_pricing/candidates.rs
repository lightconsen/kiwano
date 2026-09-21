//! Model-id normalisation and the candidate ladder the lookup walks.
//!
//! `normalize_model_id` reduces a raw upstream id to the spelling the price
//! table is keyed by, and `pricing_candidates` enumerates the reductions of it
//! (cc-switch's BFS over id space). The ladder is order-stable — first
//! occurrence wins — and `find_with` in `table` walks it, so the strip
//! functions below are called in a fixed order and the queue is a LIFO.
//!
//! A leaf: nothing here reads a table or a document.

/// The `[1m]` suffix Claude Desktop appends to 1M-context model names
/// (same marker constant as `claude_desktop_config::ONE_M_CONTEXT_MARKER`).
const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// Placeholder ids (empty / unknown / null / none) never resolve to pricing.
pub fn is_placeholder_pricing_model(model_id: &str) -> bool {
    let normalized = model_id.trim().to_ascii_lowercase();
    normalized.is_empty() || matches!(normalized.as_str(), "unknown" | "null" | "none")
}

/// cc-switch `clean_model_id_for_pricing`: keep the last `/` path segment,
/// drop everything from the first `:`, `@`->`-`, lowercase, strip `[1m]`.
pub fn normalize_model_id(model_id: &str) -> String {
    let normalized = model_id
        .rsplit_once('/')
        .map_or(model_id, |(_, r)| r)
        .split(':')
        .next()
        .unwrap_or(model_id)
        .trim()
        .replace('@', "-")
        .to_ascii_lowercase();

    normalized
        .trim_end_matches(ONE_M_CONTEXT_MARKER)
        .trim()
        .to_string()
}

/// cc-switch `model_pricing_candidates`: BFS over id-space reductions until
/// fixpoint, order-stable (first occurrence wins).
pub fn pricing_candidates(model_id: &str) -> Vec<String> {
    let cleaned = normalize_model_id(model_id);
    if is_placeholder_pricing_model(&cleaned) {
        return Vec::new();
    }

    let mut candidates = Vec::new();
    let mut queue = vec![cleaned];

    while let Some(candidate) = queue.pop() {
        if !push_unique_candidate(&mut candidates, candidate.clone()) {
            continue;
        }

        if let Some(stripped) = strip_known_model_namespace(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_claude_desktop_non_anthropic_prefix(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_bedrock_model_version_suffix(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_model_date_suffix(&candidate) {
            queue.push(stripped);
        }
        if let Some(stripped) = strip_reasoning_effort_suffix(&candidate) {
            queue.push(stripped);
        }
        if candidate.starts_with("claude-") && candidate.contains('.') {
            queue.push(candidate.replace('.', "-"));
        }
    }

    candidates
}

fn push_unique_candidate(candidates: &mut Vec<String>, candidate: String) -> bool {
    if candidate.is_empty() || candidates.iter().any(|existing| existing == &candidate) {
        return false;
    }
    candidates.push(candidate);
    true
}

/// Strip provider namespaces: anything before the last `claude-`, or a known
/// `vendor.` prefix.
fn strip_known_model_namespace(model_id: &str) -> Option<String> {
    if let Some(pos) = model_id.rfind("claude-") {
        if pos > 0 {
            return Some(model_id[pos..].to_string());
        }
    }

    for marker in [
        "openai.",
        "anthropic.",
        "google.",
        "moonshot.",
        "moonshotai.",
        "bedrock.",
        "global.",
    ] {
        if let Some(stripped) = model_id.strip_prefix(marker) {
            return Some(stripped.to_string());
        }
    }

    None
}

/// Claude Desktop third-party entries are prefixed `claude-<vendor-model>`;
/// peel the prefix when the remainder names a known non-Anthropic family.
fn strip_claude_desktop_non_anthropic_prefix(model_id: &str) -> Option<String> {
    const NON_ANTHROPIC_MARKERS: &[&str] = &[
        "abab",
        "ark-code",
        "arctic",
        "astron",
        "codex",
        "command-r",
        "deepseek",
        "doubao",
        "ernie",
        "gemini",
        "gemma",
        "glm",
        "gpt",
        "grok",
        "hermes",
        "hy3",
        "hunyuan",
        "jamba",
        "kimi",
        "lfm",
        "llama",
        "longcat",
        "mercury",
        "mimo",
        "minimax",
        "mistral",
        "mixtral",
        "moonshot",
        "nemotron",
        "nova-",
        "openai",
        "qianfan",
        "qwen",
        "seed-",
        "solar",
        "stepfun",
    ];

    let rest = model_id.strip_prefix("claude-")?;
    NON_ANTHROPIC_MARKERS
        .iter()
        .any(|marker| rest.starts_with(marker))
        .then(|| rest.to_string())
}

/// Bedrock-style `-v<digits>` version suffix.
fn strip_bedrock_model_version_suffix(model_id: &str) -> Option<String> {
    let (base, suffix) = model_id.rsplit_once("-v")?;
    (!base.is_empty() && !suffix.is_empty() && suffix.chars().all(|c| c.is_ascii_digit()))
        .then(|| base.to_string())
}

/// Date suffixes: `-YYYY-MM-DD`, `-YYYYMMDD`, and (validated) `-YYMMDD`.
fn strip_model_date_suffix(model_id: &str) -> Option<String> {
    let bytes = model_id.as_bytes();
    if bytes.len() > 11 {
        let start = bytes.len() - 11;
        let suffix = &bytes[start..];
        let is_iso_date = suffix[0] == b'-'
            && suffix[1..5].iter().all(|b| b.is_ascii_digit())
            && suffix[5] == b'-'
            && suffix[6..8].iter().all(|b| b.is_ascii_digit())
            && suffix[8] == b'-'
            && suffix[9..11].iter().all(|b| b.is_ascii_digit());
        if is_iso_date {
            return Some(model_id[..start].to_string());
        }
    }

    let (base, suffix) = model_id.rsplit_once('-')?;
    if base.is_empty() || !suffix.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    // 8-digit YYYYMMDD (OpenAI / Claude / Qwen snapshots).
    if suffix.len() == 8 {
        return Some(base.to_string());
    }
    // 6-digit YYMMDD (Volcengine doubao-seed-*): easy to confuse with version
    // numbers, so validate month 01-12 / day 01-31 before stripping.
    if suffix.len() == 6 {
        let month: u32 = suffix[2..4].parse().unwrap_or(0);
        let day: u32 = suffix[4..6].parse().unwrap_or(0);
        if (1..=12).contains(&month) && (1..=31).contains(&day) {
            return Some(base.to_string());
        }
    }
    None
}

/// Reasoning-effort suffixes (`gpt-5.1-high` etc.).
fn strip_reasoning_effort_suffix(model_id: &str) -> Option<String> {
    for suffix in ["-minimal", "-low", "-medium", "-high", "-xhigh"] {
        if let Some(stripped) = model_id.strip_suffix(suffix) {
            if !stripped.is_empty() {
                return Some(stripped.to_string());
            }
        }
    }
    None
}

/// Prefix matching is risky, so only families with enough distinguishing
/// dashes may fall back to `candidate-%` (shortest match wins). The gated scan
/// itself is in `table`, which is why this is `pub(crate)`.
pub(crate) fn should_try_pricing_prefix_match(model_id: &str) -> bool {
    let dash_count = model_id.matches('-').count();

    if model_id.starts_with("claude-") {
        return dash_count >= 3;
    }

    if ["o1", "o3", "o4", "o5"]
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
    {
        return dash_count >= 1;
    }

    const PREFIX_MATCH_FAMILIES: &[&str] = &[
        "gpt-",
        "gemini-",
        "deepseek-",
        "qwen-",
        "glm-",
        "kimi-",
        "minimax-",
    ];

    PREFIX_MATCH_FAMILIES
        .iter()
        .any(|prefix| model_id.starts_with(prefix))
        && dash_count >= 2
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- normalize / candidates (ported cases) ---------------------------

    #[test]
    fn normalize_strips_path_tag_and_case() {
        assert_eq!(
            normalize_model_id("anthropic/claude-sonnet-4-5:beta"),
            "claude-sonnet-4-5"
        );
        assert_eq!(
            normalize_model_id("  GLM-4.6@20250101 "),
            "glm-4.6-20250101"
        );
        assert_eq!(normalize_model_id("claude-sonnet-4[1m]"), "claude-sonnet-4");
        assert!(is_placeholder_pricing_model("Unknown"));
    }

    #[test]
    fn candidates_cover_all_strip_rules() {
        // Namespace strip (non-zero position claude- wins).
        assert_eq!(
            pricing_candidates("us.anthropic.claude-sonnet-4-5"),
            vec!["us.anthropic.claude-sonnet-4-5", "claude-sonnet-4-5"]
        );
        // Claude Desktop third-party prefix.
        assert_eq!(
            pricing_candidates("claude-deepseek-chat"),
            vec!["claude-deepseek-chat", "deepseek-chat"]
        );
        // ISO date suffix.
        assert_eq!(
            pricing_candidates("claude-3-5-haiku-20241022"),
            vec!["claude-3-5-haiku-20241022", "claude-3-5-haiku"]
        );
        // Effort suffix.
        assert_eq!(
            pricing_candidates("gpt-5.1-high"),
            vec!["gpt-5.1-high", "gpt-5.1"]
        );
        // Dot normalization for claude ids.
        assert!(pricing_candidates("claude-4.5-sonnet").contains(&"claude-4-5-sonnet".to_string()));
    }

    #[test]
    fn prefix_match_gating() {
        assert!(should_try_pricing_prefix_match("claude-opus-4-8"));
        assert!(!should_try_pricing_prefix_match("claude-sonnet"));
        assert!(should_try_pricing_prefix_match("o3-2025-04-16"));
        assert!(should_try_pricing_prefix_match("deepseek-chat-x"));
        assert!(!should_try_pricing_prefix_match("deepseek-chat"));
    }
}
