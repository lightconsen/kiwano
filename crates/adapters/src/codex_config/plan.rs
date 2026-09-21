//! The live-write plan: the token injector, the plan builder and its preflight
//! wrapper.
//!
//! `plan_codex_live_write` is the one entry point a provider switch calls — it
//! runs the repairs, then the gates, then injects the key the switch carries
//! into a provider-scoped `experimental_bearer_token`. All validation happens
//! while the plan is built, so a caller can build and discard one before
//! committing state and then execute the same computation for the real write.

use crate::codex_config::extract::{
    extract_codex_api_key, extract_codex_auth_api_key, extract_codex_experimental_bearer_token,
};
use crate::codex_config::gates::{
    codex_config_falls_back_to_official_auth_for_third_party,
    codex_config_routes_third_party_without_token_slot, codex_provider_table_declares_auth,
};
use crate::codex_config::is_custom_codex_model_provider_id;
use crate::codex_config::provider_id::active_codex_model_provider_id;
use crate::codex_config::repairs::{
    backfill_codex_custom_provider_names, migrate_stale_reserved_provider_tables,
    normalize_codex_legacy_openai_reroute, preflight_codex_provider_table_conflicts,
};
use crate::error::AppError;
use serde_json::Value;
use toml_edit::DocumentMut;

/// Inject the carried API key into the active provider table as a
/// provider-scoped `experimental_bearer_token` (honored since Codex 0.48).
/// Falls back to the top level when there is no custom provider to hold it —
/// which Codex 0.149 ignores, so only the gate-approved shapes ever reach there.
// Ported from cc-switch: src-tauri/src/codex_config.rs::set_codex_experimental_bearer_token
fn set_codex_experimental_bearer_token(config_text: &str, token: &str) -> Result<String, AppError> {
    if config_text.trim().is_empty() {
        // Upstream key: provider.codex.config.missing
        return Err(AppError::Message(
            "Codex third-party provider is missing config.toml, cannot write bearer token".into(),
        ));
    }

    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;

    let Some(provider_id) = active_codex_model_provider_id(&doc) else {
        doc["experimental_bearer_token"] = toml_edit::value(token);
        return Ok(doc.to_string());
    };

    if !is_custom_codex_model_provider_id(&provider_id) {
        // Reserved Codex provider IDs are owned by the CLI. Keep third-party
        // bearer tokens at the top level so we do not shadow built-in tables.
        doc["experimental_bearer_token"] = toml_edit::value(token);
        return Ok(doc.to_string());
    }

    // `as_table_like_mut` (not `as_table_mut`): inline tables would return None
    // and silently divert the token to the top level, where Codex 0.149 has no
    // such field and ignores it (401 persists).
    if let Some(provider_table) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
        .and_then(|table| table.get_mut(provider_id.as_str()))
        .and_then(|item| item.as_table_like_mut())
    {
        if codex_provider_table_declares_auth(&*provider_table) {
            return Ok(config_text.to_string());
        }
        provider_table.insert("experimental_bearer_token", toml_edit::value(token));
        return Ok(doc.to_string());
    }

    doc["experimental_bearer_token"] = toml_edit::value(token);
    Ok(doc.to_string())
}

/// Build the live Codex config for a provider switch: run the repairs, then
/// inject the key the switch carries.
///
/// This is the single normalize→inject entry point for the gate-less callers
/// (a rebuild during restore), so a pre-0.149 `openai_base_url` shape can never
/// leave its key in a top-level field Codex ignores while auth.json credentials
/// stay live. Idempotent on already-normalized text.
// Ported from cc-switch: src-tauri/src/codex_config.rs::prepare_codex_provider_live_config
pub fn prepare_codex_provider_live_config(
    auth: &Value,
    config_text: &str,
) -> Result<String, AppError> {
    let token = extract_codex_auth_api_key(auth)
        .or_else(|| extract_codex_experimental_bearer_token(config_text));

    // Unconditional: a stale reserved table makes Codex refuse the whole config
    // (0.148+), token or not. Third-party context — the route may follow the
    // renamed table when it can authenticate (see the migrator).
    let migrated = migrate_stale_reserved_provider_tables(config_text, false, token.is_some())?;
    let config_text = migrated.as_deref().unwrap_or(config_text);

    // Also unconditional (covers the keyless third-party path): 0.149 rejects
    // the whole config over any name-less custom table, active or not.
    let named = backfill_codex_custom_provider_names(config_text)?;
    let config_text = named.as_deref().unwrap_or(config_text);

    let Some(token) = token else {
        return Ok(config_text.to_string());
    };
    let normalized = normalize_codex_legacy_openai_reroute(config_text)?;
    let config_text = normalized.as_deref().unwrap_or(config_text);
    set_codex_experimental_bearer_token(config_text, &token)
}

/// A computed Codex live write. All validation (legacy-shape normalization,
/// safety gates, token injection, TOML parsing) happens while building the plan,
/// so callers can preflight a switch — build and discard — before committing any
/// state, then execute the same computation for the real write. Keeping
/// validation and execution in one builder makes it impossible for the two to
/// drift apart.
#[derive(Debug)]
pub struct CodexLiveWritePlan {
    pub write_full_auth: bool,
    pub config_text: Option<String>,
    pub remove_auth_file: bool,
}

