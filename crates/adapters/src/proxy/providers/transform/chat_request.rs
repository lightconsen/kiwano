//! OpenAI Chat Completions request → Anthropic Messages request.
//!
//! The inverse of [`super::request::anthropic_to_openai`], and not a mirror of
//! it: the two protocols disagree about where the system prompt lives, how tool
//! results are addressed, and what a required `max_tokens` means, so every rule
//! below has a reason rather than a counterpart.
//!
//! Three policies are deliberate:
//!
//! - **A field with no Anthropic home is dropped** when it is a tuning knob
//!   (penalties, seed, logit bias) and **refused** when it is content or a
//!   contract the client is relying on (`response_format` JSON, `n > 1`,
//!   logprobs, audio/file parts). Dropping a JSON schema would turn structured
//!   output into prose without saying so.
//! - **`max_tokens` is always sent.** Anthropic requires it; 8192 is accepted by
//!   every current Claude model, where the newest allow more but older ones
//!   reject anything above it.
//! - **System text is hoisted.** A `system`/`developer` message anywhere in the
//!   history is collected into the top-level `system`, in order, because
//!   Anthropic has no per-message system role. The positional loss is real and
//!   is the only place this conversion moves content rather than dropping it.

use std::collections::VecDeque;

use crate::proxy::error::ProxyError;
use crate::proxy::providers::transform::chat_message::{content_blocks, tool_result_block};
use serde_json::{json, Value};

/// Accepted by every Claude model; used when the client named no limit.
const DEFAULT_MAX_TOKENS: u64 = 8192;

/// Chat Completions request → Anthropic Messages request.
pub fn chat_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let model = body
        .get("model")
        .and_then(|m| m.as_str())
        .filter(|m| !m.trim().is_empty())
        .ok_or_else(|| ProxyError::TransformError("request carries no `model`".to_string()))?;

    reject_unsupported(&body)?;

    let messages = body
        .get("messages")
        .and_then(|m| m.as_array())
        .ok_or_else(|| {
            ProxyError::TransformError("request carries no `messages` array".to_string())
        })?;

    let mut system_parts: Vec<String> = Vec::new();
    let mut converted: Vec<Value> = Vec::new();
    let mut legacy_ids: VecDeque<String> = VecDeque::new();
    let mut synthesized = 0usize;

    let mut i = 0;
    while i < messages.len() {
        let message = &messages[i];
        let role = message
            .get("role")
            .and_then(|r| r.as_str())
            .unwrap_or("user");
        match role {
            "system" | "developer" => {
                system_parts.push(system_text(message)?);
                i += 1;
            }
            "user" => {
                let blocks = content_blocks(message.get("content"))?;
                if !blocks.is_empty() {
                    converted.push(json!({"role": "user", "content": blocks}));
                }
                i += 1;
            }
            "assistant" => {
                let blocks = assistant_blocks(message, &mut legacy_ids, &mut synthesized)?;
                if blocks.is_empty() {
                    // Anthropic rejects an empty content array; an assistant
                    // turn that carried nothing has nothing to replay.
                    log::warn!("openai→anthropic: dropped an empty assistant message");
                } else {
                    converted.push(json!({"role": "assistant", "content": blocks}));
                }
                i += 1;
            }
            "tool" | "function" => {
                // Anthropic wants every result for one assistant turn in a
                // single user message, so consecutive tool messages merge.
                let mut results = Vec::new();
                while i < messages.len() {
                    let next = &messages[i];
                    match next.get("role").and_then(|r| r.as_str()) {
                        Some("tool") => results.push(tool_result_block(next)?),
                        Some("function") => {
                            results.push(legacy_function_result(next, &mut legacy_ids)?)
                        }
                        _ => break,
                    }
                    i += 1;
                }
                converted.push(json!({"role": "user", "content": results}));
            }
            other => {
                return Err(ProxyError::TransformError(format!(
                    "message role `{other}` has no Anthropic Messages equivalent"
                )));
            }
        }
    }

    if converted.is_empty() {
        return Err(ProxyError::TransformError(
            "the request has no user or assistant messages to convert".to_string(),
        ));
    }
    // Anthropic opens the conversation with the user; inventing a turn would
    // change the prompt, so the request is refused instead.
    if converted[0]["role"] != "user" {
        return Err(ProxyError::TransformError(
            "an Anthropic conversation must open with a user turn; this request opens with an \
             assistant turn"
                .to_string(),
        ));
    }

    let mut result = json!({
        "model": model,
        "max_tokens": max_tokens(&body),
        "messages": converted
    });
    if !system_parts.is_empty() {
        result["system"] = json!(system_parts.join("\n\n"));
    }
    for key in ["temperature", "top_p", "top_k"] {
        if let Some(v) = body.get(key).filter(|v| v.is_number()) {
            result[key] = v.clone();
        }
    }
    if let Some(stop) = stop_sequences(&body)? {
        result["stop_sequences"] = stop;
    }
    if let Some(stream) = body.get("stream").and_then(|v| v.as_bool()) {
        result["stream"] = json!(stream);
    }
    if let Some(user) = user_id(&body) {
        result["metadata"] = json!({"user_id": user});
    }
    if let Some(tools) = tools(&body)? {
        result["tools"] = tools;
        if let Some(choice) = tool_choice(&body) {
            result["tool_choice"] = choice;
        }
    }

    Ok(result)
}

