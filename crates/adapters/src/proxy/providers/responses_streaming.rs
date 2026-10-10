//! Chat Completions SSE → the typed Responses SSE event stream.
//!
//! `create_responses_sse_stream` is the streaming sibling of
//! [`super::streaming::create_anthropic_sse_stream`]: a stateful adapter that
//! takes raw `Bytes`, reassembles complete SSE blocks with the helpers in
//! `proxy::sse`, and emits converted events. It reuses that parsing wholesale —
//! only the emitted event vocabulary differs.
//!
//! Two shapes appear here that the Anthropic adapter does not have:
//!
//! * The Responses wire is a **typed event sequence** (`response.created`,
//!   `response.output_text.delta`, `response.completed`, …), not a single
//!   content-block stream. Every event is written as
//!   `event: <type>\ndata: <json>\n\n`.
//! * There is no `[DONE]` sentinel to forward, deliberately: the Responses
//!   wire ends at `response.completed` (or `response.failed`), so the Chat
//!   `[DONE]` only triggers the terminal sequence and is never re-emitted.
//!
//! The terminal `response.completed` object is assembled with the same helpers
//! `transform::responses_response` uses, so the streaming and non-streaming
//! shapes cannot drift.

use crate::proxy::providers::transform::responses_response::{
    build_response_object, build_responses_usage, mint_item_id, now_unix, response_id_from,
};
use crate::proxy::sse::{append_utf8_safe, strip_sse_field, take_sse_block};
use bytes::Bytes;
use futures::stream::{Stream, StreamExt};
use serde::Deserialize;
use serde_json::{json, Value};

/// The slice of a Chat `chat.completion.chunk` this adapter reads.
#[derive(Debug, Deserialize)]
struct ChatStreamChunk {
    #[serde(default)]
    id: String,
    #[serde(default)]
    model: String,
    #[serde(default)]
    created: Option<i64>,
    #[serde(default)]
    choices: Vec<StreamChoice>,
    #[serde(default)]
    usage: Option<Value>,
    /// An OpenAI-style error object, which rides its own chunk rather than a
    /// choice.
    #[serde(default)]
    error: Option<Value>,
}

// `finish_reason` rides the Chat chunk but is not read: the Responses status
// is decided by the wire (`response.completed`), not by mapping the Chat
// reason onto it, so the field is not modelled at all.
#[derive(Debug, Deserialize)]
struct StreamChoice {
    #[serde(default)]
    delta: Delta,
}

#[derive(Debug, Default, Deserialize)]
struct Delta {
    #[serde(default)]
    content: Option<String>,
    #[serde(default)]
    tool_calls: Option<Vec<DeltaToolCall>>,
}

#[derive(Debug, Deserialize)]
struct DeltaToolCall {
    #[serde(default)]
    index: usize,
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    function: Option<DeltaFunction>,
}

#[derive(Debug, Deserialize)]
struct DeltaFunction {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    arguments: Option<String>,
}

/// One `function_call` item being assembled. The output index is fixed on
/// first appearance so `output_item.added` and its deltas agree; an item is
/// not announced until it has the id and name the Responses item requires,
/// and any arguments that arrived first wait in `pending_args` until then.
///
/// There is deliberately no `consecutive_whitespace` counter here, the one
/// `streaming.rs` carries. That guard bounds a single tool block because the
/// Copilot endpoint streams function arguments as never-ending whitespace, and
/// the Anthropic wire has to abort the block to stop it. The same endpoint is
/// not reached on this path — Codex speaks Responses, not Copilot — and a cap
/// here would truncate the legitimate long arguments a real tool call can
/// carry. If a Responses-shaped misbehaving upstream ever appears, port the
/// counter, not the Anthropic event shape.
#[derive(Debug)]
struct ToolState {
    /// The Chat tool-call index, which is how deltas are routed to this item.
    chat_index: usize,
    output_index: u32,
    item_id: String,
    call_id: String,
    name: String,
    arguments: String,
    started: bool,
    pending_args: String,
}

