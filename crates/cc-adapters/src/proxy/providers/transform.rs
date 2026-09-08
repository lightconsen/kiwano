// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/proxy/providers/transform.rs
// Copied on 2026-09-07. Modified for Kiwano (cross-crate entry points relaxed to pub; the TokenUsage-dependent dedup test rewritten to assert id passthrough directly).

//! Format conversion module
//!
//! Implements Anthropic ↔ OpenAI format conversion, for OpenRouter support
//! Reference: anthropic-proxy-rs

use crate::proxy::{
    error::ProxyError,
    json_canonical::canonical_json_string,
    tool_media::{
        chat_media_part_from_tool_part, flush_pending_chat_tool_media, plan_chat_tool_output_media,
        queue_chat_tool_output_media, ToolMediaScope,
    },
};
use serde_json::{json, Value};

const ANTHROPIC_BILLING_HEADER_PREFIX: &str = "x-anthropic-billing-header:";

/// Strip only a leading Claude Code attribution line from system text.
///
/// Claude Code can send dynamic `x-anthropic-billing-header` metadata at the
/// start of `system`. If forwarded into OpenAI Chat messages or Responses
/// `instructions`, the rotating `cch=` value changes the prompt prefix on every
/// request and prevents prefix cache reuse (#2350). Later occurrences are kept
/// to avoid deleting user-authored prompt text.
pub fn strip_leading_anthropic_billing_header(text: &str) -> &str {
    if !text.starts_with(ANTHROPIC_BILLING_HEADER_PREFIX) {
        return text;
    }

    let Some(line_end) = text
        .as_bytes()
        .iter()
        .position(|byte| *byte == b'\n' || *byte == b'\r')
    else {
        return "";
    };

    let bytes = text.as_bytes();
    let mut rest_start = line_end + 1;
    if bytes[line_end] == b'\r' && bytes.get(line_end + 1) == Some(&b'\n') {
        rest_start += 1;
    }

    let rest = &text[rest_start..];
    if let Some(stripped) = rest.strip_prefix("\r\n") {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\n') {
        stripped
    } else if let Some(stripped) = rest.strip_prefix('\r') {
        stripped
    } else {
        rest
    }
}

/// Detect OpenAI o-series reasoning models (o1, o3, o4-mini, etc.)
/// These models require `max_completion_tokens` instead of `max_tokens`.
pub fn is_openai_o_series(model: &str) -> bool {
    model.len() > 1
        && model.starts_with('o')
        && model.as_bytes().get(1).is_some_and(|b| b.is_ascii_digit())
}

/// Detect Responses-compatible models that support reasoning effort.
///
/// Supported families:
/// - o-series: o1, o3, o4-mini, etc.
/// - GPT-5+: gpt-5, gpt-5.1, gpt-5.4, gpt-5-codex, etc.
/// - xAI Grok Build models. `grok-4.5` is the current documented Grok Build
///   model; retain the previous `grok-build-*` family for saved providers.
pub fn supports_reasoning_effort(model: &str) -> bool {
    let normalized = model.to_lowercase();
    is_openai_o_series(&normalized)
        || normalized
            .strip_prefix("gpt-")
            .and_then(|rest| rest.chars().next())
            .is_some_and(|c| c.is_ascii_digit() && c >= '5')
        || normalized == "grok-4.5"
        || normalized.starts_with("grok-4.5-")
        || normalized.starts_with("grok-build-")
}

/// Resolve the appropriate OpenAI `reasoning_effort` from an Anthropic request body.
///
/// Priority:
/// 1. Explicit `output_config.effort` — preserves the user's intent directly.
///    `low`/`medium`/`high` map 1:1; `max` maps to `xhigh`
///    (supported by mainstream GPT models). Unknown values are ignored.
/// 2. Fallback: `thinking.type` + `budget_tokens`:
///    - `adaptive` → `xhigh` (adaptive = maximum reasoning effort)
///    - `enabled` with budget → `low` (<4 000) / `medium` (4 000–15 999) / `high` (≥16 000)
///    - `enabled` without budget → `high` (conservative default)
///    - `disabled` / absent → `None`
pub fn resolve_reasoning_effort(body: &Value) -> Option<&'static str> {
    // --- Priority 1: explicit output_config.effort ---
    if let Some(effort) = body
        .pointer("/output_config/effort")
        .and_then(|v| v.as_str())
    {
        return match effort {
            "low" => Some("low"),
            "medium" => Some("medium"),
            "high" => Some("high"),
            "max" => Some("xhigh"), // OpenAI xhigh = maximum reasoning effort
            _ => None,              // unknown value — do not inject
        };
    }

    // --- Priority 2: thinking.type + budget_tokens fallback ---
    let thinking = body.get("thinking")?;
    match thinking.get("type").and_then(|t| t.as_str()) {
        Some("adaptive") => Some("xhigh"),
        Some("enabled") => {
            let budget = thinking.get("budget_tokens").and_then(|b| b.as_u64());
            match budget {
                Some(b) if b < 4_000 => Some("low"),
                Some(b) if b < 16_000 => Some("medium"),
                Some(_) => Some("high"),
                None => Some("high"), // enabled but no budget — assume strong reasoning
            }
        }
        _ => None, // disabled or missing
    }
}

/// Anthropic request → OpenAI Chat Completions request
///
/// Conversion utility API: currently no production callers (the connectivity
/// check no longer sends real requests and used to be its only in-crate
/// consumer), but the conversion logic and the test suite below are kept for
/// reuse by the proxy conversion path / future wiring.
#[allow(dead_code)]
pub fn anthropic_to_openai(body: Value) -> Result<Value, ProxyError> {
    anthropic_to_openai_with_reasoning_content(body, false)
}

