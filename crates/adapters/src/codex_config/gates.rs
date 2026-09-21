//! Live-write gates (ports of cc-switch's pre-write validation).
//!
//! Predicates over a config *text*: whether the provider table a route selects
//! would resolve its auth from `auth.json` rather than from a key we inject,
//! and whether a carried key would have any table to land in. Nothing here
//! touches the filesystem — a caller judges a plan and writes only what the
//! plan approved.

use crate::codex_config::is_custom_codex_model_provider_id;
use crate::codex_config::provider_id::active_codex_model_provider_id;
use toml_edit::DocumentMut;

/// Whether a provider's `http_headers` / `env_http_headers` table carries an
/// `Authorization` entry. Header names are case-insensitive on the wire, so
/// match TOML keys case-insensitively too.
// Ported from cc-switch: src-tauri/src/codex_config.rs::table_declares_authorization_header
fn table_declares_authorization_header(item: Option<&toml_edit::Item>) -> bool {
    item.and_then(|item| item.as_table_like())
        .is_some_and(|table| {
            table
                .iter()
                .any(|(key, _)| key.eq_ignore_ascii_case("authorization"))
        })
}

/// Whether this provider table resolves its auth from `auth.json` on Codex
/// 0.149. `resolve_provider_auth` short-circuits on `env_key` /
/// `experimental_bearer_token`; with neither, `requires_openai_auth = false`
/// resolves to the unauthenticated provider — it never reads `auth.json`, no
/// matter what the table carries (x-api-key headers, query params, or nothing
/// at all for local servers). Only `requires_openai_auth = true` without a
/// short-circuit falls through to the official login.
///
/// `auth` / `aws` are deliberately NOT short-circuits here: 0.149 validates
/// both as mutually exclusive with `requires_openai_auth` (and `aws` is
/// Bedrock-only anyway), so a `requires_openai_auth = true` table carrying them
/// is a dead config the whole file fails to load with. Treating them as "own
/// credentials" would wave that dead config through the safety gate; flagging
/// it keeps it from being written.
// Ported from cc-switch: src-tauri/src/codex_config.rs::codex_provider_table_falls_back_to_official_auth
pub(crate) fn codex_provider_table_falls_back_to_official_auth(
    table: &dyn toml_edit::TableLike,
) -> bool {
    table
        .get("requires_openai_auth")
        .and_then(|item| item.as_bool())
        .unwrap_or(false)
        && table.get("env_key").is_none()
        && table.get("experimental_bearer_token").is_none()
}

/// Codex 0.149 guard: a provider table that already declares its own credential
/// source must not receive an injected bearer token. `auth` / `aws` sub-tables
/// hard-conflict with `experimental_bearer_token` at deserialization — the whole
/// config.toml fails to parse and Codex refuses to start. `env_key` outranks the
/// token at runtime, so injection buys nothing and only leaks the key into
/// config.toml. An explicit `Authorization` in `http_headers` /
/// `env_http_headers` is how header-auth providers survive on 0.149 — auth is
/// applied after provider headers and would overwrite it.
///
/// `requires_openai_auth` is deliberately NOT part of this guard, and it even
/// disables the header check: without an injected token,
/// `requires_openai_auth = true` routes auth to the preserved `auth.json` OAuth
/// login, which is applied after provider headers and would send the official
/// credentials to the third-party endpoint. The injected token short-circuits
/// that; a contradictory Authorization header loses either way on 0.149.
// Ported from cc-switch: src-tauri/src/codex_config.rs::codex_provider_table_declares_auth
pub(crate) fn codex_provider_table_declares_auth(table: &dyn toml_edit::TableLike) -> bool {
    let requires_openai_auth = table
        .get("requires_openai_auth")
        .and_then(|item| item.as_bool())
        .unwrap_or(false);
    table.get("auth").is_some()
        || table.get("aws").is_some()
        || table.get("env_key").is_some()
        || (!requires_openai_auth
            && (table_declares_authorization_header(table.get("http_headers"))
                || table_declares_authorization_header(table.get("env_http_headers"))))
}

/// Whether a config routes requests away from the official provider while
/// offering no custom provider table to carry a bearer token: a custom
/// `model_provider` whose table is missing, or a built-in/unset provider
/// rerouted by a top-level `openai_base_url`. In both shapes the token can only
/// land at the top level, which Codex 0.149 ignores — on a config-only switch
/// the preserved `auth.json` credentials would be sent to the third-party
/// endpoint. Configs without any routing directive are fine: they leave Codex on
/// the official provider, and the top-level token is our own record
/// (extract/backfill), never read by Codex.
// Ported from cc-switch: src-tauri/src/codex_config.rs::codex_config_routes_third_party_without_token_slot
pub(crate) fn codex_config_routes_third_party_without_token_slot(config_text: &str) -> bool {
    let Ok(doc) = config_text.parse::<DocumentMut>() else {
        // Syntactically invalid TOML is rejected later by the write validators.
        return false;
    };
    match active_codex_model_provider_id(&doc) {
        Some(id) if is_custom_codex_model_provider_id(&id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get(&id))
            .and_then(|item| item.as_table_like())
            .is_none(),
        _ => doc
            .get("openai_base_url")
            .and_then(|item| item.as_str())
            .map(str::trim)
            .is_some_and(|url| !url.is_empty()),
    }
}

