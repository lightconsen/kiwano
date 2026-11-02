//! Assembling a buffered SSE body into the completion it described.
//!
//! Not every provider honours `stream: false`. A reseller that only implements
//! the streaming path answers a non-streaming request with `200 OK` and an
//! event stream anyway — and the gateway, which buffers a non-SSE response and
//! parses it as one JSON document, then fails with "upstream body is not JSON"
//! even though the answer is all there, one `data:` line at a time.
//!
//! The aggregation below is the same answer the client would have got by
//! reading the stream: text concatenated in order, tool calls merged by index,
//! the last `finish_reason`, and the usage object the final chunk carried.

use serde_json::{json, Map, Value};

/// Whether a buffered body is an event stream rather than a JSON document.
///
/// Read from the bytes, not the content type: the provider that ignores
/// `stream: false` is exactly the one whose `Content-Type` may say
/// `application/json` anyway. A JSON body cannot begin with any of these
/// fields, so the test cannot claim a document it would then mis-assemble.
pub fn looks_like_sse(body: &[u8]) -> bool {
    let head = &body[..body.len().min(64)];
    let head = String::from_utf8_lossy(head);
    let head = head.trim_start();
    ["data:", "event:", "id:", "retry:"]
        .iter()
        .any(|field| head.starts_with(field))
}

/// Assemble an OpenAI chat-completion chunk stream into one `chat.completion`.
///
/// `None` when no `data:` line parsed as an object — a body that merely starts
/// with `data:` but carries nothing usable is not a completion, and the caller
/// should report what it actually received instead of inventing an empty one.
pub fn assemble_chat_sse(body: &[u8]) -> Option<Value> {
    let text = String::from_utf8_lossy(body);

    let mut id: Option<String> = None;
    let mut model: Option<String> = None;
    let mut created: Option<Value> = None;
    let mut role = "assistant".to_string();
    let mut content = String::new();
    let mut reasoning = String::new();
    // Index → the call being assembled. Ordered by the `BTreeMap`, which is the
    // order OpenAI's own indexes give.
    let mut calls: std::collections::BTreeMap<u64, ToolCall> = std::collections::BTreeMap::new();
    let mut finish_reason: Option<String> = None;
    let mut usage: Option<Value> = None;
    let mut parsed_any = false;

    for line in text.lines() {
        let Some(payload) = line
            .strip_prefix("data:")
            .map(|p| p.trim_start())
            .filter(|p| !p.is_empty() && *p != "[DONE]")
        else {
            continue;
        };
        let Ok(chunk) = serde_json::from_str::<Value>(payload) else {
            continue;
        };
        let Some(obj) = chunk.as_object() else {
            continue;
        };
        parsed_any = true;

        id = id.or_else(|| string_at(obj, "id"));
        model = model.or_else(|| string_at(obj, "model"));
        created = created.or_else(|| obj.get("created").cloned());
        // The last object that carries usage is the one that counted the whole
        // stream (`stream_options.include_usage` puts it on a final chunk).
        if let Some(u) = obj.get("usage").filter(|u| !u.is_null()) {
            usage = Some(u.clone());
        }

        let Some(choice) = obj
            .get("choices")
            .and_then(Value::as_array)
            .and_then(|c| c.first())
            .and_then(Value::as_object)
        else {
            continue;
        };
        if let Some(reason) = string_at(choice, "finish_reason") {
            finish_reason = Some(reason);
        }
        let Some(delta) = choice.get("delta").and_then(Value::as_object) else {
            continue;
        };
        if let Some(r) = string_at(delta, "role") {
            role = r;
        }
        if let Some(part) = delta.get("content") {
            content.push_str(&text_of(part));
        }
        if let Some(part) = delta
            .get("reasoning_content")
            .or_else(|| delta.get("reasoning"))
        {
            reasoning.push_str(&text_of(part));
        }
        if let Some(list) = delta.get("tool_calls").and_then(Value::as_array) {
            for call in list {
                let Some(call) = call.as_object() else {
                    continue;
                };
                let index = call.get("index").and_then(Value::as_u64).unwrap_or(0);
                let entry = calls.entry(index).or_default();
                if let Some(v) = string_at(call, "id") {
                    entry.id = Some(v);
                }
                if let Some(v) = string_at(call, "type") {
                    entry.call_type = Some(v);
                }
                if let Some(function) = call.get("function").and_then(Value::as_object) {
                    if let Some(v) = string_at(function, "name") {
                        entry.name = Some(v);
                    }
                    if let Some(v) = function.get("arguments") {
                        entry.arguments.push_str(&text_of(v));
                    }
                }
            }
        }
    }

    if !parsed_any {
        return None;
    }

    let mut message = Map::new();
    message.insert("role".into(), Value::String(role));
    // `null` is the chat-completion spelling of "no text": a turn that is only
    // tool calls has an empty `content`, and an empty string would be a claim
    // that the assistant said nothing rather than that it said nothing in text.
    let has_calls = !calls.is_empty();
    message.insert(
        "content".into(),
        if content.is_empty() && has_calls {
            Value::Null
        } else {
            Value::String(content)
        },
    );
    if !reasoning.is_empty() {
        message.insert("reasoning_content".into(), Value::String(reasoning));
    }
    if has_calls {
        message.insert(
            "tool_calls".into(),
            Value::Array(
                calls
                    .into_iter()
                    .map(|(index, call)| call.into_json(index))
                    .collect(),
            ),
        );
    }

    let mut choice = Map::new();
    choice.insert("index".into(), json!(0));
    choice.insert("message".into(), Value::Object(message));
    choice.insert(
        "finish_reason".into(),
        finish_reason.map(Value::String).unwrap_or(Value::Null),
    );

    let mut out = Map::new();
    out.insert("id".into(), id.map(Value::String).unwrap_or(Value::Null));
    out.insert("object".into(), json!("chat.completion"));
    out.insert(
        "created".into(),
        created.unwrap_or_else(|| json!(chrono::Utc::now().timestamp())),
    );
    out.insert(
        "model".into(),
        model.map(Value::String).unwrap_or(Value::Null),
    );
    out.insert("choices".into(), Value::Array(vec![Value::Object(choice)]));
    if let Some(usage) = usage {
        out.insert("usage".into(), usage);
    }
    Some(Value::Object(out))
}