/// Anthropic request → OpenAI Chat Completions request
///
/// `preserve_reasoning_content` is only for providers that explicitly need the
/// DeepSeek/MiMo `reasoning_content` compatibility field. The default
/// conversion keeps a generic OpenAI-compatible request body to avoid sending
/// unknown fields to strict backends.
pub fn anthropic_to_openai_with_reasoning_content(
    body: Value,
    preserve_reasoning_content: bool,
) -> Result<Value, ProxyError> {
    let mut result = json!({});

    // NOTE: model mapping is handled upstream (proxy::model_mapper); this format
    // conversion layer only does structural conversion.
    if let Some(model) = body.get("model").and_then(|m| m.as_str()) {
        result["model"] = json!(model);
    }

    let mut messages = Vec::new();

    // Handle the system prompt
    if let Some(system) = body.get("system") {
        if let Some(text) = system.as_str() {
            let text = strip_leading_anthropic_billing_header(text);
            if !text.is_empty() {
                messages.push(json!({"role": "system", "content": text}));
            }
        } else if let Some(arr) = system.as_array() {
            // Merge the top-level system array into a single system message
            // (byte-stable across turns; does not disturb prefix caching)
            let mut parts = Vec::new();
            for msg in arr {
                if let Some(text) = msg.get("text").and_then(|t| t.as_str()) {
                    let text = strip_leading_anthropic_billing_header(text);
                    if text.is_empty() {
                        continue;
                    }
                    parts.push(text.to_string());
                }
            }
            if !parts.is_empty() {
                messages.push(json!({"role": "system", "content": parts.join("\n")}));
            }
        }
    }

    // Convert messages
    if let Some(msgs) = body.get("messages").and_then(|m| m.as_array()) {
        for msg in msgs {
            let role = msg.get("role").and_then(|r| r.as_str()).unwrap_or("user");
            let content = msg.get("content");
            let converted = convert_message_to_openai(role, content, preserve_reasoning_content)?;
            messages.extend(converted);
        }
    }

    result["messages"] = json!(messages);

    // Convert parameters — o-series models require max_completion_tokens
    let model = body.get("model").and_then(|m| m.as_str()).unwrap_or("");
    if let Some(v) = body.get("max_tokens") {
        if is_openai_o_series(model) {
            result["max_completion_tokens"] = v.clone();
        } else {
            result["max_tokens"] = v.clone();
        }
    }
    if let Some(v) = body.get("temperature") {
        result["temperature"] = v.clone();
    }
    if let Some(v) = body.get("top_p") {
        result["top_p"] = v.clone();
    }
    if let Some(v) = body.get("stop_sequences") {
        result["stop"] = v.clone();
    }
    if let Some(v) = body.get("stream") {
        result["stream"] = v.clone();
    }

    // Map Anthropic thinking → OpenAI reasoning_effort
    if supports_reasoning_effort(model) {
        if let Some(effort) = resolve_reasoning_effort(&body) {
            result["reasoning_effort"] = json!(effort);
        }
    }

    // Convert tools (filter out BatchTool)
    if let Some(tools) = body.get("tools").and_then(|t| t.as_array()) {
        let openai_tools: Vec<Value> = tools
            .iter()
            .filter(|t| t.get("type").and_then(|v| v.as_str()) != Some("BatchTool"))
            .map(|t| {
                json!({
                    "type": "function",
                    "function": {
                        "name": t.get("name").and_then(|n| n.as_str()).unwrap_or(""),
                        "description": t.get("description"),
                        "parameters": clean_schema(t.get("input_schema").cloned().unwrap_or(json!({})))
                    }
                })
            })
            .collect();

        if !openai_tools.is_empty() {
            result["tools"] = json!(openai_tools);
        }
    }

    if let Some(v) = body.get("tool_choice") {
        result["tool_choice"] = map_tool_choice_to_chat(v);
    }

    Ok(result)
}

/// Inject `stream_options.include_usage` into OpenAI Chat Completions
/// streaming requests.
///
/// OpenAI-compatible upstreams do not return usage in the SSE stream by
/// default; `include_usage` must be declared explicitly for the final usage
/// chunk to be emitted. Without this injection, token/cost/cache accounting
/// for streaming requests is silently lost (input/output/cache all 0). Any
/// other `stream_options` fields the client passed through are preserved —
/// only `include_usage` is added; non-streaming requests are untouched.
///
/// Shared by the Claude→openai_chat (claude.rs) and Codex Responses→Chat
/// (transform_codex_chat.rs) conversion paths, keeping both client directions
/// consistent.
pub fn inject_openai_stream_include_usage(result: &mut Value) {
    let is_stream = result
        .get("stream")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    if !is_stream {
        return;
    }
    match result.get_mut("stream_options") {
        Some(Value::Object(opts)) => {
            opts.insert("include_usage".to_string(), json!(true));
        }
        _ => {
            result["stream_options"] = json!({ "include_usage": true });
        }
    }
}

/// Translate an Anthropic `tool_choice` into the OpenAI Chat Completions form.
///
/// Anthropic forms:
///   "auto" / "any" / "none"           (string enum)
///   {"type": "auto" | "any" | "none"}
///   {"type": "tool", "name": "<X>"}
///
/// OpenAI Chat forms:
///   "auto" / "none" / "required"      (note: no "any" — use "required")
///   {"type": "function", "function": {"name": "<X>"}}
///
/// The Responses API uses a flatter `{"type":"function","name":"X"}` selector,
/// so it has a sibling `map_tool_choice_to_responses` in `transform_responses.rs`.
/// Keep the two in sync.
fn map_tool_choice_to_chat(tool_choice: &Value) -> Value {
    match tool_choice {
        Value::String(s) => match s.as_str() {
            "any" => json!("required"),
            _ => json!(s),
        },
        Value::Object(obj) => match obj.get("type").and_then(|t| t.as_str()) {
            Some("any") => json!("required"),
            Some("auto") => json!("auto"),
            Some("none") => json!("none"),
            Some("tool") => {
                let name = obj.get("name").and_then(|n| n.as_str()).unwrap_or("");
                json!({
                    "type": "function",
                    "function": { "name": name }
                })
            }
            _ => tool_choice.clone(),
        },
        _ => tool_choice.clone(),
    }
}

