//! Anthropic request → OpenAI Chat Completions request.
//!
//! `anthropic_to_openai_with_reasoning_content` is the driver and the only place
//! the pieces meet: it assembles model, system, messages, parameters, tools and
//! tool_choice, reading the leaf modules around it and handing every message to
//! `message::convert_message_to_openai`. `anthropic_to_openai` is the same call
//! with the DeepSeek/MiMo `reasoning_content` compatibility field off.

use crate::proxy::error::ProxyError;
use crate::proxy::providers::transform::billing::strip_leading_anthropic_billing_header;
use crate::proxy::providers::transform::message::convert_message_to_openai;
use crate::proxy::providers::transform::reasoning::{
    is_openai_o_series, resolve_reasoning_effort, supports_reasoning_effort,
};
use crate::proxy::providers::transform::schema::clean_schema;
use crate::proxy::providers::transform::tool_choice::map_tool_choice_to_chat;
use serde_json::{json, Value};

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

    // NOTE: this format conversion layer only does structural conversion — it
    // neither maps nor chooses the model, which is the caller's business.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::json_canonical::canonical_json_string;

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

    /// A PDF used to vanish. `document` fell into the catch-all arm, so the
    /// upstream was asked to summarise text without the file and answered
    /// anyway — a confident answer about a document the model never saw.
    #[test]
    fn a_document_block_is_refused_rather_than_dropped() {
        let input = json!({
            "model": "m",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "summarise this"},
                {"type": "document", "source": {
                    "type": "base64", "media_type": "application/pdf", "data": "JVBERi0="
                }}
            ]}]
        });

        let err = anthropic_to_openai(input).unwrap_err();
        assert!(
            matches!(err, ProxyError::TransformError(_)),
            "expected a transform refusal, got {err:?}"
        );
        assert!(
            err.to_string().contains("document"),
            "the refusal names the block so the caller knows what to change: {err}"
        );
    }

    /// The other half of that rule: a type this build has never heard of is not
    /// evidence that it carries content, and failing on it would break a request
    /// the gateway could still serve. It is passed over — and logged, so it is
    /// not dropped in silence either.
    #[test]
    fn an_unknown_block_type_is_passed_over_rather_than_refused() {
        let input = json!({
            "model": "m",
            "max_tokens": 1024,
            "messages": [{"role": "user", "content": [
                {"type": "text", "text": "hello"},
                {"type": "some_future_block", "value": "?"}
            ]}]
        });

        let out = anthropic_to_openai(input).expect("an unknown block does not fail the request");
        // A single remaining text part collapses to a plain string — the shape
        // this converter produces for it. What matters here is that the text
        // survives and the unknown block leaves nothing behind.
        assert_eq!(out["messages"][0]["content"], "hello");
    }

    #[test]
    fn test_model_passthrough() {
        // The format conversion layer only does structural conversion; model
        // model choice is the caller's business, not this layer's
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
}
