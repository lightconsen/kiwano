//! Cline: the exception among these agents — its selector names a
//! provider *id*, so the takeover replaces the slot that id names instead
//! of adding a `kiwano-gateway` entry beside it.

use crate::gateway_takeover::gateway::PLACEHOLDER_MODEL_ID;
use crate::gateway_takeover::json::{parse_jsonc, require_object};
use crate::gateway_takeover::readers::CurrentProvider;
use serde_json::{json, Value};

/// The provider slot behind Cline's own "bring your own endpoint" option. Its
/// value is what Cline hands its SDK — `toProviderConfig` reads only the inner
/// `settings.provider` — and `lastUsedProvider` selects a slot *by that value*.
const CLINE_PROVIDER_ID: &str = "openai-compatible";

/// Take over Cline's provider settings.
///
/// Unlike the agents that get a `kiwano-gateway` entry of their own, this one
/// replaces the slot Cline is *using*. That is not a preference: the file holds
/// one entry per provider slot and `lastUsedProvider` names a slot by provider
/// id, so an entry under our own name is not selectable — Cline normalizes the
/// selector back to a known id on its next start. Measured against 3.0.62 by
/// running it, not read off the schema, which allows the shape we would have
/// preferred.
///
/// The model follows the entry being replaced: the gateway forwards model names
/// verbatim, so it has to stay a name the user's own endpoint serves. `now` is
/// an ISO-8601 UTC timestamp because `updatedAt` is required by the schema
/// Cline validates this file with, and a content→content transform has no clock
/// of its own.
pub fn upsert_cline_gateway(
    content: &str,
    url: &str,
    key: &str,
    now: &str,
) -> Result<String, String> {
    let root = parse_jsonc(content, "providers.json")?;
    let mut obj = require_object(root, "providers.json")?;
    if !obj.get("providers").is_some_and(Value::is_object) {
        obj.insert("providers".into(), json!({}));
    }

    let model = obj
        .get("providers")
        .and_then(|p| p.get(CLINE_PROVIDER_ID))
        .and_then(|e| e.get("settings"))
        .and_then(|s| s.get("model"))
        .and_then(Value::as_str)
        .filter(|m| !m.is_empty())
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string();

    obj["providers"][CLINE_PROVIDER_ID] = json!({
        "settings": {
            "provider": CLINE_PROVIDER_ID,
            "baseUrl": url,
            "apiKey": key,
            "model": model,
        },
        "updatedAt": now,
        "tokenSource": "manual",
    });
    obj.insert("lastUsedProvider".into(), json!(CLINE_PROVIDER_ID));
    // The schema pins this to 1; a file that already carries one keeps it, so a
    // generation bump is not silently written back down.
    if !obj.contains_key("version") {
        obj.insert("version".into(), json!(1));
    }

    serde_json::to_string_pretty(&Value::Object(obj)).map_err(|e| e.to_string())
}

