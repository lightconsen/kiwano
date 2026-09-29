//! Command Code: the `provider.<id>` block in `providers.json` and the
//! `model` / `modelProvider` pair in `settings.json` that selects it.
//!
//! Two shapes here are Command Code's own and easy to get wrong: the container
//! key is singular (`provider`, like OpenCode's) and its `models` is an object
//! keyed by model id rather than a list, and the selection keeps the whole
//! `provider/model` reference in `model` with the provider echoed beside it in
//! `modelProvider` — Command Code splits `model` at the *first* slash.
//!
//! The key cannot be written as a literal: Command Code accepts only a
//! reference there (`"$VAR"`, `"{env:VAR}"`, `"!command"`, or `false`) and
//! refuses a pasted raw secret. A keyless entry is just as unusable — our
//! gateway answers a request with no placeholder key 401 — so the reference
//! form is the only one that both carries the key and is accepted: a command
//! whose stdout is the credential, reading a file Kiwano owns. Goose reaches
//! for the same mechanism for the same reason (`goose.rs`).

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};
use crate::gateway_takeover::json::{object_field, parse_jsonc, require_object};
use serde_json::{json, Value};

// ── commandcode (~/.commandcode/settings.json + providers.json) ──

/// The `api` value Command Code's own provider blocks use for an
/// OpenAI-compatible endpoint.
const COMMANDCODE_API: &str = "openai-completions";

/// The key file Kiwano writes beside the config, and the command reference
/// that reads it. Same file-per-agent shape as goose's (`kiwano-gateway.key`).
pub const COMMANDCODE_KEY_FILE: &str = "kiwano-gateway.key";

/// The credential reference for a key file: a `!` command whose stdout is the
/// token. `/bin/cat` on unix, `type` on Windows — the same two the shell
/// itself would offer, and the only forms Command Code documents.
///
/// A path carrying a quote or a newline cannot be expressed in the reference
/// without breaking it, so it is refused rather than written broken.
pub fn commandcode_key_reference(key_file: &str) -> Result<String, String> {
    if key_file.contains('"') || key_file.contains('\n') {
        return Err(format!(
            "the key file path {key_file:?} contains a quote or newline — Command Code's key \
             reference cannot carry it"
        ));
    }
    Ok(if cfg!(windows) {
        format!("!type \"{key_file}\"")
    } else {
        format!("!cat \"{key_file}\"")
    })
}

/// The key file's content: the placeholder key, verbatim.
pub fn commandcode_key_file_content(key: &str) -> String {
    format!("{key}\n")
}

/// The model the user is on: Command Code's `model` is a whole
/// `provider/model` reference split at the first slash, and the gateway
/// forwards model names verbatim — so only the provider prefix changes. A
/// value with no slash is a bare id and passes through; so does a re-run's own
/// `kiwano-gateway/<id>`, whose prefix simply comes off again.
pub fn read_commandcode_selected_model(settings: &str) -> Option<String> {
    let root = parse_jsonc(settings, "settings.json").ok()?;
    let value = root.get("model")?.as_str()?.trim().to_string();
    if value.is_empty() {
        return None;
    }
    Some(match value.split_once('/') {
        Some((_, id)) if !id.trim().is_empty() => id.trim().to_string(),
        _ => value,
    })
}

/// The id to write into both files: what the user was on, or the placeholder.
pub fn commandcode_model_id(settings: &str) -> String {
    read_commandcode_selected_model(settings).unwrap_or_else(|| PLACEHOLDER_MODEL_ID.to_string())
}

