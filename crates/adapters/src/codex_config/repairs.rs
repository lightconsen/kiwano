//! Pre-write repairs: the config shapes Codex 0.149 refuses to load, rewritten
//! into shapes it loads.
//!
//! `migrate_stale_reserved_provider_tables` renames an overridden built-in
//! table, `backfill_codex_custom_provider_names` names a name-less one,
//! `normalize_codex_legacy_openai_reroute` rewrites the top-level
//! `openai_base_url` shape into a custom table, and
//! `preflight_codex_provider_table_conflicts` refuses the field combinations
//! that cannot be normalized away. Each is idempotent and reports the empty
//! answer when there is nothing to repair.

use crate::codex_config::gates::codex_provider_table_falls_back_to_official_auth;
use crate::codex_config::is_custom_codex_model_provider_id;
use crate::codex_config::provider_id::active_codex_model_provider_id;
use crate::error::AppError;
use toml_edit::DocumentMut;

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
pub(crate) fn migrate_stale_reserved_provider_tables(
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
pub(crate) fn backfill_codex_custom_provider_names(
    config_text: &str,
) -> Result<Option<String>, AppError> {
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
pub(crate) fn preflight_codex_provider_table_conflicts(config_text: &str) -> Result<(), AppError> {
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
pub(crate) fn normalize_codex_legacy_openai_reroute(
    config_text: &str,
) -> Result<Option<String>, AppError> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codex_config::plan::plan_codex_live_write;
    use crate::codex_config::test_support::{healthy_third_party_config, provider_table};

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
}
