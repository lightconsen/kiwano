//! Kimi CLI: a TOML document edited in place, so the user's comments and
//! formatting survive — the only format-preserving writer here.

use crate::gateway_takeover::gateway::{GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};
use toml_edit::{value, DocumentMut};

// ── kimi (~/.kimi/config.toml, and ~/.kimi-code/config.toml for its successor) ──

/// Take over Kimi CLI's config.
///
/// Unlike the JSON agents this one *adds* a provider instead of taking over an
/// existing row: Kimi names its records (`[providers.<name>]`,
/// `[models.<alias>]`), so a gateway entry and the user's own can coexist with
/// no ambiguity, and selecting ours is one line (`default_model`).
///
/// `provider_type` is the protocol name the installed generation uses —
/// `kimi` for the Python CLI, `openai` for its successor — and it is chosen
/// from the config path by the caller, since only the path tells the
/// generations apart. The Python generation gets `kimi` rather than the
/// `openai_legacy` it also accepts because the two speak the identical
/// chat-completions wire with one difference: the `kimi` provider is the
/// only type whose dispatcher attaches the conversation's `prompt_cache_key`
/// (a per-conversation uuid4), which is the session marker the gateway's
/// `session_hint` reads.
pub fn upsert_kimi_gateway(
    content: &str,
    base_url: &str,
    key: &str,
    provider_type: &str,
) -> Result<String, String> {
    let mut doc: DocumentMut = if content.trim().is_empty() {
        DocumentMut::new()
    } else {
        content
            .parse()
            .map_err(|e| format!("config.toml is not valid TOML: {e}"))?
    };

    // The model the user is on: `default_model` is `<provider>/<model>`. Its id
    // is what goes upstream verbatim, so ours keeps it — a takeover changes
    // where the request goes, not which model answers it.
    let model_id = doc
        .get("default_model")
        .and_then(|v| v.as_str())
        .and_then(|m| m.split_once('/').map(|(_, id)| id))
        .filter(|id| !id.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();
    let alias = format!("{GATEWAY_PROVIDER_ID}/{model_id}");

    doc["providers"][GATEWAY_PROVIDER_ID]["type"] = value(provider_type);
    doc["providers"][GATEWAY_PROVIDER_ID]["base_url"] = value(base_url);
    doc["providers"][GATEWAY_PROVIDER_ID]["api_key"] = value(key);

    doc["models"][&alias]["provider"] = value(GATEWAY_PROVIDER_ID);
    doc["models"][&alias]["model"] = value(model_id.as_str());
    // Capabilities drive which features Kimi offers this model, not whether the
    // request works; copying the ones the user's own model declared keeps the
    // toggles they had.
    if let Some(caps) = doc
        .get("models")
        .and_then(|m| m.get(&alias))
        .and_then(|m| m.get("capabilities"))
        .and_then(|c| c.as_array())
        .cloned()
    {
        doc["models"][&alias]["capabilities"] = value(caps);
    }

    doc["default_model"] = value(alias.as_str());

    Ok(doc.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kimi_upserts_a_provider_and_selects_it() {
        let original = r#"default_model = "kimi-code/kimi-for-coding"
default_thinking = true

[models."kimi-code/kimi-for-coding"]
provider = "managed:kimi-code"
model = "kimi-for-coding"
max_context_size = 262144
capabilities = ["thinking", "image_in"]

[providers."managed:kimi-code"]
type = "kimi"
base_url = "https://api.kimi.com/coding/v1"
api_key = "sk-old"

[loop_control]
max_steps_per_turn = 50
"#;
        let out = upsert_kimi_gateway(
            original,
            "http://127.0.0.1:8317/v1",
            "kw-ag-kimi-abcd",
            "kimi",
        )
        .unwrap();
        let doc: DocumentMut = out.parse().expect("output is valid TOML");
        let ours = &doc["providers"][GATEWAY_PROVIDER_ID];

        assert_eq!(
            doc["default_model"].as_str(),
            Some("kiwano-gateway/kimi-for-coding")
        );
        // The type whose dispatcher attaches the conversation's
        // prompt_cache_key — the session marker the gateway reads.
        assert_eq!(ours["type"].as_str(), Some("kimi"));
        assert_eq!(ours["base_url"].as_str(), Some("http://127.0.0.1:8317/v1"));
        assert_eq!(ours["api_key"].as_str(), Some("kw-ag-kimi-abcd"));
        assert_eq!(
            doc["models"]["kiwano-gateway/kimi-for-coding"]["model"].as_str(),
            Some("kimi-for-coding"),
            "the model id is what goes upstream"
        );
        // The user's own provider and the unrelated section survive.
        assert_eq!(
            doc["providers"]["managed:kimi-code"]["api_key"].as_str(),
            Some("sk-old")
        );
        assert_eq!(
            doc["loop_control"]["max_steps_per_turn"].as_integer(),
            Some(50)
        );
    }

    #[test]
    fn kimi_empty_config_creates_the_sections() {
        let out = upsert_kimi_gateway("", "http://127.0.0.1:8317/v1", "k", "openai").unwrap();
        let doc: DocumentMut = out.parse().unwrap();
        assert_eq!(
            doc["providers"][GATEWAY_PROVIDER_ID]["type"].as_str(),
            Some("openai")
        );
        assert_eq!(
            doc["default_model"].as_str(),
            Some("kiwano-gateway/kiwano"),
            "nothing to copy from: the placeholder id"
        );
    }
}
