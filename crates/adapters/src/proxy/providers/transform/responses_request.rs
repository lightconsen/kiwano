//! OpenAI Responses request → OpenAI Chat Completions request.
//!
//! `responses_to_chat` is what an agent that speaks the Responses API (Codex)
//! needs when the provider behind the gateway only offers Chat Completions.
//! It is the forward half of a pair whose reverse — a Chat request arriving
//! for a Responses-only upstream — is still refused: every client in the wild
//! already speaks one wire or the other, and inventing an `input` to receive a
//! Chat request would be a conversion nobody asked for.
//!
//! Two refusals guard the same property. The Responses API is state-bearing:
//! `previous_response_id` and `store: true` both ask an endpoint to hold a
//! response on the server and continue from it. A Chat Completions endpoint
//! holds nothing, so either field becomes a request that is served but
//! disconnected from what the client believes it is continuing — a wrong
//! answer with no way to tell. Both are refused with that reason rather than
//! approximated, the same rule `transform::request` applies to an Anthropic
//! `document` block.
//!
//! A leaf: one JSON value in, one out. `request` and this file share nothing.

use crate::proxy::error::ProxyError;
use crate::proxy::providers::transform::reasoning::supports_reasoning_effort;
use serde_json::{json, Value};

/// Effort values a Chat Completions `reasoning_effort` accepts. The Responses
/// API spells the same knob `reasoning.effort`; a value outside this set has no
/// Chat home, and forwarding it would be guessing at a vendor's vocabulary.
const ACCEPTED_REASONING_EFFORTS: &[&str] = &["minimal", "low", "medium", "high", "xhigh", "none"];

/// Top-level keys that are already Chat Completions parameters. The Responses
/// API carries none of them, so a body that has one is a client that mixed the
/// wires — pass it through rather than silently reinterpret it.
const CHAT_PASSTHROUGH: &[&str] = &[
    "frequency_penalty",
    "presence_penalty",
    "logit_bias",
    "logprobs",
    "top_logprobs",
    "n",
    "seed",
    "stop",
    "service_tier",
];

