//! Kiwano scaffolding: the gateway takeover transforms.
//!
//! A takeover rewrites the route itself rather than moving a credential, so
//! these pieces are composed in an order that keeps the gates honest — see
//! `plan_codex_takeover_live_write`. The module also owns the placeholder-key
//! vocabulary (`GATEWAY_PLACEHOLDER_PREFIX`, `codex_config_placeholder_key`),
//! because it is what reads a Codex config text back for the takeover state
//! reader, and the last-tier route removal `remove_codex_gateway_route`.

use crate::codex_config::is_custom_codex_model_provider_id;
use crate::codex_config::plan::{
    plan_codex_live_write, prepare_codex_provider_live_config, CodexLiveWritePlan,
};
use crate::codex_config::provider_id::active_codex_model_provider_id;
use crate::error::AppError;
use toml_edit::DocumentMut;

/// Prefix of the placeholder keys a takeover injects; the gateway uses them to
/// tell which agent (and which placeholder) is calling. Recognition lives here
/// because this module is what reads a Codex config text back.
pub const GATEWAY_PLACEHOLDER_PREFIX: &str = "kw-ag-";

/// The placeholder key a Codex config text carries, if any.
///
/// The takeover state reader needs to recognize its own route in a live file;
/// that cannot be an exact lookup in the `placeholder_keys` table, because the
/// table row and the file can disagree (a rolled-back registration over a
/// rewritten config, or a `~/.codex` restored by hand). Values are read from
/// the same slots the injector writes — the active custom table's
/// `experimental_bearer_token` and the top-level one — and, when the text no
/// longer parses, from a raw scan so a hand-broken config still reports the
/// route it holds.
pub fn codex_config_placeholder_key(config_text: &str) -> Option<String> {
    let as_placeholder = |raw: &str| {
        let trimmed = raw.trim();
        trimmed
            .starts_with(GATEWAY_PLACEHOLDER_PREFIX)
            .then(|| trimmed.to_string())
    };

    if let Ok(doc) = config_text.parse::<DocumentMut>() {
        let active = active_codex_model_provider_id(&doc)
            .filter(|id| is_custom_codex_model_provider_id(id))
            .and_then(|id| {
                doc.get("model_providers")
                    .and_then(|item| item.as_table_like())
                    .and_then(|table| table.get(&id))
                    .and_then(|item| item.as_table_like())
                    .and_then(|table| table.get("experimental_bearer_token"))
                    .and_then(|item| item.as_str())
                    .and_then(as_placeholder)
            })
            .or_else(|| {
                doc.get("experimental_bearer_token")
                    .and_then(|item| item.as_str())
                    .and_then(as_placeholder)
            });
        if active.is_some() {
            return active;
        }
    }

    // Raw fallback: a value shaped like one of ours, quoted or bare.
    for line in config_text.lines() {
        let Some((_, value)) = line.split_once("experimental_bearer_token") else {
            continue;
        };
        if let Some(key) = value
            .split(['"', '\'', ' ', '\t'])
            .find(|chunk| chunk.starts_with(GATEWAY_PLACEHOLDER_PREFIX))
        {
            return Some(key.to_string());
        }
    }
    None
}

/// Point the active custom provider table's `base_url` at the local gateway.
///
/// This is the one value a takeover exists to change, and it deliberately does
/// not run before the gates: no gate predicate reads a `base_url` value (they
/// read the presence of a `model_providers` table, of `openai_base_url`, and of
/// credential fields), so pointing after the gates cannot change a verdict. A
/// config with no custom provider table to point at is refused — Codex would
/// have nowhere to send the request.
fn point_codex_active_provider_at(config_text: &str, base_url: &str) -> Result<String, AppError> {
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;
    let provider_id = active_codex_model_provider_id(&doc)
        .filter(|id| is_custom_codex_model_provider_id(id))
        .ok_or_else(|| {
            AppError::Message(
                "config.toml has no custom [model_providers.<id>] entry for the gateway to route — configure a custom provider for Codex before takeover".into(),
            )
        })?;
    let Some(table) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|table| table.get_mut(provider_id.as_str()))
        .and_then(|item| item.as_table_like_mut())
    else {
        return Err(AppError::Message(format!(
            "config.toml routes to `model_provider = \"{provider_id}\"` but defines no [model_providers.{provider_id}] table to carry the route — configure a custom provider for Codex before takeover"
        )));
    };
    table.insert("base_url", toml_edit::value(base_url));
    Ok(doc.to_string())
}

