//! Anthropic Messages response → OpenAI Chat Completions response, JSON only.
//!
//! The inverse of [`super::response::openai_to_anthropic`], and the arithmetic
//! is the point: Anthropic's `input_tokens` **excludes** the cache buckets while
//! Chat's `prompt_tokens` **includes** them, so the conversion adds instead of
//! subtracting. Getting this backwards under-counts every cached request, and
//! it is also what [`super::responses_response::chat_to_responses`] and the
//! metering scanner expect.
//!
//! `anthropic_to_responses` is the non-streaming Responses leg: Chat is the
//! intermediate, and `chat_to_responses` is reused rather than reimplemented.

use crate::proxy::error::ProxyError;
use crate::proxy::json_canonical::canonical_json_string;
use crate::proxy::providers::transform::responses_response::{chat_to_responses, now_unix};
use serde_json::{json, Value};

/// Anthropic message → OpenAI Chat completion.
pub fn anthropic_to_chat(body: Value) -> Result<Value, ProxyError> {
    let blocks = body
        .get("content")
        .and_then(|c| c.as_array())
        .ok_or_else(|| ProxyError::TransformError("no `content` array in message".to_string()))?;

    let mut text = String::new();
    let mut reasoning = String::new();
    let mut tool_calls = Vec::new();

    for block in blocks {
        let kind = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match kind {
            "text" => {
                if let Some(chunk) = block.get("text").and_then(|t| t.as_str()) {
                    text.push_str(chunk);
                }
            }
            "thinking" => {
                if let Some(chunk) = block.get("thinking").and_then(|t| t.as_str()) {
                    reasoning.push_str(chunk);
                }
            }
            "tool_use" => {
                let input = block.get("input").cloned().unwrap_or(json!({}));
                tool_calls.push(json!({
                    "id": block.get("id").and_then(|i| i.as_str()).unwrap_or(""),
                    "type": "function",
                    "function": {
                        "name": block.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                        // Chat carries arguments as a JSON string, and the
                        // canonical spelling keeps a replayed turn byte-stable.
                        "arguments": canonical_json_string(&input)
                    }
                }));
            }
            // Opaque or server-side: nothing the client asked for, and no Chat
            // field it could be shown in. Dropping the trace is not dropping
            // the answer.
            "redacted_thinking"
            | "server_tool_use"
            | "web_search_tool_result"
            | "mcp_tool_use"
            | "mcp_tool_result"
            | "container_upload" => {}
            other => {
                log::warn!("anthropic→openai: no conversion for content block `{other}`");
            }
        }
    }

    let has_tool_calls = !tool_calls.is_empty();
    let finish_reason = map_stop_reason_to_chat(
        body.get("stop_reason").and_then(|r| r.as_str()),
        has_tool_calls,
    );

    let mut message = json!({"role": "assistant"});
    if text.is_empty() {
        message["content"] = Value::Null;
    } else {
        message["content"] = json!(text);
    }
    if has_tool_calls {
        message["tool_calls"] = json!(tool_calls);
    }
    if !reasoning.is_empty() {
        // The spelling the sibling streaming converter and the DeepSeek/MiMo
        // compatibility path use; Chat clients that do not know it ignore it.
        message["reasoning_content"] = json!(reasoning);
    }

    let usage = body.get("usage").cloned().unwrap_or(json!({}));
    Ok(json!({
        "id": body.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        "object": "chat.completion",
        "created": now_unix(),
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "choices": [{
            "index": 0,
            "message": message,
            "finish_reason": finish_reason
        }],
        "usage": anthropic_usage_to_chat(&usage)
    }))
}

/// Anthropic message → Responses object, through the Chat shape.
pub fn anthropic_to_responses(body: Value) -> Result<Value, ProxyError> {
    chat_to_responses(anthropic_to_chat(body)?)
}

/// Anthropic usage → the **inclusive** Chat usage shape.
///
/// `input_tokens` is fresh input only; Chat's `prompt_tokens` contains every
/// bucket, so the three are summed. The Anthropic-named cache fields ride along
/// flat: the metering scanner reads them (that is how cache writes are priced)
/// and Chat clients ignore fields they do not know.
pub(crate) fn anthropic_usage_to_chat(usage: &Value) -> Value {
    let number = |key: &str| usage.get(key).and_then(|v| v.as_u64()).unwrap_or(0);
    let input = number("input_tokens");
    let output = number("output_tokens");
    let read = number("cache_read_input_tokens");
    let creation = number("cache_creation_input_tokens");
    let prompt = input + read + creation;

    let mut details = serde_json::Map::new();
    if read > 0 {
        details.insert("cached_tokens".into(), json!(read));
    }
    if creation > 0 {
        details.insert("cache_write_tokens".into(), json!(creation));
    }

    let mut out = json!({
        "prompt_tokens": prompt,
        "completion_tokens": output,
        "total_tokens": prompt + output
    });
    if !details.is_empty() {
        out["prompt_tokens_details"] = Value::Object(details);
    }
    if read > 0 {
        out["cache_read_input_tokens"] = json!(read);
    }
    if creation > 0 {
        out["cache_creation_input_tokens"] = json!(creation);
    }
    if let Some(ttl) = usage.get("cache_creation") {
        out["cache_creation"] = ttl.clone();
    }
    out
}

