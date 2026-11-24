//! The provider entry OpenCode and its Kilo Code fork share: an OpenAI-SDK
//! provider (`npm: @ai-sdk/openai-compatible`) filed under the gateway id, and
//! the top-level `model: "<provider>/<model>"` selector that names it.
//!
//! Kilo is a fork of OpenCode and kept this schema verbatim (its
//! `ConfigProviderV1.Info` carries `options.baseURL`/`options.apiKey`, and its
//! `globalConfigFile()` picks among `kilo.jsonc`/`kilo.json`/`opencode.jsonc`/
//! `opencode.json`/`config.json`), so the transform itself is one function with
//! the file name in it — the two agents differ in *where* the file lives, which
//! is `kiwano_core::takeover::paths`'s business, not this module's.

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

/// Upsert the gateway provider and rewrite the top-level `model:
/// "<provider>/<model>"` selector to route through the gateway, keeping the
/// user's model id. A missing or non-slash-form `model` key is left alone (the
/// user picks a gateway model in the agent's own model picker).
///
/// Every other provider entry survives: this is an additive takeover, and the
/// user's config is the thing being added to, not replaced.
pub(crate) fn upsert_openai_compatible_provider(
    content: &str,
    file_label: &str,
    base_url: &str,
    key: &str,
) -> Result<String, String> {
    upsert_openai_compatible_provider_selecting(content, file_label, base_url, key, None)
}

/// [`upsert_openai_compatible_provider`], plus the selector to fall back on
/// when *this* file has none.
///
/// OpenCode's config is a single file, so the plain form is all it needs. Kilo
/// merges up to five and the takeover writes only the highest-priority one: the
/// user's selector commonly lives in another of them (`kilo.jsonc` naming the
/// model while `opencode.json` holds the endpoint). Without the fallback the
/// written file would gain the gateway entry and still not select it — the
/// user's requests would keep going to their old provider while the UI said
/// the agent was taken over.
///
/// The file's own selector wins when it has one: it is the highest-priority
/// file, so that is also what Kilo's merge resolves.
pub(crate) fn upsert_openai_compatible_provider_selecting(
    content: &str,
    file_label: &str,
    base_url: &str,
    key: &str,
    fallback_selection: Option<&str>,
) -> Result<String, String> {
    let root = parse_jsonc(content, file_label)?;
    let mut obj = require_object(root, file_label)?;

    // provider section: normalize to an object before inserting the entry
    object_field(&mut obj, file_label, "provider");
    let entry = json!({
        "npm": "@ai-sdk/openai-compatible",
        "name": GATEWAY_LABEL,
        "options": { "baseURL": base_url, "apiKey": key },
        "models": {},
    });
    obj["provider"][GATEWAY_PROVIDER_ID] = entry;

    // selection: "<old-provider>/<model>" → "kiwano-gateway/<model>"
    let model_id = obj
        .get("model")
        .and_then(Value::as_str)
        .and_then(|m| m.split_once('/'))
        .map(|(_, id)| id)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
        .or_else(|| {
            fallback_selection
                .filter(|id| !id.is_empty())
                .map(str::to_string)
        });
    if let Some(model_id) = model_id {
        obj["provider"][GATEWAY_PROVIDER_ID]["models"][&model_id] = json!({});
        // `Map`'s IndexMut only addresses an existing key: the fallback exists
        // precisely for a file without one, so this has to be an insert.
        obj.insert(
            "model".to_string(),
            json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}")),
        );
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}