/// Whether a config with NO injectable API key still routes third-party traffic
/// through the `auth.json` fallback. On 0.149 a custom provider with
/// `requires_openai_auth = true` and no `env_key` /
/// `experimental_bearer_token` short-circuit resolves to whatever `auth.json`
/// holds — under login preservation that is the official OAuth login, applied
/// after provider headers, so even an explicit `http_headers.Authorization` is
/// overwritten and the ChatGPT access token + account id go to the third-party
/// endpoint. A top-level `openai_base_url` reroutes the built-in `openai`
/// provider the same way (other built-ins never read the OAuth login). With a
/// token present the injected bearer short-circuits the fallback instead, so
/// this predicate only matters on the no-token path.
// Ported from cc-switch: src-tauri/src/codex_config.rs::codex_config_falls_back_to_official_auth_for_third_party
pub(crate) fn codex_config_falls_back_to_official_auth_for_third_party(config_text: &str) -> bool {
    let Ok(doc) = config_text.parse::<DocumentMut>() else {
        // Syntactically invalid TOML is rejected later by the write validators.
        return false;
    };
    let openai_base_url_reroutes = || {
        doc.get("openai_base_url")
            .and_then(|item| item.as_str())
            .map(str::trim)
            .is_some_and(|url| !url.is_empty())
    };
    match active_codex_model_provider_id(&doc) {
        Some(id) if is_custom_codex_model_provider_id(&id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get(&id))
            .and_then(|item| item.as_table_like())
            .is_some_and(codex_provider_table_falls_back_to_official_auth),
        Some(id) if id == "openai" => openai_base_url_reroutes(),
        None => openai_base_url_reroutes(),
        // Other reserved built-ins (ollama, lmstudio, bedrock…) have their own
        // auth paths and never fall back to the OAuth login.
        Some(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use crate::codex_config::plan::plan_codex_live_write;
    use crate::codex_config::test_support::healthy_third_party_config;

    // ---- gate: a carried key with no provider table to carry it ------------

    #[test]
    fn no_token_slot_gate_refuses_a_key_without_a_provider_table() {
        // Custom route whose table is missing: the key can only land at the top
        // level, which Codex 0.149 ignores.
        let config = "model_provider = \"custom-1\"\nmodel = \"gpt-5.1\"\n";
        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-carried" });
        let err = plan_codex_live_write(&auth, Some(config), true).unwrap_err();
        assert!(
            err.to_string().contains("custom model_providers entry"),
            "unexpected error: {err}"
        );

        // Same reason through the top-level reroute shape with no table at all.
        let rerouted =
            "model_provider = \"openai\"\nopenai_base_url = \"https://api.example.com/v1\"\n";
        assert!(plan_codex_live_write(&auth, Some(rerouted), true).is_ok());

        // A healthy config must not trip it.
        assert!(plan_codex_live_write(&auth, Some(&healthy_third_party_config()), true).is_ok());
    }

    // ---- gate: no key carried, official auth fallback ----------------------

    #[test]
    fn official_auth_fallback_gate_refuses_a_keyless_third_party_route() {
        let keyless = r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
requires_openai_auth = true
"#;
        let err = plan_codex_live_write(&serde_json::json!({}), Some(keyless), true).unwrap_err();
        assert!(
            err.to_string().contains("requires_openai_auth"),
            "unexpected error: {err}"
        );

        // A key short-circuits the fallback, so the same shape passes.
        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-carried" });
        assert!(plan_codex_live_write(&auth, Some(keyless), true).is_ok());

        // Without the flag the provider resolves unauthenticated, not official.
        let no_fallback = keyless.replace("requires_openai_auth = true\n", "");
        assert!(plan_codex_live_write(&serde_json::json!({}), Some(&no_fallback), true).is_ok());
    }

    #[test]
    fn official_auth_fallback_gate_also_covers_the_top_level_reroute() {
        let rerouted = "openai_base_url = \"https://api.example.com/v1\"\n";
        let err = plan_codex_live_write(&serde_json::json!({}), Some(rerouted), true).unwrap_err();
        assert!(
            err.to_string().contains("openai_base_url"),
            "unexpected error: {err}"
        );
    }
}
