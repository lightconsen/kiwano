// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/codex_config.rs
// Copied on 2026-09-07. Modified for Kiwano (Tier C function-level port).
//
// Ported: the config path acquisition, the atomic live-config write, TOML
// validation, auth/base-url extraction, and the live-write gates — the planner
// `plan_codex_live_write` with its `preflight_codex_live_write` wrapper, the
// pre-write repairs (`migrate_stale_reserved_provider_tables`,
// `backfill_codex_custom_provider_names`, `normalize_codex_legacy_openai_reroute`),
// the two auth-safety gates and the provider-table conflict rejection.
//
// Deliberately NOT ported, and not an oversight: the ~6k lines of managed-OAuth
// logic and the settings-override hook. `restore_preserving_newer_same_account_auth`
// exists to protect a CLI-rotated OAuth refresh token; Kiwano hosts no OAuth and
// keeps no login generation to preserve (an API-key `auth.json` has none), so
// there is nothing for it to protect. `align_codex_requires_openai_auth_with_login_preservation`
// and the unified-session-bucket injection are login-UX and settings-hook
// features with no Kiwano caller.

//! Codex (`~/.codex`) config write core ported from cc-switch.
//!
//! Each ported function is marked with a `// Ported from cc-switch:` comment
//! naming its upstream origin. Functions not marked are Kiwano scaffolding.
//!
//! The gates run on *text*, never on disk: a caller builds a plan, compares or
//! discards it, and only writes what the plan approved. `takeover.rs` uses this
//! module for every Codex transform it performs, so the user's `~/.codex` only
//! ever receives text these gates have judged.

use serde_json::Value;
use toml_edit::DocumentMut;

use crate::config::{
    atomic_write, delete_file, get_home_dir, sanitize_provider_name, write_json_file,
    write_text_file,
};
use crate::error::AppError;

/// Reserved built-in provider IDs from OpenAI Codex's config/model-provider
/// catalog. Keep in sync with Codex `RESERVED_MODEL_PROVIDER_IDS` (0.149:
/// exactly these five; 0.148 is the same minus `amazon-bedrock-runtime`).
/// `oss` / `ollama-chat` are NOT reserved on 0.148/0.149 — both load as
/// ordinary custom tables — so listing them here would strand their bearer
/// token in the ignored top level. Mirror: providerConfigUtils.ts.
// Ported from cc-switch: src-tauri/src/codex_config.rs::CODEX_RESERVED_MODEL_PROVIDER_IDS
const CODEX_RESERVED_MODEL_PROVIDER_IDS: &[&str] = &[
    "amazon-bedrock",
    "amazon-bedrock-runtime",
    "openai",
    "ollama",
    "lmstudio",
];

/// Kiwano: stub for the removed `crate::settings::get_codex_override_dir`
/// global hook. Always `None` until the gateway wires per-call overrides.
fn get_codex_override_dir() -> Option<std::path::PathBuf> {
    None
}

/// Get the Codex config directory path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_config_dir
pub fn get_codex_config_dir() -> std::path::PathBuf {
    if let Some(custom) = get_codex_override_dir() {
        return custom;
    }

    get_home_dir().join(".codex")
}

/// Get the Codex auth.json path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_auth_path
pub fn get_codex_auth_path() -> std::path::PathBuf {
    get_codex_config_dir().join("auth.json")
}

/// Get the Codex config.toml path
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_config_path
pub fn get_codex_config_path() -> std::path::PathBuf {
    get_codex_config_dir().join("config.toml")
}

/// Get the Codex provider config file paths
// Ported from cc-switch: src-tauri/src/codex_config.rs::get_codex_provider_paths
pub fn get_codex_provider_paths(
    provider_id: &str,
    provider_name: Option<&str>,
) -> (std::path::PathBuf, std::path::PathBuf) {
    let base_name = provider_name
        .map(sanitize_provider_name)
        .unwrap_or_else(|| sanitize_provider_name(provider_id));

    let auth_path = get_codex_config_dir().join(format!("auth-{base_name}.json"));
    let config_path = get_codex_config_dir().join(format!("config-{base_name}.toml"));

    (auth_path, config_path)
}

/// Atomically write Codex's `auth.json` and `config.toml`, rolling back the
/// first step if the second fails
// Ported from cc-switch: src-tauri/src/codex_config.rs::write_codex_live_atomic
pub fn write_codex_live_atomic(
    auth: &Value,
    config_text_opt: Option<&str>,
) -> Result<(), AppError> {
    let auth_path = get_codex_auth_path();
    let config_path = get_codex_config_path();

    if let Some(parent) = auth_path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    // Read the old contents for rollback
    let old_auth = if auth_path.exists() {
        Some(std::fs::read(&auth_path).map_err(|e| AppError::io(&auth_path, e))?)
    } else {
        None
    };

    // Prepare the content to write
    let cfg_text = match config_text_opt {
        Some(s) => s.to_string(),
        None => String::new(),
    };
    if !cfg_text.trim().is_empty() {
        toml::from_str::<toml::Table>(&cfg_text).map_err(|e| AppError::toml(&config_path, e))?;
    }

    // Step 1: write auth.json
    write_json_file(&auth_path, auth)?;

    // Step 2: write config.toml (roll back auth.json on failure)
    if let Err(e) = write_text_file(&config_path, &cfg_text) {
        // Roll back auth.json
        if let Some(bytes) = old_auth {
            let _ = atomic_write(&auth_path, &bytes);
        } else {
            let _ = delete_file(&auth_path);
        }
        return Err(e);
    }

    Ok(())
}