/// Create a Responses SSE stream from a Chat Completions SSE stream.
pub fn create_responses_sse_stream<E: std::error::Error + Send + 'static>(
    stream: impl Stream<Item = Result<Bytes, E>> + Send + 'static,
) -> impl Stream<Item = Result<Bytes, std::io::Error>> + Send {
    async_stream::stream! {
        let mut buffer = String::new();
        let mut utf8_remainder: Vec<u8> = Vec::new();
        let mut response_id: Option<String> = None;
        let mut model = String::new();
        let mut created_at = now_unix();
        let mut message_item_id: Option<String> = None;
        let mut text = String::new();
        let mut tools: Vec<ToolState> = Vec::new();
        // Usage can arrive on a `choices: []` chunk and/or the last chunk; the
        // latest is held and emitted once inside `response.completed` — the
        // same problem `streaming.rs` solves with `pending_message_delta`.
        let mut latest_usage: Option<Value> = None;
        let mut started = false;
        let mut failed = false;

        tokio::pin!(stream);

        'outer: while let Some(chunk) = stream.next().await {
            match chunk {
                Ok(bytes) => {
                    append_utf8_safe(&mut buffer, &mut utf8_remainder, &bytes);

                    while let Some(block) = take_sse_block(&mut buffer) {
                        if block.trim().is_empty() {
                            continue;
                        }

                        for line in block.lines() {
                            let Some(data) = strip_sse_field(line, "data") else {
                                continue;
                            };
                            let data = data.trim();

                            if data == "[DONE]" {
                                // Not forwarded: the Responses wire has no
                                // sentinel and ends at the terminal event below.
                                break 'outer;
                            }

                            // Malformed JSON is skipped, as the Anthropic
                            // adapter skips it — never a panic.
                            let Ok(parsed) = serde_json::from_str::<ChatStreamChunk>(data) else {
                                continue;
                            };

                            if let Some(error) = &parsed.error {
                                let message = error
                                    .get("message")
                                    .and_then(|m| m.as_str())
                                    .map(str::to_string)
                                    .unwrap_or_else(|| error.to_string());
                                let id = response_id.clone().unwrap_or_default();
                                yield Ok(sse_event(
                                    "response.failed",
                                    &failed_event(&id, &model, created_at, &message),
                                ));
                                failed = true;
                                break 'outer;
                            }

                            if !started {
                                response_id = Some(response_id_from(
                                    if parsed.id.is_empty() { None } else { Some(&parsed.id) },
                                ));
                                if !parsed.model.is_empty() {
                                    model = parsed.model.clone();
                                }
                                created_at = parsed.created.unwrap_or_else(now_unix);
                                let id = response_id.clone().unwrap_or_default();
                                message_item_id = Some(mint_item_id("msg", &id));
                                started = true;

                                let item_id = message_item_id.clone().unwrap_or_default();
                                for (event_type, payload) in
                                    opening_events(&id, &model, created_at, &item_id)
                                {
                                    yield Ok(sse_event(event_type, &payload));
                                }
                            }

                            if let Some(usage) = &parsed.usage {
                                latest_usage = Some(build_responses_usage(usage));
                            }

                            let Some(choice) = parsed.choices.first() else {
                                continue;
                            };

                            if let Some(content) = &choice.delta.content {
                                if !content.is_empty() {
                                    text.push_str(content);
                                    let item_id = message_item_id.clone().unwrap_or_default();
                                    yield Ok(sse_event(
                                        "response.output_text.delta",
                                        &json!({
                                            "type": "response.output_text.delta",
                                            "item_id": item_id,
                                            "output_index": 0,
                                            "content_index": 0,
                                            "delta": content
                                        }),
                                    ));
                                }
                            }

                            if let Some(tool_calls) = &choice.delta.tool_calls {
                                for tool_call in tool_calls {
                                    let slot = tools
                                        .iter()
                                        .position(|tool| tool.chat_index == tool_call.index);
                                    let slot = match slot {
                                        Some(slot) => slot,
                                        None => {
                                            // The message item owns index 0, so
                                            // the first tool call is index 1 and
                                            // the rest follow in first-appearance
                                            // order.
                                            let output_index = 1 + tools.len() as u32;
                                            let call_id =
                                                tool_call.id.clone().unwrap_or_default();
                                            let seed = if call_id.is_empty() {
                                                tool_call.index.to_string()
                                            } else {
                                                call_id.clone()
                                            };
                                            tools.push(ToolState {
                                                chat_index: tool_call.index,
                                                output_index,
                                                item_id: mint_item_id("fc", &seed),
                                                call_id,
                                                name: String::new(),
                                                arguments: String::new(),
                                                started: false,
                                                pending_args: String::new(),
                                            });
                                            tools.len() - 1
                                        }
                                    };

                                    if let Some(id) = &tool_call.id {
                                        tools[slot].call_id = id.clone();
                                    }
                                    if let Some(function) = &tool_call.function {
                                        if let Some(name) = &function.name {
                                            tools[slot].name = name.clone();
                                        }
                                    }
                                    // Arguments that arrive before the item is
                                    // announced are buffered; once it is, they
                                    // flush as the first delta.
                                    let mut immediate_args: Option<String> = None;
                                    if let Some(arguments) = tool_call
                                        .function
                                        .as_ref()
                                        .and_then(|function| function.arguments.as_ref())
                                    {
                                        if tools[slot].started {
                                            tools[slot].arguments.push_str(arguments);
                                            immediate_args = Some(arguments.clone());
                                        } else {
                                            tools[slot].pending_args.push_str(arguments);
                                        }
                                    }

                                    let should_start = !tools[slot].started
                                        && !tools[slot].call_id.is_empty()
                                        && !tools[slot].name.is_empty();
                                    if should_start {
                                        tools[slot].started = true;
                                        let pending =
                                            std::mem::take(&mut tools[slot].pending_args);
                                        tools[slot].arguments.push_str(&pending);
                                        let output_index = tools[slot].output_index;
                                        let item_id = tools[slot].item_id.clone();
                                        let call_id = tools[slot].call_id.clone();
                                        let name = tools[slot].name.clone();

                                        yield Ok(sse_event(
                                            "response.output_item.added",
                                            &json!({
                                                "type": "response.output_item.added",
                                                "output_index": output_index,
                                                "item": {
                                                    "type": "function_call",
                                                    "id": item_id,
                                                    "call_id": call_id,
                                                    "name": name,
                                                    "arguments": "",
                                                    "status": "in_progress"
                                                }
                                            }),
                                        ));

                                        if !pending.is_empty() {
                                            yield Ok(sse_event(
                                                "response.function_call_arguments.delta",
                                                &json!({
                                                    "type": "response.function_call_arguments.delta",
                                                    "item_id": item_id,
                                                    "output_index": output_index,
                                                    "delta": pending
                                                }),
                                            ));
                                        }
                                    }

                                    if let Some(arguments) = immediate_args {
                                        let output_index = tools[slot].output_index;
                                        let item_id = tools[slot].item_id.clone();
                                        yield Ok(sse_event(
                                            "response.function_call_arguments.delta",
                                            &json!({
                                                "type": "response.function_call_arguments.delta",
                                                "item_id": item_id,
                                                "output_index": output_index,
                                                "delta": arguments
                                            }),
                                        ));
                                    }
                                }
                            }
                        }
                    }
                }
                Err(e) => {
                    // The Responses wire has `response.failed`, not the
                    // Anthropic adapter's `error` event; the behaviour is the
                    // same — the failure is surfaced and the success terminal
                    // is suppressed.
                    log::error!("[Responses/Chat] stream error: {e}");
                    let message = format!("Stream error: {e}");
                    let id = response_id.clone().unwrap_or_default();
                    yield Ok(sse_event(
                        "response.failed",
                        &failed_event(&id, &model, created_at, &message),
                    ));
                    failed = true;
                    break;
                }
            }
        }

        // The stream ended naturally (or at `[DONE]`): close the response out.
        // An explicit failure keeps only its `response.failed` event, so a
        // failure is not disguised as a successful completion.
        if !failed {
            if !started {
                // Nothing was ever parsed but the stream still ended — emit a
                // well-formed empty response rather than nothing.
                response_id = Some(response_id_from(None));
                let id = response_id.clone().unwrap_or_default();
                message_item_id = Some(mint_item_id("msg", &id));
                let item_id = message_item_id.clone().unwrap_or_default();
                for (event_type, payload) in opening_events(&id, &model, created_at, &item_id) {
                    yield Ok(sse_event(event_type, &payload));
                }
            }

            let id = response_id.clone().unwrap_or_default();
            let item_id = message_item_id.clone().unwrap_or_default();
            let usage = latest_usage
                .clone()
                .unwrap_or_else(|| build_responses_usage(&json!({})));
            for (event_type, payload) in
                finalize_events(&id, &model, created_at, &item_id, &text, &tools, usage)
            {
                yield Ok(sse_event(event_type, &payload));
            }
        }
    }
}

