//! Anthropic `tool_choice` → the OpenAI Chat Completions selector.
//!
//! A leaf: one function over one JSON value. `request` is its only caller.

use serde_json::{json, Value};

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
/// The Responses API uses a flatter `{"type":"function","name":"X"}` selector.
/// Upstream this had a sibling — `map_tool_choice_to_responses` in
/// `transform_responses.rs` — and the comment asked that the two be kept in
/// sync; neither that file nor a Responses conversation path was ported, so
/// there is nothing to keep in sync with.
pub(crate) fn map_tool_choice_to_chat(tool_choice: &Value) -> Value {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::proxy::providers::transform::request::anthropic_to_openai;

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
