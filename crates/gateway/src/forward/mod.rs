//! Transparent forwarding with SSE passthrough + usage capture (tech.md §4.3).
//!
//! Non-SSE upstream responses are buffered and metered inline. SSE responses
//! are piped through byte-for-byte while a scanning stream watches the events
//! for usage fields; the metered sample is persisted after the stream ends.
//!
//! Protocol conversion (tech.md §4.3 / adapters phase): an Anthropic
//! inbound request (`POST /v1/messages`) bound to an OpenAI-compatible
//! provider is converted with the `kiwano-adapters` sublayer — request via
//! `anthropic_to_openai` (model untouched), response (JSON + SSE)
//! back via `openai_to_anthropic` / the streaming converter — and metered
//! from the upstream OpenAI usage fields.
//!
//! **No response cache, deliberately.** Every local gateway of this shape grows
//! one eventually, so the reasoning is written here rather than rediscovered:
//! an exact-match cache would almost never hit on this traffic, because every
//! turn of an agent's conversation carries a different prompt, and a *semantic*
//! cache — the kind that pays off on repetitive workloads — replays answers into
//! a context that has moved on, which for a coding agent means confidently wrong
//! edits. The saving this backend actually wants is upstream prompt-cache reuse,
//! and that is a routing problem rather than a storage one: `roundrobin` keeps a
//! session on one provider (see `crate::strategy`) so the provider's own cache
//! sees the same prefix twice, and the converter strips nothing that would break
//! it. If a cache is ever added here, it should be opt-in, off by default, and
//! scoped to traffic a human would call repetitive.
//!
//! The module is split by domain. Every `pub` item keeps the path it had when
//! this was one file (`forward::forward`): the facade below re-exports it,
//! because `crate::server::data` names it that way. The submodules are
//! `pub(crate)`, so the split adds no path to the crate's public surface.
//!
//! `sample` owns the metering vocabulary; `headers`, `upstream`, `shim` and
//! `stream` are the four pieces a forward is made of — credentials, the send
//! with its retries, the body rewriting, the SSE passthrough — and `native`
//! and `convert` are the two entry points, both of which end in `metering`.

pub mod convert;
pub mod finish;
pub mod headers;
pub mod inbound;
pub mod metering;
pub mod native;
pub mod pricing;
pub mod sample;
pub mod shim;
pub mod stream;
pub mod upstream;

pub(crate) type BoxError = Box<dyn std::error::Error + Send + Sync>;

// ── the public surface, re-exported so every `forward::x` path still resolves ──

pub use native::forward;
pub use upstream::MAX_UPSTREAM_BODY_BYTES;
// `pub use` would not compile: the item itself is only crate-visible.
pub(crate) use upstream::{is_retryable_status, RETRY_BUDGET};

/// Fixtures more than one submodule's tests need.
#[cfg(test)]
pub(crate) mod test_support {
    use crate::router::UpstreamProvider;
    use crate::store::Protocol;

    /// A fixed instant — Wed 2026-09-09 10:00 Beijing, inside the peak window
    /// the published DeepSeek schedule names — so that a fixture which gains
    /// tiers behaves predictably.
    pub(crate) const TEST_AT: i64 = 1_788_919_200;

    pub(crate) fn provider(protocol: Protocol, api_path: Option<&str>) -> UpstreamProvider {
        UpstreamProvider {
            id: "p1".into(),
            name: "p1".into(),
            catalog_id: None,
            protocol,
            base_url: "https://up.example.com".into(),
            api_path: api_path.map(Into::into),
            endpoints: Vec::new(),
            api_key: Some("sk-real-key".into()),
            extra_keys: Vec::new(),
            weight: 1,
            win_start: None,
            win_end: None,
            timeout_secs: None,
            retries: None,
            headers: None,
        }
    }
}
