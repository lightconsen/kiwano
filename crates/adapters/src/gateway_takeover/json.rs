//! The JSON/JSONC text helpers every JSON agent here starts from: a
//! tolerant parse (a missing file is an empty object), the root-object
//! assertion, and the field normalizer whose replacing half is logged.

use serde_json::{json, Map, Value};

pub(crate) fn parse_jsonc(content: &str, label: &str) -> Result<Value, String> {
    if content.trim().is_empty() {
        return Ok(Value::Object(Map::new()));
    }
    json5::from_str(content).map_err(|e| format!("{label} is not valid JSON/JSONC: {e}"))
}

pub(crate) fn require_object(v: Value, label: &str) -> Result<Map<String, Value>, String> {
    v.as_object()
        .cloned()
        .ok_or(format!("{label} root must be a JSON object"))
}

/// Make sure a field holds an object, replacing whatever else is in it.
///
/// The replacing half is destructive — whatever the user had in that field is
/// gone — and a takeover is supposed to *add* an entry, not to discard a
/// setting. So it is logged: this is the only place that knows it happened, and
/// a config the user cannot account for is worse than a takeover that refused.
/// (The other half, a field that was simply absent, is what a takeover is for
/// and is silent.)
pub(crate) fn object_field<'a>(
    obj: &'a mut serde_json::Map<String, Value>,
    file: &str,
    field: &str,
) -> &'a mut serde_json::Map<String, Value> {
    if !obj.get(field).is_some_and(Value::is_object) {
        if obj.get(field).is_some() {
            log::warn!("{file}: `{field}` is not an object; replaced it with an empty one");
        }
        obj.insert(field.to_string(), json!({}));
    }
    obj.get_mut(field)
        .and_then(Value::as_object_mut)
        .expect("a field made an object just above")
}
