//! Model pricing table + cost calculation, ported from cc-switch
//! (src-tauri/src/services/usage_stats.rs matching layer and
//! src-tauri/src/proxy/usage/calculator.rs, MIT License).
//!
//! Prices are USD per million tokens as TEXT decimals, plus top-level exchange
//! rates used for UI-side currency conversion. There is no bundled snapshot:
//! both the UI and the gateway read the Hub's `models.json` — through the
//! served document, or through the `model_pricing` mirror the GUI seeds from
//! it. An install that has never synced therefore has no prices at all and
//! costs read as "—", the same posture the catalog takes (`vm::load_catalog`).
//!
//! cc-switch uses rust_decimal; here prices are parsed to f64 and results are
//! rounded to 6 decimal places, which is ample for per-request USD amounts.

use serde::Deserialize;
use std::collections::HashMap;

/// The `[1m]` suffix Claude Desktop appends to 1M-context model names
/// (same marker constant as `claude_desktop_config::ONE_M_CONTEXT_MARKER`).
const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// One row of models.json (prices = currency per million tokens, TEXT decimals).
#[derive(Debug, Clone, Deserialize)]
pub struct ModelPriceEntry {
    /// The catalog provider entry this price belongs to. The Hub prices a model
    /// per provider, so the same `model_id` may appear once per provider at
    /// different rates (a subsidy, a margin, an off-peak tariff).
    ///
    /// Empty means "not specific to a provider": that is what documents and
    /// rows written before this field existed carry, and the lookup treats them
    /// as the general price.
    #[serde(default)]
    pub provider_id: String,
    pub model_id: String,
    pub display_name: String,
    pub input: String,
    pub output: String,
    pub cache_read: String,
    pub cache_creation: String,
    pub currency: String,
}

/// Top-level models.json document.
#[derive(Debug, Clone, Deserialize)]
pub struct ModelsDoc {
    pub version: i64,
    /// Informational only, so it tolerates being absent: the Hub serves this
    /// document remotely, and one omitted field must not strand every client
    /// on a parse error. `version` and `exchange_rates` stay required — their
    /// absence is a real defect, and rejecting it keeps the previous cache.
    #[serde(default)]
    pub generated_at: String,
    pub exchange_rates: HashMap<String, f64>,
    pub models: Vec<ModelPriceEntry>,
}

/// In-memory price lookup built from models.json.
///
/// Keyed by `(provider_id, model_id)`; both are normalized/lowercase and a row
/// whose `provider_id` is empty is the general price.
#[derive(Debug, Clone, Default)]
pub struct PricingTable {
    /// provider_id -> model_id -> entry.
    rows: HashMap<String, HashMap<String, ModelPriceEntry>>,
    /// model_id -> the providers that price it, ascending, which is what makes
    /// `fallback` deterministic and try the general row first ("" sorts first).
    providers_by_model: HashMap<String, Vec<String>>,
    /// Sorted model_id keys for the gated prefix scan (shortest match wins).
    keys: Vec<String>,
    /// Currency conversion rates for display (e.g. USD -> CNY).
    exchange_rates: HashMap<String, f64>,
    /// models.json version (seed-versioning key for the SQLite seeder).
    pub version: i64,
}

/// Parse a models.json document (`json.parse::<PricingTable>()`).
impl std::str::FromStr for PricingTable {
    type Err = serde_json::Error;

    fn from_str(json: &str) -> Result<Self, Self::Err> {
        let doc: ModelsDoc = serde_json::from_str(json)?;
        Ok(Self::build(doc.models, doc.exchange_rates, doc.version))
    }
}

