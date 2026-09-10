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

use serde_json::Value;

use crate::store::Protocol;

/// Upper bound for a scanned payload; larger bodies are skipped (never metered).
const MAX_SCAN_BYTES: usize = 2 * 1024 * 1024;

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
                // Gemini streamGenerateContent (alt=sse) chunks: cumulative
                // usageMetadata + modelVersion on every event.
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
        if let Some(n) = u
            .pointer("/prompt_tokens_details/cached_tokens")
            .and_then(Value::as_i64)
        {
            self.usage.cache_read_tokens = n;
        }
        // Gemini flavor (generateContent / streamGenerateContent).
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
    let Ok(v) = serde_json::from_slice::<Value>(body) else {
        return (None, None);
    };
    let mut model = v.get("model").and_then(Value::as_str).map(str::to_string);
    let mut scanner = UsageScanner::new();
    match protocol {
        Protocol::Anthropic => {
            // message objects carry usage at top level; error bodies do not.
            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
        }
        Protocol::OpenAI => {
            // chat.completion / response objects carry usage at top level.
            if let Some(u) = v.get("usage").filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
        }
        Protocol::Gemini => {
            // generateContent responses report usageMetadata; the model lives
            // in `modelVersion` (no `model` field exists).
            if let Some(u) = v.get("usageMetadata").filter(|u| u.is_object()) {
                scanner.merge_usage(u);
            }
            if let Some(m) = v.get("modelVersion").and_then(Value::as_str) {
                model = Some(m.to_string());
            }
        }
    }
    // No usage recorded → report none, regardless of whether a model id was
    // seen (an all-default Usage carries no numbers worth reporting).
    let usage = (scanner.usage != Usage::default()).then_some(scanner.usage);
    (usage, model)
}

/// Extract the `model` field from an inbound request body (authoritative for
/// metering; upstream stream events only serve as fallback).
pub fn request_model(body: &[u8]) -> Option<String> {
    if body.is_empty() || body.len() > MAX_SCAN_BYTES {
        return None;
    }
    let v = serde_json::from_slice::<Value>(body).ok()?;
    v.get("model").and_then(Value::as_str).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn parses_gemini_non_stream() {
        let body = br#"{"candidates":[{"content":{"parts":[{"text":"hi"}]}}],
            "modelVersion":"gemini-2.5-pro",
            "usageMetadata":{"promptTokenCount":88,"candidatesTokenCount":31,
                             "cachedContentTokenCount":12,"totalTokenCount":119}}"#;
        let (usage, model) = parse_response_usage(Protocol::Gemini, body);
        let u = usage.unwrap();
        assert_eq!(u.input_tokens, 88);
        assert_eq!(u.output_tokens, 31);
        assert_eq!(u.cache_read_tokens, 12);
        assert_eq!(model.as_deref(), Some("gemini-2.5-pro"));
    }

    #[test]
    fn scans_gemini_sse_stream() {
        let mut scanner = UsageScanner::new();
        // The first chunk only carries prompt-side counts; later chunks overwrite with cumulative totals.
        scanner.feed_line(r#"data: {"candidates":[],"usageMetadata":{"promptTokenCount":88},"modelVersion":"gemini-2.5-flash"}"#);
        scanner.feed_line(r#"data: {"candidates":[{"content":{"parts":[{"text":"x"}]}}],"usageMetadata":{"promptTokenCount":88,"candidatesTokenCount":9,"cachedContentTokenCount":12},"modelVersion":"gemini-2.5-flash"}"#);
        assert_eq!(scanner.usage().input_tokens, 88);
        assert_eq!(scanner.usage().output_tokens, 9);
        assert_eq!(scanner.usage().cache_read_tokens, 12);
        assert_eq!(scanner.model(), Some("gemini-2.5-flash"));
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
        s.feed_line(r#"data: {"type":"response.completed","response":{"model":"gpt-5.1","usage":{"input_tokens":100,"output_tokens":12}}}"#);

        let u = s.usage();
        assert_eq!(u.input_tokens, 100);
        assert_eq!(u.output_tokens, 12);
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
}
