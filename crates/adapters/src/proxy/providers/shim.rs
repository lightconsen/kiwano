//! Compat shim: sanitize request bodies on the native passthrough path.
//!
//! The passthrough forwards a body verbatim whenever the client's protocol
//! matches the provider's. That is cheap and fully faithful — but "same
//! protocol" does not mean "same dialect": clients and upstreams evolve
//! independently, and a field the client's current version considers valid
//! can be rejected by an upstream parser built from an older snapshot. The
//! failure always looks the same by the time anyone sees it — HTTP 400 — and
//! it lands on the user.
//!
//! The shim is the narrow exception to "verbatim": a small, closed list of
//! known-incompatible patterns, applied per effective protocol. Anything the
//! rules do not name is left alone, and every change is reported so the
//! request log can explain the delta. When nothing matches, the caller
//! forwards the original bytes without re-serializing — upstream prompt
//! caches key on the exact prefix, and a body we never needed to touch must
//! not rotate it.
//!
//! The rules, each named for the failure it prevents:
//! - R1 [`sanitize_anthropic`]: a top-level `thinking` value an
//!   anthropic-native upstream rejects (`adaptive`, invalid shapes).
//! - R2 [`sanitize_anthropic`]: assistant-history thinking blocks rejected
//!   by upstreams that do not implement extended thinking, once the client
//!   is not asking for thinking.
//! - R3 both: `null` tool schemas from clients like Codex, rejected by
//!   strict JSON-Schema parsers.
//!
//! Stripping perturbs an upstream prompt cache once — the alternative was a
//! 400. Idempotent by construction: a second pass finds nothing to do.

use serde_json::json;
use serde_json::Value;

/// The notes column is a diagnostic, not a transcript: cap it so a
/// pathological body cannot grow a log row without bound.
const MAX_NOTES: usize = 20;

/// Sanitize an anthropic-format body in place. Returns one human-readable
/// note per action, in application order; an empty vec means the body was
/// left untouched and the caller should forward the original bytes.
pub fn sanitize_anthropic(body: &mut Value) -> Vec<String> {
    let mut notes = Notes::default();
    remove_unsupported_thinking(body, &mut notes);
    strip_assistant_history_thinking(body, &mut notes);
    fill_null_input_schemas(body, &mut notes);
    notes.finish()
}

/// Sanitize an openai-format body in place (chat completions and the
/// Responses API's flattened tool shape). Same contract as
/// [`sanitize_anthropic`].
pub fn sanitize_openai(body: &mut Value) -> Vec<String> {
    let mut notes = Notes::default();
    fill_null_parameters(body, &mut notes);
    notes.finish()
}

/// Note collector with the row-width cap applied at the end.
#[derive(Default)]
struct Notes {
    items: Vec<String>,
}

impl Notes {
    fn push(&mut self, note: String) {
        self.items.push(note);
    }

    fn finish(self) -> Vec<String> {
        let mut items = self.items;
        if items.len() > MAX_NOTES {
            let overflow = items.len() - (MAX_NOTES - 1);
            items.truncate(MAX_NOTES - 1);
            items.push(format!("… and {overflow} more"));
        }
        items
    }
}

/// R1: the client's `thinking` dialect is newer than the upstream's parser.
///
/// Anthropic-native upstreams validate `thinking.type` against
/// `{enabled, disabled}`; anything else — Claude Code ships `adaptive` — is
/// a 400 on a request that was never going to succeed, so removal can only
/// help. `enabled` and `disabled` pass untouched, `budget_tokens` included.
/// Never rewritten: mapping `adaptive` onto `enabled` would silently change
/// generation behavior on a budget the client chose.
fn remove_unsupported_thinking(body: &mut Value, notes: &mut Notes) {
    let Some(obj) = body.as_object_mut() else {
        return;
    };
    let Some(thinking) = obj.get("thinking") else {
        return;
    };
    let removable = match thinking.get("type").and_then(Value::as_str) {
        Some("enabled" | "disabled") => false,
        Some(other) => {
            notes.push(format!("thinking: removed unsupported type \"{other}\""));
            true
        }
        // `null`, a non-object, or an object without a usable `type`: no
        // parser defines what this means, so no parser accepts it.
        _ => {
            notes.push("thinking: removed invalid value".to_string());
            true
        }
    };
    if removable {
        // shift_remove, not remove: with `preserve_order` the plain remove is
        // a swap_remove and would drag the last key into thinking's slot,
        // reordering keys the rules never touched.
        obj.shift_remove("thinking");
    }
}