impl PricingTable {
    /// Index the rows. Shared by the JSON document and the mirror-read path so
    /// the two can never disagree about what a key means.
    fn build(
        entries: Vec<ModelPriceEntry>,
        exchange_rates: HashMap<String, f64>,
        version: i64,
    ) -> Self {
        let mut rows: HashMap<String, HashMap<String, ModelPriceEntry>> =
            HashMap::with_capacity(entries.len());
        let mut providers_by_model: HashMap<String, Vec<String>> = HashMap::new();
        for entry in entries {
            let provider = entry.provider_id.trim().to_ascii_lowercase();
            let model = entry.model_id.trim().to_ascii_lowercase();
            rows.entry(provider.clone())
                .or_default()
                .insert(model.clone(), entry);
            providers_by_model.entry(model).or_default().push(provider);
        }
        for providers in providers_by_model.values_mut() {
            providers.sort();
            providers.dedup();
        }
        let mut keys: Vec<String> = providers_by_model.keys().cloned().collect();
        keys.sort_by_key(|k| (k.len(), k.clone()));
        Self {
            rows,
            providers_by_model,
            keys,
            exchange_rates,
            version,
        }
    }

    /// Build a table from rows read back from the `model_pricing` mirror. The
    /// gateway resolves prices in memory, so this is how a Hub-refreshed price
    /// table reaches a forwarded request. Exchange rates stay empty: they are a
    /// GUI display concern and nothing in the gateway reads them.
    ///
    /// `version` is 0 — it keys the SQLite seeder, not in-memory lookups.
    pub fn from_entries(entries: Vec<ModelPriceEntry>) -> Self {
        Self::build(entries, HashMap::new(), 0)
    }

    pub fn exchange_rates(&self) -> &HashMap<String, f64> {
        &self.exchange_rates
    }

    /// Exact-key hit: `provider`'s own row for `model`.
    fn get_exact(&self, provider: &str, model: &str) -> Option<&ModelPriceEntry> {
        self.rows
            .get(provider)
            .and_then(|by_model| by_model.get(model))
    }

    /// The general price for `model`, or failing that the lowest-`provider_id`
    /// row that prices it.
    ///
    /// This tail is tolerance, not policy. The Hub prices a model per provider
    /// entry, and a local provider that has not been matched to its catalog
    /// entry (`Provider.catalog_id`) can only be found by model. The ordering is
    /// the point: `""` is the lowest key, so the general row wins when there is
    /// one, and the rest is stable — a model-keyed table returned whichever row
    /// happened to be seeded last, so a cost could move without any price moving.
    fn fallback(&self, model: &str) -> Option<&ModelPriceEntry> {
        self.providers_by_model
            .get(model)?
            .iter()
            .find_map(|provider| self.get_exact(provider, model))
    }

    /// One id, two rungs — the provider's price, else the fallback.
    fn lookup(&self, provider: &str, model: &str) -> Option<&ModelPriceEntry> {
        self.get_exact(provider, model)
            .or_else(|| self.fallback(model))
    }

    /// Gated prefix scan: shortest table key that starts with `candidate-`,
    /// resolved for this provider. The caller applies
    /// `should_try_pricing_prefix_match` first.
    fn get_prefix(&self, provider: &str, candidate: &str) -> Option<&ModelPriceEntry> {
        let mut prefix = String::with_capacity(candidate.len() + 1);
        prefix.push_str(candidate);
        prefix.push('-');
        let key = self.keys.iter().find(|k| k.starts_with(&prefix))?;
        self.lookup(provider, key)
    }

    /// Resolve pricing for a raw (upstream) model id using cc-switch's matching
    /// ladder: exact candidate hits first, then a gated prefix scan, with the
    /// provider breaking ties at every rung.
    ///
    /// `provider_id` is the *catalog* entry id, not the local provider's row id
    /// (a provider added from the shelf is named whatever the user called it).
    /// An empty string asks for the general price, which is also what a provider
    /// with no catalog entry gets.
    pub fn find(&self, provider_id: &str, model_id: &str) -> Option<&ModelPriceEntry> {
        let provider = provider_id.trim().to_ascii_lowercase();
        let candidates = pricing_candidates(model_id);
        for candidate in &candidates {
            if let Some(entry) = self.lookup(&provider, candidate) {
                return Some(entry);
            }
        }
        for candidate in &candidates {
            if should_try_pricing_prefix_match(candidate) {
                if let Some(entry) = self.get_prefix(&provider, candidate) {
                    return Some(entry);
                }
            }
        }
        None
    }
}

