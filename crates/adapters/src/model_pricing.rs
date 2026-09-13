//! Model pricing table + cost calculation, ported from cc-switch
//! (src-tauri/src/services/usage_stats.rs matching layer and
//! src-tauri/src/proxy/usage/calculator.rs, MIT License).
//!
//! Prices are per million tokens as TEXT decimals in the row's own currency,
//! plus top-level exchange rates used for the Dashboard's display conversion.
//! There is no bundled snapshot: both the UI and the gateway read the Hub's
//! `models.json` — through the served document, or through the `model_pricing`
//! mirror the GUI seeds from it. An install that has never synced therefore has
//! no prices at all and costs read as "—", the same posture the catalog takes
//! (`vm::load_catalog`).
//!
//! A row may also publish **time-of-day** pricing: its own rates are then the
//! peak ones, applied inside `peak_hours` in the *vendor's* clock, with
//! `off_peak` in force outside them (`is_peak`, `compute_cost_pair`).
//!
//! cc-switch uses rust_decimal; here prices are parsed to f64 and results are
//! rounded to 6 decimal places, which is ample for per-request USD amounts.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The `[1m]` suffix Claude Desktop appends to 1M-context model names
/// (same marker constant as `claude_desktop_config::ONE_M_CONTEXT_MARKER`).
const ONE_M_CONTEXT_MARKER: &str = "[1m]";

/// A row's rates outside its peak hours.
///
/// The document spells these `in` / `out` — short keys, and not ours to rename:
/// the published shape is what the Hub and every other client share. The rates
/// are read as they are written, so a vendor that discounts input but not output
/// is expressible.
///
/// Both cache rates are defaulted rather than required: `models/README.md` says
/// an absent cache field means 0, and a parse failure here is not a local
/// problem — it fails the whole document, which the seeder answers by keeping
/// the *previous* prices for every model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OffPeakRates {
    #[serde(rename = "in")]
    pub input: String,
    #[serde(rename = "out")]
    pub output: String,
    #[serde(default = "zero_rate")]
    pub cache_read: String,
    #[serde(default = "zero_rate")]
    pub cache_creation: String,
}

/// The cache rates the document leaves out default to 0, as the data repo's
/// README states — not to the peak rate, which would invent a charge.
fn zero_rate() -> String {
    "0".to_string()
}

/// When a row's own (peak) rates apply, in the **vendor's** clock.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeakHours {
    /// Minutes east of UTC. Required when a schedule is published: these windows
    /// are business hours somewhere, and judging "is it peak now" against the
    /// reader's own timezone would silently pick the wrong rate.
    #[serde(default)]
    pub tz_offset: i32,
    #[serde(default)]
    pub windows: Vec<PeakWindow>,
}

/// One peak window. `start`/`end` are `HH:MM` in the vendor's clock and the
/// window is half-open — `[start, end)` — which is what lets adjacent windows
/// compose without a gap or an overlap. A window never wraps midnight; the
/// document writes `22:00–02:00` as two windows.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PeakWindow {
    #[serde(default)]
    pub days: Vec<String>,
    #[serde(default)]
    pub start: String,
    #[serde(default)]
    pub end: String,
}

/// The two halves of a time-of-day price, as they are stored between the
/// document and the mirror: one JSON blob, so the seeder and the mirror reader
/// cannot disagree about the shape.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PriceTiers {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_peak: Option<OffPeakRates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_hours: Option<PeakHours>,
}

/// One row of models.json (prices = currency per million tokens, TEXT decimals).
#[derive(Debug, Clone, Deserialize, Serialize)]
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
    /// The discounted rates in force outside `peak_hours`. The row's own rates
    /// above are the **peak** ones — they apply inside the windows.
    #[serde(default)]
    pub off_peak: Option<OffPeakRates>,
    #[serde(default)]
    pub peak_hours: Option<PeakHours>,
}

impl ModelPriceEntry {
    /// The tiers as they travel to the mirror, or `None` when the row has none.
    /// "No tiers" is NULL rather than `"{}"`, so a forced re-seed of a tiered
    /// document stays write-free for the rows that never had any.
    pub fn tiers_json(&self) -> Option<String> {
        let tiers = PriceTiers {
            off_peak: self.off_peak.clone(),
            peak_hours: self.peak_hours.clone(),
        };
        if tiers.off_peak.is_none() && tiers.peak_hours.is_none() {
            return None;
        }
        serde_json::to_string(&tiers).ok()
    }