/// Whether a URL points at a loopback endpoint — the shape a takeover writes.
/// Deliberately narrow (the two spellings a takeover emits) so a user's own
/// local server on another port is not mistaken for our route.
pub fn is_loopback_gateway_url(url: &str) -> bool {
    let lower = url.trim().to_ascii_lowercase();
    lower.starts_with("http://127.0.0.1:") || lower.starts_with("http://localhost:")
}

/// Build the taken-over live Codex config: repairs → gates → token → gateway.
///
/// Not a port. Upstream's `plan_codex_live_write` plans a *direct* switch, where
/// the config already names the upstream and only the credential has to travel.
/// A takeover rewrites the route itself, so the pieces are composed in an order
/// that keeps the gates honest: the repairs run first (so the route the gateway
/// takes over is the migrated one), the gates then judge that text, the
/// carried placeholder key is injected into it, and only the `base_url` value —
/// which no gate reads — is pointed at the gateway last. The caller writes
/// exactly `plan.config_text`.
pub fn plan_codex_takeover_live_write(
    config_text: &str,
    placeholder_key: &str,
    gateway_base_url: &str,
) -> Result<CodexLiveWritePlan, AppError> {
    // The takeover carries the placeholder key, so `carried_key.is_some()`
    // holds for the whole plan: the reroute repair applies and the no-key gate
    // cannot fire (the injected token short-circuits the auth.json fallback).
    let carried = serde_json::json!({ "OPENAI_API_KEY": placeholder_key });
    // preserve_official_login = true: a takeover never deletes auth.json — the
    // file is part of the write (pre-0.48 Codex reads the key from there) and
    // part of the escape hatch's backup.
    let plan = plan_codex_live_write(&carried, Some(config_text), true)?;

    let text = plan.config_text.unwrap_or_default();
    let pointed = point_codex_active_provider_at(&text, gateway_base_url)?;

    // The key has to have landed inside a provider table. When the active table
    // declares its own credential source the injector deliberately leaves the
    // config alone (upstream's rule for real upstreams), which here would leave
    // the gateway unable to tell which agent is calling — refuse instead of
    // writing a route that 401s.
    if codex_config_placeholder_key(&pointed).as_deref() != Some(placeholder_key) {
        return Err(AppError::Message(format!(
            "config.toml's active provider already declares its own credentials, so the gateway key cannot be attached to it — remove `env_key` / `auth` / a hard-coded `Authorization` header from [model_providers.*], or point that provider at the gateway through a provider card first. Key: {placeholder_key}"
        )));
    }

    Ok(CodexLiveWritePlan {
        write_full_auth: plan.write_full_auth,
        config_text: Some(pointed),
        remove_auth_file: plan.remove_auth_file,
    })
}

/// Rebuild a taken-over Codex config from the provider the gateway routes for
/// the agent: replace the placeholder credential with the provider's real key
/// and point the active table at the provider's own endpoint.
///
/// This is the middle tier of restore's degradation chain, used when the
/// takeover backup is gone or unusable. The repairs run through the gate-less
/// `prepare_codex_provider_live_config` — a rebuild must not fail closed on the
/// user's own config, and it is not writing the placeholder route the gates
/// exist to protect.
pub fn rebuild_codex_live_from_provider(
    config_text: &str,
    base_url: &str,
    api_key: &str,
) -> Result<String, AppError> {
    let auth = serde_json::json!({ "OPENAI_API_KEY": api_key });
    let prepared = prepare_codex_provider_live_config(&auth, config_text)?;
    point_codex_active_provider_at(&prepared, base_url)
}

