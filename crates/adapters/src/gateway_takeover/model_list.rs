//! What the two model-list agents (workbuddy, codebuddy) share: they name
//! their provider by URL inside each model row rather than by a provider
//! entry, so a takeover rewrites the row it finds — `GATEWAY_VENDOR` is how
//! the second run finds the row the first one wrote — and creates one from
//! the placeholder only when the list was empty.

use crate::gateway_takeover::gateway::GATEWAY_LABEL;
use serde_json::{json, Value};

/// The vendor mark on every entry this family of transforms writes. It is how
/// a second takeover finds the row it wrote the first time, so enabling twice
/// updates one entry instead of taking over another.
pub const GATEWAY_VENDOR: &str = "Kiwano";

fn is_our_entry(entry: &Value) -> bool {
    entry.get("vendor").and_then(Value::as_str) == Some(GATEWAY_VENDOR)
}

/// The entry to take over in a model-list config: ours from a previous
/// takeover, else the user's first, else none (the list is empty).
///
/// The first entry rather than all of them: WorkBuddy and CodeBuddy name their
/// provider by URL inside each model row and have no provider-prefix dimension
/// to swap (unlike OpenCode's `provider/model` selector), so *adding* an entry
/// would leave two rows with one id and an ambiguous pick. Taking over the row
/// keeps one id, one meaning.
pub(crate) fn takeover_target_index(entries: &[Value]) -> Option<usize> {
    entries
        .iter()
        .position(is_our_entry)
        .or(if entries.is_empty() { None } else { Some(0) })
}

/// Write our URL and key onto an entry, keeping everything else — the id above
/// all, because that is the model name the request goes upstream with.
pub(crate) fn point_entry_at_gateway(
    entry: &mut Value,
    url: &str,
    key: &str,
) -> Result<(), String> {
    let obj = entry
        .as_object_mut()
        .ok_or("model entries must be JSON objects")?;
    obj.insert("vendor".into(), json!(GATEWAY_VENDOR));
    obj.insert("url".into(), json!(url));
    obj.insert("apiKey".into(), json!(key));
    Ok(())
}

pub(crate) fn new_entry(model_id: &str, url: &str, key: &str) -> Value {
    json!({
        "id": model_id,
        "name": GATEWAY_LABEL,
        "vendor": GATEWAY_VENDOR,
        "url": url,
        "apiKey": key,
        "supportsToolCall": true,
        "supportsImages": false,
    })
}
