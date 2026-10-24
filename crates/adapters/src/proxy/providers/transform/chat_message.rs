//! One OpenAI Chat message's content → the Anthropic blocks it becomes.
//!
//! The driver in `chat_request` owns ordering, system hoisting and the merging
//! of parallel tool results; this leaf only answers "what blocks is this
//! content?" for the two roles that carry content, plus the one block a
//! `role: "tool"` message turns into.
//!
//! Refusing beats dropping. A part this conversion cannot carry is something
//! the user attached or the model was handed, and an answer that quietly
//! ignored a screenshot is worse than one that says the provider cannot
//! receive it — the same rule `message::UNCONVERTIBLE_BLOCKS` follows in the
//! opposite direction.

use crate::proxy::error::ProxyError;
use serde_json::{json, Value};

/// Blocks for a `user` or `assistant` message's `content` field.
pub(crate) fn content_blocks(content: Option<&Value>) -> Result<Vec<Value>, ProxyError> {
    let Some(content) = content else {
        return Ok(Vec::new());
    };
    if content.is_null() {
        return Ok(Vec::new());
    }
    if let Some(text) = content.as_str() {
        return Ok(text_block(text).into_iter().collect());
    }
    let parts = content.as_array().ok_or_else(|| {
        ProxyError::TransformError(
            "message content must be a string or an array of parts".to_string(),
        )
    })?;

    let mut blocks = Vec::new();
    for part in parts {
        let part_type = part.get("type").and_then(|t| t.as_str()).unwrap_or("");
        match part_type {
            "text" | "input_text" | "output_text" => {
                if let Some(text) = part.get("text").and_then(|t| t.as_str()) {
                    blocks.extend(text_block(text));
                }
            }
            "refusal" => {
                if let Some(text) = part.get("refusal").and_then(|t| t.as_str()) {
                    blocks.extend(text_block(text));
                }
            }
            "image_url" | "input_image" => blocks.push(image_block(part)?),
            other => {
                return Err(ProxyError::TransformError(format!(
                    "content part `{other}` has no Anthropic Messages equivalent: the provider \
                     speaks `anthropic` and this request carries content only an OpenAI endpoint \
                     can receive"
                )));
            }
        }
    }
    Ok(blocks)
}

/// The `tool_result` block one `role: "tool"` message becomes.
pub(crate) fn tool_result_block(message: &Value) -> Result<Value, ProxyError> {
    let id = message
        .get("tool_call_id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ProxyError::TransformError(
                "a `role: \"tool\"` message has no `tool_call_id`, so its result cannot be \
                 matched to the `tool_use` that asked for it"
                    .to_string(),
            )
        })?;

    let content = message.get("content");
    // Anthropic accepts a string or an array of text/image blocks. Parallel
    // results from ordinary clients are strings; media inside a tool output is
    // the case that has to stay as blocks.
    let content = match content {
        None | Some(Value::Null) => json!(""),
        Some(Value::String(s)) => json!(s),
        Some(Value::Array(_)) => {
            let blocks = content_blocks(content)?;
            if blocks.iter().all(|b| b["type"] == "text") {
                let text: Vec<&str> = blocks
                    .iter()
                    .filter_map(|b| b.get("text").and_then(|t| t.as_str()))
                    .collect();
                json!(text.join("\n"))
            } else {
                json!(blocks)
            }
        }
        Some(other) => {
            return Err(ProxyError::TransformError(format!(
                "tool message content must be a string or an array of parts, not `{other}`"
            )));
        }
    };

    Ok(json!({
        "type": "tool_result",
        "tool_use_id": id,
        "content": content
    }))
}

fn text_block(text: &str) -> Option<Value> {
    if text.is_empty() {
        None
    } else {
        Some(json!({"type": "text", "text": text}))
    }
}

fn image_block(part: &Value) -> Result<Value, ProxyError> {
    // Chat: `{"type":"image_url","image_url":{"url":…}}` (or a bare string).
    // Responses, after `responses_to_chat`, is the same Chat part.
    let image = part.get("image_url").unwrap_or(part);
    let url = image
        .as_str()
        .or_else(|| image.get("url").and_then(|u| u.as_str()))
        .ok_or_else(|| {
            ProxyError::TransformError("an image part carries no `image_url.url`".to_string())
        })?;

    let source = if let Some(rest) = url.strip_prefix("data:") {
        // `data:<media-type>;base64,<payload>` — Anthropic takes the same
        // bytes as a base64 source, and nothing else.
        let (meta, data) = rest.split_once(',').ok_or_else(|| {
            ProxyError::TransformError(
                "a `data:` image URI is missing its `,` separator".to_string(),
            )
        })?;
        let media_type = meta.strip_suffix(";base64").ok_or_else(|| {
            ProxyError::TransformError(
                "only `data:<media-type>;base64,` images can be sent to an Anthropic endpoint"
                    .to_string(),
            )
        })?;
        if media_type.is_empty() || !media_type.contains('/') {
            return Err(ProxyError::TransformError(format!(
                "image data URI has no usable media type: `{meta}`"
            )));
        }
        json!({"type": "base64", "media_type": media_type, "data": data})
    } else if url.starts_with("http://") || url.starts_with("https://") {
        // `detail` has no Anthropic field; the source is the whole part.
        json!({"type": "url", "url": url})
    } else {
        return Err(ProxyError::TransformError(format!(
            "image URL `{url}` is neither a `data:` URI nor http(s), which are the two sources \
             an Anthropic endpoint accepts"
        )));
    };

    Ok(json!({"type": "image", "source": source}))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_image_parts_become_blocks() {
        let parts = json!([
            {"type": "text", "text": "look"},
            {"type": "image_url", "image_url": {"url": "https://x.test/a.png"}}
        ]);
        let blocks = content_blocks(Some(&parts)).unwrap();
        assert_eq!(blocks[0], json!({"type": "text", "text": "look"}));
        assert_eq!(
            blocks[1],
            json!({"type": "image", "source": {"type": "url", "url": "https://x.test/a.png"}})
        );
    }

    #[test]
    fn a_base64_data_uri_keeps_its_media_type() {
        let parts = json!([
            {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
        ]);
        let blocks = content_blocks(Some(&parts)).unwrap();
        assert_eq!(
            blocks[0],
            json!({"type": "image",
                   "source": {"type": "base64", "media_type": "image/png", "data": "AAAA"}})
        );
    }

    #[test]
    fn an_unconvertible_part_is_refused_by_name() {
        let parts = json!([{"type": "input_audio", "input_audio": {}}]);
        let err = content_blocks(Some(&parts)).unwrap_err().to_string();
        assert!(err.contains("input_audio"), "{err}");
    }

    #[test]
    fn a_tool_message_needs_its_call_id() {
        let msg = json!({"role": "tool", "content": "ok"});
        assert!(tool_result_block(&msg).is_err());
        let msg = json!({"role": "tool", "tool_call_id": "t1", "content": "ok"});
        assert_eq!(
            tool_result_block(&msg).unwrap(),
            json!({"type": "tool_result", "tool_use_id": "t1", "content": "ok"})
        );
    }

    #[test]
    fn tool_output_media_stays_a_block_array() {
        let msg = json!({
            "role": "tool",
            "tool_call_id": "t1",
            "content": [
                {"type": "text", "text": "shot"},
                {"type": "image_url", "image_url": {"url": "https://x.test/s.png"}}
            ]
        });
        let block = tool_result_block(&msg).unwrap();
        assert_eq!(block["content"][0]["type"], "text");
        assert_eq!(block["content"][1]["type"], "image");
    }
}