/// Refuse what the Anthropic wire cannot carry at all.
fn reject_unsupported(body: &Value) -> Result<(), ProxyError> {
    if let Some(n) = body.get("n").and_then(|v| v.as_u64()) {
        if n > 1 {
            return Err(ProxyError::TransformError(format!(
                "`n: {n}` asks for parallel completions, and an Anthropic endpoint returns one \
                 message per request"
            )));
        }
    }
    if body.get("logprobs").and_then(|v| v.as_bool()) == Some(true) {
        return Err(ProxyError::TransformError(
            "`logprobs` has no Anthropic Messages equivalent, and returning a response without \
             them would break the caller that asked"
                .to_string(),
        ));
    }
    if let Some(format) = body.get("response_format") {
        let kind = format
            .get("type")
            .and_then(|t| t.as_str())
            .or_else(|| format.as_str())
            .unwrap_or("");
        if kind != "text" {
            return Err(ProxyError::TransformError(format!(
                "`response_format: {kind}` has no Anthropic Messages equivalent: the provider \
                 speaks `anthropic`, and dropping the schema would turn structured output into \
                 prose without saying so"
            )));
        }
    }
    Ok(())
}

fn system_text(message: &Value) -> Result<String, ProxyError> {
    match message.get("content") {
        None | Some(Value::Null) => Ok(String::new()),
        Some(Value::String(s)) => Ok(s.clone()),
        Some(Value::Array(parts)) => {
            let mut out = Vec::new();
            for part in parts {
                let ty = part.get("type").and_then(|t| t.as_str()).unwrap_or("text");
                match ty {
                    "text" | "input_text" | "output_text" => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            out.push(text.to_string());
                        }
                    }
                    other => {
                        return Err(ProxyError::TransformError(format!(
                            "system message part `{other}` cannot be hoisted into the Anthropic \
                             `system` field, which carries text only"
                        )));
                    }
                }
            }
            Ok(out.join("\n"))
        }
        Some(other) => Err(ProxyError::TransformError(format!(
            "system message content must be a string or an array of text parts, not `{other}`"
        ))),
    }
}

fn assistant_blocks(
    message: &Value,
    legacy_ids: &mut VecDeque<String>,
    synthesized: &mut usize,
) -> Result<Vec<Value>, ProxyError> {
    let mut blocks = match message.get("content") {
        // Assistant turns on the Anthropic wire carry text; an image there has
        // no source side to come from.
        None | Some(Value::Null) => Vec::new(),
        Some(Value::String(s)) => {
            if s.is_empty() {
                Vec::new()
            } else {
                vec![json!({"type": "text", "text": s})]
            }
        }
        Some(Value::Array(parts)) => {
            let mut blocks = Vec::new();
            for part in parts {
                let ty = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match ty {
                    "text" | "input_text" | "output_text" => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                blocks.push(json!({"type": "text", "text": text}));
                            }
                        }
                    }
                    "refusal" => {
                        if let Some(text) = part.get("refusal").and_then(|r| r.as_str()) {
                            if !text.is_empty() {
                                blocks.push(json!({"type": "text", "text": text}));
                            }
                        }
                    }
                    "image_url" | "input_image" => {
                        return Err(ProxyError::TransformError(
                            "an assistant message carries an image part; Anthropic accepts images \
                             only in user turns"
                                .to_string(),
                        ));
                    }
                    other => {
                        return Err(ProxyError::TransformError(format!(
                            "assistant content part `{other}` has no Anthropic Messages equivalent"
                        )));
                    }
                }
            }
            blocks
        }
        Some(other) => {
            return Err(ProxyError::TransformError(format!(
                "assistant content must be a string or an array of parts, not `{other}`"
            )));
        }
    };

    let mut has_tool_calls = false;
    if let Some(calls) = message.get("tool_calls").and_then(|v| v.as_array()) {
        for call in calls {
            blocks.push(tool_use_block(call, synthesized)?);
            has_tool_calls = true;
        }
    }
    // Legacy `function_call`, kept for clients that still send it. The id is
    // remembered so the matching `role: "function"` result can find it back.
    if !has_tool_calls {
        if let Some(call) = message.get("function_call") {
            let id = call
                .get("id")
                .and_then(|i| i.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .unwrap_or_else(|| {
                    *synthesized += 1;
                    format!("call_{synthesized}")
                });
            let block = tool_use_block(
                &json!({
                    "id": id,
                    "type": "function",
                    "function": call
                }),
                synthesized,
            )?;
            legacy_ids.push_back(id);
            blocks.push(block);
        }
    }
    Ok(blocks)
}

