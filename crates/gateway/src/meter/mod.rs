//! Usage capture (tech.md §4.3: parse usage into the store, streaming + non-streaming).
//!
//! The gateway parses `usage` fields from upstream responses to meter every
//! request into the `usage` table. Anthropic and OpenAI flavor differ:
//!
//! * Anthropic: `usage.input_tokens` / `output_tokens` /
//!   `cache_read_input_tokens` / `cache_creation_input_tokens`; in SSE
//!   `message_start` carries input + cache, cumulative `output_tokens`
//!   arrives with `message_delta`.
//! * OpenAI: `usage.prompt_tokens` / `completion_tokens` /
//!   `prompt_tokens_details.cached_tokens`; chat chunks only attach `usage`
//!   on the final chunk (with `stream_options.include_usage`), Responses API
//!   reports on the `response.completed` event.
//!
//! The cache bucket is the one field with three spellings, and reading only
//! one of them reports a provider's prefix cache as never earning its keep:
//!
//! * `prompt_cache_hit_tokens` — DeepSeek's own, top-level in `usage`.
//! * `input_tokens_details.cached_tokens` — the Responses API (Codex).
//! * `prompt_tokens_details.cached_tokens` — chat completions.
//!
//! `cache_creation_tokens` is a different matter, and a zero there means
//! something else: only Anthropic has an explicit cache *write* — you mark the
//! blocks to keep and it bills for storing them. The others cache implicitly,
//! with no such concept and no such field, so their rows will always read zero.
//! A reader must not take that for "nothing is being cached"; on those
//! providers the *read* side is the only side that exists.

use serde_json::Value;

use crate::store::Protocol;

/// Bound on the payload a scanner will look at.
///
/// This does not decide how much memory the gateway will spend on a body — that
/// is decided where the body is read, `server::MAX_BODY_BYTES` for an inbound
/// request (32 MiB) and `forward::MAX_UPSTREAM_BODY_BYTES` for a response
/// (16 MiB). It is set to the larger of the two so that nothing the gateway
/// accepted is skipped, because a skipped body is not metered *at all*: the
/// biggest requests are the ones whose cost used to go missing, and they are the
/// expensive ones.
///
/// It used to be 2 MiB, chosen when scanning meant building a `serde_json::Value`
/// of the whole document — several times the body size in allocations. The scans
/// below now read only the fields they want ([`UsageEnvelope`], [`ModelEnvelope`])
/// and let serde consume the rest without materializing it, so the ceiling no
/// longer has to be about memory.
const MAX_SCAN_BYTES: usize = crate::server::MAX_BODY_BYTES;

/// The part of a response the meter reads. Everything else — the messages, the
/// completion, a 16 MiB tool result — is consumed by serde and dropped on the
/// floor rather than turned into a `Value` tree.
#[derive(serde::Deserialize)]
struct UsageEnvelope {
    #[serde(default)]
    model: Option<String>,
    #[serde(default)]
    usage: Option<Value>,
    // Gemini spells both of these differently: the counts ride `usageMetadata`
    // and the model id `modelVersion` (there is no top-level `model`).
    #[serde(default, rename = "usageMetadata")]
    usage_metadata: Option<Value>,
    #[serde(default, rename = "modelVersion")]
    model_version: Option<String>,
}

/// The part of a request the meter reads: which model is being asked for, which
/// is what prices the request.
#[derive(serde::Deserialize)]
struct ModelEnvelope {
    #[serde(default)]
    model: Option<String>,
}

/// Token counts captured from one response.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

/// Incremental scanner fed with complete SSE lines; merges usage events.
#[derive(Debug, Default)]
pub struct UsageScanner {
    usage: Usage,
    model: Option<String>,
}