/// Read `~/.codex/config.toml`, returning an empty string if it does not exist
// Ported from cc-switch: src-tauri/src/codex_config.rs::read_codex_config_text
pub fn read_codex_config_text() -> Result<String, AppError> {
    let path = get_codex_config_path();
    if path.exists() {
        std::fs::read_to_string(&path).map_err(|e| AppError::io(&path, e))
    } else {
        Ok(String::new())
    }
}

/// Validate the syntax of non-empty TOML text
// Ported from cc-switch: src-tauri/src/codex_config.rs::validate_config_toml
pub fn validate_config_toml(text: &str) -> Result<(), AppError> {
    if text.trim().is_empty() {
        return Ok(());
    }
    toml::from_str::<toml::Table>(text)
        .map(|_| ())
        .map_err(|e| AppError::toml(std::path::Path::new("config.toml"), e))
}

/// Read and validate `~/.codex/config.toml`, returning the text (possibly empty)
// Ported from cc-switch: src-tauri/src/codex_config.rs::read_and_validate_codex_config_text
pub fn read_and_validate_codex_config_text() -> Result<String, AppError> {
    let s = read_codex_config_text()?;
    validate_config_toml(&s)?;
    Ok(s)
}

/// Write only Codex `config.toml` for provider switching.
///
/// Codex login state lives in `auth.json`; provider routing, endpoint, model,
/// and provider-scoped bearer tokens live in `config.toml`. Provider switches
/// should not overwrite the user's ChatGPT login cache.
// Ported from cc-switch: src-tauri/src/codex_config.rs::write_codex_live_config_atomic
pub fn write_codex_live_config_atomic(config_text_opt: Option<&str>) -> Result<(), AppError> {
    let config_path = get_codex_config_path();
    let cfg_text = match config_text_opt {
        Some(config_text) => config_text.to_string(),
        None => String::new(),
    };

    if !cfg_text.trim().is_empty() {
        toml::from_str::<toml::Table>(&cfg_text).map_err(|e| AppError::toml(&config_path, e))?;
    }

    write_text_file(&config_path, &cfg_text)
}

pub fn extract_codex_auth_api_key(auth: &Value) -> Option<String> {
    auth.get("OPENAI_API_KEY")
        .and_then(|value| value.as_str())
        .map(str::trim)
        .filter(|key| !key.is_empty())
        .map(str::to_string)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_api_key
pub fn extract_codex_api_key(auth: Option<&Value>, config_text: Option<&str>) -> Option<String> {
    auth.and_then(extract_codex_auth_api_key)
        .or_else(|| config_text.and_then(extract_codex_experimental_bearer_token))
}

/// Extract the upstream base URL from a Codex `config.toml` string.
///
/// Prefers the active `[model_providers.<model_provider>].base_url`, falling
/// back to a top-level `base_url`. Deliberately never reads a non-active
/// `[model_providers.*]` section — the frontend `extractCodexBaseUrl`
/// (`getRecoverableBaseUrlAssignments`) excludes those too, and a leftover
/// section unrelated to the active provider must not leak into `{{baseUrl}}`.
// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_base_url
pub fn extract_codex_base_url(config_text: &str) -> Option<String> {
    let doc = config_text.parse::<toml::Value>().ok()?;

    if let Some(active_provider) = doc.get("model_provider").and_then(|v| v.as_str()) {
        if let Some(base_url) = doc
            .get("model_providers")
            .and_then(|providers| providers.get(active_provider))
            .and_then(|provider| provider.get("base_url"))
            .and_then(|v| v.as_str())
        {
            return Some(base_url.to_string());
        }
    }

    doc.get("base_url")
        .and_then(|v| v.as_str())
        .map(ToString::to_string)
}

fn active_codex_model_provider_id(doc: &DocumentMut) -> Option<String> {
    doc.get("model_provider")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|id| !id.is_empty())
        .map(str::to_string)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::is_custom_codex_model_provider_id
pub(crate) fn is_custom_codex_model_provider_id(id: &str) -> bool {
    // Exact match, mirroring upstream: both the built-in provider lookup and
    // validate_reserved_model_provider_ids are case-sensitive, so `OpenAI`
    // etc. are legitimate custom ids whose tables must receive the token.
    // Keep in sync with the frontend list in src/utils/providerConfigUtils.ts.
    let id = id.trim();
    !id.is_empty() && !CODEX_RESERVED_MODEL_PROVIDER_IDS.contains(&id)
}

// Ported from cc-switch: src-tauri/src/codex_config.rs::extract_codex_experimental_bearer_token
pub fn extract_codex_experimental_bearer_token(config_text: &str) -> Option<String> {
    if !config_text.contains("experimental_bearer_token") {
        return None;
    }
    let doc = config_text.parse::<DocumentMut>().ok()?;
    let provider_id = active_codex_model_provider_id(&doc);

    let top_level_token = || {
        doc.get("experimental_bearer_token")
            .and_then(|item| item.as_str())
    };
    let token = match provider_id.as_deref() {
        // `as_table_like` (not `as_table`): user configs may use inline tables
        // (`model_providers = { foo = {...} }`), which `as_table` rejects.
        Some(id) if is_custom_codex_model_provider_id(id) => doc
            .get("model_providers")
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get(id))
            .and_then(|item| item.as_table_like())
            .and_then(|table| table.get("experimental_bearer_token"))
            .and_then(|item| item.as_str())
            .or_else(top_level_token),
        Some(_) => top_level_token(),
        None => top_level_token(),
    };

    token
        .map(str::trim)
        .filter(|token| !token.is_empty())
        .map(str::to_string)
}