/// cline: the entry `lastUsedProvider` names, whose `settings` hold the
/// endpoint and the key.
///
/// Read through the selector rather than by looking up a provider id, because
/// the two are not the same thing here: the map key is a *slot* and the inner
/// `settings.provider` is the wire implementation Cline hands its SDK
/// (`toProviderConfig` reads only the latter). Returns None when nothing is
/// selected, or when the entry is missing either half.
///
/// The endpoint is returned **without** a trailing `/v1`, which is the one place
/// this reader reshapes what it reads. Cline's own `baseUrl` is documented and
/// defaulted to include the version root (`https://api.openai.com/v1`), while a
/// stored Kiwano provider holds the host and lets the gateway append the
/// protocol's path (`gateway::server::data::compose_upstream`, whose output is
/// what `Api.deepseek.com` rows in the catalog rely on). Carrying `/v1` across
/// would make this provider reach `<host>/v1/v1/chat/completions` — measured,
/// not theorised: that is what the first end-to-end takeover of this agent sent.
pub fn read_cline_current(content: &str) -> Option<CurrentProvider> {
    let root = parse_jsonc(content, "providers.json").ok()?;
    let obj = require_object(root, "providers.json").ok()?;
    let selected = obj.get("lastUsedProvider")?.as_str()?;
    let settings = obj.get("providers")?.get(selected)?.get("settings")?;
    let base_url = settings
        .get("baseUrl")?
        .as_str()?
        .trim_end_matches('/')
        .trim_end_matches("/v1")
        .to_string();
    let api_key = settings.get("apiKey")?.as_str()?.to_string();
    if base_url.is_empty() || api_key.is_empty() {
        return None;
    }
    Some(CurrentProvider {
        name: selected.to_string(),
        base_url,
        api_key,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CLINE_NOW: &str = "2026-09-17T12:00:00Z";

    /// The user's other slots, and the `modes` block, survive; the slot being
    /// replaced keeps its model id, because that is the name the request goes
    /// upstream with.
    #[test]
    fn cline_takes_over_the_selected_slot_and_keeps_the_rest() {
        let original = r#"{
  "version": 1,
  "lastUsedProvider": "openai-compatible",
  "modes": { "voiceInput": { "providerId": "deepgram", "modelId": "nova-2" } },
  "providers": {
    "openai-compatible": {
      "settings": {
        "provider": "openai-compatible",
        "apiKey": "sk-theirs",
        "baseUrl": "https://api.deepseek.com/v1",
        "model": "deepseek-v3"
      },
      "updatedAt": "2026-09-01T00:00:00Z",
      "tokenSource": "manual"
    },
    "anthropic": {
      "settings": { "provider": "anthropic", "apiKey": "sk-ant", "model": "claude-sonnet-5" },
      "updatedAt": "2026-09-01T00:00:00Z",
      "tokenSource": "manual"
    }
  }
}"#;
        let out = upsert_cline_gateway(
            original,
            "http://127.0.0.1:8317/v1",
            "kw-ag-cline-abcd",
            CLINE_NOW,
        )
        .unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();

        assert_eq!(v["lastUsedProvider"], CLINE_PROVIDER_ID);
        let ours = &v["providers"][CLINE_PROVIDER_ID];
        assert_eq!(ours["settings"]["provider"], CLINE_PROVIDER_ID);
        assert_eq!(ours["settings"]["baseUrl"], "http://127.0.0.1:8317/v1");
        assert_eq!(ours["settings"]["apiKey"], "kw-ag-cline-abcd");
        assert_eq!(
            ours["settings"]["model"], "deepseek-v3",
            "the model id is what goes upstream, so it is kept"
        );
        assert_eq!(ours["updatedAt"], CLINE_NOW);
        assert_eq!(ours["tokenSource"], "manual");
        assert_eq!(
            v["providers"]["anthropic"]["settings"]["apiKey"], "sk-ant",
            "another slot is not this takeover's business"
        );
        assert_eq!(
            v["modes"]["voiceInput"]["modelId"], "nova-2",
            "an unrelated block survives the round-trip"
        );
        assert_eq!(v["version"], 1);
    }

    /// An existing but empty config is the one case where the slot is created
    /// rather than replaced: nothing to copy a model id from, so the
    /// placeholder the other agents use is what goes in.
    #[test]
    fn cline_empty_file_is_created_with_the_selector() {
        let out = upsert_cline_gateway("", "http://127.0.0.1:8317/v1", "k", CLINE_NOW).unwrap();
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["lastUsedProvider"], CLINE_PROVIDER_ID);
        assert_eq!(
            v["providers"][CLINE_PROVIDER_ID]["settings"]["model"],
            PLACEHOLDER_MODEL_ID
        );
    }

    /// Enabling twice replaces the key on the slot it already wrote instead of
    /// adding a second one.
    #[test]
    fn cline_second_takeover_updates_the_same_slot() {
        let once = upsert_cline_gateway("{}", "http://127.0.0.1:8317/v1", "k1", CLINE_NOW).unwrap();
        let twice =
            upsert_cline_gateway(&once, "http://127.0.0.1:8317/v1", "k2", CLINE_NOW).unwrap();
        let v: Value = serde_json::from_str(&twice).unwrap();
        assert_eq!(v["providers"].as_object().unwrap().len(), 1);
        assert_eq!(
            v["providers"][CLINE_PROVIDER_ID]["settings"]["apiKey"],
            "k2"
        );
    }

    /// A `version` the file already carries is not written back down: the
    /// schema pins it today, and a later generation is not ours to overwrite.
    #[test]
    fn cline_keeps_a_version_it_did_not_write() {
        let out = upsert_cline_gateway(
            r#"{"version": 2}"#,
            "http://127.0.0.1:8317/v1",
            "k",
            CLINE_NOW,
        )
        .unwrap();
        assert_eq!(serde_json::from_str::<Value>(&out).unwrap()["version"], 2);
    }

    #[test]
    fn cline_reader_takes_the_selected_slot() {
        let content = r#"{
  "lastUsedProvider": "openai-compatible",
  "providers": {
    "openai-compatible": {
      "settings": {
        "provider": "openai-compatible",
        "apiKey": "sk-theirs",
        "baseUrl": "https://api.deepseek.com/v1/"
      }
    },
    "anthropic": { "settings": { "provider": "anthropic", "apiKey": "sk-ant" } }
  }
}"#;
        let p = read_cline_current(content).expect("the selected entry");
        assert_eq!(
            p.base_url, "https://api.deepseek.com",
            "the trailing slash and the version root both go: the first is not part \
             of the host, and the second is Cline's convention rather than the \
             stored provider's — the gateway appends the protocol path itself"
        );
        assert_eq!(p.api_key, "sk-theirs");
        // The slot's *name* is a provider type, not a friendly name — the
        // caller decides what to call the import.
        assert_eq!(p.name, "openai-compatible");

        // Nothing selected, a selector naming no entry, and an entry missing
        // half of what a request needs are all "nothing to import".
        assert!(read_cline_current("{}").is_none());
        assert!(read_cline_current(r#"{"lastUsedProvider": "gone"}"#).is_none());
        assert!(read_cline_current(
            r#"{"lastUsedProvider": "a", "providers": {"a": {"settings": {"baseUrl": "https://x/v1"}}}}"#
        )
        .is_none());
    }
}