/// Anthropic `stop_reason` → Chat `finish_reason`.
pub(crate) fn map_stop_reason_to_chat(
    stop_reason: Option<&str>,
    has_tool_calls: bool,
) -> &'static str {
    match stop_reason {
        Some("end_turn") | Some("stop_sequence") | Some("pause_turn") | None => {
            if has_tool_calls {
                "tool_calls"
            } else {
                "stop"
            }
        }
        Some("max_tokens") => "length",
        // `tool_use` with no surviving tool call is not a tool-call answer.
        Some("tool_use") => {
            if has_tool_calls {
                "tool_calls"
            } else {
                "stop"
            }
        }
        Some("refusal") => "content_filter",
        Some(other) => {
            log::warn!("anthropic→openai: unknown stop_reason `{other}` read as `stop`");
            "stop"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn message(content: Value, stop: &str, usage: Value) -> Value {
        json!({
            "id": "msg_1",
            "type": "message",
            "role": "assistant",
            "model": "claude-sonnet-5-5",
            "content": content,
            "stop_reason": stop,
            "usage": usage
        })
    }

    #[test]
    fn text_becomes_the_content_string() {
        let out = anthropic_to_chat(message(
            json!([{"type": "text", "text": "he"}, {"type": "text", "text": "llo"}]),
            "end_turn",
            json!({"input_tokens": 5, "output_tokens": 2}),
        ))
        .unwrap();
        assert_eq!(out["choices"][0]["message"]["content"], "hello");
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
        assert_eq!(out["usage"]["prompt_tokens"], 5);
        assert_eq!(out["usage"]["completion_tokens"], 2);
    }

    #[test]
    fn tool_use_becomes_tool_calls_with_a_json_string() {
        let out = anthropic_to_chat(message(
            json!([
                {"type": "text", "text": "checking"},
                {"type": "tool_use", "id": "t1", "name": "read", "input": {"path": "a.rs"}}
            ]),
            "tool_use",
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))
        .unwrap();
        let message = &out["choices"][0]["message"];
        assert_eq!(message["content"], "checking");
        assert_eq!(message["tool_calls"][0]["id"], "t1");
        assert_eq!(message["tool_calls"][0]["function"]["name"], "read");
        let args: Value = serde_json::from_str(
            message["tool_calls"][0]["function"]["arguments"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(args, json!({"path": "a.rs"}));
        assert_eq!(out["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn thinking_survives_as_reasoning_content() {
        let out = anthropic_to_chat(message(
            json!([
                {"type": "thinking", "thinking": "hmm"},
                {"type": "text", "text": "ok"}
            ]),
            "end_turn",
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))
        .unwrap();
        assert_eq!(out["choices"][0]["message"]["reasoning_content"], "hmm");
    }

    #[test]
    fn usage_is_inclusive_and_keeps_the_anthropic_buckets() {
        let out = anthropic_to_chat(message(
            json!([{"type": "text", "text": "ok"}]),
            "end_turn",
            json!({
                "input_tokens": 10,
                "output_tokens": 7,
                "cache_read_input_tokens": 30,
                "cache_creation_input_tokens": 5,
                "cache_creation": {"ephemeral_1h_input_tokens": 3}
            }),
        ))
        .unwrap();
        let usage = &out["usage"];
        assert_eq!(usage["prompt_tokens"], 45);
        assert_eq!(usage["total_tokens"], 52);
        assert_eq!(usage["prompt_tokens_details"]["cached_tokens"], 30);
        assert_eq!(usage["prompt_tokens_details"]["cache_write_tokens"], 5);
        assert_eq!(usage["cache_read_input_tokens"], 30);
        assert_eq!(usage["cache_creation_input_tokens"], 5);
        assert_eq!(usage["cache_creation"]["ephemeral_1h_input_tokens"], 3);
    }

    #[test]
    fn tool_use_without_a_surviving_call_is_a_stop() {
        let out = anthropic_to_chat(message(
            json!([{"type": "text", "text": "ok"}]),
            "tool_use",
            json!({"input_tokens": 1, "output_tokens": 1}),
        ))
        .unwrap();
        assert_eq!(out["choices"][0]["finish_reason"], "stop");
    }

    #[test]
    fn the_responses_composition_reads_the_inclusive_usage() {
        let responses = anthropic_to_responses(message(
            json!([{"type": "text", "text": "ok"}]),
            "end_turn",
            json!({
                "input_tokens": 10,
                "output_tokens": 7,
                "cache_read_input_tokens": 30
            }),
        ))
        .unwrap();
        assert_eq!(responses["object"], "response");
        assert_eq!(responses["usage"]["input_tokens"], 40);
        assert_eq!(
            responses["usage"]["input_tokens_details"]["cached_tokens"],
            30
        );
        assert_eq!(responses["output"][0]["content"][0]["text"], "ok");
    }
}
