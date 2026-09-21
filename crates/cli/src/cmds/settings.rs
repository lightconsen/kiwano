//! App settings, the raw config get/set, the catalog, and `import`.
//!
//! `import` is the second consumer of the `input` layer: a provider file is parsed
//! into the same `vm::NewProviderInput` the `providers` flags build.

use super::render::{render_catalog, render_settings};
use super::runtime;
use crate::cli::{CatalogCmd, ConfigCmd, ImportCmd, SettingsCmd};
use crate::{CliError, Ctx};
use kiwano_core::{import, pricing, share, sync, vm};

// ── settings / config / catalog / import ────────────────────────────────────

pub fn settings(cmd: &SettingsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        SettingsCmd::Get => {
            let settings = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                // The home-taking form: `build_settings` would resolve $HOME
                // itself, ignoring --home and reporting on the wrong tree.
                vm::build_settings_with_home(store, aux, &ctx.home, ctx.config_vars())?
            };
            let text = render_settings(&settings);
            ctx.out.emit(&settings, || text);
            Ok(())
        }
        SettingsCmd::Set { keys, patch } => {
            let patch = settings_patch(keys, patch.as_deref())?;
            let settings = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                vm::update_settings(store, aux, &patch, ctx.config_vars())?
            };
            // Only some settings change routing; reloading for the rest would
            // be noise on every `settings set`.
            if touches_routing(&patch) {
                ctx.after_mutation();
            }
            let text = render_settings(&settings);
            ctx.out.emit(&settings, || text);
            Ok(())
        }
    }
}

/// `--key k=v` pairs plus an optional `--patch` object, merged.
///
/// Values are parsed as JSON when they parse, so `false` is a boolean and `30`
/// a number; anything else stays the string that was typed. That is what makes
/// `settings set --key cost_alert=false` work without a typed flag per setting.
fn settings_patch(pairs: &[String], patch: Option<&str>) -> Result<serde_json::Value, CliError> {
    let mut merged = match patch {
        Some(raw) => serde_json::from_str::<serde_json::Value>(raw)
            .map_err(|e| CliError::usage(format!("--patch is not valid JSON: {e}")))?,
        None => serde_json::json!({}),
    };
    let object = merged
        .as_object_mut()
        .ok_or_else(|| CliError::usage("--patch must be a JSON object"))?;

    for pair in pairs {
        let (key, value) = pair
            .split_once('=')
            .ok_or_else(|| CliError::usage(format!("--key expects KEY=VALUE, got {pair:?}")))?;
        let key = key.trim();
        if key.is_empty() {
            return Err(CliError::usage(format!(
                "--key has an empty name: {pair:?}"
            )));
        }
        let value = match serde_json::from_str::<serde_json::Value>(value) {
            Ok(parsed) => parsed,
            Err(_) => serde_json::Value::String(value.to_string()),
        };
        object.insert(key.to_string(), value);
    }
    Ok(merged)
}

/// Whether a settings patch can change what the gateway routes.
///
/// The GUI reloads on any settings change; for a scripted `settings set` that
/// would mean an admin round-trip per key, most of which cannot affect routing
/// at all. The set here is the conservative one: anything that could plausibly
/// reach the route table or the limits evaluation.
fn touches_routing(patch: &serde_json::Value) -> bool {
    const ROUTING_KEYS: [&str; 4] = [
        "auto_failover",
        "gateway_listen",
        "request_logs",
        "log_retention_days",
    ];
    match patch.as_object() {
        Some(map) => map.keys().any(|k| ROUTING_KEYS.contains(&k.as_str())),
        None => false,
    }
}

pub fn config(cmd: &ConfigCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Export { out, include_keys } => {
            let path = out.to_string_lossy();
            let count = {
                let store = ctx.store()?;
                share::export_config_to_file(store, &path, *include_keys)?
            };
            let text = format!("wrote {count} providers to {path}");
            ctx.out.emit(
                &serde_json::json!({ "path": path, "providers": count }),
                || text,
            );
            if *include_keys {
                ctx.out
                    .note("note: the file contains provider API keys — it is written owner-only");
            }
            Ok(())
        }
        ConfigCmd::Import { file } => {
            let path = file.to_string_lossy();
            let json = std::fs::read_to_string(file)
                .map_err(|e| runtime(format!("cannot read {path}: {e}")))?;
            let report = {
                let store = ctx.store()?;
                share::import_config(store, &json)?
            };
            let text = format!(
                "added {} providers, kept {}, applied {} routes",
                report.providers_added, report.providers_kept, report.routes_applied
            );
            ctx.out.emit(&report, || text);
            ctx.after_mutation();
            Ok(())
        }
    }
}

pub fn catalog(cmd: &CatalogCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        CatalogCmd::List { tag, search } => {
            let mut catalog = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                vm::load_catalog(store, aux)
            };
            if let Some(tag) = tag {
                catalog.entries.retain(|e| e.tag == *tag);
            }
            if let Some(search) = search {
                let needle = search.to_lowercase();
                catalog
                    .entries
                    .retain(|e| e.name.to_lowercase().contains(&needle));
            }
            let text = render_catalog(&catalog);
            ctx.out.emit(&catalog, || text);
            Ok(())
        }
        CatalogCmd::Sync => {
            let hub_url = {
                let aux = ctx.aux()?;
                vm::ui_settings(aux).hub_url
            };
            let report = {
                let aux = ctx.aux()?;
                kiwano_core::block_on(sync::sync_from_hub(aux, &hub_url))?
            };
            let text = if report.unchanged {
                format!(
                    "already current ({} entries, synced {})",
                    report.fetched, report.synced_at
                )
            } else {
                format!("synced {} entries from {hub_url}", report.fetched)
            };
            ctx.out.emit(&report, || text);
            // The sync may have brought the catalog a provider can now be
            // matched against. Before the reload below, so one pass picks up
            // both.
            let linked = {
                let (store, aux) = (ctx.store()?, ctx.aux()?);
                vm::link_providers(store, aux)?
            };
            if linked > 0 {
                ctx.out.note(format!(
                    "linked {linked} provider(s) to their catalog entry"
                ));
            }
            // A price refresh only reaches cost recording through a reload.
            ctx.after_mutation();
            Ok(())
        }
        CatalogCmd::Currency => {
            let meta = {
                let aux = ctx.aux()?;
                pricing::currency_meta(aux)?
            };
            let text = format!(
                "preferred {} · {} currencies",
                meta.preferred,
                meta.currencies.len()
            );
            ctx.out.emit(&meta, || text);
            Ok(())
        }
    }
}

pub fn import(cmd: &ImportCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ImportCmd::CcSwitch => {
            let root = ctx.home.join(".cc-switch");
            let report = {
                let store = ctx.store()?;
                import::run_import(
                    store,
                    Some(&root.join("cc-switch.db")),
                    Some(&root.join("config.json")),
                )
            };
            let text = format!(
                "imported {}, skipped {}{}",
                report.imported,
                report.skipped,
                if report.detail.is_empty() {
                    String::new()
                } else {
                    format!("\n{}", report.detail.join("\n"))
                }
            );
            ctx.out.emit(&report, || text);
            if report.imported > 0 {
                ctx.after_mutation();
            }
            Ok(())
        }
    }
}
