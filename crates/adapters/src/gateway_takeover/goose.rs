//! Goose: three files that together are the custom endpoint — the selection
//! (`config.yaml`), the provider definition (`custom_providers/*.json`), and a
//! key file the provider's `auth.command` reads the placeholder key from.
//!
//! Why a key *file* at all: goose has no literal-key field in a custom
//! provider — the JSON carries `api_key_env`, a name resolved env → keyring →
//! `secrets.yaml`, and the keyring is chosen at startup as the whole store
//! (crates/goose/src/config/base.rs), so a key written to `secrets.yaml` is
//! invisible to every user whose keyring holds a goose blob. The one
//! keyring-independent, file-based source is `auth: {command, args}` — a
//! command whose stdout *is* the credential, honored by the openai engine's
//! declarative builder (`openai_def.rs` wraps it the way the Anthropic one
//! does). `/bin/cat` of a file Kiwano owns is the whole mechanism.

use serde_yaml::Value;

use crate::gateway_takeover::gateway::{GATEWAY_LABEL, GATEWAY_PROVIDER_ID, PLACEHOLDER_MODEL_ID};

// ── goose (etcetera-resolved config root, GOOSE_PATH_ROOT not honored) ──

/// The file `auth.command` reads the key from, inside the goose config root —
/// the path the provider JSON's rewrite derives (the directory above
/// `custom_providers/`) and the takeover's file list agree on.
pub const GOOSE_KEY_FILE: &str = "kiwano-gateway.key";

/// The id the selection gets when the user's `GOOSE_MODEL` had nothing to copy
/// from (ours included — a re-run replaces it with a fresh placeholder rather
/// than forwarding "kiwano-gateway" as a model name).
pub fn goose_model_id(selected: Option<&str>) -> String {
    selected
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != GATEWAY_PROVIDER_ID)
        .unwrap_or(PLACEHOLDER_MODEL_ID)
        .to_string()
}

/// Rewrite the selection: `GOOSE_PROVIDER` picks our entry, `GOOSE_MODEL`
/// keeps the id the user's requests were using (the gateway forwards it
/// verbatim). Comments do not survive serde_yaml's re-serialization; restore
/// puts the user's bytes back the way they were.
pub fn upsert_goose_config(content: &str, model_id: &str) -> Result<String, String> {
    let yaml: Value = if content.trim().is_empty() {
        Value::Mapping(Default::default())
    } else {
        serde_yaml::from_str(content).map_err(|e| format!("config.yaml is not valid YAML: {e}"))?
    };
    let mut root = yaml
        .as_mapping()
        .cloned()
        .ok_or("config.yaml root must be a YAML mapping")?;

    let insert = |root: &mut serde_yaml::Mapping, key: &str, value: &str| {
        root.insert(Value::String(key.into()), Value::String(value.into()));
    };
    insert(&mut root, "GOOSE_PROVIDER", GATEWAY_PROVIDER_ID);
    insert(&mut root, "GOOSE_MODEL", model_id);

    serde_yaml::to_string(&Value::Mapping(root)).map_err(|e| e.to_string())
}

/// The id the selection names today — what the takeover is rerouting.
pub fn read_goose_selected_model(content: &str) -> Option<String> {
    let yaml: Value = serde_yaml::from_str(content).ok()?;
    yaml.get("GOOSE_MODEL")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty() && *s != GATEWAY_PROVIDER_ID)
        .map(str::to_string)
}

/// Build the provider definition (the file is ours by name — `goose configure`
/// names custom-provider files after the provider id — so there is no user
/// document to preserve). `key_file` is the absolute path `auth.command` cats.
pub fn build_goose_provider_json(
    base_url: &str,
    key_file: &str,
    model_id: &str,
) -> Result<String, String> {
    serde_json::to_string_pretty(&serde_json::json!({
        "name": GATEWAY_PROVIDER_ID,
        "engine": "openai",
        "display_name": GATEWAY_LABEL,
        "description": "Custom openai provider managed by Kiwano",
        // Empty: `auth` is the credential source, and the two are mutually
        // exclusive (DeclarativeProviderConfig::validate_auth).
        "api_key_env": "",
        "auth": {
            // Executed directly, never through a shell (the struct's own
            // contract); the output is trimmed, so the file's trailing
            // newline is harmless.
            "command": "/bin/cat",
            "args": [key_file],
            "refresh_interval": 0
        },
        "base_url": base_url,
        "models": [{ "name": model_id, "context_limit": 200000 }],
        "requires_auth": true
    }))
    .map_err(|e| e.to_string())
}

/// The key file's content: the placeholder key, verbatim.
pub fn goose_key_file_content(key: &str) -> String {
    format!("{key}\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goose_config_selects_the_gateway_and_keeps_the_model_id() {
        let original = "# goose's own settings\nGOOSE_PROVIDER: anthropic\nGOOSE_MODEL: claude-sonnet-4-6\ntutorial_mode: false\n";
        let out = upsert_goose_config(original, "claude-sonnet-4-6").unwrap();
        let v: Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["GOOSE_PROVIDER"], "kiwano-gateway");
        assert_eq!(v["GOOSE_MODEL"], "claude-sonnet-4-6");
        assert_eq!(v["tutorial_mode"], false); // unrelated settings survive
    }

    #[test]
    fn goose_model_id_replaces_ours_with_a_placeholder() {
        assert_eq!(
            goose_model_id(Some("claude-sonnet-4-6")),
            "claude-sonnet-4-6"
        );
        // A re-run's selection ("kiwano-gateway") is not a model name.
        assert_eq!(goose_model_id(Some("kiwano-gateway")), "kiwano");
        assert_eq!(goose_model_id(Some("  ")), "kiwano");
        assert_eq!(goose_model_id(None), "kiwano");
    }

    #[test]
    fn goose_provider_json_carries_the_auth_command() {
        let json = build_goose_provider_json(
            "http://127.0.0.1:8317/v1",
            "/home/u/Library/Block/goose/kiwano-gateway.key",
            "claude-sonnet-4-6",
        )
        .unwrap();
        let v: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(v["name"], "kiwano-gateway");
        assert_eq!(v["engine"], "openai");
        assert_eq!(v["api_key_env"], "");
        assert_eq!(v["auth"]["command"], "/bin/cat");
        assert_eq!(
            v["auth"]["args"][0],
            "/home/u/Library/Block/goose/kiwano-gateway.key"
        );
        assert_eq!(v["auth"]["refresh_interval"], 0);
        assert_eq!(v["base_url"], "http://127.0.0.1:8317/v1");
        assert_eq!(v["models"][0]["name"], "claude-sonnet-4-6");
        assert_eq!(v["requires_auth"], true);
    }

    #[test]
    fn goose_reader_takes_the_selection_only() {
        assert_eq!(
            read_goose_selected_model("GOOSE_PROVIDER: anthropic\nGOOSE_MODEL: gpt-5.2\n"),
            Some("gpt-5.2".to_string())
        );
        // Pointed at us (a re-run): no user model to read.
        assert_eq!(
            read_goose_selected_model("GOOSE_MODEL: kiwano-gateway\n"),
            None
        );
        assert_eq!(read_goose_selected_model("nothing: here\n"), None);
    }
}