/// Remove the gateway route a takeover wrote: the loopback `base_url`, the
/// placeholder bearer token, and — when nothing but our own backfilled fields
/// is left — the provider table and its `model_provider` selection, so Codex
/// falls back to its built-in provider instead of a table with no endpoint.
///
/// Last resort of restore's degradation chain. Returns None when there was no
/// route to remove.
pub fn remove_codex_gateway_route(config_text: &str) -> Result<Option<String>, AppError> {
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;
    let mut changed = false;

    let route_provider = active_codex_model_provider_id(&doc)
        .filter(|id| is_custom_codex_model_provider_id(id))
        .filter(|id| {
            doc.get("model_providers")
                .and_then(|item| item.as_table_like())
                .and_then(|table| table.get(id))
                .and_then(|item| item.as_table_like())
                .is_some_and(|table| {
                    table
                        .get("base_url")
                        .and_then(|item| item.as_str())
                        .is_some_and(is_loopback_gateway_url)
                })
        });

    if let Some(id) = route_provider {
        let emptied = {
            let Some(table) = doc
                .get_mut("model_providers")
                .and_then(|item| item.as_table_like_mut())
                .and_then(|table| table.get_mut(id.as_str()))
                .and_then(|item| item.as_table_like_mut())
            else {
                return Ok(None);
            };
            table.remove("base_url");
            let token_is_ours = table
                .get("experimental_bearer_token")
                .and_then(|item| item.as_str())
                .is_some_and(|token| token.trim().starts_with(GATEWAY_PLACEHOLDER_PREFIX));
            if token_is_ours {
                table.remove("experimental_bearer_token");
            }
            // Only fields we or the injector wrote are left: the table is a
            // placeholder route with no endpoint, so drop it and the selection
            // that points at it rather than leave Codex on a broken entry.
            table.is_empty()
                || table
                    .iter()
                    .all(|(key, _)| matches!(key, "name" | "wire_api" | "requires_openai_auth"))
        };
        if emptied {
            if let Some(providers) = doc
                .get_mut("model_providers")
                .and_then(|item| item.as_table_like_mut())
            {
                providers.remove(id.as_str());
            }
            doc.as_table_mut().remove("model_provider");
        }
        changed = true;
    }

    if doc
        .get("openai_base_url")
        .and_then(|item| item.as_str())
        .is_some_and(is_loopback_gateway_url)
    {
        doc.as_table_mut().remove("openai_base_url");
        changed = true;
    }
    let top_level_token_is_ours = doc
        .get("experimental_bearer_token")
        .and_then(|item| item.as_str())
        .is_some_and(|token| token.trim().starts_with(GATEWAY_PLACEHOLDER_PREFIX));
    if top_level_token_is_ours {
        doc.as_table_mut().remove("experimental_bearer_token");
        changed = true;
    }

    Ok(changed.then(|| doc.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_config::test_support::{
        healthy_third_party_config, provider_table, valid_config_text,
    };

    // ---- Kiwano: takeover planning -----------------------------------------

    #[test]
    fn takeover_plan_points_the_route_at_the_gateway_and_injects_the_placeholder() {
        let plan = plan_codex_takeover_live_write(
            &healthy_third_party_config(),
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap();
        let text = plan.config_text.expect("takeover plan carries config text");
        let table = provider_table(&text, "custom-1");
        assert_eq!(
            table.get("base_url").and_then(|v| v.as_str()),
            Some("http://127.0.0.1:8317/v1")
        );
        assert_eq!(
            table
                .get("experimental_bearer_token")
                .and_then(|v| v.as_str()),
            Some("kw-ag-codex-abcd")
        );
        // Everything the config already said survives.
        assert_eq!(table.get("name").and_then(|v| v.as_str()), Some("Custom"));
        assert_eq!(
            table.get("wire_api").and_then(|v| v.as_str()),
            Some("responses")
        );

        // A legacy reroute shape is migrated first, then pointed at the gateway.
        let legacy = "model = \"gpt-5.1\"\nopenai_base_url = \"https://api.example.com/v1\"\n";
        let plan =
            plan_codex_takeover_live_write(legacy, "kw-ag-codex-abcd", "http://127.0.0.1:8317/v1")
                .unwrap();
        let migrated = plan.config_text.unwrap();
        assert!(migrated.contains("cc-switch"));
        assert!(migrated.contains("http://127.0.0.1:8317/v1"));
        assert!(!migrated.contains("openai_base_url"));
    }

    #[test]
    fn takeover_plan_refuses_configs_it_cannot_route() {
        // No custom provider table at all: nothing to point at the gateway.
        let err = plan_codex_takeover_live_write(
            "model = \"gpt-5.1\"\n",
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("no custom [model_providers"),
            "{err}"
        );

        // A table that declares its own credential source cannot also carry the
        // gateway key — upstream's injector stands down, so the takeover must.
        let declares_auth = r#"model_provider = "custom-1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
env_key = "CUSTOM_KEY"
"#;
        let err = plan_codex_takeover_live_write(
            declares_auth,
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap_err();
        assert!(
            err.to_string().contains("declares its own credentials"),
            "{err}"
        );

        // A provider-table conflict still fails the switch, not just the write.
        let conflicted = r#"model_provider = "custom-1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
aws = { region = "us-east-1" }
"#;
        assert!(plan_codex_takeover_live_write(
            conflicted,
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1"
        )
        .is_err());
    }

    #[test]
    fn rebuild_from_provider_replaces_the_placeholder_route() {
        let taken_over = plan_codex_takeover_live_write(
            &healthy_third_party_config(),
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap()
        .config_text
        .unwrap();

        let rebuilt =
            rebuild_codex_live_from_provider(&taken_over, "https://api.deepseek.com/v1", "sk-real")
                .unwrap();
        let table = provider_table(&rebuilt, "custom-1");
        assert_eq!(
            table.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.deepseek.com/v1")
        );
        assert_eq!(
            table
                .get("experimental_bearer_token")
                .and_then(|v| v.as_str()),
            Some("sk-real")
        );
        assert_eq!(codex_config_placeholder_key(&rebuilt), None);
    }

    // ---- Kiwano: the placeholder detector ----------------------------------

    #[test]
    fn placeholder_detector_finds_our_key_in_every_shape() {
        // In the active provider table.
        let taken_over = plan_codex_takeover_live_write(
            &healthy_third_party_config(),
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap()
        .config_text
        .unwrap();
        assert_eq!(
            codex_config_placeholder_key(&taken_over).as_deref(),
            Some("kw-ag-codex-abcd")
        );

        // At the top level (no provider selected).
        assert_eq!(
            codex_config_placeholder_key("experimental_bearer_token = \"kw-ag-codex-1234\"\n")
                .as_deref(),
            Some("kw-ag-codex-1234")
        );

        // A hand-broken config that no longer parses still reports its route.
        assert_eq!(
            codex_config_placeholder_key(
                "experimental_bearer_token = \"kw-ag-codex-9xyz\"\n[broken"
            )
            .as_deref(),
            Some("kw-ag-codex-9xyz")
        );

        // Someone else's credential, or no credential at all, is not ours.
        assert_eq!(codex_config_placeholder_key(valid_config_text()), None);
        let other = "model_provider = \"custom-1\"\n\n[model_providers.custom-1]\nexperimental_bearer_token = \"sk-real\"\n";
        assert_eq!(codex_config_placeholder_key(other), None);
    }

    // ---- Kiwano: last-resort route removal ---------------------------------

    #[test]
    fn gateway_route_removal_drops_the_placeholder_table_and_is_idempotent() {
        let taken_over = plan_codex_takeover_live_write(
            &healthy_third_party_config(),
            "kw-ag-codex-abcd",
            "http://127.0.0.1:8317/v1",
        )
        .unwrap()
        .config_text
        .unwrap();

        let stripped = remove_codex_gateway_route(&taken_over).unwrap().unwrap();
        let doc: toml::Value = toml::from_str(&stripped).unwrap();
        assert!(doc.get("model_provider").is_none());
        assert!(doc.get("model_providers").is_none());
        assert_eq!(codex_config_placeholder_key(&stripped), None);
        // Nothing left to remove.
        assert_eq!(remove_codex_gateway_route(&stripped).unwrap(), None);
        // A healthy config was never ours to touch.
        assert_eq!(
            remove_codex_gateway_route(&healthy_third_party_config()).unwrap(),
            None
        );
    }

    #[test]
    fn gateway_route_removal_keeps_a_table_that_still_holds_user_fields() {
        let taken_over = r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "http://127.0.0.1:8317/v1"
wire_api = "responses"
env_key = "CUSTOM_KEY"
experimental_bearer_token = "kw-ag-codex-abcd"
"#;
        let stripped = remove_codex_gateway_route(taken_over).unwrap().unwrap();
        let table = provider_table(&stripped, "custom-1");
        assert!(table.get("base_url").is_none());
        assert!(table.get("experimental_bearer_token").is_none());
        // The user-authored row survives, so the table and its selection stay.
        assert_eq!(
            table.get("env_key").and_then(|v| v.as_str()),
            Some("CUSTOM_KEY")
        );
        let doc: toml::Value = toml::from_str(&stripped).unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("custom-1")
        );
    }
}