/// The three events that open a response, shared so the chunk path and the
/// empty-stream path cannot disagree. Written by `response.created` first with
/// no output, then the assistant message item and its empty text part.
fn opening_events(
    id: &str,
    model: &str,
    created_at: i64,
    message_item_id: &str,
) -> Vec<(&'static str, Value)> {
    vec![
        (
            "response.created",
            json!({
                "type": "response.created",
                "response": build_response_object(
                    id, model, created_at, Vec::new(), Value::Null, "in_progress"
                )
            }),
        ),
        (
            "response.output_item.added",
            json!({
                "type": "response.output_item.added",
                "output_index": 0,
                "item": {
                    "type": "message",
                    "id": message_item_id,
                    "status": "in_progress",
                    "role": "assistant",
                    "content": []
                }
            }),
        ),
        (
            "response.content_part.added",
            json!({
                "type": "response.content_part.added",
                "item_id": message_item_id,
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": "", "annotations": []}
            }),
        ),
    ]
}

/// The events that close a response: the message's `…done` trio, then each
/// tool call's `function_call_arguments.done` and `output_item.done`, then
/// `response.completed` carrying the assembled whole.
fn finalize_events(
    id: &str,
    model: &str,
    created_at: i64,
    message_item_id: &str,
    text: &str,
    tools: &[ToolState],
    usage: Value,
) -> Vec<(&'static str, Value)> {
    let mut events: Vec<(&'static str, Value)> = vec![
        (
            "response.output_text.done",
            json!({
                "type": "response.output_text.done",
                "item_id": message_item_id,
                "output_index": 0,
                "content_index": 0,
                "text": text
            }),
        ),
        (
            "response.content_part.done",
            json!({
                "type": "response.content_part.done",
                "item_id": message_item_id,
                "output_index": 0,
                "content_index": 0,
                "part": {"type": "output_text", "text": text, "annotations": []}
            }),
        ),
        (
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": 0,
                "item": {
                    "type": "message",
                    "id": message_item_id,
                    "status": "completed",
                    "role": "assistant",
                    "content": [{"type": "output_text", "text": text, "annotations": []}]
                }
            }),
        ),
    ];

    let mut output = vec![json!({
        "type": "message",
        "id": message_item_id,
        "status": "completed",
        "role": "assistant",
        "content": [{"type": "output_text", "text": text, "annotations": []}]
    })];

    for tool in tools {
        if !tool.started {
            continue;
        }
        events.push((
            "response.function_call_arguments.done",
            json!({
                "type": "response.function_call_arguments.done",
                "item_id": tool.item_id,
                "output_index": tool.output_index,
                "arguments": tool.arguments
            }),
        ));
        events.push((
            "response.output_item.done",
            json!({
                "type": "response.output_item.done",
                "output_index": tool.output_index,
                "item": {
                    "type": "function_call",
                    "id": tool.item_id,
                    "call_id": tool.call_id,
                    "name": tool.name,
                    "arguments": tool.arguments,
                    "status": "completed"
                }
            }),
        ));
        output.push(json!({
            "type": "function_call",
            "id": tool.item_id,
            "call_id": tool.call_id,
            "name": tool.name,
            "arguments": tool.arguments,
            "status": "completed"
        }));
    }

    events.push((
        "response.completed",
        json!({
            "type": "response.completed",
            "response": build_response_object(id, model, created_at, output, usage, "completed")
        }),
    ));

    events
}