/// OpenAI Responses request → OpenAI Chat Completions request.
pub fn responses_to_chat(body: Value) -> Result<Value, ProxyError> {
    let mut result = json!({});

    // The Responses API is state-bearing. Both of these name a response the
    // upstream is expected to have kept; a Chat endpoint keeps none, so the
    // request would be answered from nothing.
    if body
        .get("previous_response_id")
        .is_some_and(|v| !v.is_null())
    {
        return Err(ProxyError::TransformError(
            "cannot convert a Responses request carrying `previous_response_id`: the Responses API is state-bearing and a Chat Completions endpoint has no prior response to continue from; replay the earlier turns in `input` instead".to_string(),
        ));
    }
    if body.get("store").and_then(|v| v.as_bool()) == Some(true) {
        return Err(ProxyError::TransformError(
            "cannot convert a Responses request with `store: true` to Chat Completions: a Chat endpoint has no store to keep the response in, so a later `previous_response_id` would name a response that was never kept".to_string(),
        ));
    }

    // As in the Anthropic direction, this layer only moves the shape around —
    // choosing or rewriting the model is the caller's business.
    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        result["model"] = json!(model);
    }

    let mut messages = Vec::new();

    if let Some(instructions) = body.get("instructions").and_then(|v| v.as_str()) {
        if !instructions.is_empty() {
            messages.push(json!({"role": "system", "content": instructions}));
        }
    }

    match body.get("input") {
        Some(Value::String(text)) => {
            messages.push(json!({"role": "user", "content": text}));
        }
        Some(Value::Array(items)) => {
            for item in items {
                convert_input_item(item, &mut messages)?;
            }
        }
        _ => {
            // A body with neither `input` nor `instructions` but a Chat
            // `messages` array is already Chat-shaped, so it is carried over
            // rather than dropped.
            if let Some(chat_messages) = body.get("messages").and_then(|m| m.as_array()) {
                messages.extend(chat_messages.iter().cloned());
            }
        }
    }

    result["messages"] = json!(messages);

    if let Some(v) = body.get("max_output_tokens") {
        result["max_tokens"] = v.clone();
    }
    if let Some(v) = body.get("parallel_tool_calls") {
        result["parallel_tool_calls"] = v.clone();
    }
    if let Some(v) = body.get("temperature") {
        result["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        result["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stream") {
        result["stream"] = v.clone();
    }
    if let Some(v) = body.get("user") {
        result["user"] = v.clone();
    }

    // `reasoning.effort` is the same knob as Chat's `reasoning_effort`, but the
    // two vocabularies differ and only a reasoning model understands either.
    if let Some(effort) = body.pointer("/reasoning/effort").and_then(|v| v.as_str()) {
        let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
        if supports_reasoning_effort(model) && ACCEPTED_REASONING_EFFORTS.contains(&effort) {
            result["reasoning_effort"] = json!(effort);
        } else {
            log::debug!(
                "[Responses/Chat] dropping reasoning effort `{effort}` for model `{model}`: not an accepted value"
            );
        }
    }

    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let mut openai_tools = Vec::new();
        for tool in tools {
            // A Chat endpoint has no built-in tools at all: `web_search`,
            // `file_search`, `code_interpreter`, `computer_use`, `mcp` and the
            // rest are features of the Responses host. Dropping one silently
            // would turn a request that asked for grounding into an answer the
            // model produced without it, so an unknown type is refused by name.
            let tool_type = tool
                .get("type")
                .and_then(|t| t.as_str())
                .unwrap_or("function");
            if tool_type != "function" {
                return Err(ProxyError::TransformError(format!(
                    "cannot convert the built-in tool `{tool_type}` to Chat Completions: a Chat endpoint has no built-in tools, and dropping it would answer a request that asked for grounding without it"
                )));
            }

            let mut function = json!({
                "name": tool.get("name").and_then(|n| n.as_str()).unwrap_or(""),
            });
            if let Some(description) = tool.get("description") {
                function["description"] = description.clone();
            }
            function["parameters"] = tool
                .get("parameters")
                .cloned()
                .unwrap_or_else(|| json!({"type": "object", "properties": {}}));
            // `strict` is a Responses-only structured-output hint; Chat
            // Completions has no matching field, so it is dropped rather than
            // renamed into one that means something else.
            openai_tools.push(json!({"type": "function", "function": function}));
        }
        if !openai_tools.is_empty() {
            result["tools"] = json!(openai_tools);
        }
    }

    if let Some(tool_choice) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice(tool_choice);
    }

    // `text.format` is the Responses spelling of structured output; its
    // `json_schema` form is the one Chat can express.
    if let Some(format) = body.pointer("/text/format") {
        if format.get("type").and_then(|t| t.as_str()) == Some("json_schema") {
            let mut json_schema = json!({
                "name": format.get("name").and_then(|n| n.as_str()).unwrap_or("response"),
                "schema": format.get("schema").cloned().unwrap_or_else(|| json!({})),
            });
            if let Some(strict) = format.get("strict") {
                json_schema["strict"] = strict.clone();
            }
            result["response_format"] = json!({"type": "json_schema", "json_schema": json_schema});
        }
        // `text.format` also spells `text` and `json_object`; neither has a Chat
        // Completions spelling that means the same thing, so both are dropped
        // rather than mapped onto a different mode of structured output.
    }

    for &key in CHAT_PASSTHROUGH {
        if let Some(v) = body.get(key) {
            result[key] = v.clone();
        }
    }

    // Deliberately dropped, each for the same reason: `metadata`, `include`,
    // `truncation`, `prompt_cache_key`, `store` and `stream_options` configure
    // the Responses endpoint (an opaque client tag, encrypted reasoning
    // payloads, server-side history, a server-held cache) and have no Chat
    // Completions field that would mean the same thing.

    Ok(result)
}