/// Select the gateway in Command Code's settings: `model` keeps the user's
/// model id under our provider prefix, `modelProvider` names the provider it
/// belongs to. Every other setting survives.
pub fn select_commandcode_gateway(settings: &str, model_id: &str) -> Result<String, String> {
    let root = parse_jsonc(settings, "settings.json")?;
    let mut obj = require_object(root, "settings.json")?;
    obj.insert(
        "model".into(),
        json!(format!("{GATEWAY_PROVIDER_ID}/{model_id}")),
    );
    obj.insert("modelProvider".into(), json!(GATEWAY_PROVIDER_ID));
    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// Upsert the gateway provider into `providers.json`. `settings` is the
/// *original* settings content: the model id it selected is the one this
/// block has to declare, and reading it from the settings file rather than
/// from this block keeps a re-run stable. `key_file` is the absolute path the
/// credential reference reads.
pub fn upsert_commandcode_providers(
    content: &str,
    settings: &str,
    base_url: &str,
    key_file: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "providers.json")?;
    let mut obj = require_object(root, "providers.json")?;

    let model_id = commandcode_model_id(settings);
    let reference = commandcode_key_reference(key_file)?;
    let provider = object_field(&mut obj, "providers.json", "provider");
    provider.insert(
        GATEWAY_PROVIDER_ID.into(),
        json!({
            "name": GATEWAY_LABEL,
            "api": COMMANDCODE_API,
            "baseURL": base_url,
            "apiKey": reference,
            "models": { model_id.clone(): { "name": model_id } },
        }),
    );

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SETTINGS: &str = r#"{"theme":"dark","model":"deepseek/pro","modelProvider":"deepseek"}"#;

    #[test]
    fn commandcode_selects_the_gateway_and_keeps_the_model() {
        let out = select_commandcode_gateway(SETTINGS, "pro").unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["model"], "kiwano-gateway/pro");
        assert_eq!(v["modelProvider"], "kiwano-gateway");
        assert_eq!(v["theme"], "dark"); // unrelated settings survive
    }

    #[test]
    fn commandcode_model_id_strips_the_provider_prefix_and_is_stable() {
        // Command Code splits `model` at the first slash, so `deepseek/pro`
        // names the provider `deepseek` serving the model `pro`.
        assert_eq!(commandcode_model_id(SETTINGS), "pro");
        assert_eq!(commandcode_model_id(r#"{"model":"gpt-6"}"#), "gpt-6");
        assert_eq!(commandcode_model_id("{}"), PLACEHOLDER_MODEL_ID);
        // A re-run reads back what we wrote.
        let written = select_commandcode_gateway("{}", PLACEHOLDER_MODEL_ID).unwrap();
        assert_eq!(commandcode_model_id(&written), PLACEHOLDER_MODEL_ID);
    }

    #[test]
    fn commandcode_upserts_the_provider_and_keeps_the_users_own() {
        let providers = r#"{"provider":{"mine":{"name":"Mine","api":"openai-completions"}}}"#;
        let out = upsert_commandcode_providers(
            providers,
            SETTINGS,
            "http://127.0.0.1:8317/v1",
            "/home/u/.commandcode/kiwano-gateway.key",
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        let entry = &v["provider"][GATEWAY_PROVIDER_ID];
        assert_eq!(entry["baseURL"], "http://127.0.0.1:8317/v1");
        assert_eq!(entry["api"], COMMANDCODE_API);
        // The key is a reference, never a literal: Command Code refuses those.
        let reference = entry["apiKey"].as_str().unwrap();
        assert!(reference.starts_with('!'), "{reference}");
        assert!(reference.contains(".commandcode/kiwano-gateway.key"));
        // `models` is an object keyed by id, not a list.
        assert_eq!(entry["models"]["pro"]["name"], "pro");
        assert_eq!(v["provider"]["mine"]["name"], "Mine");
    }

    #[test]
    fn commandcode_refuses_a_key_path_it_cannot_express() {
        assert!(commandcode_key_reference("/home/u/with\"quote/key").is_err());
        assert!(commandcode_key_reference("/home/u/line\nbreak/key").is_err());
    }

    #[test]
    fn commandcode_rerun_replaces_our_entry_rather_than_adding_one() {
        let once =
            upsert_commandcode_providers("", SETTINGS, "http://127.0.0.1:8317/v1", "/k").unwrap();
        let twice = upsert_commandcode_providers(&once, SETTINGS, "http://127.0.0.1:8317/v1", "/k")
            .unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["provider"].as_object().unwrap().len(), 1);
    }
}