/// R2: thinking blocks in assistant history outlive the upstream that
/// produced them — switch providers mid-conversation and the new upstream
/// rejects blocks it cannot verify.
///
/// Stripped only while the request is *not* asking for thinking: top-level
/// `thinking` absent, `disabled`, already removed by R1, or invalid. When
/// `thinking.type == "enabled"` the blocks are part of the extended-thinking
/// contract (a tool loop's assistant turns must carry theirs), so stripping
/// would break genuine extended-thinking conversations; they stay, and any
/// incompatibility remains the upstream's 400 rather than a silent rewrite.
fn strip_assistant_history_thinking(body: &mut Value, notes: &mut Notes) {
    if thinking_enabled(body) {
        return;
    }
    let Some(messages) = body.get_mut("messages").and_then(Value::as_array_mut) else {
        return;
    };
    let mut removed = 0usize;
    let mut placeholders: Vec<String> = Vec::new();
    for (i, message) in messages.iter_mut().enumerate() {
        let Some(obj) = message.as_object_mut() else {
            continue;
        };
        if obj.get("role").and_then(Value::as_str) != Some("assistant") {
            continue;
        }
        let Some(content) = obj.get_mut("content").and_then(Value::as_array_mut) else {
            continue;
        };
        let before = content.len();
        content.retain(|block| {
            !matches!(
                block.get("type").and_then(Value::as_str),
                Some("thinking" | "redacted_thinking")
            )
        });
        let dropped = before - content.len();
        if dropped == 0 {
            continue;
        }
        removed += dropped;
        if content.is_empty() {
            // A thinking-only assistant message would be left with an empty
            // content array, which strict parsers reject in its own right;
            // a one-block placeholder keeps the message — and the alternating
            // role rhythm — intact.
            *content = vec![json!({ "type": "text", "text": "[thinking omitted]" })];
            placeholders.push(format!(
                "assistant message #{i}: thinking-only content replaced with a text placeholder"
            ));
        }
    }
    if removed > 0 {
        notes.push(format!(
            "assistant history: removed {removed} thinking/redacted_thinking block(s)"
        ));
        notes.items.extend(placeholders);
    }
}

/// True when the (post-R1) body explicitly asks for extended thinking.
fn thinking_enabled(body: &Value) -> bool {
    body.get("thinking")
        .and_then(|t| t.get("type"))
        .and_then(Value::as_str)
        == Some("enabled")
}

/// R3 (anthropic format): a `null` `input_schema` is legal JSON but no
/// schema parser accepts it — a schema must be an object.
fn fill_null_input_schemas(body: &mut Value, notes: &mut Notes) {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for (i, tool) in tools.iter_mut().enumerate() {
        let Some(tool) = tool.as_object_mut() else {
            continue;
        };
        if tool.get("input_schema").map(Value::is_null) != Some(true) {
            continue;
        }
        let name = tool_name(tool.get("name"), i);
        tool.insert("input_schema".to_string(), minimal_object_schema());
        notes.push(format!(
            "tool \"{name}\": null input_schema replaced with an empty object schema"
        ));
    }
}

/// R3 (openai formats): Codex sends tools whose `parameters` is `null`; a
/// strict JSON-Schema parser requires an object and 400s.
///
/// Only an explicit JSON null is filled. A *missing* `parameters` key is
/// optional per the OpenAI schema, and filling it would change behavior on
/// requests that may be perfectly valid. Both wire shapes are covered:
/// chat completions nests the schema under `function`, the Responses API
/// lays it on the tool itself.
fn fill_null_parameters(body: &mut Value, notes: &mut Notes) {
    let Some(tools) = body.get_mut("tools").and_then(Value::as_array_mut) else {
        return;
    };
    for (i, tool) in tools.iter_mut().enumerate() {
        let Some(tool) = tool.as_object_mut() else {
            continue;
        };
        let is_function = tool
            .get("type")
            .and_then(Value::as_str)
            .is_none_or(|t| t == "function");
        if !is_function {
            continue;
        }
        if let Some(function) = tool.get_mut("function").and_then(Value::as_object_mut) {
            if function.get("parameters").map(Value::is_null) == Some(true) {
                let name = tool_name(function.get("name"), i);
                function.insert("parameters".to_string(), minimal_object_schema());
                notes.push(format!(
                    "tool \"{name}\": null parameters schema replaced with an empty object schema"
                ));
            }
        } else if tool.get("parameters").map(Value::is_null) == Some(true) {
            let name = tool_name(tool.get("name"), i);
            tool.insert("parameters".to_string(), minimal_object_schema());
            notes.push(format!(
                "tool \"{name}\": null parameters schema replaced with an empty object schema"
            ));
        }
    }
}