/// Route a Codex live write between full auth+config or config-only.
///
/// Third-party providers only touch `config.toml`: since Codex 0.149 custom
/// providers no longer inherit ambient auth from auth.json, so the API key
/// travels as a provider-scoped `experimental_bearer_token` (honored since
/// Codex 0.48). auth.json is reserved for the official ChatGPT login, kept when
/// the preservation setting is on and deleted otherwise. It never carries
/// third-party keys, so a `requires_openai_auth = true` fallback has no
/// third-party credential to mis-send and pre-0.48 auth.json-only Codex releases
/// are the only casualty.
///
/// Kiwano has no official-card path (no managed OAuth) and no session-history
/// settings hook, so upstream's `category` branch and its unified-session
/// injection are not carried: every call here is upstream's `official = false`
/// third-party branch.
// Ported from cc-switch: src-tauri/src/codex_config.rs::plan_codex_live_write
pub fn plan_codex_live_write(
    auth: &Value,
    config_text: Option<&str>,
    preserve_official_login: bool,
) -> Result<CodexLiveWritePlan, AppError> {
    // Semantic preflight over EVERY provider table (active and idle alike):
    // field combinations 0.149 rejects at load can't be normalized away, so
    // refuse the switch with an actionable error instead of writing a config
    // Codex won't start on. Independent of the two auth-safety gates below —
    // those only judge the active route and are skipped when a key is carried.
    if let Some(text) = config_text {
        preflight_codex_provider_table_conflicts(text)?;
    }

    // The key may live in auth.OPENAI_API_KEY or already sit in the config text
    // (e.g. `auth = {}` raw-edited providers) — mirror
    // prepare_codex_provider_live_config's token sources.
    let carried_key = extract_codex_api_key(Some(auth), config_text);

    // Stale reserved tables are migrated BEFORE the safety gates so the gates
    // judge the same text prepare will write (a mixed stale-table +
    // openai_base_url shape would otherwise be mis-refused). prepare migrates
    // again internally (idempotent) for the gate-less paths.
    let migrated = match config_text {
        Some(text) => migrate_stale_reserved_provider_tables(text, false, carried_key.is_some())?,
        None => None,
    };
    let config_text = migrated.as_deref().or(config_text);

    // The legacy reroute shape (built-in `openai` provider + top-level
    // `openai_base_url`) has no provider table to carry the key — rewrite it
    // into a provider table of ours before the safety gates run. prepare
    // normalizes again internally (idempotent); the gates need the normalized
    // text here.
    let normalized = match config_text {
        Some(text) if carried_key.is_some() => normalize_codex_legacy_openai_reroute(text)?,
        _ => None,
    };
    let config_text = normalized.as_deref().or(config_text);

    // The preservation setting decides whether the official login in auth.json
    // survives a third-party switch. Off means the file is deleted — a lingering
    // login next to a third-party route is the leak shape the gates exist to
    // prevent, and `{}` is not logout, the file must go.
    let remove_auth_file = !preserve_official_login;

    let live_config = match config_text {
        Some(text) if !text.trim().is_empty() => {
            // Both safety gates protect the same invariant: the auth Codex
            // resolves for a third-party route must never come from auth.json
            // (official OAuth under preservation, nothing at all otherwise —
            // either way the switch would be broken or unsafe).
            if carried_key.is_some() && codex_config_routes_third_party_without_token_slot(text) {
                // Upstream key: provider.codex.config.no_custom_provider
                return Err(AppError::Message(
                    "A Codex third-party config must define a custom model_providers entry to carry the API key (Codex ignores a top-level experimental_bearer_token)".into(),
                ));
            }
            if carried_key.is_none()
                && codex_config_falls_back_to_official_auth_for_third_party(text)
            {
                // Upstream key: provider.codex.config.official_auth_fallback
                return Err(AppError::Message(
                    "This Codex config has no usable API key, and requires_openai_auth = true (or a top-level openai_base_url) would make Codex fall back to whatever login auth.json holds for a third-party route. Add an API key to the provider or remove the fallback directive".into(),
                ));
            }
            prepare_codex_provider_live_config(auth, text)?
        }
        // Empty config: with a key to carry this errs inside
        // set_codex_experimental_bearer_token (no table to attach it to);
        // without a key the empty config is passed through as-is.
        other => prepare_codex_provider_live_config(auth, other.unwrap_or(""))?,
    };

    Ok(CodexLiveWritePlan {
        write_full_auth: false,
        config_text: Some(live_config),
        remove_auth_file,
    })
}

/// Validate a Codex live write without touching the filesystem. Callers use
/// this to fail a provider switch BEFORE committing any state: a write-layer
/// refusal after the caller moved on would leave the app describing a switch
/// that never happened.
// Ported from cc-switch: src-tauri/src/codex_config.rs::preflight_codex_live_write
pub fn preflight_codex_live_write(
    auth: &Value,
    config_text: Option<&str>,
    preserve_official_login: bool,
) -> Result<(), AppError> {
    plan_codex_live_write(auth, config_text, preserve_official_login).map(|_| ())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_config::test_support::{healthy_third_party_config, provider_table};

    // ---- the plan itself ---------------------------------------------------

    #[test]
    fn plan_injects_the_key_into_the_active_table_and_routes_auth_policy() {
        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-carried" });
        let plan = plan_codex_live_write(&auth, Some(&healthy_third_party_config()), true).unwrap();
        let text = plan.config_text.expect("a plan always carries config text");
        assert_eq!(
            provider_table(&text, "custom-1")
                .get("experimental_bearer_token")
                .and_then(|v| v.as_str()),
            Some("sk-carried")
        );
        // Third-party switches are config-only; auth.json survives only under
        // the preservation setting.
        assert!(!plan.write_full_auth);
        assert!(!plan.remove_auth_file);
        assert!(
            plan_codex_live_write(&auth, Some(&healthy_third_party_config()), false)
                .unwrap()
                .remove_auth_file
        );

        // preflight = plan-and-discard, same verdicts.
        assert!(
            preflight_codex_live_write(&auth, Some(&healthy_third_party_config()), true).is_ok()
        );
        assert!(
            preflight_codex_live_write(&auth, Some("model_provider = \"gone\"\n"), true).is_err()
        );
    }
}