fn tool_use_block(call: &Value, synthesized: &mut usize) -> Result<Value, ProxyError> {
    let kind = call
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("function");
    if kind != "function" {
        return Err(ProxyError::TransformError(format!(
            "tool call type `{kind}` has no Anthropic Messages equivalent"
        )));
    }
    let function = call.get("function").unwrap_or(call);
    let name = function
        .get("name")
        .and_then(|n| n.as_str())
        .filter(|n| !n.is_empty())
        .ok_or_else(|| {
            ProxyError::TransformError("a tool call carries no function name".to_string())
        })?;
    let id = call
        .get("id")
        .and_then(|i| i.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .unwrap_or_else(|| {
            *synthesized += 1;
            format!("call_{synthesized}")
        });

    // Anthropic takes the arguments as an object; Chat carries a JSON string.
    // Invalid JSON is refused rather than read as `{}`: replacing the arguments
    // would silently change the call the model made.
    let input = match function.get("arguments") {
        None | Some(Value::Null) => json!({}),
        Some(Value::String(s)) if s.trim().is_empty() => json!({}),
        Some(Value::String(s)) => serde_json::from_str::<Value>(s).map_err(|e| {
            ProxyError::TransformError(format!(
                "tool `{name}` sent arguments that are not valid JSON: {e}"
            ))
        })?,
        Some(v @ Value::Object(_)) | Some(v @ Value::Array(_)) => v.clone(),
        Some(other) => {
            return Err(ProxyError::TransformError(format!(
                "tool `{name}` sent arguments of an unsupported shape: `{other}`"
            )));
        }
    };

    Ok(json!({
        "type": "tool_use",
        "id": id,
        "name": name,
        "input": input
    }))
}

fn legacy_function_result(
    message: &Value,
    legacy_ids: &mut VecDeque<String>,
) -> Result<Value, ProxyError> {
    let Some(id) = legacy_ids.pop_front() else {
        return Err(ProxyError::TransformError(
            "a `role: \"function\"` message has no preceding `function_call` to answer".to_string(),
        ));
    };
    let content = match message.get("content") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => s.clone(),
        Some(other) => other.to_string(),
    };
    Ok(json!({"type": "tool_result", "tool_use_id": id, "content": content}))
}

fn max_tokens(body: &Value) -> u64 {
    body.get("max_completion_tokens")
        .or_else(|| body.get("max_tokens"))
        .and_then(|v| v.as_u64())
        .filter(|n| *n > 0)
        .unwrap_or(DEFAULT_MAX_TOKENS)
}

fn stop_sequences(body: &Value) -> Result<Option<Value>, ProxyError> {
    let Some(stop) = body.get("stop") else {
        return Ok(None);
    };
    let list: Vec<&str> = match stop {
        Value::String(s) => vec![s.as_str()],
        Value::Array(items) => items.iter().filter_map(|v| v.as_str()).collect(),
        _ => {
            return Err(ProxyError::TransformError(
                "`stop` must be a string or an array of strings".to_string(),
            ));
        }
    };
    if list.is_empty() {
        return Ok(None);
    }
    for s in &list {
        if s.trim().is_empty() {
            return Err(ProxyError::TransformError(
                "`stop` contains an empty or whitespace-only sequence, which Anthropic rejects"
                    .to_string(),
            ));
        }
    }
    Ok(Some(json!(list)))
}