impl UsageScanner {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one complete line (`\r\n`/`\n` stripped by the caller's `lines()`).
    pub fn feed_line(&mut self, line: &str) {
        let Some(payload) = line.strip_prefix("data:") else {
            return; // event:/id:/retry:/comments carry no usage
        };
        let payload = payload.trim();
        if payload.is_empty() || payload == "[DONE]" || payload.len() > MAX_SCAN_BYTES {
            return;
        }
        let Ok(v) = serde_json::from_str::<Value>(payload) else {
            return; // not JSON (e.g. malformed) — ignore
        };
        self.absorb(&v);
    }

    fn absorb(&mut self, v: &Value) {
        match v.get("type").and_then(Value::as_str) {
            Some("message_start") => {
                if let Some(u) = v.pointer("/message/usage") {
                    self.merge_usage(u);
                }
                if let Some(m) = v.pointer("/message/model").and_then(Value::as_str) {
                    self.model = Some(m.to_string());
                }
            }
            Some("message_delta") => {
                if let Some(u) = v.get("usage") {
                    self.merge_usage(u);
                }
            }
            Some("response.completed") => {
                if let Some(u) = v.pointer("/response/usage") {
                    self.merge_usage(u);
                }
                if let Some(m) = v.pointer("/response/model").and_then(Value::as_str) {
                    self.model = Some(m.to_string());
                }
            }
            _ => {
                // OpenAI chat.completion.chunk: usage rides the final chunk.
                if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                    self.merge_usage(u);
                }
                // Gemini streamGenerateContent (alt=sse): every event carries a
                // cumulative `usageMetadata`, with `modelVersion` alongside.
                if let Some(u) = v.get("usageMetadata").filter(|u| u.is_object()) {
                    self.merge_usage(u);
                }
                if let Some(m) = v.get("modelVersion").and_then(Value::as_str) {
                    self.model = Some(m.to_string());
                }
            }
        }
    }