    /// Read the tiers back from the mirror, **tolerating** a blob this build
    /// cannot parse: an unreadable row means peak-only for that row, never an
    /// error. An error here would reach `resolve_pricing`, which answers a read
    /// failure with an empty table — every cost NULL, which is far worse than
    /// one row billed at its peak rate.
    pub fn apply_tiers(&mut self, raw: Option<&str>) {
        let Some(raw) = raw else { return };
        let Ok(tiers) = serde_json::from_str::<PriceTiers>(raw) else {
            return;
        };
        self.off_peak = tiers.off_peak;
        self.peak_hours = tiers.peak_hours;
    }
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
    /// table reaches a forwarded request. Exchange rates stay empty: the mirror
    /// stores rows, not the rates above them, and the one place the gateway
    /// converts money — comparing a provider's spend against its limit — reads
    /// them from the cached document instead.
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

/// Convert an amount between currencies using the document's rates
/// (`rates[currency]` = units per 1 USD; USD pivots).
///
/// An unknown currency returns the amount unchanged. There is no honest rate for
/// it, and passing the raw number through is at least the number the reader was
/// already looking at — inventing one would be worse than not converting.
///
/// This lives here rather than beside its callers because both planes need it:
/// the app converts for display, and the gateway converts to compare usage
/// against a spending limit. Two copies of this arithmetic would eventually
/// disagree, and a limit that disagrees with the total beside it is the failure
/// the shared `limits` module exists to prevent.
pub fn convert_amount(amount: f64, from: &str, to: &str, rates: &HashMap<String, f64>) -> f64 {
    if from == to {
        return amount;
    }
    let (Some(per_usd_from), Some(per_usd_to)) = (rates.get(from), rates.get(to)) else {
        return amount;
    };
    if *per_usd_from == 0.0 {
        return amount;
    }
    let usd = amount / per_usd_from;
    usd * per_usd_to
}

/// Sum per-currency cost buckets into one amount denominated in `to`.
///
/// The buckets are what `SUM(cost) GROUP BY cost_currency` returns, so a provider
/// whose usage spans two currencies arrives as two entries and has to be
/// converted before it is added: summing them raw adds USD to CNY at 1:1, which
/// silently turns a `¥50` limit into a ceiling of nothing in particular. A
/// provider reaches that state honestly — its own models are priced in its own
/// currency, but a model it does not price is billed from the general row, which
/// may be denominated in someone else's.
///
/// Rows with no currency at all are dropped rather than counted as zero: they
/// are unpriced, not free.
pub fn convert_cost_buckets(
    buckets: &[(Option<String>, f64)],
    to: &str,
    rates: &HashMap<String, f64>,
) -> f64 {
    // Folded from `0.0` rather than `.sum()`ed: Rust's `Sum` for floats starts
    // at `-0.0`, so an empty bucket list — a window whose rows are all unpriced
    // — yields a *negative* zero, and `{:.4}` prints its sign. `-0.0000` in a
    // column of costs reads as a bug because it looks like one.
    buckets
        .iter()
        .fold(0.0, |total, (currency, cost)| match currency {
            Some(c) => total + convert_amount(*cost, c, to, rates),
            None => total,
        })
}

/// Parse a per-million price string; invalid decimals yield no cost.
fn parse_price(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A fully parsed set of per-million rates.
struct Rates {
    input: f64,
    output: f64,
    cache_read: f64,
    cache_creation: f64,
}

/// Is `at` (unix seconds) inside a peak window?
///
/// Read in the **vendor's** clock: `tz_offset` is minutes east of UTC and the
/// windows are that vendor's business hours, so judging them against the
/// reader's own timezone would pick the wrong rate silently — the one mistake
/// this shape exists to prevent.
///
/// A window is half-open, `[start, end)`, which is what lets two adjacent
/// windows meet without a gap. A window whose times are not clock times at all,
/// or whose day list is empty, never matches: matching is what turns a typo into
/// a wrong price, and the conservative direction is to charge the peak.
///
/// Note that a row carrying tiers is the row's *own* schedule: when one general
/// row prices several providers (no published row names a provider yet), a
/// reseller is billed on the publisher's clock too.
pub fn is_peak(hours: &PeakHours, at: i64) -> bool {
    use chrono::{Datelike, Timelike};

    if hours.windows.is_empty() {
        return false;
    }
    // The shifted DateTime is wrong as an instant and right as a wall clock,
    // which is exactly what a vendor's business hours need. Same idiom as
    // `gateway::limits::period_start`.
    let shifted = chrono::DateTime::from_timestamp(at, 0).unwrap_or_default()
        + chrono::Duration::minutes(hours.tz_offset as i64);
    let today = shifted.weekday().num_days_from_monday() as usize;
    let minutes = (shifted.time().hour() * 60 + shifted.time().minute()) as i64;

    hours.windows.iter().any(|w| {
        if !w.days.iter().any(|d| day_index(d) == Some(today)) {
            return false;
        }
        let (Some(start), Some(end)) = (minutes_of_day(&w.start), minutes_of_day(&w.end)) else {
            return false;
        };
        start <= minutes && minutes < end
    })
}

/// `"HH:MM"` to minutes since midnight, or `None` when it is not that — a
/// malformed time must leave the window unmatched rather than match broadly.
fn minutes_of_day(hhmm: &str) -> Option<i64> {
    let (h, m) = hhmm.trim().split_once(':')?;
    let (h, m) = (h.trim().parse::<i64>().ok()?, m.trim().parse::<i64>().ok()?);
    ((0..24).contains(&h) && (0..60).contains(&m)).then_some(h * 60 + m)
}

/// The weekday names the document uses, in `num_days_from_monday` order.
/// Case-insensitive: a hand-edited `"Mon"` that silently stopped matching would
/// misprice a whole week.
const DAYS: [&str; 7] = ["mon", "tue", "wed", "thu", "fri", "sat", "sun"];

fn day_index(day: &str) -> Option<usize> {
    let d = day.trim().to_ascii_lowercase();
    DAYS.iter().position(|x| *x == d)
}

/// Parse a set of rates; `None` when the input or output rate is not a number.
fn rates_of(input: &str, output: &str, cache_read: &str, cache_creation: &str) -> Option<Rates> {
    Some(Rates {
        input: parse_price(input)?,
        output: parse_price(output)?,
        cache_read: parse_price(cache_read)?,
        cache_creation: parse_price(cache_creation)?,
    })
}

/// The cost of `tokens` at `rates`, rounded to 6 decimal places.
fn cost_of(
    rates: &Rates,
    input: u64,
    output: u64,
    cache_read: u64,
    cache_creation: u64,
    cache_inclusive: bool,
) -> f64 {
    let billable_input = if cache_inclusive {
        input
            .saturating_sub(cache_read)
            .saturating_sub(cache_creation)
    } else {
        input
    };

    let million = 1_000_000f64;
    let total = (billable_input as f64 * rates.input
        + output as f64 * rates.output
        + cache_read as f64 * rates.cache_read
        + cache_creation as f64 * rates.cache_creation)
        / million;

    (total * 1e6).round() / 1e6
}

/// Compute the request cost (in the entry's currency) from token counts.
///
/// `cache_inclusive` mirrors cc-switch's `calculate_for_app` semantics:
/// OpenAI/Gemini style `input_tokens` already contain the cache buckets and
/// must be reduced before billing at the input rate; Anthropic's are fresh
/// input only. Result rounded to 6 decimal places.
///
/// This is the **peak** price: the row's own rates. `compute_cost_pair` is what
/// a caller with a clock wants.
pub fn compute_cost(
    entry: &ModelPriceEntry,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cache_inclusive: bool,
) -> Option<f64> {
    let rates = rates_of(
        &entry.input,
        &entry.output,
        &entry.cache_read,
        &entry.cache_creation,
    )?;
    Some(cost_of(
        &rates,
        input_tokens,
        output_tokens,
        cache_read_tokens,
        cache_creation_tokens,
        cache_inclusive,
    ))
}

/// The cost at the tier `at` (unix seconds) falls in, paired with what the same
/// tokens would have cost at the row's **off-peak** rates.
///
/// The pair is one call because the second number is a property of the first:
/// when the request was already off-peak, or the row publishes no tiers, the two
/// are equal by construction — which is what makes a report of their difference
/// a plain sum over every priced row, with no tier flag to keep in step.
///
/// An `off_peak` whose rates do not parse means the row has no off-peak tier
/// (peak-only), never zero rates: a broken discount must not become a free one.
pub fn compute_cost_pair(
    entry: &ModelPriceEntry,
    at: i64,
    input_tokens: u64,
    output_tokens: u64,
    cache_read_tokens: u64,
    cache_creation_tokens: u64,
    cache_inclusive: bool,
) -> Option<(f64, f64)> {
    let peak = rates_of(
        &entry.input,
        &entry.output,
        &entry.cache_read,
        &entry.cache_creation,
    )?;
    let cost = |rates: &Rates| {
        cost_of(
            rates,
            input_tokens,
            output_tokens,
            cache_read_tokens,
            cache_creation_tokens,
            cache_inclusive,
        )
    };
    let off_peak = entry
        .off_peak
        .as_ref()
        .and_then(|o| rates_of(&o.input, &o.output, &o.cache_read, &o.cache_creation));

    let billed = match (&entry.peak_hours, &off_peak) {
        // A schedule and something to discount: outside the windows the off-peak
        // rates are the ones that apply.
        (Some(hours), Some(off)) if !is_peak(hours, at) => cost(off),
        // Everything else bills at the listed rates — including a row whose
        // off-peak rates do not parse, where a broken discount must not become
        // a free one, and a row whose schedule is unreadable, where charging the
        // peak is the conservative direction the document itself prescribes.
        _ => cost(&peak),
    };
    let off_peak_cost = off_peak.as_ref().map_or(billed, cost);

    Some((billed, off_peak_cost))
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
            off_peak: None,
            peak_hours: None,
        }
    }