/// Convert a single message into OpenAI format (may produce multiple messages)
fn convert_message_to_openai(
    role: &str,
    content: Option<&Value>,
    preserve_reasoning_content: bool,
) -> Result<Vec<Value>, ProxyError> {
    let mut result = Vec::new();

    let content = match content {
        Some(c) => c,
        None => {
            result.push(json!({"role": role, "content": null}));
            return Ok(result);
        }
    };

    // String content
    if let Some(text) = content.as_str() {
        result.push(json!({"role": role, "content": text}));
        return Ok(result);
    }

    // Array content (multimodal / tool calls)
    if let Some(blocks) = content.as_array() {
        let mut content_parts = Vec::new();
        let mut tool_calls = Vec::new();
        let mut pending_tool_media = Vec::new();
        // reasoning_parts: only generate reasoning_content on the
        // DeepSeek/MiMo thinking tool-call compatibility path; the generic
        // OpenAI-compatible path does not send that non-standard field.
        let mut reasoning_parts = Vec::new();

        for block in blocks {
            let block_type = block.get("type").and_then(|t| t.as_str()).unwrap_or("");

            match block_type {
                "text" => {
                    if let Some(text) = block.get("text").and_then(|t| t.as_str()) {
                        content_parts.push(json!({"type": "text", "text": text}));
                    }
                }
                "image" => {
                    if let Some(image) =
                        chat_media_part_from_tool_part(block, ToolMediaScope::ImagesOnly)
                    {
                        content_parts.push(image);
                    }
                }
                "tool_use" => {
                    let id = block.get("id").and_then(|i| i.as_str()).unwrap_or("");
                    let name = block.get("name").and_then(|n| n.as_str()).unwrap_or("");
                    let input = block.get("input").cloned().unwrap_or(json!({}));
                    tool_calls.push(json!({
                        "id": id,
                        "type": "function",
                        "function": {
                            "name": name,
                            "arguments": canonical_json_string(&input)
                        }
                    }));
                }
                "tool_result" => {
                    // tool_result becomes a separate tool role message
                    let tool_use_id = block
                        .get("tool_use_id")
                        .and_then(|i| i.as_str())
                        .unwrap_or("");
                    let content_val = block.get("content");
                    let media_plan = content_val.cloned().and_then(plan_chat_tool_output_media);
                    let content_str = if let Some(media_plan) = media_plan {
                        queue_chat_tool_output_media(
                            &mut pending_tool_media,
                            tool_use_id,
                            media_plan.media_parts,
                        );
                        media_plan.tool_content
                    } else {
                        // Keep the no-media representation exactly equal to
                        // the legacy converter for prompt-cache stability.
                        match content_val {
                            Some(Value::String(s)) => s.clone(),
                            Some(v) => canonical_json_string(v),
                            None => String::new(),
                        }
                    };
                    result.push(json!({
                        "role": "tool",
                        "tool_call_id": tool_use_id,
                        "content": content_str
                    }));
                }
                "thinking" => {
                    // Extract thinking content; it can later be passed as
                    // reasoning_content to upstreams that need it.
                    if let Some(thinking) = block.get("thinking").and_then(|t| t.as_str()) {
                        if !thinking.is_empty() {
                            reasoning_parts.push(thinking.to_string());
                        }
                    }
                }
                "redacted_thinking" if preserve_reasoning_content => {
                    // Claude Code encrypts historical thinking into redacted_thinking blocks.
                    // MiMo/DeepSeek require non-empty reasoning_content on assistant tool-call
                    // messages, so inject a minimal placeholder when the real content is
                    // unavailable. Skip when preserve_reasoning_content is off (generic
                    // OpenAI-compatible path).
                    reasoning_parts.push("[redacted thinking]".to_string());
                }
                _ => {}
            }
        }

        // Chat tool messages cannot carry image parts. Keep parallel tool
        // results adjacent, then present all extracted media in one user turn
        // before any ordinary message content from the same Anthropic turn.
        flush_pending_chat_tool_media(&mut result, &mut pending_tool_media);

        // Add the message with content and/or tool calls
        if !content_parts.is_empty() || !tool_calls.is_empty() {
            let mut msg = json!({"role": role});

            // Content handling
            if content_parts.is_empty() {
                msg["content"] = Value::Null;
            } else if content_parts.len() == 1 {
                // Simplify a single text block into a plain string
                if let Some(text) = content_parts[0].get("text") {
                    msg["content"] = text.clone();
                } else {
                    msg["content"] = json!(content_parts);
                }
            } else {
                msg["content"] = json!(content_parts);
            }

            // Tool calls
            if !tool_calls.is_empty() {
                msg["tool_calls"] = json!(tool_calls);
            }

            if preserve_reasoning_content && role == "assistant" && !tool_calls.is_empty() {
                let reasoning_content = if reasoning_parts.is_empty() {
                    "tool call".to_string()
                } else {
                    reasoning_parts.join("\n")
                };
                msg["reasoning_content"] = json!(reasoning_content);
            }

            result.push(msg);
        }

        return Ok(result);
    }

    // All other cases: pass through as-is
    result.push(json!({"role": role, "content": content}));
    Ok(result)
}

/// Clean up a tool parameters JSON schema, and fill in the object type OpenAI
/// requires on the root schema.
pub fn clean_schema(schema: Value) -> Value {
    clean_schema_inner(schema, true)
}

fn clean_schema_inner(mut schema: Value, is_root: bool) -> Value {
    if let Some(obj) = schema.as_object_mut() {
        let missing_type = is_root && !obj.contains_key("type");
        if missing_type {
            obj.insert("type".to_string(), json!("object"));
        }
        if missing_type && !obj.contains_key("properties") {
            obj.insert("properties".to_string(), json!({}));
        }

        // Remove "format": "uri"
        if obj.get("format").and_then(|v| v.as_str()) == Some("uri") {
            obj.remove("format");
        }

        // Recursively clean nested schemas
        if let Some(properties) = obj.get_mut("properties").and_then(|v| v.as_object_mut()) {
            for (_, value) in properties.iter_mut() {
                *value = clean_schema_inner(value.clone(), false);
            }
        }

        if let Some(items) = obj.get_mut("items") {
            *items = clean_schema_inner(items.clone(), false);
        }
    }
    schema
}