    /// Merge a usage object (either flavor; last write wins per key).
    fn merge_usage(&mut self, u: &Value) {
        let get = |key: &str| u.get(key).and_then(Value::as_i64);
        // Anthropic flavor (input/output_tokens are also the Responses API keys).
        if let Some(n) = get("input_tokens") {
            self.usage.input_tokens = n;
        }
        if let Some(n) = get("output_tokens") {
            self.usage.output_tokens = n;
        }
        if let Some(n) = get("cache_read_input_tokens") {
            self.usage.cache_read_tokens = n;
        }
        if let Some(n) = get("cache_creation_input_tokens") {
            self.usage.cache_creation_tokens = n;
        }
        // OpenAI chat completions flavor.
        if let Some(n) = get("prompt_tokens") {
            self.usage.input_tokens = n;
        }
        if let Some(n) = get("completion_tokens") {
            self.usage.output_tokens = n;
        }
        // The cache bucket, in the three spellings that reach this scanner.
        // Each read can only fill in a number still absent, so an upstream
        // reporting more than one cannot have its value displaced — and the
        // chat-completions key is last because it is the one that was already
        // being read, which keeps every body that worked before working
        // identically.
        //
        // * `prompt_cache_hit_tokens` — DeepSeek's own, a top-level sibling of
        //   `prompt_tokens` (with `prompt_cache_miss_tokens` for the rest)
        //   rather than a nested object.
        // * `input_tokens_details.cached_tokens` — the Responses API, which is
        //   what Codex sends. Its absence was invisible for longer than the
        //   others: `input_tokens`/`output_tokens` are spelled the same way in
        //   the Responses shape and the Anthropic one above, so those two
        //   numbers were read correctly all along and only the cache half of
        //   the object was dropped.
        // * `prompt_tokens_details.cached_tokens` — OpenAI chat completions.
        if let Some(n) = get("prompt_cache_hit_tokens") {
            self.usage.cache_read_tokens = n;
        }
        if let Some(n) = u
            .pointer("/input_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
        {
            self.usage.cache_read_tokens = n;
        }
        if let Some(n) = u
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
        {
            self.usage.cache_read_tokens = n;
        }
        // Gemini flavor (`usageMetadata` on generateContent /
        // streamGenerateContent). Its `promptTokenCount` includes the cached
        // bucket, which is why the provider's `cache_inclusive` is set.
        if let Some(n) = get("promptTokenCount") {
            self.usage.input_tokens = n;
        }
        if let Some(n) = get("candidatesTokenCount") {
            self.usage.output_tokens = n;
        }
        if let Some(n) = get("cachedContentTokenCount") {
            self.usage.cache_read_tokens = n;
        }
    }

    /// Model seen in the stream (message_start / response.completed).
    pub fn model(&self) -> Option<&str> {
        self.model.as_deref()
    }

    /// Final accumulated usage.
    pub fn usage(&self) -> Usage {
        self.usage.clone()
    }
}

/// Parse usage + model from a complete non-streaming JSON response body.
pub fn parse_response_usage(protocol: Protocol, body: &[u8]) -> (Option<Usage>, Option<String>) {
    if body.is_empty() || body.len() > MAX_SCAN_BYTES {
        return (None, None);
    }
    let Ok(envelope) = serde_json::from_slice::<UsageEnvelope>(body) else {
        return (None, None);
    };
    let mut scanner = UsageScanner::new();
    match protocol {
        Protocol::Anthropic => {
            // message objects carry usage at top level; error bodies do not.
            if let Some(u) = envelope.usage.as_ref().filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
        }
        Protocol::OpenAI => {
            // chat.completion / response objects carry usage at top level.
            if let Some(u) = envelope.usage.as_ref().filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
        }
        Protocol::Gemini => {
            // generateContent reports `usageMetadata`; the model id is
            // `modelVersion` (no `model` field exists).
            if let Some(u) = envelope.usage_metadata.as_ref().filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
        }
    }
    // No usage recorded → report none, regardless of whether a model id was
    // seen (an all-default Usage carries no numbers worth reporting).
    let usage = (scanner.usage != Usage::default()).then_some(scanner.usage);
    (usage, envelope.model.or(envelope.model_version))
}

/// Extract the `model` field from an inbound request body (authoritative for
/// metering; upstream stream events only serve as fallback).
///
/// The request is the body that gets large — long contexts, pasted files, base64
/// images — so this reads the one field it wants and skips the rest without
/// allocating it. It used to give up past 2 MiB, which left exactly those
/// requests unpriced: no model, so no price, so a NULL cost in the dashboard.
pub fn request_model(body: &[u8]) -> Option<String> {
    if body.is_empty() || body.len() > MAX_SCAN_BYTES {
        return None;
    }
    let envelope = serde_json::from_slice::<ModelEnvelope>(body).ok()?;
    envelope.model
}

/// The model id for the native Gemini API, which names it in the path
/// (`/v1beta/models/{model}:{method}`) instead of the body. `None` for every
/// other path shape — a body that states its model stays the authority.
///
/// Without this a Gemini request is metered with no model, and a request with
/// no model is a request with no price: a NULL cost in the dashboard.
pub fn model_from_path(path: &str) -> Option<String> {
    let rest = path.strip_prefix("/v1beta/models/")?;
    // The action shares the component with the model (`{model}:{method}`), so
    // the id is everything before the colon. The list path has no segment left
    // after the prefix, and anything with a further slash is not this shape.
    let model = rest.split(':').next().unwrap_or_default();
    (!model.is_empty() && !model.contains('/')).then(|| model.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    // A body past the old 2 MiB ceiling came back with nothing to meter, and a
    // request with no model is a request with no price — so the largest requests,
    // which are the expensive ones, were the ones whose cost went missing. The
    // scan reads only the fields it wants now, which is what lets it take a body
    // this size at all: serde walks the rest without building it.
    #[test]
    fn a_large_body_is_still_scanned_for_its_model_and_usage() {
        let filler = "x".repeat(3 * 1024 * 1024);

        // The field on the far side of the filler is still reached: nothing
        // short-circuits on size, and nothing stops at the first unknown key.
        let model_first = format!(r#"{{"model":"claude-sonnet-4-5","messages":["{filler}"]}}"#);
        assert_eq!(
            request_model(model_first.as_bytes()).as_deref(),
            Some("claude-sonnet-4-5")
        );
        let model_last = format!(r#"{{"messages":["{filler}"],"model":"deepseek-chat"}}"#);
        assert_eq!(
            request_model(model_last.as_bytes()).as_deref(),
            Some("deepseek-chat")
        );

        let response = format!(
            r#"{{"choices":["{filler}"],"model":"deepseek-chat","usage":{{"prompt_tokens":11,"completion_tokens":22}}}}"#
        );
        let (usage, model) = parse_response_usage(Protocol::OpenAI, response.as_bytes());
        assert_eq!(model.as_deref(), Some("deepseek-chat"));
        let usage = usage.expect("usage past the filler");
        assert_eq!(usage.input_tokens, 11);
        assert_eq!(usage.output_tokens, 22);
    }

    #[test]
    fn parses_anthropic_non_stream() {
        let body = br#"{"id":"msg_1","type":"message","role":"assistant",
            "model":"claude-sonnet-4-5",
            "usage":{"input_tokens":2095,"output_tokens":503,
                     "cache_creation_input_tokens":2095,"cache_read_input_tokens":0}}"#;
        let (usage, model) = parse_response_usage(Protocol::Anthropic, body);
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 2095);
        assert_eq!(u.output_tokens, 503);
        assert_eq!(u.cache_creation_tokens, 2095);
        assert_eq!(u.cache_read_tokens, 0);
        assert_eq!(model.as_deref(), Some("claude-sonnet-4-5"));
    }

    #[test]
    fn parses_openai_non_stream() {
        let body = br#"{"id":"c1","object":"chat.completion","model":"gpt-4o",
            "usage":{"prompt_tokens":42,"completion_tokens":7,
                     "prompt_tokens_details":{"cached_tokens":32},"total_tokens":49}}"#;
        let (usage, model) = parse_response_usage(Protocol::OpenAI, body);
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 42);
        assert_eq!(u.output_tokens, 7);
        assert_eq!(u.cache_read_tokens, 32);
        assert_eq!(model.as_deref(), Some("gpt-4o"));
    }

