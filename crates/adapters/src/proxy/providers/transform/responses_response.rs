//! OpenAI Chat Completions response → OpenAI Responses response, JSON only.
//!
//! `chat_to_responses` is the reverse of `responses_request::responses_to_chat`
//! and the shape half of the streaming adapter (`providers::responses_streaming`
//! builds its terminal `response.completed` out of the same helpers below, so
//! the two cannot drift).
//!
//! The invariant worth stating once: a Chat tool call's `id` becomes the
//! Responses item's `call_id`, and `responses_to_chat` turns that `call_id`
//! back into a Chat `tool_call_id`. A client that round-trips a tool call
//! through both directions sees the id it sent come back unchanged — the test
//! `the_tool_call_id_round_trips_through_both_directions` asserts it.
//!
//! A leaf: one JSON value in, one out.

use crate::proxy::error::ProxyError;
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// Disambiguates ids minted in the same second, so two responses never collide.
static ID_COUNTER: AtomicU64 = AtomicU64::new(0);

/// OpenAI Chat Completions response → OpenAI Responses response.
pub fn chat_to_responses(body: Value) -> Result<Value, ProxyError> {
    let id = response_id_from(body.get("id").and_then(|v| v.as_str()));
    let model = body
        .get("model")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let created_at = body
        .get("created")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(now_unix);

    let choice = body
        .get("choices")
        .and_then(|c| c.as_array())
        .and_then(|c| c.first());

    let mut output = Vec::new();
    let mut status = "completed";

    if let Some(choice) = choice {
        if let Some(message) = choice.get("message") {
            let reasoning = message
                .get("reasoning_content")
                .and_then(|r| r.as_str())
                .filter(|r| !r.is_empty());
            if let Some(reasoning) = reasoning {
                // `encrypted_content` is the Responses API's opaque payload for
                // carrying a trace back to a storing upstream; a Chat provider
                // never emits it and this build cannot synthesize one, so the
                // field is left absent rather than faked.
                output.push(json!({
                    "type": "reasoning",
                    "summary": [{"type": "summary_text", "text": reasoning}]
                }));
            }

            let text = message_text(message);
            if !text.is_empty() {
                output.push(json!({
                    "type": "message",
                    "id": mint_item_id("msg", &id),
                    "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text, "annotations": []}]
                }));
            }

            if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
                for tool_call in tool_calls {
                    let call_id = tool_call.get("id").and_then(|v| v.as_str()).unwrap_or("");
                    let function = tool_call.get("function");
                    let name = function
                        .and_then(|f| f.get("name"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    let arguments = function
                        .and_then(|f| f.get("arguments"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("{}");
                    output.push(json!({
                        "type": "function_call",
                        "id": mint_item_id("fc", call_id),
                        // The Chat id, kept verbatim: it is what the client
                        // echoes back in a `function_call_output` item.
                        "call_id": call_id,
                        "name": name,
                        "arguments": arguments,
                        "status": "completed"
                    }));
                }
            }
        }

        if choice.get("finish_reason").and_then(|v| v.as_str()) == Some("length") {
            status = "incomplete";
        }
    }
    // A 200 with no choices is a valid, if useless, answer — an empty response
    // is reported as such, not turned into an error. The gateway's meter reads
    // `usage` and `model` off the same object and neither depends on output.

    let usage = build_responses_usage(&body.get("usage").cloned().unwrap_or_else(|| json!({})));
    let mut response = build_response_object(&id, &model, created_at, output, usage, status);
    if status == "incomplete" {
        response["incomplete_details"] = json!({"reason": "max_output_tokens"});
    }
    // `error` and `incomplete_details` are omitted when they carry nothing:
    // nothing in this crate reads them, and a JSON `null` for a field the
    // client treats as optional is noise the client would parse anyway.

    Ok(response)
}

/// The Responses `usage` object, built from a Chat `usage` object.
///
/// `input_tokens` is Chat's `prompt_tokens` **inclusive** of the cached bucket,
/// which is what the Responses wire means by it — unlike the Anthropic
/// direction, nothing is subtracted here. The gateway's meter already reads
/// this exact shape (`input_tokens_details.cached_tokens` and
/// `output_tokens_details.reasoning_tokens`), so it is reproduced, not
/// reinvented.
pub(crate) fn build_responses_usage(usage: &Value) -> Value {
    let get = |key: &str| usage.get(key).and_then(|v| v.as_u64());
    let input_tokens = get("prompt_tokens")
        .or_else(|| get("input_tokens"))
        .unwrap_or(0);
    let output_tokens = get("completion_tokens")
        .or_else(|| get("output_tokens"))
        .unwrap_or(0);
    let total_tokens = get("total_tokens").unwrap_or(input_tokens + output_tokens);
    // The cache bucket has more than one spelling upstream; the OpenAI nested
    // details come first, then DeepSeek's flat `prompt_cache_hit_tokens`.
    let cached_tokens = usage
        .pointer("/prompt_tokens_details/cached_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| get("prompt_cache_hit_tokens"))
        .unwrap_or(0);
    // Thinking tokens, as a slice of the output — never added to it.
    let reasoning_tokens = get("reasoning_tokens")
        .or_else(|| {
            usage
                .pointer("/completion_tokens_details/reasoning_tokens")
                .and_then(|v| v.as_u64())
        })
        .or_else(|| {
            usage
                .pointer("/output_tokens_details/reasoning_tokens")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);

    json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens,
        "total_tokens": total_tokens,
        "input_tokens_details": {"cached_tokens": cached_tokens},
        "output_tokens_details": {"reasoning_tokens": reasoning_tokens}
    })
}

/// The skeleton both directions share: a Responses object with the fields that
/// are the same everywhere. `status` is the caller's, because it is the one
/// field the two endpoints disagree about.
pub(crate) fn build_response_object(
    id: &str,
    model: &str,
    created_at: i64,
    output: Vec<Value>,
    usage: Value,
    status: &str,
) -> Value {
    json!({
        "id": id,
        "object": "response",
        "created_at": created_at,
        "status": status,
        "model": model,
        "output": output,
        "usage": usage
    })
}

/// The Responses id for an upstream Chat response. An id that is already a
/// Responses id passes through; anything else is hashed into one, so the
/// derived id is stable for a given upstream id.
pub(crate) fn response_id_from(upstream: Option<&str>) -> String {
    match upstream {
        Some(id) if id.starts_with("resp_") => id.to_string(),
        Some(id) if !id.is_empty() => mint_id("resp", id),
        _ => mint_id("resp", ""),
    }
}

/// An item id (`msg_`, `fc_`, …), seeded so it is reproducible for a given
/// upstream value.
pub(crate) fn mint_item_id(prefix: &str, seed: &str) -> String {
    mint_id(prefix, seed)
}

/// The current Unix time in seconds, or 0 if the clock is before the epoch.
pub(crate) fn now_unix() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// `prefix_<hex>` — a SHA-256 over the seed, the clock and a counter. The
/// counter is what makes this unique rather than merely stable when the same
/// seed is minted twice in one second (two tool calls with the same id, say).
fn mint_id(prefix: &str, seed: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(seed.as_bytes());
    hasher.update(now_unix().to_le_bytes());
    hasher.update(ID_COUNTER.fetch_add(1, Ordering::Relaxed).to_le_bytes());
    let digest = hasher.finalize();

    let mut hex = String::with_capacity(24);
    for byte in digest.iter().take(12) {
        hex.push_str(&format!("{byte:02x}"));
    }
    format!("{prefix}_{hex}")
}

/// The assistant text a Chat message carries, string or part array.
fn message_text(message: &Value) -> String {
    let mut text = match message.get("content") {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Array(parts)) => {
            let mut out = String::new();
            for part in parts {
                match part.get("type").and_then(|t| t.as_str()) {
                    Some("text") | Some("output_text") => {
                        if let Some(t) = part.get("text").and_then(|v| v.as_str()) {
                            out.push_str(t);
                        }
                    }
                    Some("refusal") => {
                        if let Some(r) = part.get("refusal").and_then(|v| v.as_str()) {
                            out.push_str(r);
                        }
                    }
                    _ => {}
                }
            }
            out
        }
        _ => String::new(),
    };
    // Some providers put the refusal at message level instead of as a part.
    if text.is_empty() {
        if let Some(refusal) = message.get("refusal").and_then(|r| r.as_str()) {
            text.push_str(refusal);
        }
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::providers::transform::responses_request::responses_to_chat;

    #[test]
    fn a_text_message_becomes_a_message_item_with_an_output_text_part() {
        let body = json!({
            "id": "chatcmpl-1",
            "model": "gpt-4o",
            "created": 100,
            "choices": [{
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let out = chat_to_responses(body).unwrap();
        assert_eq!(out["object"], "response");
        assert_eq!(out["status"], "completed");
        assert_eq!(out["created_at"], 100);
        assert_eq!(out["model"], "gpt-4o");
        assert!(out["id"].as_str().unwrap().starts_with("resp_"));
        assert_eq!(out["output"][0]["type"], "message");
        assert_eq!(out["output"][0]["role"], "assistant");
        assert_eq!(out["output"][0]["status"], "completed");
        assert_eq!(out["output"][0]["content"][0]["type"], "output_text");
        assert_eq!(out["output"][0]["content"][0]["text"], "Hello!");
        assert_eq!(out["output"][0]["content"][0]["annotations"], json!([]));
    }

    #[test]
    fn a_responses_id_from_the_upstream_is_kept() {
        let body = json!({"id": "resp_abc", "model": "m", "choices": []});
        let out = chat_to_responses(body).unwrap();
        assert_eq!(out["id"], "resp_abc");
    }

    #[test]
    fn the_usage_shape_is_the_one_the_gateway_meter_reads() {
        let body = json!({
            "id": "chatcmpl-2",
            "model": "deepseek-v4-flash",
            "choices": [{
                "message": {"role": "assistant", "content": "hi"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 10,
                "completion_tokens": 5,
                "total_tokens": 15,
                "prompt_tokens_details": {"cached_tokens": 4},
                "completion_tokens_details": {"reasoning_tokens": 2}
            }
        });

        let usage = chat_to_responses(body).unwrap()["usage"].clone();
        // prompt_tokens is inclusive of the cached bucket, so input keeps 10
        // and the cache is reported alongside — not subtracted.
        assert_eq!(usage["input_tokens"], 10);
        assert_eq!(usage["output_tokens"], 5);
        assert_eq!(usage["total_tokens"], 15);
        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 4);
        assert_eq!(usage["output_tokens_details"]["reasoning_tokens"], 2);
    }

    #[test]
    fn deepseeks_flat_cache_key_is_read_for_the_cache_bucket() {
        let body = json!({
            "id": "c",
            "model": "m",
            "choices": [],
            "usage": {"prompt_tokens": 100, "completion_tokens": 1, "prompt_cache_hit_tokens": 96}
        });
        let usage = chat_to_responses(body).unwrap()["usage"].clone();
        assert_eq!(usage["input_tokens_details"]["cached_tokens"], 96);
    }

    #[test]
    fn tool_calls_become_function_call_items_keeping_the_chat_id_as_call_id() {
        let body = json!({
            "id": "chatcmpl-3",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_xyz",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"city\":\"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 1, "completion_tokens": 1}
        });

        let out = chat_to_responses(body).unwrap();
        let function_call = out["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .expect("a function_call item");
        assert_eq!(function_call["call_id"], "call_xyz");
        assert!(function_call["id"].as_str().unwrap().starts_with("fc_"));
        assert_eq!(function_call["name"], "get_weather");
        assert_eq!(function_call["arguments"], "{\"city\":\"Tokyo\"}");
        assert_eq!(function_call["status"], "completed");
        // No empty message item is invented for a tool-only turn.
        assert!(!out["output"]
            .as_array()
            .unwrap()
            .iter()
            .any(|item| item["type"] == "message"));
    }

    /// The round trip is the invariant: the Chat id becomes `call_id`, and
    /// feeding that back through `responses_to_chat` yields the original Chat
    /// `tool_call_id`. This is what lets a client continue a tool call.
    #[test]
    fn the_tool_call_id_round_trips_through_both_directions() {
        let upstream = json!({
            "id": "chatcmpl-4",
            "model": "gpt-4o",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_round",
                        "type": "function",
                        "function": {"name": "lookup", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }]
        });

        let responses = chat_to_responses(upstream).unwrap();
        let call_id = responses["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .and_then(|item| item["call_id"].as_str())
            .expect("a call_id")
            .to_string();
        assert_eq!(call_id, "call_round");

        let follow_up = json!({
            "model": "gpt-4o",
            "input": [{"type": "function_call_output", "call_id": call_id, "output": "done"}]
        });
        let chat = responses_to_chat(follow_up).unwrap();
        assert_eq!(chat["messages"][0]["role"], "tool");
        assert_eq!(chat["messages"][0]["tool_call_id"], "call_round");
    }

    #[test]
    fn reasoning_content_becomes_a_summary_item() {
        let body = json!({
            "id": "c",
            "model": "deepseek-v4-flash",
            "choices": [{
                "message": {
                    "role": "assistant",
                    "reasoning_content": "Think first.",
                    "content": "Answer."
                },
                "finish_reason": "stop"
            }]
        });

        let out = chat_to_responses(body).unwrap();
        let reasoning = out["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "reasoning")
            .expect("a reasoning item");
        assert_eq!(reasoning["summary"][0]["type"], "summary_text");
        assert_eq!(reasoning["summary"][0]["text"], "Think first.");
        // The summary is unencrypted; encrypted_content is not truthfully
        // producible here and must stay absent.
        assert!(reasoning.get("encrypted_content").is_none());
    }

    #[test]
    fn no_choices_is_an_empty_completed_response_not_an_error() {
        let out = chat_to_responses(json!({"id": "c", "model": "m"})).unwrap();
        assert_eq!(out["status"], "completed");
        assert_eq!(out["output"], json!([]));
        assert_eq!(out["usage"]["input_tokens"], 0);
    }

    #[test]
    fn an_empty_choices_array_is_also_an_empty_completed_response() {
        let out = chat_to_responses(json!({"id": "c", "model": "m", "choices": []})).unwrap();
        assert_eq!(out["status"], "completed");
        assert_eq!(out["output"], json!([]));
    }

    #[test]
    fn a_length_finish_sets_incomplete_with_the_reason() {
        let body = json!({
            "id": "c",
            "model": "m",
            "choices": [{
                "message": {"role": "assistant", "content": "cut off"},
                "finish_reason": "length"
            }]
        });

        let out = chat_to_responses(body).unwrap();
        assert_eq!(out["status"], "incomplete");
        assert_eq!(out["incomplete_details"]["reason"], "max_output_tokens");
        // The message item still reports itself complete; only the envelope is
        // incomplete, which is what the client reads.
        assert_eq!(out["output"][0]["status"], "completed");
    }
}