/// OpenAI response → Anthropic response
pub fn openai_to_anthropic(body: Value) -> Result<Value, ProxyError> {
    let choices = body
        .get("choices")
        .and_then(|c| c.as_array())
        .ok_or_else(|| ProxyError::TransformError("No choices in response".to_string()))?;

    let choice = choices
        .first()
        .ok_or_else(|| ProxyError::TransformError("Empty choices array".to_string()))?;

    let message = choice
        .get("message")
        .ok_or_else(|| ProxyError::TransformError("No message in choice".to_string()))?;

    let mut content = Vec::new();
    let mut has_tool_use = false;

    // DeepSeek providers put thinking content in message.reasoning_content.
    if let Some(reasoning_content) = message.get("reasoning_content").and_then(|r| r.as_str()) {
        if !reasoning_content.is_empty() {
            content.push(json!({"type": "thinking", "thinking": reasoning_content}));
        }
    }

    // Text / refusal content
    if let Some(msg_content) = message.get("content") {
        if let Some(text) = msg_content.as_str() {
            if !text.is_empty() {
                content.push(json!({"type": "text", "text": text}));
            }
        } else if let Some(parts) = msg_content.as_array() {
            for part in parts {
                let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
                match part_type {
                    "text" | "output_text" => {
                        if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                            if !text.is_empty() {
                                content.push(json!({"type": "text", "text": text}));
                            }
                        }
                    }
                    "refusal" => {
                        if let Some(refusal) = part.get("refusal").and_then(|r| r.as_str()) {
                            if !refusal.is_empty() {
                                content.push(json!({"type": "text", "text": refusal}));
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }
    // Some providers put refusal at message-level.
    if let Some(refusal) = message.get("refusal").and_then(|r| r.as_str()) {
        if !refusal.is_empty() {
            content.push(json!({"type": "text", "text": refusal}));
        }
    }

    // Tool calls (tool_calls)
    if let Some(tool_calls) = message.get("tool_calls").and_then(|t| t.as_array()) {
        if !tool_calls.is_empty() {
            has_tool_use = true;
        }
        for tc in tool_calls {
            let id = tc.get("id").and_then(|i| i.as_str()).unwrap_or("");
            let empty_obj = json!({});
            let func = tc.get("function").unwrap_or(&empty_obj);
            let name = func.get("name").and_then(|n| n.as_str()).unwrap_or("");
            let args_str = func
                .get("arguments")
                .and_then(|a| a.as_str())
                .unwrap_or("{}");
            let input: Value = serde_json::from_str(args_str).unwrap_or(json!({}));

            content.push(json!({
                "type": "tool_use",
                "id": id,
                "name": name,
                "input": input
            }));
        }
    }
    // Legacy format compatibility (function_call)
    if !has_tool_use {
        if let Some(function_call) = message.get("function_call") {
            let id = function_call
                .get("id")
                .and_then(|i| i.as_str())
                .unwrap_or("");
            let name = function_call
                .get("name")
                .and_then(|n| n.as_str())
                .unwrap_or("");
            let has_arguments = function_call.get("arguments").is_some();

            let input = match function_call.get("arguments") {
                Some(Value::String(s)) => serde_json::from_str(s).unwrap_or(json!({})),
                Some(v @ Value::Object(_)) | Some(v @ Value::Array(_)) => v.clone(),
                _ => json!({}),
            };

            if !name.is_empty() || has_arguments {
                content.push(json!({
                    "type": "tool_use",
                    "id": id,
                    "name": name,
                    "input": input
                }));
                has_tool_use = true;
            }
        }
    }

    // Map finish_reason → stop_reason
    let stop_reason = choice
        .get("finish_reason")
        .and_then(|r| r.as_str())
        .map(|r| match r {
            "stop" => "end_turn",
            "length" => "max_tokens",
            "tool_calls" | "function_call" => "tool_use",
            "content_filter" => "end_turn",
            other => {
                log::warn!(
                    "[Claude/OpenAI] Unknown finish_reason in non-streaming response: {other}"
                );
                "end_turn"
            }
        })
        .or(if has_tool_use { Some("tool_use") } else { None });

    // usage — map cache tokens from OpenAI format to Anthropic format
    let usage = body.get("usage").cloned().unwrap_or(json!({}));
    // OpenAI prompt_tokens includes cache hits; Anthropic input_tokens does not —
    // subtract cache_read and cache_creation so input becomes fresh input. This
    // path is accounted with app_type="claude" (the calculator no longer
    // subtracts); without the subtraction, cached tokens would be counted twice,
    // in input and in the cache buckets. The three buckets are mutually
    // exclusive with the identity:
    // input + cache_read + cache_creation == prompt_tokens (inclusive upstream).
    // Symmetric with the streaming build_anthropic_usage_json (#2774) and
    // transform_gemini's saturating_sub.
    // Final cache_read/cache_creation: direct fields win over OpenAI nested details.
    let cached = usage
        .get("cache_read_input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .pointer("/prompt_tokens_details/cached_tokens")
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);
    let cache_creation = usage
        .get("cache_creation_input_tokens")
        .and_then(|v| v.as_u64())
        .or_else(|| {
            usage
                .pointer("/prompt_tokens_details/cache_write_tokens")
                .or_else(|| usage.pointer("/input_tokens_details/cache_write_tokens"))
                .and_then(|v| v.as_u64())
        })
        .unwrap_or(0);
    let input_tokens = usage
        .get("prompt_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0)
        .saturating_sub(cached)
        .saturating_sub(cache_creation) as u32;
    let output_tokens = usage
        .get("completion_tokens")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as u32;

    let mut usage_json = json!({
        "input_tokens": input_tokens,
        "output_tokens": output_tokens
    });

    if cached > 0 {
        usage_json["cache_read_input_tokens"] = json!(cached);
    }
    if cache_creation > 0 {
        usage_json["cache_creation_input_tokens"] = json!(cache_creation);
    }

    let result = json!({
        "id": body.get("id").and_then(|i| i.as_str()).unwrap_or(""),
        "type": "message",
        "role": "assistant",
        "content": content,
        "model": body.get("model").and_then(|m| m.as_str()).unwrap_or(""),
        "stop_reason": stop_reason,
        "stop_sequence": null,
        "usage": usage_json
    });

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_anthropic_to_openai_simple() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["model"], "claude-3-opus");
        assert_eq!(result["max_tokens"], 1024);
        assert_eq!(result["messages"][0]["role"], "user");
        assert_eq!(result["messages"][0]["content"], "Hello");
    }

    #[test]
    fn test_anthropic_to_openai_with_system() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": "You are a helpful assistant.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "You are a helpful assistant."
        );
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_strips_leading_billing_header_from_system_string() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": "x-anthropic-billing-header: cc_version=2.1.119.47e; cc_entrypoint=sdk-cli; cch=a7754;\n\nYou are a helpful assistant.",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "You are a helpful assistant."
        );
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_strips_billing_header_from_system_array_parts() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.119.47e; cc_entrypoint=sdk-cli; cch=a7754;\n"},
                {"type": "text", "text": "Stable prompt"}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(result["messages"][0]["content"], "Stable prompt");
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_preserves_prompt_after_billing_header_in_same_part() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "x-anthropic-billing-header: cc_version=2.1.119.47e; cc_entrypoint=sdk-cli; cch=a7754;\n\nStable prompt part 1"},
                {"type": "text", "text": "Stable prompt part 2"}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "Stable prompt part 1\nStable prompt part 2"
        );
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_keeps_non_leading_billing_header_text() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": "Keep this literal:\nx-anthropic-billing-header: example",
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "Keep this literal:\nx-anthropic-billing-header: example"
        );
    }

    #[test]
    fn test_anthropic_to_openai_with_tools() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather info",
                "input_schema": {"type": "object", "properties": {"location": {"type": "string"}}}
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["tools"][0]["type"], "function");
        assert_eq!(result["tools"][0]["function"]["name"], "get_weather");
        assert_eq!(
            result["tools"][0]["function"]["parameters"]["type"],
            json!("object")
        );
        assert_eq!(
            result["tools"][0]["function"]["parameters"]["properties"]["location"]["type"],
            json!("string")
        );
    }

    #[test]
    fn test_anthropic_to_openai_defaults_missing_tool_schema_type() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "What's the weather?"}],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather info",
                "input_schema": {"properties": {"location": {"type": "string"}}}
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let parameters = &result["tools"][0]["function"]["parameters"];
        assert_eq!(parameters["type"], json!("object"));
        assert_eq!(
            parameters["properties"]["location"]["type"],
            json!("string")
        );
    }

    #[test]
    fn test_anthropic_to_openai_defaults_empty_tool_schema() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Do work"}],
            "tools": [{"name": "do_work", "input_schema": {}}]
        });

        let result = anthropic_to_openai(input).unwrap();
        let parameters = &result["tools"][0]["function"]["parameters"];
        assert_eq!(parameters, &json!({"type": "object", "properties": {}}));
    }

    #[test]
    fn test_clean_schema_only_defaults_root_to_object() {
        let schema = json!({
            "properties": {
                "nullable_value": {
                    "anyOf": [{"type": "string"}, {"type": "null"}]
                },
                "list": {
                    "items": {"type": "string"}
                }
            }
        });

        let result = clean_schema(schema);
        assert_eq!(result["type"], json!("object"));
        assert_eq!(
            result["properties"]["nullable_value"],
            json!({"anyOf": [{"type": "string"}, {"type": "null"}]})
        );
        assert_eq!(
            result["properties"]["list"],
            json!({"items": {"type": "string"}})
        );
    }

    #[test]
    fn test_anthropic_to_openai_strips_cache_control_from_merged_system() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "You are Claude Code.", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "Be concise.", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"].as_array().unwrap().len(), 2);
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "You are Claude Code.\nBe concise."
        );
        assert!(result["messages"][0].get("cache_control").is_none());
        assert_eq!(result["messages"][1]["role"], "user");
    }

    #[test]
    fn test_anthropic_to_openai_strips_cache_control_from_mixed_system() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "You are Claude Code.", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "Be concise."}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "You are Claude Code.\nBe concise."
        );
        assert!(result["messages"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_preserves_mid_conversation_system_in_place() {
        // Claude Code injects system messages mid-conversation (e.g.
        // <total_tokens>); they must stay in place — no merging or hoisting,
        // otherwise prefix caching breaks.
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": "You are Claude Code.",
            "messages": [
                {"role": "user", "content": "Hello"},
                {"role": "assistant", "content": "Hi there!"},
                {"role": "system", "content": "<total_tokens>14963538 tokens left</total_tokens>"},
                {"role": "user", "content": "Continue"}
            ]
        });

        let result = anthropic_to_openai(input).unwrap();
        let messages = result["messages"].as_array().unwrap();

        // Top-level system is first
        assert_eq!(messages[0]["role"], "system");
        assert_eq!(messages[0]["content"], "You are Claude Code.");

        // Mid-conversation system stays in place (3rd message, index=3),
        // not merged or hoisted
        assert_eq!(messages[3]["role"], "system");
        assert_eq!(
            messages[3]["content"],
            "<total_tokens>14963538 tokens left</total_tokens>"
        );

        // 5 messages in total, no merging
        assert_eq!(messages.len(), 5);
    }

    #[test]
    fn test_anthropic_to_openai_strips_cache_control_from_conflicting_system() {
        let input = json!({
            "model": "claude-3-sonnet",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "You are Claude Code.", "cache_control": {"type": "ephemeral"}},
                {"type": "text", "text": "Be concise.", "cache_control": {"type": "ephemeral", "ttl": "5m"}}
            ],
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"][0]["role"], "system");
        assert_eq!(
            result["messages"][0]["content"],
            "You are Claude Code.\nBe concise."
        );
        assert!(result["messages"][0].get("cache_control").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_tool_use() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "text", "text": "Let me check"},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert!(msg.get("tool_calls").is_some());
        assert_eq!(msg["tool_calls"][0]["id"], "call_123");
        assert!(msg.get("reasoning_content").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_tool_use_preserves_reasoning_content() {
        let input = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "I should call the tool."},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_openai_with_reasoning_content(input, true).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["reasoning_content"], "I should call the tool.");
        assert!(msg.get("tool_calls").is_some());
        assert_eq!(msg["tool_calls"][0]["id"], "call_123");
    }

    #[test]
    fn test_anthropic_to_openai_tool_use_injects_placeholder_reasoning_content_when_missing() {
        let input = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_openai_with_reasoning_content(input, true).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert_eq!(msg["reasoning_content"], "tool call");
        assert!(msg.get("tool_calls").is_some());
        assert_eq!(msg["tool_calls"][0]["id"], "call_123");
    }

    #[test]
    fn test_anthropic_to_openai_tool_use_uses_redacted_thinking_placeholder() {
        let input = json!({
            "model": "mimo-v2.5-pro",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "redacted_thinking", "data": "opaque"},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_openai_with_reasoning_content(input, true).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["reasoning_content"], "[redacted thinking]");
        assert_eq!(msg["tool_calls"][0]["id"], "call_123");
    }

    #[test]
    fn test_anthropic_to_openai_does_not_emit_reasoning_content_by_default() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "I should call the tool."},
                    {"type": "tool_use", "id": "call_123", "name": "get_weather", "input": {"location": "Tokyo"}}
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["role"], "assistant");
        assert!(msg.get("tool_calls").is_some());
        assert!(msg.get("reasoning_content").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_skips_thinking_only_message() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": [
                    {"type": "thinking", "thinking": "No visible content yet."}
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["messages"].as_array().unwrap().len(), 0);
    }

    #[test]
    fn test_anthropic_to_openai_tool_result() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "tool_result", "tool_use_id": "call_123", "content": "Sunny, 25°C"}
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let msg = &result["messages"][0];
        assert_eq!(msg["role"], "tool");
        assert_eq!(msg["tool_call_id"], "call_123");
        assert_eq!(msg["content"], "Sunny, 25°C");
    }

    #[test]
    fn test_anthropic_to_openai_no_media_tool_results_keep_legacy_representation() {
        let raw_json_string = "{ \"status\": \"ok\", \"count\": 2 }";
        let input = json!({
            "model": "claude-3-opus",
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_string",
                        "content": raw_json_string
                    },
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_array",
                        "content": [{"type": "text", "text": "plain"}]
                    }
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let messages = result["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["content"], raw_json_string);
        assert_eq!(
            messages[1]["content"],
            canonical_json_string(&json!([{"type": "text", "text": "plain"}]))
        );
    }

    #[test]
    fn test_anthropic_to_openai_moves_tool_result_image_to_user_message() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "tool_result",
                    "tool_use_id": "call_image",
                    "content": [
                        {"type": "text", "text": "caption"},
                        {
                            "type": "image",
                            "source": {
                                "type": "base64",
                                "media_type": "image/png",
                                "data": "CLAUDE_CHAT_IMAGE_SENTINEL"
                            },
                            "cache_control": {"type": "ephemeral"},
                            "prompt_cache_breakpoint": true
                        }
                    ]
                }]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let messages = result["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 2);
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[0]["tool_call_id"], "call_image");
        assert!(messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("tool result media moved"));
        assert!(!messages[0]["content"]
            .as_str()
            .unwrap()
            .contains("CLAUDE_CHAT_IMAGE_SENTINEL"));
        assert_eq!(messages[1]["role"], "user");
        assert_eq!(
            messages[1]["content"][0]["text"],
            "[cc-switch: media output of tool call call_image]"
        );
        assert_eq!(messages[1]["content"][1]["type"], "image_url");
        assert!(messages[1]["content"][1].get("cache_control").is_none());
        assert!(messages[1]["content"][1]
            .get("prompt_cache_breakpoint")
            .is_none());
        assert_eq!(
            messages[1]["content"][1]["image_url"]["url"],
            "data:image/png;base64,CLAUDE_CHAT_IMAGE_SENTINEL"
        );
    }

    #[test]
    fn test_anthropic_to_openai_batches_parallel_tool_result_media() {
        let input = json!({
            "model": "claude-3-opus",
            "messages": [{
                "role": "user",
                "content": [
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_1",
                        "content": [{
                            "type": "image",
                            "source": {"type": "base64", "media_type": "image/png", "data": "ONE"}
                        }]
                    },
                    {
                        "type": "tool_result",
                        "tool_use_id": "call_2",
                        "content": [{
                            "type": "image",
                            "source": {"type": "base64", "media_type": "image/jpeg", "data": "TWO"}
                        }]
                    }
                ]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        let messages = result["messages"].as_array().unwrap();

        assert_eq!(messages.len(), 3);
        assert_eq!(messages[0]["role"], "tool");
        assert_eq!(messages[1]["role"], "tool");
        assert_eq!(messages[2]["role"], "user");
        assert_eq!(messages[2]["content"].as_array().unwrap().len(), 4);
    }

    #[test]
    fn test_anthropic_to_openai_maps_remote_image_source() {
        let input = json!({
            "model": "claude-3-opus",
            "messages": [{
                "role": "user",
                "content": [{
                    "type": "image",
                    "source": {
                        "type": "url",
                        "url": "https://example.com/image.png"
                    },
                    "cache_control": {"type": "ephemeral"},
                    "prompt_cache_breakpoint": true
                }]
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(
            result["messages"][0]["content"][0]["image_url"]["url"],
            "https://example.com/image.png"
        );
        assert!(result["messages"][0]["content"][0]
            .get("cache_control")
            .is_none());
        assert!(result["messages"][0]["content"][0]
            .get("prompt_cache_breakpoint")
            .is_none());
    }

    #[test]
    fn test_openai_to_anthropic_simple() {
        let input = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["id"], "chatcmpl-123");
        assert_eq!(result["type"], "message");
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "Hello!");
        assert_eq!(result["stop_reason"], "end_turn");
        assert_eq!(result["usage"]["input_tokens"], 10);
        assert_eq!(result["usage"]["output_tokens"], 5);
    }

    #[test]
    fn test_openai_to_anthropic_preserves_id_for_usage_dedup() {
        let input = json!({
            "id": "chatcmpl-claude-compatible",
            "object": "chat.completion",
            "model": "claude-sonnet-4-5",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        // Kiwano: the original test round-tripped through the proxy's usage
        // parser (out of scope here); the same contract is asserted directly —
        // the converted Anthropic message must keep the upstream `id` so usage
        // dedup keyed on the message id still works.
        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["id"], "chatcmpl-claude-compatible");
    }

    #[test]
    fn test_openai_to_anthropic_with_tool_calls() {
        let input = json!({
            "id": "chatcmpl-123",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "tool_calls": [{
                        "id": "call_123",
                        "type": "function",
                        "function": {"name": "get_weather", "arguments": "{\"location\": \"Tokyo\"}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "tool_use");
        assert_eq!(result["content"][0]["id"], "call_123");
        assert_eq!(result["content"][0]["name"], "get_weather");
        assert_eq!(result["content"][0]["input"]["location"], "Tokyo");
        assert_eq!(result["stop_reason"], "tool_use");
    }

    #[test]
    fn test_deepseek_reasoning_content_round_trips_for_tool_calls() {
        let upstream_response = json!({
            "id": "chatcmpl-deepseek",
            "object": "chat.completion",
            "created": 1234567890,
            "model": "deepseek-v4-flash",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "reasoning_content": "Need the current date before calling weather.",
                    "content": "Let me check the date first.",
                    "tool_calls": [{
                        "id": "call_date",
                        "type": "function",
                        "function": {"name": "get_date", "arguments": "{}"}
                    }]
                },
                "finish_reason": "tool_calls"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5, "total_tokens": 15}
        });

        let anthropic_response = openai_to_anthropic(upstream_response).unwrap();
        assert_eq!(anthropic_response["content"][0]["type"], "thinking");
        assert_eq!(
            anthropic_response["content"][0]["thinking"],
            "Need the current date before calling weather."
        );
        assert_eq!(anthropic_response["content"][1]["type"], "text");
        assert_eq!(anthropic_response["content"][2]["type"], "tool_use");
        assert_eq!(anthropic_response["content"][2]["id"], "call_date");

        let follow_up_request = json!({
            "model": "deepseek-v4-flash",
            "max_tokens": 1024,
            "messages": [{
                "role": "assistant",
                "content": anthropic_response["content"].clone()
            }]
        });
        let replayed = anthropic_to_openai_with_reasoning_content(follow_up_request, true).unwrap();
        let msg = &replayed["messages"][0];

        assert_eq!(
            msg["reasoning_content"],
            "Need the current date before calling weather."
        );
        assert_eq!(msg["tool_calls"][0]["id"], "call_date");
        assert_eq!(msg["tool_calls"][0]["function"]["name"], "get_date");
    }

    #[test]
    fn test_model_passthrough() {
        // The format conversion layer only does structural conversion; model
        // mapping is handled by the upstream proxy::model_mapper
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["model"], "gpt-4o");
    }

    #[test]
    fn test_anthropic_to_openai_does_not_inject_prompt_cache_key() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert!(result.get("prompt_cache_key").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_strips_all_cache_control() {
        let input = json!({
            "model": "claude-3-opus",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "System prompt", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [{
                "role": "user",
                "content": [
                    {"type": "text", "text": "Hello", "cache_control": {"type": "ephemeral", "ttl": "5m"}}
                ]
            }],
            "tools": [{
                "name": "get_weather",
                "description": "Get weather",
                "input_schema": {"type": "object"},
                "cache_control": {"type": "ephemeral"}
            }]
        });

        let result = anthropic_to_openai(input).unwrap();
        // System message: no cache_control
        assert!(result["messages"][0].get("cache_control").is_none());
        // User message: content simplified to string (no cache_control → flat string)
        assert_eq!(result["messages"][1]["content"], "Hello");
        // Tool: no cache_control
        assert!(result["tools"][0].get("cache_control").is_none());
    }

    /// Exactly reproduce the 400 error scenario reported in Issue #3805:
    /// strictly validating models such as GLM/Qwen reject cache_control and
    /// array-form content
    #[test]
    fn test_regression_gh3805_no_cache_control_leak_to_openai() {
        let input = json!({
            "model": "glm-5.1",
            "max_tokens": 1024,
            "system": [
                {"type": "text", "text": "You are helpful.", "cache_control": {"type": "ephemeral"}}
            ],
            "messages": [
                {"role": "user", "content": [
                    {"type": "text", "text": "Hello", "cache_control": {"type": "ephemeral"}}
                ]}
            ],
            "tools": [{
                "name": "search",
                "description": "Search the web",
                "input_schema": {"type": "object"},
                "cache_control": {"type": "ephemeral"}
            }]
        });

        let result = anthropic_to_openai(input).unwrap();

        // Verify: no cache_control anywhere in messages
        for (i, msg) in result["messages"].as_array().unwrap().iter().enumerate() {
            assert!(
                msg.get("cache_control").is_none(),
                "messages[{i}] must not have cache_control"
            );
        }

        // Verify: no cache_control in content
        for (i, msg) in result["messages"].as_array().unwrap().iter().enumerate() {
            if let Some(content) = msg.get("content") {
                assert!(
                    !content.is_array()
                        || content
                            .as_array()
                            .unwrap()
                            .iter()
                            .all(|part| part.get("cache_control").is_none()),
                    "messages[{i}] content parts must not have cache_control"
                );
            }
        }

        // Verify: system content is a plain string (not an array)
        let sys_msg = &result["messages"][0];
        assert_eq!(sys_msg["role"], "system");
        assert!(
            sys_msg["content"].is_string(),
            "system content must be string, got: {}",
            sys_msg["content"]
        );

        // Verify: user content is a plain string (not an array)
        let user_msg = &result["messages"][1];
        assert_eq!(user_msg["role"], "user");
        assert!(
            user_msg["content"].is_string(),
            "user content must be string, got: {}",
            user_msg["content"]
        );

        // Verify: no cache_control in tools
        if let Some(tools) = result["tools"].as_array() {
            for (i, tool) in tools.iter().enumerate() {
                assert!(
                    tool.get("cache_control").is_none(),
                    "tools[{i}] must not have cache_control"
                );
            }
        }
    }

    #[test]
    fn test_openai_to_anthropic_with_cache_tokens() {
        let input = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "prompt_tokens_details": {
                    "cached_tokens": 80
                }
            }
        });

        let result = openai_to_anthropic(input).unwrap();
        // prompt_tokens(100) includes cached(80); after conversion input should
        // be fresh = 100 - 80 = 20
        assert_eq!(result["usage"]["input_tokens"], 20);
        assert_eq!(result["usage"]["output_tokens"], 50);
        assert_eq!(result["usage"]["cache_read_input_tokens"], 80);
    }

    #[test]
    fn test_openai_to_anthropic_with_direct_cache_fields() {
        let input = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Hello!"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 50,
                "cache_read_input_tokens": 60,
                "cache_creation_input_tokens": 20
            }
        });

        let result = openai_to_anthropic(input).unwrap();
        // cache_read(60)+cache_creation(20) are both subtracted from prompt(100);
        // fresh = 100 - 60 - 20 = 20
        // Conservation: input(20) + cache_read(60) + cache_creation(20) == prompt(100)
        assert_eq!(result["usage"]["input_tokens"], 20);
        assert_eq!(result["usage"]["cache_read_input_tokens"], 60);
        assert_eq!(result["usage"]["cache_creation_input_tokens"], 20);
    }

    #[test]
    fn test_openai_to_anthropic_clamps_input_when_cache_exceeds_prompt() {
        // prompt(100) < cache_read(60)+cache_creation(50)=110: saturating clamps
        // to 0, preventing underflow.
        // Tripwire: guards against future changes that swap saturating_sub for
        // plain subtraction (debug panic / release wrap).
        let input = json!({
            "id": "chatcmpl-uf",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "x"},
                "finish_reason": "stop"
            }],
            "usage": {
                "prompt_tokens": 100,
                "completion_tokens": 10,
                "cache_read_input_tokens": 60,
                "cache_creation_input_tokens": 50
            }
        });
        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["usage"]["input_tokens"], 0);
        assert_eq!(result["usage"]["cache_read_input_tokens"], 60);
        assert_eq!(result["usage"]["cache_creation_input_tokens"], 50);
    }

    #[test]
    fn test_openai_to_anthropic_finish_reason_content_filter_maps_end_turn() {
        let input = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {"role": "assistant", "content": "Blocked"},
                "finish_reason": "content_filter"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 1}
        });

        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["stop_reason"], "end_turn");
    }

    #[test]
    fn test_openai_to_anthropic_with_legacy_function_call() {
        let input = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": null,
                    "function_call": {
                        "name": "get_weather",
                        "arguments": "{\"location\":\"Tokyo\"}"
                    }
                },
                "finish_reason": "function_call"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });

        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "tool_use");
        assert_eq!(result["content"][0]["name"], "get_weather");
        assert_eq!(result["content"][0]["input"]["location"], "Tokyo");
        assert_eq!(result["stop_reason"], "tool_use");
    }

    #[test]
    fn test_openai_to_anthropic_with_content_parts_and_refusal() {
        let input = json!({
            "id": "chatcmpl-123",
            "model": "gpt-4",
            "choices": [{
                "index": 0,
                "message": {
                    "role": "assistant",
                    "content": [
                        {"type": "text", "text": "Hello"},
                        {"type": "refusal", "refusal": "I can't do that"}
                    ]
                },
                "finish_reason": "stop"
            }],
            "usage": {"prompt_tokens": 10, "completion_tokens": 5}
        });

        let result = openai_to_anthropic(input).unwrap();
        assert_eq!(result["content"][0]["type"], "text");
        assert_eq!(result["content"][0]["text"], "Hello");
        assert_eq!(result["content"][1]["type"], "text");
        assert_eq!(result["content"][1]["text"], "I can't do that");
    }

    #[test]
    fn test_is_openai_o_series() {
        assert!(is_openai_o_series("o1"));
        assert!(is_openai_o_series("o1-preview"));
        assert!(is_openai_o_series("o1-mini"));
        assert!(is_openai_o_series("o3"));
        assert!(is_openai_o_series("o3-mini"));
        assert!(is_openai_o_series("o4-mini"));
        assert!(!is_openai_o_series("gpt-4o"));
        assert!(!is_openai_o_series("openai-gpt"));
        assert!(!is_openai_o_series("o"));
        assert!(!is_openai_o_series(""));
    }

    #[test]
    fn test_supports_reasoning_effort() {
        assert!(supports_reasoning_effort("o1"));
        assert!(supports_reasoning_effort("o3-mini"));
        assert!(supports_reasoning_effort("gpt-5"));
        assert!(supports_reasoning_effort("gpt-5.4"));
        assert!(supports_reasoning_effort("gpt-5-codex"));
        assert!(supports_reasoning_effort("grok-4.5"));
        assert!(supports_reasoning_effort("grok-build-0.1"));
        assert!(!supports_reasoning_effort("gpt-4o"));
        assert!(!supports_reasoning_effort("claude-sonnet-4-6"));
    }

    // ── resolve_reasoning_effort unit tests ──

    #[test]
    fn test_output_config_low_maps_to_reasoning_effort_low() {
        let body = json!({"output_config": {"effort": "low"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_output_config_medium_maps_to_reasoning_effort_medium() {
        let body = json!({"output_config": {"effort": "medium"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("medium"));
    }

    #[test]
    fn test_output_config_high_maps_to_reasoning_effort_high() {
        let body = json!({"output_config": {"effort": "high"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_output_config_max_maps_to_reasoning_effort_xhigh() {
        let body = json!({"output_config": {"effort": "max"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("xhigh"));
    }

    #[test]
    fn test_output_config_takes_priority_over_thinking() {
        // Even with thinking.adaptive present, explicit effort wins
        let body = json!({
            "output_config": {"effort": "low"},
            "thinking": {"type": "adaptive"}
        });
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_output_config_unknown_value_no_reasoning_effort() {
        let body = json!({"output_config": {"effort": "turbo"}});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }

    #[test]
    fn test_thinking_enabled_small_budget_maps_low() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 1024}});
        assert_eq!(resolve_reasoning_effort(&body), Some("low"));
    }

    #[test]
    fn test_thinking_enabled_medium_budget_maps_medium() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 8000}});
        assert_eq!(resolve_reasoning_effort(&body), Some("medium"));
    }

    #[test]
    fn test_thinking_enabled_large_budget_maps_high() {
        let body = json!({"thinking": {"type": "enabled", "budget_tokens": 32000}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_thinking_enabled_without_budget_maps_high() {
        let body = json!({"thinking": {"type": "enabled"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("high"));
    }

    #[test]
    fn test_thinking_adaptive_maps_xhigh() {
        let body = json!({"thinking": {"type": "adaptive"}});
        assert_eq!(resolve_reasoning_effort(&body), Some("xhigh"));
    }

    #[test]
    fn test_thinking_disabled_no_reasoning_effort() {
        let body = json!({"thinking": {"type": "disabled"}});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }

    #[test]
    fn test_no_thinking_field_no_reasoning_effort() {
        let body = json!({"messages": [{"role": "user", "content": "Hello"}]});
        assert_eq!(resolve_reasoning_effort(&body), None);
    }

    // ── Integration: anthropic_to_openai with resolve_reasoning_effort ──

    #[test]
    fn test_non_reasoning_model_no_reasoning_effort() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert!(result.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_reasoning_model_with_output_config_effort() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "medium"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["reasoning_effort"], "medium");
    }

    #[test]
    fn test_reasoning_model_with_output_config_max() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "output_config": {"effort": "max"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["reasoning_effort"], "xhigh");
    }

    #[test]
    fn test_reasoning_model_thinking_enabled_small_budget() {
        let input = json!({
            "model": "o3",
            "max_tokens": 1024,
            "thinking": {"type": "enabled", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["reasoning_effort"], "low");
    }

    #[test]
    fn test_reasoning_model_thinking_adaptive() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "thinking": {"type": "adaptive"},
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["reasoning_effort"], "xhigh");
    }

    #[test]
    fn test_reasoning_model_no_thinking_no_effort() {
        let input = json!({
            "model": "gpt-5.4",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert!(result.get("reasoning_effort").is_none());
    }

    #[test]
    fn test_anthropic_to_openai_o_series_max_completion_tokens() {
        for model in &["o1", "o3-mini", "o4-mini"] {
            let input = json!({
                "model": model,
                "max_tokens": 4096,
                "messages": [{"role": "user", "content": "Hello"}]
            });

            let result = anthropic_to_openai(input).unwrap();
            assert!(
                result.get("max_tokens").is_none(),
                "{model} should not have max_tokens"
            );
            assert_eq!(
                result["max_completion_tokens"], 4096,
                "{model} should use max_completion_tokens"
            );
        }
    }

    #[test]
    fn test_anthropic_to_openai_non_o_series_keeps_max_tokens() {
        let input = json!({
            "model": "gpt-4o",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": "Hello"}]
        });

        let result = anthropic_to_openai(input).unwrap();
        assert_eq!(result["max_tokens"], 1024);
        assert!(result.get("max_completion_tokens").is_none());
    }

    fn run_tool_choice(value: Value) -> Value {
        let input = json!({
            "model": "gpt-4o",
            "messages": [{"role": "user", "content": "Hello"}],
            "tools": [{
                "name": "search",
                "description": "search the web",
                "input_schema": {"type": "object", "properties": {}}
            }],
            "tool_choice": value,
        });
        anthropic_to_openai(input).unwrap()["tool_choice"].clone()
    }

    #[test]
    fn tool_choice_string_any_maps_to_required() {
        assert_eq!(run_tool_choice(json!("any")), json!("required"));
    }

    #[test]
    fn tool_choice_string_auto_and_none_pass_through() {
        assert_eq!(run_tool_choice(json!("auto")), json!("auto"));
        assert_eq!(run_tool_choice(json!("none")), json!("none"));
    }

    #[test]
    fn tool_choice_object_any_maps_to_required() {
        assert_eq!(run_tool_choice(json!({"type": "any"})), json!("required"));
    }

    #[test]
    fn tool_choice_object_auto_and_none_collapse_to_string() {
        assert_eq!(run_tool_choice(json!({"type": "auto"})), json!("auto"));
        assert_eq!(run_tool_choice(json!({"type": "none"})), json!("none"));
    }

    #[test]
    fn tool_choice_forced_tool_maps_to_nested_function_selector() {
        // Anthropic {"type":"tool","name":"X"} must become OpenAI Chat
        // {"type":"function","function":{"name":"X"}} — the *nested* form, not
        // the flat Responses-API form.
        assert_eq!(
            run_tool_choice(json!({"type": "tool", "name": "search"})),
            json!({"type": "function", "function": {"name": "search"}}),
        );
    }
}