    #[test]
    fn ignores_bodies_without_usage() {
        let (usage, _) = parse_response_usage(Protocol::Anthropic, br#"{"data":[]}"#);
        assert!(usage.is_none());
        let (usage, _) = parse_response_usage(Protocol::OpenAI, b"not json");
        assert!(usage.is_none());
        let (usage, _) = parse_response_usage(Protocol::OpenAI, b"");
        assert!(usage.is_none());
    }

    #[test]
    fn scans_anthropic_sse_stream() {
        let mut s = UsageScanner::new();
        s.feed_line("event: message_start");
        s.feed_line(r#"data: {"type":"message_start","message":{"model":"claude-sonnet-4-5","usage":{"input_tokens":25,"output_tokens":1,"cache_read_input_tokens":11,"cache_creation_input_tokens":3}}}"#);
        s.feed_line("");
        s.feed_line(r#"event: content_block_delta"#);
        s.feed_line(
            r#"data: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hi"}}"#,
        );
        s.feed_line(r#"event: message_delta"#);
        s.feed_line(r#"data: {"type":"message_delta","delta":{"stop_reason":"end_turn"},"usage":{"output_tokens":171}}"#);
        s.feed_line("data: [DONE]");

        let u = s.usage();
        assert_eq!(u.input_tokens, 25);
        assert_eq!(u.output_tokens, 171); // cumulative from message_delta
        assert_eq!(u.cache_read_tokens, 11);
        assert_eq!(u.cache_creation_tokens, 3);
        assert_eq!(s.model(), Some("claude-sonnet-4-5"));
    }

    /// DeepSeek reports its cache bucket as a top-level sibling of
    /// `prompt_tokens` rather than nested inside `prompt_tokens_details`, so a
    /// reader that only knows the OpenAI key sees every cache hit as a miss —
    /// and `prompt_tokens` there is hit + miss, which is what makes the
    /// omission invisible rather than obviously wrong.
    #[test]
    fn parses_deepseek_native_cache_field() {
        let body = br#"{"id":"c1","object":"chat.completion","model":"deepseek-v4-flash",
            "usage":{"prompt_tokens":15302,"completion_tokens":102,
                     "prompt_cache_hit_tokens":14000,"prompt_cache_miss_tokens":1302,
                     "total_tokens":15404}}"#;
        let (usage, model) = parse_response_usage(Protocol::OpenAI, body);
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 15302);
        assert_eq!(u.output_tokens, 102);
        assert_eq!(
            u.cache_read_tokens, 14000,
            "the provider's own account of its cache is the only one on offer"
        );
        assert_eq!(model.as_deref(), Some("deepseek-v4-flash"));
    }

    /// Streaming arrives as the final chunk, like the OpenAI flavor it
    /// otherwise is.
    #[test]
    fn scans_deepseek_sse_stream() {
        let mut s = UsageScanner::new();
        s.feed_line(r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"delta":{"content":"he"}}]}"#);
        s.feed_line(r#"data: {"id":"1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":900,"completion_tokens":12,"prompt_cache_hit_tokens":896}}"#);
        s.feed_line("data: [DONE]");

        let u = s.usage();
        assert_eq!(u.input_tokens, 900);
        assert_eq!(u.cache_read_tokens, 896);
    }

    /// An upstream that reports both shapes is not counted twice, and the
    /// nested OpenAI key keeps the last word — the new read can only add a
    /// number that was absent, never displace one that arrived another way.
    #[test]
    fn the_openai_cache_shape_wins_when_both_are_present() {
        let body = br#"{"usage":{"prompt_tokens":100,"completion_tokens":5,
            "prompt_cache_hit_tokens":90,
            "prompt_tokens_details":{"cached_tokens":60}}}"#;
        let (usage, _) = parse_response_usage(Protocol::OpenAI, body);
        assert_eq!(usage.unwrap().cache_read_tokens, 60);
    }