    /// The published DeepSeek schedule: weekdays 09:00–12:00 and 14:00–18:00,
    /// Beijing time. The instants below are what the vendor's clock makes of
    /// them; 2026-09-09 is a Wednesday and 2026-09-12 a Saturday.
    fn deepseek_hours() -> PeakHours {
        let window = |start: &str, end: &str| PeakWindow {
            days: ["mon", "tue", "wed", "thu", "fri"]
                .iter()
                .map(|d| d.to_string())
                .collect(),
            start: start.into(),
            end: end.into(),
        };
        PeakHours {
            tz_offset: 480,
            windows: vec![window("09:00", "12:00"), window("14:00", "18:00")],
        }
    }

    #[test]
    fn is_peak_reads_the_vendor_clock() {
        let hours = deepseek_hours();
        // Wed 2026-09-09, Beijing: 08:59:59 off, 09:00:00 on (start inclusive),
        // 11:59:59 on, 12:00:00 off (end exclusive), 13:00 the lunch gap off,
        // 14:00 on again, 18:00 off.
        for (epoch, peak) in [
            (1_788_915_599_i64, false),
            (1_788_915_600, true),
            (1_788_919_200, true),
            (1_788_926_399, true),
            (1_788_926_400, false),
            (1_788_930_000, false),
            (1_788_933_600, true),
            (1_788_948_000, false),
            // Sat 2026-09-12 10:00 Beijing: the weekend is never peak.
            (1_789_178_400, false),
        ] {
            assert_eq!(is_peak(&hours, epoch), peak, "at {epoch}");
        }
    }