/// A `response.failed` payload. `error` is the one place this build emits the
/// field, because a failure with no reason is not worth surfacing.
fn failed_event(id: &str, model: &str, created_at: i64, message: &str) -> Value {
    let mut response =
        build_response_object(id, model, created_at, Vec::new(), Value::Null, "failed");
    response["error"] = json!({"code": null, "message": message});
    json!({"type": "response.failed", "response": response})
}

/// One SSE event in the Responses frame. JSON that failed to serialize yields
/// an empty data line rather than a panic, as the Anthropic adapter does.
fn sse_event(event_type: &str, payload: &Value) -> Bytes {
    Bytes::from(format!(
        "event: {event_type}\ndata: {}\n\n",
        serde_json::to_string(payload).unwrap_or_default()
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::stream;

    /// Feed a body of SSE blocks as one chunk and parse the emitted `data:`
    /// payloads back into `Value`s.
    async fn collect_events(input: &str) -> Vec<Value> {
        let upstream = stream::iter(vec![Ok::<_, std::io::Error>(Bytes::from(
            input.as_bytes().to_vec(),
        ))]);
        let converted = create_responses_sse_stream(upstream);
        let chunks: Vec<_> = converted.collect().await;
        let merged = chunks
            .into_iter()
            .map(|chunk| String::from_utf8_lossy(chunk.unwrap().as_ref()).to_string())
            .collect::<String>();

        merged
            .split("\n\n")
            .filter_map(|block| {
                let data = block
                    .lines()
                    .find_map(|line| strip_sse_field(line, "data"))?;
                serde_json::from_str::<Value>(data).ok()
            })
            .collect()
    }

    fn types(events: &[Value]) -> Vec<&str> {
        events
            .iter()
            .filter_map(|event| event.get("type").and_then(Value::as_str))
            .collect()
    }

    #[tokio::test]
    async fn a_two_chunk_text_stream_emits_the_typed_sequence_and_the_usage() {
        let input = concat!(
            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"created\":100,\"choices\":[{\"delta\":{\"content\":\"Hel\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"id\":\"chatcmpl_1\",\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n"
        );

        let events = collect_events(input).await;
        assert_eq!(
            types(&events),
            vec![
                "response.created",
                "response.output_item.added",
                "response.content_part.added",
                "response.output_text.delta",
                "response.output_text.delta",
                "response.output_text.done",
                "response.content_part.done",
                "response.output_item.done",
                "response.completed",
            ]
        );

        assert_eq!(events[3]["delta"], "Hel");
        assert_eq!(events[3]["content_index"], 0);
        assert_eq!(events[3]["output_index"], 0);
        assert_eq!(events[4]["delta"], "lo");

        let completed = events.last().unwrap();
        assert_eq!(completed["response"]["status"], "completed");
        assert_eq!(completed["response"]["model"], "gpt-4o");
        assert_eq!(completed["response"]["created_at"], 100);
        assert_eq!(
            completed["response"]["output"][0]["content"][0]["text"],
            "Hello"
        );
        assert_eq!(completed["response"]["usage"]["input_tokens"], 3);
        assert_eq!(completed["response"]["usage"]["output_tokens"], 2);
    }

    #[tokio::test]
    async fn a_tool_call_stream_names_the_arguments_by_its_chat_id() {
        let input = concat!(
            "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_7\",\"type\":\"function\",\"function\":{\"name\":\"get_weather\",\"arguments\":\"{\\\"city\\\":\"}}]}}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"\\\"Tokyo\\\"}\"}}]}}]}\n\n",
            "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
            "data: [DONE]\n\n"
        );

        let events = collect_events(input).await;

        let added = events
            .iter()
            .find(|event| {
                event["type"] == "response.output_item.added"
                    && event["item"]["type"] == "function_call"
            })
            .expect("a function_call output_item.added");
        assert_eq!(added["item"]["call_id"], "call_7");
        assert_eq!(added["item"]["name"], "get_weather");
        assert_eq!(added["output_index"], 1);

        let deltas: Vec<&str> = events
            .iter()
            .filter(|event| event["type"] == "response.function_call_arguments.delta")
            .filter_map(|event| event["delta"].as_str())
            .collect();
        assert_eq!(deltas, vec!["{\"city\":", "\"Tokyo\"}"]);

        let done = events
            .iter()
            .rev()
            .find(|event| {
                event["type"] == "response.output_item.done"
                    && event["item"]["type"] == "function_call"
            })
            .expect("a function_call output_item.done");
        assert_eq!(done["item"]["call_id"], "call_7");
        assert_eq!(done["item"]["arguments"], "{\"city\":\"Tokyo\"}");

        // The assembled function call rides the completed response too.
        let completed = events.last().unwrap();
        let function_call = completed["response"]["output"]
            .as_array()
            .unwrap()
            .iter()
            .find(|item| item["type"] == "function_call")
            .unwrap();
        assert_eq!(function_call["call_id"], "call_7");
    }

    #[tokio::test]
    async fn a_done_terminated_stream_ends_at_response_completed_without_an_extra_event() {
        let input = concat!(
            "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: [DONE]\n\n"
        );

        let events = collect_events(input).await;
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        assert_eq!(
            events
                .iter()
                .filter(|event| event["type"] == "response.completed")
                .count(),
            1,
            "the terminal event is emitted exactly once"
        );
        assert!(!types(&events).contains(&"response.failed"));
    }

    #[tokio::test]
    async fn a_stream_without_done_still_completes() {
        let input = "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hi\"},\"finish_reason\":\"stop\"}]}\n\n";

        let events = collect_events(input).await;
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        assert_eq!(
            events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
            "hi"
        );
    }

    #[tokio::test]
    async fn a_malformed_json_chunk_is_skipped_not_fatal() {
        let input = concat!(
            "data: {\"id\":\"c\",\"model\":\"gpt-4o\",\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n\n",
            "data: {this is not json}\n\n",
            "data: [DONE]\n\n"
        );

        let events = collect_events(input).await;
        assert_eq!(events.last().unwrap()["type"], "response.completed");
        assert_eq!(
            events.last().unwrap()["response"]["output"][0]["content"][0]["text"],
            "hi"
        );
    }

    #[tokio::test]
    async fn an_upstream_error_chunk_surfaces_as_response_failed() {
        let input =
            "data: {\"error\":{\"message\":\"upstream said no\",\"type\":\"server_error\"}}\n\n";

        let events = collect_events(input).await;
        let failed = events
            .iter()
            .find(|event| event["type"] == "response.failed")
            .expect("a response.failed event");
        assert_eq!(failed["response"]["status"], "failed");
        assert_eq!(failed["response"]["error"]["message"], "upstream said no");
        assert!(!types(&events).contains(&"response.completed"));
    }
}