    #[test]
    fn scans_openai_chat_sse_stream() {
        let mut s = UsageScanner::new();
        s.feed_line(r#"data: {"id":"1","object":"chat.completion.chunk","choices":[{"delta":{"content":"he"}}]}"#);
        s.feed_line(r#"data: {"id":"1","object":"chat.completion.chunk","choices":[],"usage":{"prompt_tokens":10,"completion_tokens":5,"prompt_tokens_details":{"cached_tokens":4}}}"#);
        s.feed_line("data: [DONE]");

        let u = s.usage();
        assert_eq!(u.input_tokens, 10);
        assert_eq!(u.output_tokens, 5);
        assert_eq!(u.cache_read_tokens, 4);
    }

    #[test]
    fn scans_openai_responses_sse_stream() {
        let mut s = UsageScanner::new();
        s.feed_line(r#"data: {"type":"response.in_progress","response":{"id":"r1"}}"#);
        // The usage object as a real Responses-shaped upstream reports it —
        // Codex through a DeepSeek endpoint, taken from the gateway's own
        // request log. Its cache bucket is `input_tokens_details`, which
        // nothing read until this test had a reason to look: the other two
        // numbers arrive under keys the Anthropic branch already handles, so
        // only the cache half went missing, and it went missing quietly.
        s.feed_line(r#"data: {"type":"response.completed","response":{"model":"gpt-5.1","usage":{"input_tokens":15302,"input_tokens_details":{"cached_tokens":15232},"output_tokens":102,"output_tokens_details":{"reasoning_tokens":71},"total_tokens":15404}}}"#);

        let u = s.usage();
        assert_eq!(u.input_tokens, 15302);
        assert_eq!(u.output_tokens, 102);
        assert_eq!(u.cache_read_tokens, 15232);
        assert_eq!(s.model(), Some("gpt-5.1"));
    }

    #[test]
    fn request_model_extraction() {
        assert_eq!(
            request_model(br#"{"model":"claude-opus-4-6","stream":true}"#).as_deref(),
            Some("claude-opus-4-6")
        );
        assert_eq!(request_model(b"{}"), None);
        assert_eq!(request_model(b""), None);
    }

    // Gemini names its model in the path, not the body — and without a model
    // there is no price, so this is what keeps a Gemini request off NULL cost.
    #[test]
    fn model_from_gemini_path() {
        assert_eq!(
            model_from_path("/v1beta/models/gemini-2.5-pro:generateContent").as_deref(),
            Some("gemini-2.5-pro")
        );
        assert_eq!(
            model_from_path("/v1beta/models/gemini-2.5-flash:streamGenerateContent").as_deref(),
            Some("gemini-2.5-flash")
        );
        assert_eq!(
            model_from_path("/v1beta/models/gemini-2.5-pro:countTokens").as_deref(),
            Some("gemini-2.5-pro")
        );
        // The list path and every other protocol's paths say nothing.
        assert_eq!(model_from_path("/v1beta/models"), None);
        assert_eq!(model_from_path("/v1beta/models/"), None);
        assert_eq!(model_from_path("/v1/messages"), None);
        assert_eq!(model_from_path("/v1beta/models/a/b:generateContent"), None);
    }

    #[test]
    fn parses_gemini_non_stream() {
        let body = br#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}],
            "modelVersion":"gemini-2.5-pro",
            "usageMetadata":{"promptTokenCount":88,"candidatesTokenCount":31,
                             "cachedContentTokenCount":12,"totalTokenCount":119}}"#;
        let (usage, model) = parse_response_usage(Protocol::Gemini, body);
        let u = usage.expect("usageMetadata is metered");
        assert_eq!(u.input_tokens, 88);
        assert_eq!(u.output_tokens, 31);
        assert_eq!(u.cache_read_tokens, 12);
        assert_eq!(model.as_deref(), Some("gemini-2.5-pro"));
    }

    // Every Gemini SSE event carries a cumulative `usageMetadata`, so the last
    // one to arrive is the request's total.
    #[test]
    fn scans_gemini_sse_stream() {
        let mut scanner = UsageScanner::new();
        scanner.feed_line(
            r#"data: {"candidates":[],"usageMetadata":{"promptTokenCount":88},"modelVersion":"gemini-2.5-flash"}"#,
        );
        scanner.feed_line(
            r#"data: {"candidates":[{"content":{"parts":[{"text":"x"}]}}],"usageMetadata":{"promptTokenCount":88,"candidatesTokenCount":9,"cachedContentTokenCount":12},"modelVersion":"gemini-2.5-flash"}"#,
        );
        assert_eq!(scanner.usage().input_tokens, 88);
        assert_eq!(scanner.usage().output_tokens, 9);
        assert_eq!(scanner.usage().cache_read_tokens, 12);
        assert_eq!(scanner.model(), Some("gemini-2.5-flash"));
    }
}