    /// The same instant, judged on the wrong clock: this is the mistake the
    /// offset exists to prevent, so it gets its own test.
    #[test]
    fn is_peak_ignores_the_readers_timezone() {
        let mut hours = deepseek_hours();
        // Wed 10:00 Beijing is 02:00 UTC — outside the windows if read as UTC.
        assert!(is_peak(&hours, 1_788_919_200));
        hours.tz_offset = 0;
        assert!(!is_peak(&hours, 1_788_919_200));
    }

    /// A schedule must never match broadly: a typo in a field disables the
    /// window, and the peak rate applies.
    #[test]
    fn unparseable_windows_never_match() {
        let at = 1_788_919_200; // Wed 10:00 Beijing
        let with = |window: PeakWindow| PeakHours {
            tz_offset: 480,
            windows: vec![window],
        };
        let days = vec!["wed".to_string()];
        // Case-insensitive, so a hand-edited "Wed" still matches.
        assert!(is_peak(
            &with(PeakWindow {
                days: vec!["Wed".into()],
                start: "09:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        // "9:00" is nine o'clock and matches — leniency here cannot charge the
        // wrong rate. What must not match is a value that is not a clock time.
        assert!(!is_peak(
            &with(PeakWindow {
                days: vec![],
                start: "09:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        assert!(is_peak(
            &with(PeakWindow {
                days: days.clone(),
                start: "9:00".into(),
                end: "12:00".into()
            }),
            at
        ));
        for (start, end) in [("abc", "12:00"), ("09:00", "25:00"), ("09:00", "12:60")] {
            assert!(
                !is_peak(
                    &with(PeakWindow {
                        days: days.clone(),
                        start: start.into(),
                        end: end.into()
                    }),
                    at
                ),
                "{start}-{end} must not match"
            );
        }
        // An empty schedule is the same answer.
        assert!(!is_peak(
            &PeakHours {
                tz_offset: 480,
                windows: vec![]
            },
            at
        ));
    }

    /// A row with tiers: the listed rates inside the windows, the discounted
    /// ones outside, and the pair's second element always the off-peak answer.
    fn tiered_entry() -> ModelPriceEntry {
        ModelPriceEntry {
            input: "9.0".into(),
            output: "27.0".into(),
            cache_read: "0.30".into(),
            cache_creation: "0".into(),
            off_peak: Some(OffPeakRates {
                input: "4.5".into(),
                output: "13.5".into(),
                cache_read: "0.15".into(),
                cache_creation: "0".into(),
            }),
            peak_hours: Some(deepseek_hours()),
            ..usage_entry()
        }
    }

    #[test]
    fn tiered_cost_picks_the_rate_in_force() {
        let e = tiered_entry();
        let peak_at = 1_788_919_200; // Wed 10:00 Beijing
        let off_at = 1_788_930_000; // Wed 13:00, the lunch gap
        let million = 1_000_000;

        let (peak_cost, peak_off) =
            compute_cost_pair(&e, peak_at, million, 0, 0, 0, false).unwrap();
        let (off_cost, off_off) = compute_cost_pair(&e, off_at, million, 0, 0, 0, false).unwrap();
        assert!((peak_cost - 9.0).abs() < 1e-9, "peak: {peak_cost}");
        assert!((off_cost - 4.5).abs() < 1e-9, "off-peak: {off_cost}");
        // The second element is the same answer either way — what matters for
        // the report is that it does not depend on when the request ran.
        assert!((peak_off - 4.5).abs() < 1e-9, "off-peak side: {peak_off}");
        assert_eq!(off_off, off_cost);
        // A row without tiers has no discount to state, so both are its own rate.
        let plain = usage_entry();
        let (cost, off) = compute_cost_pair(&plain, peak_at, million, 0, 0, 0, false).unwrap();
        assert_eq!(cost, off);
        assert_eq!(cost, compute_cost(&plain, million, 0, 0, 0, false).unwrap());
    }

    /// A discount nobody can read is not a discount: the row bills at its
    /// listed rates rather than at zero.
    #[test]
    fn unreadable_off_peak_rates_are_ignored() {
        let e = ModelPriceEntry {
            off_peak: Some(OffPeakRates {
                input: "abc".into(),
                output: "13.5".into(),
                cache_read: "0.15".into(),
                cache_creation: "0".into(),
            }),
            ..tiered_entry()
        };
        let off_at = 1_788_930_000;
        let (cost, off) = compute_cost_pair(&e, off_at, 1_000_000, 0, 0, 0, false).unwrap();
        assert!(
            (cost - 9.0).abs() < 1e-9,
            "billed at the listed rate: {cost}"
        );
        assert_eq!(cost, off);
    }

    /// `off_peak` omits `cache_creation` in the published document; absent means
    /// 0, not "the peak rate stands in".
    #[test]
    fn absent_off_peak_cache_creation_is_zero() {
        let doc: ModelsDoc = serde_json::from_str(
            r#"{"version": 1, "exchange_rates": {"USD": 1.0}, "models": [
                {"model_id": "m", "display_name": "M", "input": "9", "output": "27",
                 "cache_read": "0.3", "cache_creation": "1", "currency": "USD",
                 "off_peak": {"in": "4.5", "out": "13.5", "cache_read": "0.15"}}]}"#,
        )
        .expect("the published shape parses");
        let e = &doc.models[0];
        assert_eq!(e.off_peak.as_ref().unwrap().cache_creation, "0");
        // …and a document whose off_peak has no cache fields at all still parses,
        // rather than failing every price this install has.
        let sparse: ModelsDoc = serde_json::from_str(
            r#"{"version": 1, "exchange_rates": {"USD": 1.0}, "models": [
                {"model_id": "m", "display_name": "M", "input": "9", "output": "27",
                 "cache_read": "0.3", "cache_creation": "1", "currency": "USD",
                 "off_peak": {"in": "4.5", "out": "13.5"}}]}"#,
        )
        .expect("absent cache rates default to 0");
        assert_eq!(sparse.models[0].off_peak.as_ref().unwrap().cache_read, "0");
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