/// Parse a per-million price string; invalid decimals yield no cost.
fn parse_price(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// Compute the request cost (in the entry's currency) from token counts.
///
/// `cache_inclusive` mirrors cc-switch's `calculate_for_app` semantics:
/// OpenAI/Gemini style `input_tokens` already contain the cache buckets and
/// must be reduced before billing at the input rate; Anthropic's are fresh
/// input only. Result rounded to 6 decimal places.
pub fn compute_cost(
    entry: &ModelPriceEntry,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cache_inclusive: bool,
) -> Option<f64> {
    let input_price = parse_price(&entry.input)?;
    let output_price = parse_price(&entry.output)?;
    let cache_read_price = parse_price(&entry.cache_read)?;
    let cache_creation_price = parse_price(&entry.cache_creation)?;

    let billable_input = if cache_inclusive {
        input_tokens
            .saturating_sub(cache_read_tokens)
            .saturating_sub(cache_creation_tokens)
    } else {
        input_tokens
    };

    let million = 1_000_000f64;
    let total = (billable_input as f64 * input_price
        + output_tokens as f64 * output_price
        + cache_read_tokens as f64 * cache_read_price
        + cache_creation_tokens as f64 * cache_creation_price)
        / million;

    Some((total * 1e6).round() / 1e6)
}

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
/// dashes may fall back to `candidate-%` (shortest match wins).
fn should_try_pricing_prefix_match(model_id: &str) -> bool {
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

    /// A stand-in for a Hub document. There is no bundled snapshot to read any
    /// more, so the lookup tests carry the rows they look up: one general price
    /// with a provider's own beside it, and one model priced per provider only.
    fn table() -> PricingTable {
        r#"{
            "version": 1,
            "exchange_rates": {"USD": 1.0, "CNY": 7.1},
            "models": [
                {"model_id": "claude-opus-4-8", "display_name": "Claude Opus 4.8",
                 "input": "5", "output": "25", "cache_read": "0.5",
                 "cache_creation": "6.25", "currency": "USD"},
                {"model_id": "claude-3-5-haiku", "display_name": "Claude 3.5 Haiku",
                 "input": "0.8", "output": "4", "cache_read": "0.08",
                 "cache_creation": "1", "currency": "USD"},
                {"provider_id": "zenmux", "model_id": "claude-opus-4-8",
                 "display_name": "Claude Opus 4.8 (ZenMux)", "input": "6",
                 "output": "30", "cache_read": "0.6", "cache_creation": "7.5",
                 "currency": "USD"},
                {"provider_id": "kimi", "model_id": "kimi-k2",
                 "display_name": "Kimi K2 (Kimi)", "input": "1", "output": "4",
                 "cache_read": "0.1", "cache_creation": "1", "currency": "USD"},
                {"provider_id": "moonshot", "model_id": "kimi-k2",
                 "display_name": "Kimi K2 (Moonshot)", "input": "2", "output": "8",
                 "cache_read": "0.2", "cache_creation": "2", "currency": "USD"}
            ]
        }"#
        .parse()
        .expect("fixture table parses")
    }

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

    // ---- table lookup ----------------------------------------------------

    #[test]
    fn find_resolves_exact_normalized_ids() {
        let t = table();
        let entry = t.find("", "anthropic/claude-opus-4-8:beta").unwrap();
        assert_eq!(entry.model_id, "claude-opus-4-8");
        assert_eq!(entry.currency, "USD");
    }

    #[test]
    fn find_resolves_date_snapshot_via_candidates() {
        let t = table();
        // The full snapshot id has no row of its own; the candidate pass strips
        // the date and lands on the family row.
        let entry = t.find("", "claude-3-5-haiku-20241022").unwrap();
        assert_eq!(entry.model_id, "claude-3-5-haiku");
    }

    #[test]
    fn find_unknown_model_is_none() {
        let t = table();
        assert!(t.find("", "totally-made-up-model").is_none());
        assert!(t.find("", "unknown").is_none());
        assert!(t.find("", "").is_none());
        // A known provider asking for a model nobody prices is still a miss.
        assert!(t.find("kimi", "totally-made-up-model").is_none());
    }

    /// The reason the key carries a provider at all: two providers may price the
    /// same model differently, and each request costs at its own provider's rate.
    #[test]
    fn find_uses_the_price_of_the_provider_that_was_asked_for() {
        let t = table();
        assert_eq!(t.find("kimi", "kimi-k2").unwrap().input, "1");
        assert_eq!(t.find("moonshot", "kimi-k2").unwrap().input, "2");
        // A provider's own row beats the general one for the same model.
        assert_eq!(t.find("", "claude-opus-4-8").unwrap().input, "5");
        assert_eq!(t.find("zenmux", "claude-opus-4-8").unwrap().input, "6");
    }

    /// Falling back is the point of the ladder: an unmatched provider (a manual
    /// one, or one added before it carried a catalog id) must still be costed.
    /// The order is fixed — general row first, then lowest provider_id — so a
    /// cost never depends on which row was seeded last.
    #[test]
    fn find_falls_back_to_the_general_row_then_by_provider_id() {
        let t = table();
        // claude-opus-4-8 has a general row and a zenmux one; the general row
        // is what an unlisted provider gets ("" sorts below "zenmux").
        let entry = t.find("no-such-provider", "claude-opus-4-8").unwrap();
        assert_eq!(entry.provider_id, "", "the general price, not zenmux's");
        assert_eq!(entry.input, "5");
        // claude-3-5-haiku has no provider row at all: general row either way.
        assert_eq!(t.find("zenmux", "claude-3-5-haiku").unwrap().input, "0.8");
        // kimi-k2 exists only per provider, so the fallback picks by name.
        assert_eq!(t.find("no-such-provider", "kimi-k2").unwrap().input, "1");
    }

    // ---- cost calculation (ported from calculator.rs) --------------------

    fn usage_entry() -> ModelPriceEntry {
        ModelPriceEntry {
            provider_id: String::new(),
            model_id: "test".into(),
            display_name: "Test".into(),
            input: "3.0".into(),
            output: "15.0".into(),
            cache_read: "0.3".into(),
            cache_creation: "3.75".into(),
            currency: "USD".into(),
        }
    }

    #[test]
    fn cost_anthropic_semantics_keeps_input() {
        let e = usage_entry();
        let cost = compute_cost(&e, 1000, 500, 200, 100, false).unwrap();
        // 0.003 + 0.0075 + 0.00006 + 0.000375 = 0.010935
        assert!((cost - 0.010935).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_openai_semantics_deducts_cache_buckets() {
        let e = usage_entry();
        let cost = compute_cost(&e, 1000, 500, 200, 100, true).unwrap();
        // billable input 700 -> 0.0021; total 0.010035
        assert!((cost - 0.010035).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_grokbuild_no_double_billing() {
        let e = ModelPriceEntry {
            input: "10".into(),
            output: "0".into(),
            cache_read: "1".into(),
            cache_creation: "0".into(),
            ..usage_entry()
        };
        let cost = compute_cost(&e, 1000, 0, 600, 0, true).unwrap();
        assert!((cost - 0.0046).abs() < 1e-9, "got {cost}");
    }

    #[test]
    fn cost_invalid_price_is_none() {
        let e = ModelPriceEntry {
            input: "abc".into(),
            ..usage_entry()
        };
        assert!(compute_cost(&e, 1, 1, 0, 0, false).is_none());
    }
}