// ---------------------------------------------------------------------------
// Live-write gates (ports of cc-switch's pre-write validation)
// ---------------------------------------------------------------------------

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
fn codex_provider_table_falls_back_to_official_auth(table: &dyn toml_edit::TableLike) -> bool {
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
fn codex_provider_table_declares_auth(table: &dyn toml_edit::TableLike) -> bool {
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
fn codex_config_routes_third_party_without_token_slot(config_text: &str) -> bool {
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
fn codex_config_falls_back_to_official_auth_for_third_party(config_text: &str) -> bool {
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

/// The provider id our migrations own. Not a Codex reserved id, so an injected
/// token lands inside the table.
// Ported from cc-switch: src-tauri/src/codex_config.rs::CODEX_MIGRATED_PROVIDER_ID
const CODEX_MIGRATED_PROVIDER_ID: &str = "cc-switch";

/// Pick the first free id of ours (`cc-switch`, `cc-switch-2`, …) so migrations
/// never overwrite a user-authored table.
// Ported from cc-switch: src-tauri/src/codex_config.rs::first_free_cc_switch_provider_id
fn first_free_cc_switch_provider_id(model_providers: Option<&dyn toml_edit::TableLike>) -> String {
    let mut candidate = CODEX_MIGRATED_PROVIDER_ID.to_string();
    let mut suffix = 2usize;
    while model_providers.is_some_and(|table| table.get(&candidate).is_some()) {
        candidate = format!("{CODEX_MIGRATED_PROVIDER_ID}-{suffix}");
        suffix += 1;
    }
    candidate
}

/// The reserved built-in ids whose `[model_providers.<id>]` tables make Codex
/// reject the WHOLE config at load (`validate_reserved_model_provider_ids`,
/// present since 0.148, case-sensitive; the bedrock ids are exempt).
// Ported from cc-switch: src-tauri/src/codex_config.rs::CODEX_STALE_RESERVED_TABLE_IDS
const CODEX_STALE_RESERVED_TABLE_IDS: &[&str] = &["openai", "ollama", "lmstudio"];

/// Migrate stale reserved provider tables (`[model_providers.openai]`,
/// `.ollama`, `.lmstudio`). Codex rejects the WHOLE config at load when one of
/// these reserved built-in ids is overridden, so any surviving table means
/// "switch reports success, Codex refuses to start" — older takeover
/// projections created exactly these shapes.
///
/// The reserved-id match is EXACT: `OpenAI` and other case variants are
/// legitimate custom ids and must not be touched. Each table is renamed
/// losslessly to the first free id of ours (nothing proves which of its keys
/// the user cares about), with `wire_api = "responses"` defaulted in — all three
/// built-ins speak Responses on 0.149.
///
/// Route policy: when the renamed table was the active route, a third-party
/// write follows to the migrated id unless the table would resolve its auth
/// from auth.json (`codex_provider_table_falls_back_to_official_auth`) with no
/// injectable token to short-circuit it. Tables that never fall back — own
/// credentials (env_key / experimental_bearer_token), header or query-param
/// auth, or unauthenticated local servers — keep their legitimate route; only a
/// credential-less `requires_openai_auth = true` table without a token snaps
/// back to the built-in provider, because following it would send the preserved
/// OAuth login to a stale address. The renamed table is also normalized into a
/// shape 0.149 will load: `wire_api` forced to "responses" (the chat wire API
/// was removed; any other value fails deserialization of the whole config) and
/// an empty/missing `name` backfilled (rejected at load otherwise, active or
/// not). The shape never loaded since 0.148, so there is no prior behavior to
/// preserve. `official` writes never follow — an official card's route belongs
/// to the built-in provider; Kiwano has no official-card path (no managed
/// OAuth) and always passes `false`, but the policy is kept whole so the port
/// reads like upstream. Returns None when there is nothing to migrate.
// Ported from cc-switch: src-tauri/src/codex_config.rs::migrate_stale_reserved_provider_tables
fn migrate_stale_reserved_provider_tables(
    config_text: &str,
    official: bool,
    has_token: bool,
) -> Result<Option<String>, AppError> {
    if !config_text.contains("model_providers") {
        return Ok(None);
    }
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;

    let stale_ids: Vec<&str> = CODEX_STALE_RESERVED_TABLE_IDS
        .iter()
        .copied()
        .filter(|id| {
            doc.get("model_providers")
                .and_then(|item| item.as_table_like())
                .and_then(|table| table.get(id))
                .and_then(|item| item.as_table_like())
                .is_some()
        })
        .collect();
    if stale_ids.is_empty() {
        return Ok(None);
    }

    for stale_id in stale_ids {
        let migrated_id = first_free_cc_switch_provider_id(
            doc.get("model_providers")
                .and_then(|item| item.as_table_like()),
        );
        // `model_provider` unset defaults to the built-in openai provider.
        let table_is_active_route = match active_codex_model_provider_id(&doc) {
            None => stale_id == "openai",
            Some(active) => active == stale_id,
        };

        let Some(model_providers) = doc
            .get_mut("model_providers")
            .and_then(|item| item.as_table_like_mut())
        else {
            return Ok(None);
        };
        let Some(mut stale_item) = model_providers.remove(stale_id) else {
            continue;
        };
        let mut falls_back_to_official = false;
        if let Some(table) = stale_item.as_table_like_mut() {
            // 0.149 removed the chat wire API entirely: `wire_api = "chat"` (or
            // any other non-"responses" value) fails deserialization for the
            // WHOLE config, so normalize unconditionally. These tables never
            // loaded since 0.148 — there is no prior behavior to keep.
            if table.get("wire_api").and_then(|item| item.as_str()) != Some("responses") {
                table.insert("wire_api", toml_edit::value("responses"));
            }
            // Non-bedrock tables with an empty/missing `name` are rejected at
            // load ("provider name must not be empty"), active or not — the
            // legacy update path created name-less tables.
            if table
                .get("name")
                .and_then(|item| item.as_str())
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .is_none()
            {
                table.insert("name", toml_edit::value("Custom"));
            }
            falls_back_to_official = codex_provider_table_falls_back_to_official_auth(&*table);
        }
        model_providers.insert(&migrated_id, stale_item);

        // Follow the rename whenever the table cannot leak the official login:
        // an injected token short-circuits the auth.json fallback, and a table
        // that never falls back (own credentials, header/query auth, or
        // unauthenticated local servers) keeps its legitimate third-party
        // route. Only a credential-less `requires_openai_auth = true` table
        // without a token snaps back to the built-in provider — following it
        // would send the preserved OAuth login to the stale base_url.
        if table_is_active_route && !official && (has_token || !falls_back_to_official) {
            doc["model_provider"] = toml_edit::value(migrated_id.as_str());
        }
    }

    Ok(Some(doc.to_string()))
}

/// Codex 0.149 rejects the WHOLE config at deserialization when any non-Bedrock
/// provider table has an empty/missing `name` — active or not ("provider name
/// must not be empty"). Historic updates and hand-written configs created
/// tables carrying only `base_url`, so every live write normalizes custom
/// tables into a loadable shape; the name is cosmetic, so the table id is as
/// good a value as any. Bedrock tables are the opposite: 0.149 only lets them
/// override base_url/auth/http_headers/aws.*, and any other non-default field —
/// `name` included — fails the built-in merge for the whole config, so the
/// reserved ids are skipped entirely.
// Ported from cc-switch: src-tauri/src/codex_config.rs::backfill_codex_custom_provider_names
fn backfill_codex_custom_provider_names(config_text: &str) -> Result<Option<String>, AppError> {
    if !config_text.contains("model_providers") {
        return Ok(None);
    }
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;
    let Some(model_providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
    else {
        return Ok(None);
    };

    let ids: Vec<String> = model_providers
        .iter()
        .filter(|(id, item)| {
            is_custom_codex_model_provider_id(id) && item.as_table_like().is_some()
        })
        .map(|(id, _)| id.to_string())
        .collect();
    let mut changed = false;
    for id in ids {
        let Some(table) = model_providers
            .get_mut(&id)
            .and_then(toml_edit::Item::as_table_like_mut)
        else {
            continue;
        };
        if table
            .get("name")
            .and_then(|item| item.as_str())
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .is_none()
        {
            table.insert("name", toml_edit::value(id.as_str()));
            changed = true;
        }
    }
    Ok(changed.then(|| doc.to_string()))
}

/// Codex 0.149 validates EVERY provider table at deserialization — active or
/// not — and rejects the whole config over field combinations it forbids: `aws`
/// outside the two Bedrock built-ins, and a command-backed `auth` combined with
/// `requires_openai_auth` / `env_key` / `experimental_bearer_token`
/// (ModelProviderInfo::validate). None of these can be normalized away
/// (dropping user-authored fields is not ours to do), so the switch path
/// refuses up front with an actionable error instead of writing a config Codex
/// refuses to start on. Deliberately called only from plan_codex_live_write: the
/// gate-less paths (backup/rebuild) must not fail closed on the user's own
/// backup.
///
/// Upstream localizes these through `AppError::localized` under
/// `provider.codex.config.invalid_provider_table`; Kiwano has no localization
/// layer and its message strings are English only, so the English text is
/// carried as a plain `Message`.
// Ported from cc-switch: src-tauri/src/codex_config.rs::preflight_codex_provider_table_conflicts
fn preflight_codex_provider_table_conflicts(config_text: &str) -> Result<(), AppError> {
    if !config_text.contains("model_providers") {
        return Ok(());
    }
    let Ok(doc) = config_text.parse::<DocumentMut>() else {
        // Syntactically invalid TOML is rejected later by the write validators.
        return Ok(());
    };
    let Some(model_providers) = doc
        .get("model_providers")
        .and_then(|item| item.as_table_like())
    else {
        return Ok(());
    };
    for (id, item) in model_providers.iter() {
        let Some(table) = item.as_table_like() else {
            continue;
        };
        let is_bedrock = matches!(id, "amazon-bedrock" | "amazon-bedrock-runtime");
        if !is_bedrock && table.get("aws").is_some() {
            return Err(AppError::Message(format!(
                "Codex 0.149 refuses to load this config: `aws` is only supported on the built-in amazon-bedrock / amazon-bedrock-runtime providers, so [model_providers.{id}] must not carry it. Remove the field or use a Bedrock built-in id"
            )));
        }
        if table.get("auth").is_some() {
            let requires_openai_auth = table
                .get("requires_openai_auth")
                .and_then(|item| item.as_bool())
                .unwrap_or(false);
            let conflict = if requires_openai_auth {
                Some("requires_openai_auth")
            } else if table.get("env_key").is_some() {
                Some("env_key")
            } else if table.get("experimental_bearer_token").is_some() {
                Some("experimental_bearer_token")
            } else {
                None
            };
            if let Some(conflict) = conflict {
                return Err(AppError::Message(format!(
                    "Codex 0.149 refuses to load this config: `auth` on [model_providers.{id}] cannot be combined with `{conflict}`. Remove one of them"
                )));
            }
        }
    }
    Ok(())
}

/// Rewrite the legacy "reroute the built-in openai provider" shape —
/// `model_provider` unset/"openai" plus a top-level `openai_base_url` — into a
/// custom provider table named `cc-switch`. Before Codex 0.149 this shape worked
/// because the built-in provider read the third-party key from auth.json
/// (ambient auth); auth.json no longer carries third-party keys, so the key
/// needs a provider-scoped slot. The built-in `openai` provider speaks the
/// Responses wire protocol, so the table pins `wire_api = "responses"` and
/// traffic semantics stay unchanged.
// Ported from cc-switch: src-tauri/src/codex_config.rs::normalize_codex_legacy_openai_reroute
fn normalize_codex_legacy_openai_reroute(config_text: &str) -> Result<Option<String>, AppError> {
    if !config_text.contains("openai_base_url") {
        return Ok(None);
    }
    let mut doc = config_text
        .parse::<DocumentMut>()
        .map_err(|e| AppError::Message(format!("Invalid Codex config.toml: {e}")))?;

    // Exact match: `openai_base_url` reroutes only the built-in provider, and
    // the built-in lookup is case-sensitive — a config routing to `OpenAI`
    // targets a custom table, not the knob.
    let targets_built_in_openai = match active_codex_model_provider_id(&doc) {
        None => true,
        Some(id) => id == "openai",
    };
    if !targets_built_in_openai {
        return Ok(None);
    }
    let Some(base_url) = doc
        .get("openai_base_url")
        .and_then(|item| item.as_str())
        .map(str::trim)
        .filter(|url| !url.is_empty())
        .map(str::to_string)
    else {
        return Ok(None);
    };
    // `model_providers` present but not any table shape (scalar garbage): leave
    // it to the safety gates instead of guessing. Inline tables ARE handled —
    // a restore path calls prepare without the gates, so skipping them would
    // leave the key in a dead top-level field next to live auth.json
    // credentials.
    if let Some(item) = doc.get("model_providers") {
        if item.as_table_like().is_none() {
            return Ok(None);
        }
    }

    // A user-authored table may already claim our id: nothing proves it is ours
    // to overwrite (their headers/query params would be lost and later
    // backfilled into the DB for good), so pick the first free suffixed id
    // instead. Idempotency is unaffected: a normalized config routes to the
    // migrated id, so this function early-returns before reaching here.
    let migrated_id = first_free_cc_switch_provider_id(
        doc.get("model_providers")
            .and_then(|item| item.as_table_like()),
    );

    doc.as_table_mut().remove("openai_base_url");
    doc["model_provider"] = toml_edit::value(migrated_id.as_str());

    // Match the container's own style: a standard table gets a sub-table, an
    // inline `model_providers = { … }` gets an inline member.
    let container_is_inline = doc
        .get("model_providers")
        .is_some_and(|item| item.as_table().is_none());
    if doc.get("model_providers").is_none() {
        let mut table = toml_edit::Table::new();
        table.set_implicit(true);
        doc.insert("model_providers", toml_edit::Item::Table(table));
    }
    let Some(model_providers) = doc
        .get_mut("model_providers")
        .and_then(|item| item.as_table_like_mut())
    else {
        return Ok(None);
    };
    if container_is_inline {
        let mut provider_table = toml_edit::InlineTable::new();
        provider_table.insert("name", "Custom".into());
        provider_table.insert("base_url", base_url.into());
        provider_table.insert("wire_api", "responses".into());
        model_providers.insert(
            &migrated_id,
            toml_edit::Item::Value(toml_edit::Value::InlineTable(provider_table)),
        );
    } else {
        let mut provider_table = toml_edit::Table::new();
        provider_table.insert("name", toml_edit::value("Custom"));
        provider_table.insert("base_url", toml_edit::value(base_url));
        provider_table.insert("wire_api", toml_edit::value("responses"));
        model_providers.insert(&migrated_id, toml_edit::Item::Table(provider_table));
    }

    Ok(Some(doc.to_string()))
}

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

// ---------------------------------------------------------------------------
// Kiwano scaffolding: gateway takeover transforms
// ---------------------------------------------------------------------------

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
    use serial_test::serial;
    use tempfile::TempDir;

    /// Point get_home_dir()/CC_SWITCH_TEST_HOME at a temp dir for the
    /// duration of the test (env-var mutation requires serial execution).
    struct TestHomeGuard(Option<std::ffi::OsString>);

    impl TestHomeGuard {
        fn set(path: &std::path::Path) -> Self {
            let previous = std::env::var_os("CC_SWITCH_TEST_HOME");
            std::env::set_var("CC_SWITCH_TEST_HOME", path);
            Self(previous)
        }
    }

    impl Drop for TestHomeGuard {
        fn drop(&mut self) {
            match self.0.take() {
                Some(v) => std::env::set_var("CC_SWITCH_TEST_HOME", v),
                None => std::env::remove_var("CC_SWITCH_TEST_HOME"),
            }
        }
    }

    fn valid_config_text() -> &'static str {
        r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
wire_api = "responses"
"#
    }

    #[test]
    #[serial]
    fn writes_valid_config_toml_and_auth_json_on_tempdir() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-codex-test" });
        write_codex_live_atomic(&auth, Some(valid_config_text())).unwrap();

        // Both live files landed under the isolated home with valid contents.
        let config_path = get_codex_config_path();
        assert_eq!(config_path, temp.path().join(".codex").join("config.toml"));
        let config_text = std::fs::read_to_string(&config_path).unwrap();
        assert_eq!(config_text, valid_config_text());

        let auth_path = get_codex_auth_path();
        let read_back: Value = crate::config::read_json_file(&auth_path).unwrap();
        assert_eq!(read_back["OPENAI_API_KEY"], "sk-codex-test");

        // Round-trip through the readers + validators.
        let text = read_and_validate_codex_config_text().unwrap();
        assert_eq!(text, valid_config_text());
    }

    #[test]
    fn validate_config_toml_accepts_valid_and_rejects_broken() {
        validate_config_toml("").unwrap();
        validate_config_toml("   \n").unwrap();
        validate_config_toml(valid_config_text()).unwrap();

        let err = validate_config_toml("not = [valid").unwrap_err();
        assert!(matches!(err, AppError::Toml { .. }));
    }

    #[test]
    #[serial]
    fn write_codex_live_atomic_rejects_invalid_toml_without_touching_auth() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let err = write_codex_live_atomic(
            &serde_json::json!({ "OPENAI_API_KEY": "sk" }),
            Some("broken = ["),
        )
        .unwrap_err();
        assert!(matches!(err, AppError::Toml { .. }));

        // The auth file must not exist: validation fails before any write.
        assert!(!get_codex_auth_path().exists());
    }

    #[test]
    #[serial]
    fn write_codex_live_config_atomic_keeps_auth_json_intact() {
        let temp = TempDir::new().unwrap();
        let _guard = TestHomeGuard::set(temp.path());

        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-login" });
        write_codex_live_atomic(&auth, Some(valid_config_text())).unwrap();

        // Provider switch rewrites only config.toml.
        write_codex_live_config_atomic(Some("model_provider = \"openai\"\n")).unwrap();
        let config = read_codex_config_text().unwrap();
        assert_eq!(config, "model_provider = \"openai\"\n");

        let auth_back: Value = crate::config::read_json_file(&get_codex_auth_path()).unwrap();
        assert_eq!(auth_back["OPENAI_API_KEY"], "sk-login");
    }

    #[test]
    fn extract_codex_base_url_prefers_active_provider_section() {
        // Top-level base_url must precede the table headers; appended after
        // them it would land inside `[model_providers.custom-1]` as a
        // duplicate key and fail to parse.
        let text = format!(
            "base_url = \"https://top-level.example.com\"\n{}",
            valid_config_text()
        );
        assert_eq!(
            extract_codex_base_url(&text).as_deref(),
            Some("https://api.example.com/v1")
        );

        // Top-level fallback when no active provider section exists. The
        // top-level key must precede any `[table]` headers, or it would be
        // parsed as a (duplicate) key inside the last table.
        assert_eq!(
            extract_codex_base_url("base_url = \"https://top-level.example.com\"\n").as_deref(),
            Some("https://top-level.example.com")
        );
        let top_level_only =
            "base_url = \"https://top-level.example.com\"\n\n[model_providers.custom-1]\nname = \"C\"\n";
        assert_eq!(
            extract_codex_base_url(top_level_only).as_deref(),
            Some("https://top-level.example.com")
        );
        assert_eq!(extract_codex_base_url(""), None);
    }

    #[test]
    fn extract_codex_api_key_reads_auth_then_bearer_fallback() {
        let auth = serde_json::json!({ "OPENAI_API_KEY": " sk-auth " });
        assert_eq!(
            extract_codex_api_key(Some(&auth), None).as_deref(),
            Some("sk-auth")
        );

        // Bearer token inside the active custom provider table.
        let config = format!(
            "{}\n[model_providers.custom-1]\nexperimental_bearer_token = \"sk-bearer\"\n",
            "model_provider = \"custom-1\"\n"
        );
        assert_eq!(
            extract_codex_api_key(None, Some(&config)).as_deref(),
            Some("sk-bearer")
        );
        assert_eq!(extract_codex_api_key(None, Some("model = \"x\"\n")), None);
    }

    /// A healthy pre-0.149-shaped third-party config: one active custom
    /// provider table, a Responses wire API, and a name.
    fn healthy_third_party_config() -> String {
        r#"model_provider = "custom-1"
model = "gpt-5.1"

[model_providers.custom-1]
name = "Custom"
base_url = "https://api.example.com/v1"
wire_api = "responses"
"#
        .to_string()
    }

    fn provider_table(text: &str, id: &str) -> toml::Value {
        let mut doc: toml::Value = toml::from_str(text).expect("plan output parses");
        doc.get_mut("model_providers")
            .and_then(|p| p.get_mut(id))
            .map(|table| table.clone())
            .unwrap_or_else(|| panic!("[model_providers.{id}] missing from plan output"))
    }

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

    // ---- rejection: provider tables 0.149 refuses to load ------------------

    #[test]
    fn provider_table_conflicts_are_rejected_and_healthy_tables_pass() {
        let conflict = |table: &str| {
            format!("model_provider = \"custom-1\"\n\n[model_providers.custom-1]\nname = \"C\"\nbase_url = \"https://api.example.com/v1\"\n{table}")
        };
        let auth = serde_json::json!({ "OPENAI_API_KEY": "sk-carried" });

        // `aws` outside the two Bedrock built-ins.
        let err = plan_codex_live_write(
            &auth,
            Some(&conflict("aws = { region = \"us-east-1\" }\n")),
            true,
        )
        .unwrap_err();
        assert!(err.to_string().contains("`aws` is only supported"), "{err}");

        // `auth` combined with each of the three forbidden partners.
        for partner in [
            "requires_openai_auth = true",
            "env_key = \"CUSTOM_KEY\"",
            "experimental_bearer_token = \"sk-hard\"",
        ] {
            let text = conflict(&format!("auth = {{ command = \"get-cred\" }}\n{partner}\n"));
            let err = plan_codex_live_write(&auth, Some(&text), true).unwrap_err();
            assert!(
                err.to_string().contains("cannot be combined"),
                "{partner} should be refused: {err}"
            );
        }

        // Bedrock may carry `aws`; a lone `auth` table is not a conflict.
        let bedrock = "model_provider = \"amazon-bedrock\"\n\n[model_providers.amazon-bedrock]\naws = { region = \"us-east-1\" }\n";
        assert!(plan_codex_live_write(&auth, Some(bedrock), true).is_ok());
        assert!(plan_codex_live_write(
            &auth,
            Some(&conflict("auth = { command = \"get-cred\" }\n")),
            true
        )
        .is_ok());
        // The healthy config is untouched by the rejection.
        assert!(plan_codex_live_write(&auth, Some(&healthy_third_party_config()), true).is_ok());
    }

    // ---- repair: stale reserved provider tables ----------------------------

    fn stale_reserved_config() -> String {
        r#"model_provider = "openai"

[model_providers.openai]
base_url = "https://api.example.com/v1"
"#
        .to_string()
    }

    #[test]
    fn stale_reserved_tables_are_renamed_normalized_and_idempotent() {
        let migrated =
            migrate_stale_reserved_provider_tables(&stale_reserved_config(), false, true)
                .unwrap()
                .expect("a stale [model_providers.openai] table must migrate");
        let table = provider_table(&migrated, "cc-switch");
        assert_eq!(
            table.get("wire_api").and_then(|v| v.as_str()),
            Some("responses")
        );
        assert_eq!(table.get("name").and_then(|v| v.as_str()), Some("Custom"));
        assert_eq!(
            table.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.example.com/v1")
        );
        // The active route follows to the renamed id.
        let doc: toml::Value = toml::from_str(&migrated).unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("cc-switch")
        );

        // Idempotent: nothing reserved is left to migrate.
        assert_eq!(
            migrate_stale_reserved_provider_tables(&migrated, false, true).unwrap(),
            None
        );
        // And a healthy config is not touched.
        assert_eq!(
            migrate_stale_reserved_provider_tables(&healthy_third_party_config(), false, true)
                .unwrap(),
            None
        );
    }

    #[test]
    fn stale_reserved_migration_leaves_bedrock_and_case_variants_alone() {
        let bedrock = "model_provider = \"amazon-bedrock\"\n\n[model_providers.amazon-bedrock]\nregion = \"us-east-1\"\n";
        assert_eq!(
            migrate_stale_reserved_provider_tables(bedrock, false, true).unwrap(),
            None
        );

        // `OpenAI` is a legitimate custom id (the match is exact/case-sensitive).
        let cased = "model_provider = \"OpenAI\"\n\n[model_providers.OpenAI]\nname = \"X\"\nbase_url = \"https://api.example.com/v1\"\n";
        assert_eq!(
            migrate_stale_reserved_provider_tables(cased, false, true).unwrap(),
            None
        );
    }

    #[test]
    fn stale_reserved_migration_follows_the_rename_only_when_the_route_is_safe() {
        // Keyless `requires_openai_auth = true`: following would send the
        // preserved official login to the stale address, so the route snaps
        // back to the built-in provider (which no longer has a table).
        let leaky = r#"model_provider = "ollama"

[model_providers.ollama]
base_url = "https://api.example.com/v1"
requires_openai_auth = true
"#;
        let migrated = migrate_stale_reserved_provider_tables(leaky, false, false)
            .unwrap()
            .expect("stale table migrates even when the route does not follow");
        let doc: toml::Value = toml::from_str(&migrated).unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("ollama")
        );

        // With a token to short-circuit, the same shape follows the rename.
        let migrated = migrate_stale_reserved_provider_tables(leaky, false, true)
            .unwrap()
            .unwrap();
        let doc: toml::Value = toml::from_str(&migrated).unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("cc-switch")
        );

        // An official write never follows, token or not.
        let migrated = migrate_stale_reserved_provider_tables(&stale_reserved_config(), true, true)
            .unwrap()
            .unwrap();
        let doc: toml::Value = toml::from_str(&migrated).unwrap();
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("openai")
        );
    }

    // ---- repair: name backfill ---------------------------------------------

    #[test]
    fn name_backfill_rewrites_name_less_tables_and_is_idempotent() {
        let nameless = "model_provider = \"custom-1\"\n\n[model_providers.custom-1]\nbase_url = \"https://api.example.com/v1\"\n";
        let named = backfill_codex_custom_provider_names(nameless)
            .unwrap()
            .unwrap();
        assert_eq!(
            provider_table(&named, "custom-1")
                .get("name")
                .and_then(|v| v.as_str()),
            Some("custom-1")
        );
        assert_eq!(backfill_codex_custom_provider_names(&named).unwrap(), None);

        // Bedrock tables must not gain a name (the built-in merge rejects it).
        let bedrock = "model_provider = \"amazon-bedrock\"\n\n[model_providers.amazon-bedrock]\nregion = \"us-east-1\"\n";
        assert_eq!(backfill_codex_custom_provider_names(bedrock).unwrap(), None);
        // A table that already has a name is left alone.
        assert_eq!(
            backfill_codex_custom_provider_names(&healthy_third_party_config()).unwrap(),
            None
        );
    }

    // ---- repair: legacy openai reroute -------------------------------------

    #[test]
    fn legacy_openai_reroute_is_rewritten_into_a_custom_table_and_is_idempotent() {
        let legacy = "model = \"gpt-5.1\"\nopenai_base_url = \"https://api.example.com/v1\"\n";
        let fixed = normalize_codex_legacy_openai_reroute(legacy)
            .unwrap()
            .unwrap();
        let doc: toml::Value = toml::from_str(&fixed).unwrap();
        assert!(doc.get("openai_base_url").is_none());
        assert_eq!(
            doc.get("model_provider").and_then(|v| v.as_str()),
            Some("cc-switch")
        );
        let table = provider_table(&fixed, "cc-switch");
        assert_eq!(
            table.get("base_url").and_then(|v| v.as_str()),
            Some("https://api.example.com/v1")
        );
        assert_eq!(
            table.get("wire_api").and_then(|v| v.as_str()),
            Some("responses")
        );
        assert_eq!(table.get("name").and_then(|v| v.as_str()), Some("Custom"));

        assert_eq!(normalize_codex_legacy_openai_reroute(&fixed).unwrap(), None);

        // A config routing to a custom id is not the built-in reroute shape.
        let custom =
            "model_provider = \"custom-1\"\nopenai_base_url = \"https://api.example.com/v1\"\n";
        assert_eq!(normalize_codex_legacy_openai_reroute(custom).unwrap(), None);

        // Inline `model_providers` containers are handled too (a restore path
        // calls this without the gates).
        let inline = "openai_base_url = \"https://api.example.com/v1\"\nmodel_providers = { }\n";
        let fixed_inline = normalize_codex_legacy_openai_reroute(inline)
            .unwrap()
            .unwrap();
        assert!(fixed_inline.contains("cc-switch"));
        assert!(fixed_inline.contains("https://api.example.com/v1"));
        assert!(!fixed_inline.contains("openai_base_url"));
    }

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

    #[test]
    fn codex_provider_paths_are_suffixed_per_provider() {
        let (auth, config) = get_codex_provider_paths("p1", Some("My Provider"));
        // sanitize_provider_name only replaces filesystem-forbidden
        // characters and lowercases — spaces are preserved verbatim.
        assert!(auth.ends_with("auth-my provider.json"));
        assert!(config.ends_with("config-my provider.toml"));
    }
}