#[derive(Default)]
struct ToolCall {
    id: Option<String>,
    call_type: Option<String>,
    name: Option<String>,
    arguments: String,
}

impl ToolCall {
    fn into_json(self, index: u64) -> Value {
        let mut function = Map::new();
        function.insert(
            "name".into(),
            self.name.map(Value::String).unwrap_or(Value::Null),
        );
        function.insert("arguments".into(), Value::String(self.arguments));
        let mut call = Map::new();
        call.insert("index".into(), json!(index));
        call.insert(
            "id".into(),
            self.id.map(Value::String).unwrap_or(Value::Null),
        );
        call.insert(
            "type".into(),
            Value::String(self.call_type.unwrap_or_else(|| "function".into())),
        );
        call.insert("function".into(), Value::Object(function));
        Value::Object(call)
    }
}

fn string_at(obj: &Map<String, Value>, key: &str) -> Option<String> {
    obj.get(key)
        .and_then(Value::as_str)
        .map(str::to_string)
        .filter(|s| !s.is_empty())
}

/// A delta field that may be a string or an array of content parts.
fn text_of(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Array(parts) => parts
            .iter()
            .filter_map(|p| p.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_chunk_stream_becomes_one_completion() {
        let body = concat!(
            "data: {\"id\":\"c1\",\"model\":\"m\",\"created\":1700000000,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hel\"}}]}\n\n",
            "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"}}]}\n\n",
            "data: {\"id\":\"c1\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":3,\"completion_tokens\":2,\"total_tokens\":5}}\n\n",
            "data: [DONE]\n\n",
        );
        let v = assemble_chat_sse(body.as_bytes()).expect("assembled");
        assert_eq!(v["object"], "chat.completion");
        assert_eq!(v["id"], "c1");
        assert_eq!(v["created"], 1700000000);
        assert_eq!(v["choices"][0]["message"]["role"], "assistant");
        assert_eq!(v["choices"][0]["message"]["content"], "Hello");
        assert_eq!(v["choices"][0]["finish_reason"], "stop");
        assert_eq!(v["usage"]["total_tokens"], 5);
    }

    #[test]
    fn tool_calls_merge_by_index_across_chunks() {
        let body = concat!(
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"t1\",\"type\":\"function\",\"function\":{\"name\":\"read\",\"arguments\":\"{\\\"pa\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"function\":{\"arguments\":\"th\\\":1}\"}}]}}]}\n\n",
            "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}]}\n\n",
        );
        let v = assemble_chat_sse(body.as_bytes()).expect("assembled");
        let m = &v["choices"][0]["message"];
        assert_eq!(
            m["content"],
            Value::Null,
            "no text is null, not an empty string"
        );
        let calls = m["tool_calls"].as_array().expect("a call");
        assert_eq!(calls.len(), 1);
        assert_eq!(calls[0]["id"], "t1");
        assert_eq!(calls[0]["function"]["name"], "read");
        assert_eq!(calls[0]["function"]["arguments"], "{\"path\":1}");
        assert_eq!(v["choices"][0]["finish_reason"], "tool_calls");
    }

    #[test]
    fn a_body_that_is_not_a_stream_is_not_assembled() {
        assert!(!looks_like_sse(br#"{"choices":[]}"#));
        assert!(looks_like_sse(b"data: {\"choices\":[]}\n\n"));
        // `data:` with nothing usable in it is not a completion to invent.
        assert_eq!(assemble_chat_sse(b"data: [DONE]\n\n"), None);
        assert_eq!(assemble_chat_sse(b"data: not json\n\n"), None);
    }
}
