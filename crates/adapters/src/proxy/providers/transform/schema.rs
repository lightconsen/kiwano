//! The tool-parameter schema cleanup.
//!
//! OpenAI requires `type: "object"` on the root schema and rejects
//! `format: "uri"`; `clean_schema` fills in the first and removes the second,
//! recursively. A leaf: one function over one JSON value.

use serde_json::{json, Value};

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

#[cfg(test)]
mod tests {
    use super::*;

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
}
