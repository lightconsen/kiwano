//! The metering vocabulary: what one forwarded request reports and what it
//! logs, plus how the agent was attributed.

use crate::log_capture::RequestCapture;
use crate::meter::Usage;
use crate::store::now_rfc3339;
use crate::store::UsageRecord;

/// Response/request facts for the full request log, gathered by the forward
/// leg and persisted together with the metered usage sample.
#[derive(Clone)]
pub(crate) struct CompletedLog {
    pub(crate) capture: RequestCapture,
    pub(crate) attribution: String,
    pub(crate) status_code: u16,
    pub(crate) error_kind: Option<String>,
    pub(crate) error_message: Option<String>,
    pub(crate) is_streaming: bool,
    pub(crate) first_token_ms: Option<i64>,
    /// Client-visible response body (converted stream for the Anthropic path).
    pub(crate) response_body: Option<String>,
    pub(crate) response_size: i64,
    pub(crate) truncated: bool,
    pub(crate) response_headers: Option<String>,
    /// What the compat shim changed, one line per action — `None` when it
    /// touched nothing, which is the statement the row should carry.
    pub(crate) request_notes: Option<String>,
}

/// One metered request, ready for the `usage` table (+ full request log).
#[derive(Clone)]
pub(crate) struct UsageSample {
    pub(crate) agent: String,
    pub(crate) provider_id: String,
    /// The catalog entry the provider was added from, when it came from the
    /// shelf. That — not the local row id — is the key its price is published
    /// under; None is priced at the general rate, which is what a hand-added
    /// provider should get.
    pub(crate) catalog_id: Option<String>,
    /// Unix seconds when the request began, which is what decides the price
    /// tier: a row may charge its peak rates only inside its own windows, and a
    /// request belongs to the window it *started* in. Not `ts`, which is the
    /// write time — a long stream can end in a later window than it began.
    pub(crate) started_unix: i64,
    pub(crate) model: Option<String>,
    pub(crate) usage: Usage,
    pub(crate) latency_ms: i64,
    pub(crate) status: &'static str,
    /// Input-token semantics of the outbound protocol: openai `input_tokens`
    /// already contain the cache buckets (deducted before billing); anthropic
    /// reports fresh input only.
    pub(crate) cache_inclusive: bool,
    /// The upstream reported no usage object. Filled by the parse for a
    /// one-shot response, and by the scanner's own memory for a stream — the
    /// zeros are the same either way, and only this says which they are.
    pub(crate) usage_missing: bool,
    /// Full-log payload; None while request logging is disabled.
    pub(crate) log: Option<CompletedLog>,
}

impl UsageSample {
    /// `cost_off_peak` is what these tokens would have cost at the row's
    /// off-peak rates — equal to `cost` when it publishes no schedule. The pair
    /// is frozen here and never re-derived later (from `ts`, or from whatever
    /// the price table says next week): a request that started at 17:59 on a
    /// Friday is billed peak and may carry an off-peak `ts`.
    pub(crate) fn into_record(
        self,
        cost: Option<f64>,
        cost_off_peak: Option<f64>,
        cost_currency: Option<String>,
    ) -> UsageRecord {
        UsageRecord {
            ts: now_rfc3339(),
            agent: self.agent,
            provider_id: self.provider_id,
            model: self.model,
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cache_read_tokens: self.usage.cache_read_tokens,
            cache_creation_tokens: self.usage.cache_creation_tokens,
            latency_ms: Some(self.latency_ms),
            status: self.status.to_string(),
            cost,
            cost_currency,
            cost_off_peak,
        }
    }
}

/// How the agent was attributed, as stored in `request_logs.attribution`.
pub(crate) fn attribution_str(a: crate::router::Attribution) -> String {
    match a {
        crate::router::Attribution::PlaceholderKey => "key".to_string(),
        crate::router::Attribution::PathFallback => "path_fallback".to_string(),
    }
}
