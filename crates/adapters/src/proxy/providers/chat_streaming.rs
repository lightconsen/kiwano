//! Anthropic SSE → OpenAI Chat SSE, and the Responses composition on top of it.
//!
//! Anthropic reports a message as typed events; Chat reports it as deltas on one
//! object. The mapping that matters is the tool index: Anthropic's `index` is
//! the content-block index and counts text and thinking blocks, while OpenAI's
//! `tool_calls[].index` counts tool calls only and starts at zero. Routing a
//! delta by the wrong number attaches arguments to another call — for Codex,
//! through `responses_streaming`, that is a corrupted command.
//!
//! Usage is accumulated across `message_start` (input and cache buckets) and
//! `message_delta` (output) and emitted once, in a trailing `choices: []` chunk.
//! It is emitted whether or not the client asked: the gateway's metering
//! scanner reads it, and `StreamPolicy::strip_usage_chunk` drops it from the
//! client's view when the request did not carry `stream_options.include_usage`.
//!
//! [`create_responses_sse_stream_from_anthropic`] pipes this stream into the
//! existing Chat → Responses adapter, so the Responses leg reuses both halves
//! instead of growing a third converter.

use crate::proxy::providers::responses_streaming::create_responses_sse_stream;
use crate::proxy::providers::transform::chat_response::{
    anthropic_usage_to_chat, map_stop_reason_to_chat,
};
use crate::proxy::providers::transform::responses_response::now_unix;
use crate::proxy::sse::{append_utf8_safe, strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde_json::{json, Value};
use std::collections::HashMap;

const DONE: &str = "data: [DONE]\n\n";

/// Anthropic event stream → OpenAI Chat chunk stream.
pub fn create_chat_sse_stream_from_anthropic<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let created = now_unix();
        let mut id = format!("chatcmpl-{created}");
        let mut model = String::new();
        let mut usage = json!({});
        let mut usage_seen = false;
        let mut role_sent = false;
        let mut finish_sent = false;
        let mut done = false;
        let mut failed = false;
        let mut next_tool_index: usize = 0;
        // Anthropic content-block index → OpenAI tool index.
        let mut tools_by_block: HashMap<u64, usize> = HashMap::new();

        tokio::pin!(stream);

        while let Some(chunk) = stream.next().await {
            let bytes = match chunk {
                Ok(bytes) => bytes,
                Err(e) => {
                    log::error!("[anthropic→openai] stream error: {e}");
                    if !done {
                        let error = json!({
                            "error": {
                                "message": format!("Stream error: {e}"),
                                "type": "stream_error"
                            }
                        });
                        yield Ok(sse_data(error));
                        yield Ok(Bytes::from_static(DONE.as_bytes()));
                    }
                    failed = true;
                    done = true;
                    break;
                }
            };

            append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

            while let Some(block) = take_sse_block(&mut buffer) {
                if block.trim().is_empty() {
                    continue;
                }
                let mut data: Option<&str> = None;
                let mut event_name = "";
                for line in block.lines() {
                    if let Some(value) = strip_sse_field(line, "event") {
                        event_name = value.trim();
                    }
                    if let Some(value) = strip_sse_field(line, "data") {
                        data = Some(value);
                    }
                }
                let Some(data) = data else { continue };
                if data.trim() == "[DONE]" {
                    // Anthropic has no sentinel; tolerate one from a proxy.
                    continue;
                }
                let Ok(event) = serde_json::from_str::<Value>(data) else {
                    log::debug!("[anthropic→openai] skipped a malformed event");
                    continue;
                };
                let kind = event
                    .get("type")
                    .and_then(|t| t.as_str())
                    .unwrap_or(event_name);

                match kind {
                    "message_start" => {
                        if let Some(message) = event.get("message") {
                            if let Some(value) = message.get("id").and_then(|v| v.as_str()) {
                                if !value.is_empty() {
                                    id = value.to_string();
                                }
                            }
                            if let Some(value) = message.get("model").and_then(|v| v.as_str()) {
                                model = value.to_string();
                            }
                            if let Some(start_usage) = message.get("usage") {
                                merge_usage(&mut usage, start_usage);
                                usage_seen = true;
                            }
                        }
                        if !role_sent {
                            role_sent = true;
                            yield Ok(delta_chunk(&id, &model, created, json!({"role": "assistant", "content": ""}), None));
                        }
                    }
                    "ping" => {}
                    "content_block_start" => {
                        let block_index = event.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let content = event.get("content_block").cloned().unwrap_or(json!({}));
                        if content.get("type").and_then(|t| t.as_str()) == Some("tool_use") {
                            // A tool call starts before any of its arguments
                            // arrive, which is how Chat clients learn its name.
                            let tool_index = next_tool_index;
                            next_tool_index += 1;
                            tools_by_block.insert(block_index, tool_index);
                            let delta = json!({
                                "tool_calls": [{
                                    "index": tool_index,
                                    "id": content.get("id").and_then(|v| v.as_str()).unwrap_or(""),
                                    "type": "function",
                                    "function": {
                                        "name": content.get("name").and_then(|v| v.as_str()).unwrap_or(""),
                                        "arguments": ""
                                    }
                                }]
                            });
                            yield Ok(delta_chunk(&id, &model, created, delta, None));
                        }
                    }
                    "content_block_delta" => {
                        let block_index = event.get("index").and_then(|v| v.as_u64()).unwrap_or(0);
                        let delta = event.get("delta").cloned().unwrap_or(json!({}));
                        match delta.get("type").and_then(|t| t.as_str()) {
                            Some("text_delta") => {
                                if let Some(text) = delta.get("text").and_then(|t| t.as_str()) {
                                    yield Ok(delta_chunk(&id, &model, created, json!({"content": text}), None));
                                }
                            }
                            Some("input_json_delta") => {
                                let Some(tool_index) = tools_by_block.get(&block_index).copied() else {
                                    log::warn!(
                                        "[anthropic→openai] argument delta for an unopened tool block {block_index}"
                                    );
                                    continue;
                                };
                                if let Some(args) = delta.get("partial_json").and_then(|v| v.as_str()) {
                                    let delta = json!({
                                        "tool_calls": [{
                                            "index": tool_index,
                                            "function": {"arguments": args}
                                        }]
                                    });
                                    yield Ok(delta_chunk(&id, &model, created, delta, None));
                                }
                            }
                            Some("thinking_delta") => {
                                if let Some(thinking) = delta.get("thinking").and_then(|t| t.as_str()) {
                                    yield Ok(delta_chunk(
                                        &id,
                                        &model,
                                        created,
                                        json!({"reasoning_content": thinking}),
                                        None,
                                    ));
                                }
                            }
                            // `signature_delta` and `citations_delta` carry
                            // Anthropic-side verification data with no Chat
                            // field to hold it.
                            _ => {}
                        }
                    }
                    "content_block_stop" => {}
                    "message_delta" => {
                        if let Some(delta_usage) = event.get("usage") {
                            merge_usage(&mut usage, delta_usage);
                            usage_seen = true;
                        }
                        if !finish_sent {
                            let stop_reason = event
                                .pointer("/delta/stop_reason")
                                .and_then(|v| v.as_str());
                            let finish = map_stop_reason_to_chat(stop_reason, next_tool_index > 0);
                            finish_sent = true;
                            yield Ok(delta_chunk(&id, &model, created, json!({}), Some(finish)));
                        }
                    }
                    "message_stop" => {
                        if !done {
                            if !finish_sent {
                                finish_sent = true;
                                let finish = map_stop_reason_to_chat(None, next_tool_index > 0);
                                yield Ok(delta_chunk(&id, &model, created, json!({}), Some(finish)));
                            }
                            if usage_seen {
                                yield Ok(usage_chunk(&id, &model, created, &usage));
                            }
                            yield Ok(Bytes::from_static(DONE.as_bytes()));
                            done = true;
                        }
                    }
                    "error" => {
                        let error = event.get("error").cloned().unwrap_or(json!({}));
                        yield Ok(sse_data(json!({"error": error})));
                        yield Ok(Bytes::from_static(DONE.as_bytes()));
                        failed = true;
                        done = true;
                    }
                    other => {
                        log::debug!("[anthropic→openai] ignored event `{other}`");
                    }
                }
            }
        }

        // A stream that ended without `message_stop` still has to finish: a
        // client waiting on a terminal event would otherwise hang. A stream that
        // failed keeps its error and gets no success terminal.
        if !done && !failed {
            if !finish_sent {
                let finish = map_stop_reason_to_chat(None, next_tool_index > 0);
                yield Ok(delta_chunk(&id, &model, created, json!({}), Some(finish)));
            }
            if usage_seen {
                yield Ok(usage_chunk(&id, &model, created, &usage));
            }
            yield Ok(Bytes::from_static(DONE.as_bytes()));
        }
    }
}