/// One Responses `input` item → zero or more Chat messages.
fn convert_input_item(item: &Value, messages: &mut Vec<Value>) -> Result<(), ProxyError> {
    let Some(item_type) = item.get("type").and_then(|t| t.as_str()) else {
        // No `type` but a `role` is already a Chat message — pass it through
        // rather than guess a conversion for it.
        if item.get("role").is_some() {
            messages.push(item.clone());
        }
        return Ok(());
    };

    match item_type {
        "message" => {
            let role = item.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = convert_content_parts(item.get("content"));
            messages.push(json!({"role": role, "content": content}));
        }
        "function_call" => {
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            let arguments = match item.get("arguments") {
                Some(Value::String(s)) => s.clone(),
                Some(other) => other.to_string(),
                None => "{}".to_string(),
            };
            messages.push(json!({
                "role": "assistant",
                "tool_calls": [{
                    "id": call_id,
                    "type": "function",
                    "function": {"name": name, "arguments": arguments}
                }]
            }));
        }
        "function_call_output" => {
            let call_id = item.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            let content = tool_output_text(item.get("output"));
            messages.push(json!({
                "role": "tool",
                "tool_call_id": call_id,
                "content": content
            }));
        }
        // A reasoning item is the model's private trace as the Responses API
        // stores it. A Chat upstream has no field to carry one back, and
        // dropping it costs nothing: the trace is not replayed on the next turn.
        "reasoning" => {}
        other => {
            return Err(ProxyError::TransformError(format!(
                "cannot convert the Responses input item type `{other}` to Chat Completions: this build has no mapping for it, and dropping it would change the conversation in silence"
            )));
        }
    }
    Ok(())
}

/// The `content` of a Responses message → the Chat content it becomes.
fn convert_content_parts(content: Option<&Value>) -> Value {
    match content {
        Some(Value::String(text)) => json!(text),
        Some(Value::Array(parts)) => {
            let mut out = Vec::new();
            for part in parts {
                match part.get("type").and_then(|t| t.as_str()) {
                    Some("input_text") | Some("output_text") => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            out.push(json!({"type": "text", "text": text}));
                        }
                    }
                    Some("input_image") => {
                        if let Some(url) = chat_image_url(part) {
                            out.push(json!({"type": "image_url", "image_url": {"url": url}}));
                        }
                    }
                    // Other part types (audio, files) have no Chat content-block
                    // form this build carries; they are passed over.
                    _ => {}
                }
            }
            json!(out)
        }
        _ => json!([]),
    }
}

/// A Responses `input_image` part → the Chat image URL. Bytes are folded into
/// the `data:` URI Chat expects.
fn chat_image_url(part: &Value) -> Option<String> {
    if let Some(url) = part.get("image_url").and_then(|v| v.as_str()) {
        return Some(url.to_string());
    }
    let data = part.get("data").and_then(|v| v.as_str())?;
    let media_type = part
        .get("media_type")
        .and_then(|v| v.as_str())
        .unwrap_or("image/png");
    Some(format!("data:{media_type};base64,{data}"))
}

/// A `function_call_output.output` → the string a Chat `tool` message carries.
fn tool_output_text(output: Option<&Value>) -> String {
    match output {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(parts)) => parts
            .iter()
            .filter_map(|part| part.get("text").and_then(|t| t.as_str()))
            .collect::<Vec<_>>()
            .join(""),
        Some(other) => other.to_string(),
        None => String::new(),
    }
}