fn tool_name(name: Option<&Value>, index: usize) -> String {
    name.and_then(Value::as_str)
        .map_or_else(|| format!("#{index}"), ToString::to_string)
}

/// The minimal schema `clean_schema` itself produces for an empty root:
/// satisfies every "must be an object" requirement without inventing shape.
fn minimal_object_schema() -> Value {
    json!({ "type": "object", "properties": {} })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // --- R1: top-level thinking ---

    #[test]
    fn r1_gh2693_strips_adaptive_thinking_for_anthropic_native() {
        let mut body = json!({
            "model": "m",
            "thinking": {"type": "adaptive", "budget_tokens": 2048},
            "messages": [{"role": "user", "content": "hi"}]
        });
        let notes = sanitize_anthropic(&mut body);
        assert!(body.get("thinking").is_none());
        assert_eq!(
            notes,
            vec!["thinking: removed unsupported type \"adaptive\""]
        );
    }

    #[test]
    fn r1_keeps_enabled_and_disabled_thinking() {
        for enabled in ["enabled", "disabled"] {
            let mut body = json!({
                "model": "m",
                "thinking": {"type": enabled, "budget_tokens": 1024},
                "messages": []
            });
            let notes = sanitize_anthropic(&mut body);
            assert_eq!(body["thinking"]["type"], enabled);
            assert_eq!(body["thinking"]["budget_tokens"], 1024);
            assert!(notes.is_empty());
        }
    }

    #[test]
    fn r1_removes_null_and_invalid_thinking_values() {
        let mut null_value = json!({"model": "m", "thinking": null, "messages": []});
        let notes = sanitize_anthropic(&mut null_value);
        assert!(null_value.get("thinking").is_none());
        assert_eq!(notes, vec!["thinking: removed invalid value"]);

        let mut shapeless = json!({"model": "m", "thinking": {"budget_tokens": 1}, "messages": []});
        let notes = sanitize_anthropic(&mut shapeless);
        assert!(shapeless.get("thinking").is_none());
        assert_eq!(notes, vec!["thinking: removed invalid value"]);
    }

    // --- R2: assistant-history thinking blocks ---

    #[test]
    fn r2_gh3216_strips_thinking_blocks_when_thinking_is_absent_or_disabled() {
        let history = json!([
            {"role": "user", "content": "go"},
            {"role": "assistant", "content": [
                {"type": "thinking", "thinking": "step one", "signature": "sig"},
                {"type": "tool_use", "id": "t1", "name": "read", "input": {}}
            ]},
            {"role": "user", "content": [{"type": "tool_result", "tool_use_id": "t1", "content": "ok"}]},
            {"role": "assistant", "content": [
                {"type": "redacted_thinking", "data": "opaque"},
                {"type": "text", "text": "done"}
            ]}
        ]);
        for thinking in [Value::Null, json!({"type": "disabled"})] {
            let mut body = json!({"model": "m", "messages": history, "thinking": thinking});
            let notes = sanitize_anthropic(&mut body);
            let messages = body["messages"].as_array().unwrap();
            assert!(messages[1]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["type"] != "thinking"));
            assert!(messages[3]["content"]
                .as_array()
                .unwrap()
                .iter()
                .all(|b| b["type"] != "redacted_thinking"));
            assert!(notes
                .iter()
                .any(|n| n == "assistant history: removed 2 thinking/redacted_thinking block(s)"));
        }
    }

    #[test]
    fn r2_keeps_history_when_thinking_is_enabled() {
        let mut body = json!({
            "model": "m",
            "thinking": {"type": "enabled", "budget_tokens": 4096},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "step one", "signature": "sig"},
                    {"type": "text", "text": "done"}
                ]}
            ]
        });
        let notes = sanitize_anthropic(&mut body);
        assert_eq!(
            body["messages"][0]["content"][0]["type"], "thinking",
            "extended-thinking tool loops require their history blocks"
        );
        assert!(notes.is_empty());
    }

    #[test]
    fn r2_replaces_a_thinking_only_assistant_message_with_a_text_placeholder() {
        let mut body = json!({
            "model": "m",
            "messages": [
                {"role": "user", "content": "go"},
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm", "signature": "sig"}
                ]}
            ]
        });
        let notes = sanitize_anthropic(&mut body);
        let content = body["messages"][1]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
        assert_eq!(content[0]["text"], "[thinking omitted]");
        assert!(notes.iter().any(|n| n.starts_with(
            "assistant message #1: thinking-only content replaced with a text placeholder"
        )));
    }

    #[test]
    fn r2_composes_with_r1() {
        // The exact #3216 shape: Claude Code ships `thinking: adaptive` AND a
        // history of thinking blocks; removing the parameter must unlock the
        // history strip in the same pass.
        let mut body = json!({
            "model": "m",
            "thinking": {"type": "adaptive"},
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm", "signature": "sig"},
                    {"type": "text", "text": "done"}
                ]}
            ]
        });
        let notes = sanitize_anthropic(&mut body);
        assert!(body.get("thinking").is_none());
        assert!(body["messages"][0]["content"][0]["type"] == "text");
        assert!(notes
            .iter()
            .any(|n| n == "assistant history: removed 1 thinking/redacted_thinking block(s)"));
    }

    // --- R3: null tool schemas ---

    #[test]
    fn r3_gh5327_fills_null_function_parameters() {
        let mut body = json!({
            "model": "m",
            "tools": [
                {"type": "function", "function": {
                    "name": "automation_update", "parameters": null
                }},
                {"type": "function", "function": {
                    "name": "read_file", "parameters": {"type": "object", "properties": {}}
                }}
            ]
        });
        let notes = sanitize_openai(&mut body);
        assert_eq!(body["tools"][0]["function"]["parameters"]["type"], "object");
        assert_eq!(
            body["tools"][1]["function"]["parameters"]["type"], "object",
            "a real schema passes through untouched"
        );
        assert_eq!(
            notes,
            vec![
                "tool \"automation_update\": null parameters schema replaced with an empty object schema"
            ]
        );
    }

    #[test]
    fn r3_fills_null_parameters_on_responses_flattened_tools() {
        let mut body = json!({
            "model": "m",
            "tools": [
                {"type": "function", "name": "codex_app__run", "parameters": null}
            ]
        });
        let notes = sanitize_openai(&mut body);
        assert_eq!(body["tools"][0]["parameters"]["type"], "object");
        assert_eq!(notes.len(), 1);
    }

    #[test]
    fn r3_leaves_missing_parameters_alone() {
        let mut body = json!({
            "model": "m",
            "tools": [
                {"type": "function", "function": {"name": "no_params_tool"}}
            ]
        });
        let notes = sanitize_openai(&mut body);
        assert!(body["tools"][0]["function"].get("parameters").is_none());
        assert!(notes.is_empty());
    }

    #[test]
    fn r3_fills_null_input_schema_for_anthropic() {
        let mut body = json!({
            "model": "m",
            "tools": [
                {"name": "read", "description": "d", "input_schema": null}
            ]
        });
        let notes = sanitize_anthropic(&mut body);
        assert_eq!(body["tools"][0]["input_schema"]["type"], "object");
        assert_eq!(
            notes,
            vec!["tool \"read\": null input_schema replaced with an empty object schema"]
        );
    }

    // --- cross-rule contracts ---

    #[test]
    fn sanitize_is_idempotent() {
        let mut anthropic = json!({
            "model": "m",
            "thinking": {"type": "adaptive"},
            "tools": [{"name": "read", "input_schema": null}],
            "messages": [
                {"role": "assistant", "content": [
                    {"type": "thinking", "thinking": "hmm", "signature": "s"}
                ]}
            ]
        });
        sanitize_anthropic(&mut anthropic);
        let once = anthropic.clone();
        let second = sanitize_anthropic(&mut anthropic);
        assert_eq!(anthropic, once);
        assert!(second.is_empty());

        let mut openai = json!({
            "model": "m",
            "tools": [{"type": "function", "name": "t", "parameters": null}]
        });
        sanitize_openai(&mut openai);
        let once = openai.clone();
        let second = sanitize_openai(&mut openai);
        assert_eq!(openai, once);
        assert!(second.is_empty());
    }

    #[test]
    fn notes_are_capped() {
        let mut messages = Vec::new();
        messages.push(json!({"role": "user", "content": "go"}));
        for _ in 0..25 {
            messages.push(json!({"role": "assistant", "content": [
                {"type": "thinking", "thinking": "hmm", "signature": "s"}
            ]}));
        }
        let mut body = json!({"model": "m", "messages": messages});
        let notes = sanitize_anthropic(&mut body);
        assert_eq!(notes.len(), MAX_NOTES);
        assert_eq!(notes.last().unwrap(), "… and 7 more");
    }

    #[test]
    fn order_is_preserved() {
        // serde_json's preserve_order keeps the client's key order through a
        // re-serialization; a shim pass must not rotate what it leaves.
        let mut body = json!({
            "zzz_custom": 1,
            "thinking": {"type": "adaptive"},
            "aaa_custom": 2,
            "messages": []
        });
        sanitize_anthropic(&mut body);
        let keys: Vec<&str> = body
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        assert_eq!(keys, vec!["zzz_custom", "aaa_custom", "messages"]);
    }
}
