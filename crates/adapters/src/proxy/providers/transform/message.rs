//! One Anthropic message → the OpenAI Chat messages it becomes.
//!
//! `convert_message_to_openai` returns a `Vec` because the mapping is
//! one-to-many: a `tool_result` leaves as its own `tool` role message, and media
//! pulled out of a tool result is flushed into a user turn *after* the block
//! loop, so parallel tool results stay adjacent. `UNCONVERTIBLE_BLOCKS` names
//! the blocks that fail the request instead of being dropped in silence.
//!
//! A leaf: `request` is its only caller.

use crate::proxy::{
    error::ProxyError,
    json_canonical::canonical_json_string,
    tool_media::{
        chat_media_part_from_tool_part, flush_pending_chat_tool_media, plan_chat_tool_output_media,
        queue_chat_tool_output_media, ToolMediaScope,
    },
};
use serde_json::{json, Value};

/// Convert a single message into OpenAI format (may produce multiple messages)
/// Anthropic content blocks this conversion has no OpenAI Chat equivalent for,
/// and that carry something the user attached or the model was handed.
///
/// Refusing beats dropping, and the list is deliberately not "everything
/// unknown": a block type from a newer Anthropic API must not break a request
/// the gateway could still serve, so only these fail and anything else passes
/// with a log line.
const UNCONVERTIBLE_BLOCKS: &[&str] = &[
    // PDFs and other files.
    "document",
    // Results the client supplied, or a server-side tool produced.
    "search_result",
    "web_search_tool_result",
    "mcp_tool_result",
    "container_upload",
];

pub(crate) fn convert_message_to_openai(
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
                other => {
                    // Nothing here has an OpenAI Chat equivalent, and most of it
                    // is something the user attached or the model was handed.
                    // Dropping it produced a confident answer about a document
                    // nobody read, which is worse than a refusal: the caller can
                    // act on "this provider cannot receive it", and cannot act on
                    // an answer that quietly left it out.
                    if UNCONVERTIBLE_BLOCKS.contains(&other) {
                        return Err(ProxyError::TransformError(format!(
                            "content block `{other}` has no OpenAI Chat equivalent: the provider \
                             speaks `openai` and the request carries content only an Anthropic \
                             endpoint can receive"
                        )));
                    }
                    // An unknown type is not evidence that it carries content —
                    // a newer client may add one this build predates — so it is
                    // passed over rather than failing the request, but not in
                    // silence.
                    if !other.is_empty() {
                        log::warn!("anthropic→openai: no conversion for content block `{other}`");
                    }
                }
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