fn user_id(body: &Value) -> Option<String> {
    body.get("user")
        .and_then(|u| u.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .or_else(|| {
            body.get("metadata")
                .and_then(|m| m.get("user_id"))
                .and_then(|u| u.as_str())
                .filter(|s| !s.is_empty())
                .map(str::to_string)
        })
}

fn tools(body: &Value) -> Result<Option<Value>, ProxyError> {
    let Some(list) = body.get("tools").and_then(|t| t.as_array()) else {
        return Ok(None);
    };
    if list.is_empty() {
        return Ok(None);
    }
    let mut converted = Vec::new();
    for tool in list {
        let kind = tool
            .get("type")
            .and_then(|t| t.as_str())
            .unwrap_or("function");
        if kind != "function" {
            return Err(ProxyError::TransformError(format!(
                "tool type `{kind}` has no Anthropic Messages equivalent: only function tools \
                 can be forwarded"
            )));
        }
        let function = tool.get("function").unwrap_or(tool);
        let name = function.get("name").and_then(|n| n.as_str()).unwrap_or("");
        if !valid_tool_name(name) {
            return Err(ProxyError::TransformError(format!(
                "tool name `{name}` is not accepted by Anthropic: it must be 1–64 characters of \
                 letters, digits, `_` or `-`"
            )));
        }
        let schema = match function.get("parameters") {
            None | Some(Value::Null) => json!({"type": "object", "properties": {}}),
            Some(Value::Object(_)) => function["parameters"].clone(),
            Some(other) => {
                return Err(ProxyError::TransformError(format!(
                    "tool `{name}` has a schema that is not an object: `{other}`"
                )));
            }
        };
        let mut converted_tool = json!({"name": name, "input_schema": schema});
        if let Some(description) = function
            .get("description")
            .and_then(|d| d.as_str())
            .filter(|d| !d.is_empty())
        {
            converted_tool["description"] = json!(description);
        }
        converted.push(converted_tool);
    }
    Ok(Some(json!(converted)))
}

fn valid_tool_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn tool_choice(body: &Value) -> Option<Value> {
    let parallel_disabled =
        body.get("parallel_tool_calls").and_then(|v| v.as_bool()) == Some(false);
    let Some(choice) = body.get("tool_choice") else {
        // No explicit choice, but the client did forbid parallel calls: the
        // only place Anthropic can be told is inside a `tool_choice`, so the
        // default one is stated to carry the flag.
        return parallel_disabled
            .then(|| json!({"type": "auto", "disable_parallel_tool_use": true}));
    };
    let mut converted = match choice {
        Value::String(s) => match s.as_str() {
            "auto" => json!({"type": "auto"}),
            "none" => json!({"type": "none"}),
            "required" | "any" => json!({"type": "any"}),
            other => {
                log::warn!("openai→anthropic: unknown tool_choice `{other}` dropped");
                return None;
            }
        },
        Value::Object(map) => {
            let kind = map.get("type").and_then(|t| t.as_str()).unwrap_or("");
            match kind {
                "auto" | "any" | "none" => json!({"type": kind}),
                "required" => json!({"type": "any"}),
                "function" | "tool" => {
                    let name = map
                        .get("function")
                        .and_then(|f| f.get("name"))
                        .and_then(|n| n.as_str())
                        .or_else(|| map.get("name").and_then(|n| n.as_str()))?;
                    json!({"type": "tool", "name": name})
                }
                other => {
                    log::warn!("openai→anthropic: unknown tool_choice type `{other}` dropped");
                    return None;
                }
            }
        }
        _ => return None,
    };
    // Anthropic defaults parallel tool use on, so only the false case has to
    // be said — and it is said inside `tool_choice`, its only home.
    if parallel_disabled {
        converted["disable_parallel_tool_use"] = json!(true);
    }
    Some(converted)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn convert(body: Value) -> Value {
        chat_to_anthropic(body).unwrap()
    }

    #[test]
    fn system_and_developer_are_hoisted_in_order() {
        let out = convert(json!({
            "model": "claude-sonnet-5-5",
            "messages": [
                {"role": "system", "content": "first"},
                {"role": "user", "content": "hi"},
                {"role": "developer", "content": [{"type": "text", "text": "second"}]}
            ]
        }));
        assert_eq!(out["system"], "first\n\nsecond");
        assert_eq!(out["messages"].as_array().unwrap().len(), 1);
    }

    #[test]
    fn tool_calls_become_tool_use_and_results_merge() {
        let out = convert(json!({
            "model": "claude-sonnet-5-5",
            "messages": [
                {"role": "user", "content": "read both"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "a", "type": "function",
                     "function": {"name": "read", "arguments": "{\"p\":1}"}},
                    {"id": "b", "type": "function",
                     "function": {"name": "read", "arguments": ""}}
                ]},
                {"role": "tool", "tool_call_id": "a", "content": "one"},
                {"role": "tool", "tool_call_id": "b", "content": "two"}
            ]
        }));
        let messages = out["messages"].as_array().unwrap();
        assert_eq!(messages[1]["content"][0]["type"], "tool_use");
        assert_eq!(messages[1]["content"][0]["input"], json!({"p": 1}));
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"][0]["tool_use_id"], "a");
        assert_eq!(messages[2]["content"][1]["tool_use_id"], "b");
    }

    #[test]
    fn invalid_tool_arguments_are_refused_not_emptied() {
        let err = chat_to_anthropic(json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "hi"},
                {"role": "assistant", "content": null, "tool_calls": [
                    {"id": "a", "type": "function",
                     "function": {"name": "read", "arguments": "not json"}}
                ]}
            ]
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("read"), "{err}");
    }

    #[test]
    fn parameters_are_mapped_and_defaulted() {
        let out = convert(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "temperature": 0.2,
            "stop": "END",
            "stream": true,
            "user": "u1",
            "presence_penalty": 0.5,
            "reasoning_effort": "high"
        }));
        assert_eq!(out["max_tokens"], DEFAULT_MAX_TOKENS);
        assert_eq!(out["temperature"], 0.2);
        assert_eq!(out["stop_sequences"], json!(["END"]));
        assert_eq!(out["stream"], true);
        assert_eq!(out["metadata"]["user_id"], "u1");
        assert!(out.get("presence_penalty").is_none());
        assert!(out.get("reasoning_effort").is_none());
    }

    #[test]
    fn max_completion_tokens_wins() {
        let out = convert(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "max_tokens": 100,
            "max_completion_tokens": 200
        }));
        assert_eq!(out["max_tokens"], 200);
    }

    #[test]
    fn tools_map_to_input_schema_and_parallel_is_disabled() {
        let out = convert(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {
                "name": "read", "description": "read a file",
                "parameters": {"type": "object", "properties": {"p": {"type": "string"}}}
            }}],
            "tool_choice": "required",
            "parallel_tool_calls": false
        }));
        assert_eq!(out["tools"][0]["name"], "read");
        assert_eq!(out["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(
            out["tool_choice"],
            json!({"type": "any", "disable_parallel_tool_use": true})
        );
    }

    #[test]
    fn parallel_disabled_without_a_tool_choice_still_says_so() {
        let out = convert(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "tools": [{"type": "function", "function": {"name": "read"}}],
            "parallel_tool_calls": false
        }));
        assert_eq!(
            out["tool_choice"],
            json!({"type": "auto", "disable_parallel_tool_use": true})
        );
    }

    #[test]
    fn refusals_name_what_cannot_cross() {
        for (body, needle) in [
            (
                json!({"model": "m", "messages": [{"role": "user", "content": "hi"}],
                       "response_format": {"type": "json_schema", "json_schema": {}}}),
                "response_format",
            ),
            (
                json!({"model": "m", "messages": [{"role": "user", "content": "hi"}], "n": 2}),
                "n: 2",
            ),
            (
                json!({"model": "m", "messages": [{"role": "user", "content": "hi"}],
                       "logprobs": true}),
                "logprobs",
            ),
        ] {
            let err = chat_to_anthropic(body).unwrap_err().to_string();
            assert!(err.contains(needle), "{needle}: {err}");
        }
    }

    #[test]
    fn a_conversation_must_open_with_the_user() {
        let err = chat_to_anthropic(json!({
            "model": "m",
            "messages": [{"role": "assistant", "content": "hello"}]
        }))
        .unwrap_err()
        .to_string();
        assert!(err.contains("open with a user turn"), "{err}");
    }

    #[test]
    fn response_format_text_is_dropped_quietly() {
        let out = convert(json!({
            "model": "m",
            "messages": [{"role": "user", "content": "hi"}],
            "response_format": {"type": "text"}
        }));
        assert!(out.get("response_format").is_none());
    }
}