/// The Responses forced-tool selector is flat (`{"type":"function","name":"X"}`);
/// Chat nests it.
fn map_tool_choice(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::Object(obj) if obj.get("type").and_then(|t| t.as_str()) == Some("function") => {
            let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
            json!({"type": "function", "function": {"name": name}})
        }
        other => other.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_input_string_becomes_a_single_user_message() {
        let out = responses_to_chat(json!({"model": "gpt-4o", "input": "hello"})).unwrap();
        assert_eq!(out["messages"].as_array().unwrap().len(), 1);
        assert_eq!(out["messages"][0]["role"], "user");
        assert_eq!(out["messages"][0]["content"], "hello");
    }

    #[test]
    fn instructions_lead_the_messages_and_items_follow_in_order() {
        let body = json!({
            "model": "gpt-4o",
            "instructions": "be terse",
            "input": [
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "hi"}]},
                {"type": "message", "role": "assistant",
                 "content": [{"type": "output_text", "text": "ok"}]}
            ]
        });

        let out = responses_to_chat(body).unwrap();
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "be terse");
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(messages[1]["content"][0]["type"], "text");
        assert_eq!(messages[1]["content"][0]["text"], "hi");
        assert_eq!(messages[2]["role"], "assistant");
        assert_eq!(messages[2]["content"][0]["text"], "ok");
    }

    #[test]
    fn a_function_call_and_its_output_become_a_tool_call_and_a_tool_turn() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"type": "function_call", "call_id": "call_1", "name": "get_weather",
                 "arguments": "{\"city\":\"Tokyo\"}"},
                {"type": "function_call_output", "call_id": "call_1", "output": "sunny, 25C"}
            ]
        });

        let out = responses_to_chat(body).unwrap();
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages[0]["role"], "assistant");
        assert_eq!(messages[0]["tool_calls"][0]["id"], "call_1");
        assert_eq!(messages[0]["tool_calls"][0]["type"], "function");
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["name"],
            "get_weather"
        );
        assert_eq!(
            messages[0]["tool_calls"][0]["function"]["arguments"],
            "{\"city\":\"Tokyo\"}"
        );
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[1]["tool_call_id"], "call_1");
        assert_eq!(messages[1]["content"], "sunny, 25C");
    }

    #[test]
    fn max_output_tokens_becomes_max_tokens() {
        let out =
            responses_to_chat(json!({"model": "gpt-4o", "input": "hi", "max_output_tokens": 256}))
                .unwrap();
        assert_eq!(out["max_tokens"], 256);
        assert!(out.get("max_output_tokens").is_none());
    }

    #[test]
    fn an_inline_image_becomes_a_data_uri() {
        let body = json!({
            "model": "gpt-4o",
            "input": [{"type": "message", "role": "user", "content": [
                {"type": "input_text", "text": "what is this"},
                {"type": "input_image", "data": "QUJD", "media_type": "image/png"}
            ]}]
        });

        let out = responses_to_chat(body).unwrap();
        let content = out["messages"][0]["content"].as_array().unwrap();
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[1]["type"], "image_url");
        assert_eq!(content[1]["image_url"]["url"], "data:image/png;base64,QUJD");
    }

    #[test]
    fn a_remote_image_keeps_its_url() {
        let body = json!({
            "model": "gpt-4o",
            "input": [{"type": "message", "role": "user", "content": [
                {"type": "input_image", "image_url": "https://example.com/a.png"}
            ]}]
        });

        let out = responses_to_chat(body).unwrap();
        assert_eq!(
            out["messages"][0]["content"][0]["image_url"]["url"],
            "https://example.com/a.png"
        );
    }

    #[test]
    fn a_previous_response_id_is_refused_as_state_the_upstream_cannot_hold() {
        let body = json!({"model": "gpt-4o", "input": "hi", "previous_response_id": "resp_1"});

        let err = responses_to_chat(body).unwrap_err();
        assert!(
            matches!(err, ProxyError::TransformError(_)),
            "expected a transform refusal, got {err:?}"
        );
        assert!(
            err.to_string().contains("previous_response_id"),
            "the refusal names the field: {err}"
        );
        assert!(
            err.to_string().contains("state-bearing"),
            "the refusal says why, not just that it is unsupported: {err}"
        );
    }

    #[test]
    fn store_true_is_refused_because_a_chat_endpoint_has_no_store() {
        let body = json!({"model": "gpt-4o", "input": "hi", "store": true});

        let err = responses_to_chat(body).unwrap_err();
        assert!(matches!(err, ProxyError::TransformError(_)));
        assert!(err.to_string().contains("store"));
        assert!(
            err.to_string().contains("no store"),
            "the refusal explains the missing store: {err}"
        );
    }

    #[test]
    fn store_false_is_not_a_refusal() {
        let out = responses_to_chat(json!({"model": "gpt-4o", "input": "hi", "store": false}))
            .expect("store: false names no retained state");
        assert_eq!(out["messages"][0]["content"], "hi");
    }

    #[test]
    fn a_built_in_tool_is_refused_by_name_rather_than_dropped() {
        let body = json!({
            "model": "gpt-4o",
            "input": "what happened today",
            "tools": [{"type": "web_search"}]
        });

        let err = responses_to_chat(body).unwrap_err();
        assert!(matches!(err, ProxyError::TransformError(_)));
        assert!(
            err.to_string().contains("web_search"),
            "the refusal names the tool so the caller knows what to change: {err}"
        );
    }

    #[test]
    fn a_function_tool_drops_strict_and_keeps_the_schema() {
        let body = json!({
            "model": "gpt-4o",
            "input": "hi",
            "tools": [{
                "type": "function",
                "name": "get_weather",
                "description": "Get weather",
                "parameters": {"type": "object", "properties": {"city": {"type": "string"}}},
                "strict": true
            }]
        });

        let out = responses_to_chat(body).unwrap();
        let tool = &out["tools"][0];
        assert_eq!(tool["type"], "function");
        assert_eq!(tool["function"]["name"], "get_weather");
        assert_eq!(
            tool["function"]["parameters"]["properties"]["city"]["type"],
            "string"
        );
        assert!(
            tool["function"].get("strict").is_none() && tool.get("strict").is_none(),
            "strict has no Chat home and must not leak through"
        );
    }

    #[test]
    fn an_unknown_input_item_type_is_refused_rather_than_dropped() {
        let body = json!({"model": "gpt-4o", "input": [{"type": "some_future_item"}]});

        let err = responses_to_chat(body).unwrap_err();
        assert!(matches!(err, ProxyError::TransformError(_)));
        assert!(
            err.to_string().contains("some_future_item"),
            "the refusal names the type: {err}"
        );
    }

    #[test]
    fn a_reasoning_item_is_dropped_without_failing_the_request() {
        let body = json!({
            "model": "gpt-4o",
            "input": [
                {"type": "reasoning", "summary": []},
                {"type": "message", "role": "user",
                 "content": [{"type": "input_text", "text": "carry on"}]}
            ]
        });

        let out = responses_to_chat(body).unwrap();
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0]["content"][0]["text"], "carry on");
    }

    #[test]
    fn a_forced_tool_choice_is_nested_for_chat() {
        let body = json!({
            "model": "gpt-4o",
            "input": "hi",
            "tools": [{"type": "function", "name": "search", "parameters": {"type": "object"}}],
            "tool_choice": {"type": "function", "name": "search"}
        });

        let out = responses_to_chat(body).unwrap();
        assert_eq!(
            out["tool_choice"],
            json!({"type": "function", "function": {"name": "search"}})
        );
    }

    #[test]
    fn a_json_schema_text_format_becomes_response_format() {
        let body = json!({
            "model": "gpt-4o",
            "input": "hi",
            "text": {"format": {
                "type": "json_schema",
                "name": "answer",
                "schema": {"type": "object"},
                "strict": true
            }}
        });

        let out = responses_to_chat(body).unwrap();
        assert_eq!(out["response_format"]["type"], "json_schema");
        assert_eq!(out["response_format"]["json_schema"]["name"], "answer");
        assert_eq!(out["response_format"]["json_schema"]["strict"], true);
    }

    #[test]
    fn reasoning_effort_is_forwarded_only_for_a_reasoning_model() {
        let body = json!({
            "model": "gpt-5.4",
            "input": "hi",
            "reasoning": {"effort": "medium"}
        });
        let out = responses_to_chat(body).unwrap();
        assert_eq!(out["reasoning_effort"], "medium");

        let body = json!({
            "model": "gpt-4o",
            "input": "hi",
            "reasoning": {"effort": "medium"}
        });
        let out = responses_to_chat(body).unwrap();
        assert!(out.get("reasoning_effort").is_none());
    }

    #[test]
    fn the_chat_parameters_are_carried_over_unchanged() {
        let body = json!({
            "model": "gpt-4o",
            "input": "hi",
            "temperature": 0.5,
            "top_p": 0.9,
            "stream": true,
            "parallel_tool_calls": false,
            "user": "u-1"
        });

        let out = responses_to_chat(body).unwrap();
        assert_eq!(out["temperature"], 0.5);
        assert_eq!(out["top_p"], 0.9);
        assert_eq!(out["stream"], true);
        assert_eq!(out["parallel_tool_calls"], false);
        assert_eq!(out["user"], "u-1");
    }
}