/// Anthropic event stream → Responses event stream, through Chat.
pub fn create_responses_sse_stream_from_anthropic<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    create_responses_sse_stream(create_chat_sse_stream_from_anthropic(stream))
}

fn delta_chunk(
    id: &str,
    model: &str,
    created: i64,
    delta: Value,
    finish_reason: Option<&str>,
) -> Bytes {
    sse_data(json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [{
            "index": 0,
            "delta": delta,
            "finish_reason": finish_reason
        }]
    }))
}

fn usage_chunk(id: &str, model: &str, created: i64, usage: &Value) -> Bytes {
    sse_data(json!({
        "id": id,
        "object": "chat.completion.chunk",
        "created": created,
        "model": model,
        "choices": [],
        "usage": anthropic_usage_to_chat(usage)
    }))
}

fn sse_data(value: Value) -> Bytes {
    Bytes::from(format!(
        "data: {}\n\n",
        serde_json::to_string(&value).unwrap_or_default()
    ))
}

/// Merge a usage object into the accumulator, one level deep for the nested
/// `cache_creation` TTL split. `message_start` carries input and the cache
/// buckets, `message_delta` the final output count.
fn merge_usage(acc: &mut Value, add: &Value) {
    let (Some(acc), Some(add)) = (acc.as_object_mut(), add.as_object()) else {
        return;
    };
    for (key, value) in add {
        if key == "cache_creation" {
            if let (Some(existing), Some(incoming)) = (
                acc.get_mut(key).and_then(|v| v.as_object_mut()),
                value.as_object(),
            ) {
                for (inner, inner_value) in incoming {
                    existing.insert(inner.clone(), inner_value.clone());
                }
                continue;
            }
        }
        acc.insert(key.clone(), value.clone());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream::iter;

    /// Collect the converted stream into `(data line, parsed JSON or None for
    /// `[DONE]`)` pairs.
    async fn run(chunks: Vec<String>) -> Vec<(String, Option<Value>)> {
        let input: Vec<Result<Bytes, std::io::Error>> =
            chunks.into_iter().map(|c| Ok(Bytes::from(c))).collect();
        let stream = create_chat_sse_stream_from_anthropic(iter(input));
        let output: Vec<Bytes> = stream.map(|r| r.unwrap()).collect().await;
        let text = output
            .into_iter()
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect::<String>();
        text.split("\n\n")
            .filter(|s| !s.trim().is_empty())
            .map(|block| {
                let data = block
                    .lines()
                    .find_map(|l| strip_sse_field(l, "data"))
                    .unwrap_or("")
                    .to_string();
                if data == "[DONE]" {
                    (data, None)
                } else {
                    (data.clone(), Some(serde_json::from_str(&data).unwrap()))
                }
            })
            .collect()
    }

    fn event(kind: &str, payload: Value) -> String {
        format!("event: {kind}\ndata: {payload}\n\n")
    }

    #[test]
    fn text_stream_emits_role_deltas_finish_usage_and_done() {
        let chunks = vec![
            event(
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1", "model": "claude-sonnet-5-5",
                    "usage": {"input_tokens": 10, "cache_read_input_tokens": 30}
                }}),
            ),
            event(
                "content_block_start",
                json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
            ),
            event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "hi"}}),
            ),
            event(
                "message_delta",
                json!({"type": "message_delta",
                "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 7}}),
            ),
            event("message_stop", json!({"type": "message_stop"})),
        ];
        let out = futures::executor::block_on(run(chunks));
        let payloads: Vec<&Value> = out.iter().filter_map(|(_, v)| v.as_ref()).collect();
        assert_eq!(out.last().unwrap().0, "[DONE]");
        assert_eq!(payloads[0]["choices"][0]["delta"]["role"], "assistant");
        let text: String = payloads
            .iter()
            .filter_map(|p| {
                p.pointer("/choices/0/delta/content")
                    .and_then(|v| v.as_str())
            })
            .collect();
        assert_eq!(text, "hi");
        let finish = payloads
            .iter()
            .find_map(|p| {
                p.pointer("/choices/0/finish_reason")
                    .and_then(|v| v.as_str())
            })
            .unwrap();
        assert_eq!(finish, "stop");
        let usage = payloads
            .iter()
            .find(|p| p.get("usage").is_some())
            .expect("usage chunk");
        assert_eq!(usage["usage"]["prompt_tokens"], 40);
        assert_eq!(usage["usage"]["prompt_tokens_details"]["cached_tokens"], 30);
        assert_eq!(usage["usage"]["completion_tokens"], 7);
    }

    #[test]
    fn tool_indices_count_tools_not_content_blocks() {
        let chunks = vec![
            event(
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1", "model": "m", "usage": {"input_tokens": 1}
                }}),
            ),
            event(
                "content_block_start",
                json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "text", "text": ""}}),
            ),
            event(
                "content_block_start",
                json!({"type": "content_block_start", "index": 1,
                "content_block": {"type": "tool_use", "id": "t1", "name": "read", "input": {}}}),
            ),
            event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 1,
                "delta": {"type": "input_json_delta", "partial_json": "{\"p\":"}}),
            ),
            event(
                "content_block_start",
                json!({"type": "content_block_start", "index": 2,
                "content_block": {"type": "tool_use", "id": "t2", "name": "write", "input": {}}}),
            ),
            event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 2,
                "delta": {"type": "input_json_delta", "partial_json": "{}"}}),
            ),
            event(
                "message_delta",
                json!({"type": "message_delta",
                "delta": {"stop_reason": "tool_use"}, "usage": {"output_tokens": 3}}),
            ),
            event("message_stop", json!({"type": "message_stop"})),
        ];
        let out = futures::executor::block_on(run(chunks));
        let tool_deltas: Vec<&Value> = out
            .iter()
            .filter_map(|(_, v)| v.as_ref())
            .filter_map(|p| p.pointer("/choices/0/delta/tool_calls"))
            .collect();
        assert_eq!(tool_deltas[0][0]["index"], 0);
        assert_eq!(tool_deltas[0][0]["id"], "t1");
        assert_eq!(tool_deltas[1][0]["index"], 0);
        assert_eq!(tool_deltas[1][0]["function"]["arguments"], "{\"p\":");
        assert_eq!(tool_deltas[2][0]["index"], 1);
        assert_eq!(tool_deltas[2][0]["id"], "t2");
        let finish = out
            .iter()
            .filter_map(|(_, v)| v.as_ref())
            .find_map(|p| {
                p.pointer("/choices/0/finish_reason")
                    .and_then(|v| v.as_str())
            })
            .unwrap();
        assert_eq!(finish, "tool_calls");
    }

    #[test]
    fn an_error_event_ends_the_stream_without_a_success_terminal() {
        let chunks = vec![
            event(
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1", "model": "m", "usage": {"input_tokens": 1}
                }}),
            ),
            event(
                "error",
                json!({"type": "error",
                "error": {"type": "overloaded_error", "message": "busy"}}),
            ),
        ];
        let out = futures::executor::block_on(run(chunks));
        assert_eq!(out.len(), 3, "{out:?}");
        assert_eq!(out[1].1.as_ref().unwrap()["error"]["message"], "busy");
        assert_eq!(out[2].0, "[DONE]");
    }

    #[test]
    fn a_stream_without_message_stop_still_finishes() {
        let chunks = vec![
            event(
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1", "model": "m", "usage": {"input_tokens": 1}
                }}),
            ),
            event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "hi"}}),
            ),
        ];
        let out = futures::executor::block_on(run(chunks));
        assert_eq!(out.last().unwrap().0, "[DONE]");
        assert!(out
            .iter()
            .filter_map(|(_, v)| v.as_ref())
            .any(|p| p.pointer("/choices/0/finish_reason").is_some()));
    }

    #[test]
    fn the_responses_composition_emits_typed_events() {
        let chunks = vec![
            event(
                "message_start",
                json!({"type": "message_start", "message": {
                    "id": "msg_1", "model": "m", "usage": {"input_tokens": 2}
                }}),
            ),
            event(
                "content_block_delta",
                json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "text_delta", "text": "ok"}}),
            ),
            event(
                "message_delta",
                json!({"type": "message_delta",
                "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 1}}),
            ),
            event("message_stop", json!({"type": "message_stop"})),
        ];
        let input: Vec<Result<Bytes, std::io::Error>> =
            chunks.into_iter().map(|c| Ok(Bytes::from(c))).collect();
        let stream = create_responses_sse_stream_from_anthropic(iter(input));
        let output: Vec<Bytes> = futures::executor::block_on(stream.map(|r| r.unwrap()).collect());
        let text: String = output
            .into_iter()
            .map(|b| String::from_utf8(b.to_vec()).unwrap())
            .collect();
        assert!(text.contains("event: response.created"), "{text}");
        assert!(text.contains("event: response.output_text.delta"), "{text}");
        assert!(text.contains("event: response.completed"), "{text}");
        assert!(!text.contains("chat.completion.chunk"), "{text}");
    }
}
