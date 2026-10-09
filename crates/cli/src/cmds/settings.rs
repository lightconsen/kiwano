//! App settings, the raw config get/set, the catalog, and `import`.
//!
//! `import` is the second consumer of the `input` layer: a provider file is parsed
//! into the same `vm::NewProviderInput` the `providers` flags build.

use super::render::{render_catalog, render_settings};
use super::runtime;
use crate::cli::{CatalogCmd, ConfigCmd, ImportCmd, SettingsCmd};
use crate::{CliError, Ctx};
use kiwano_core::{import, share, vm};

// ── settings / config / catalog / import ────────────────────────────────────

pub fn settings(cmd: &SettingsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        SettingsCmd::Get => {
            // `--home` decides which tree the takeovers are read out of, so the
            // home-taking form is the one to use — `settings_view` over the
            // daemon's blob and this machine's config files.
            let settings = vm::settings_view(&ctx.api, &ctx.home, ctx.config_vars())?;
            let text = render_settings(&settings);
            ctx.out.emit(&settings, || text);
            Ok(())
        }
        SettingsCmd::Set { keys, patch } => {
            let patch = settings_patch(keys, patch.as_deref())?;
            // The blob and the gateway-facing mirrors are the daemon's, and it
            // re-reads its own config after the patch — which is what the reload
            // ping this used to send was for (`migrate.local.md` §10.16).
            let mut settings = ctx.api.update_settings(&patch)?;
            vm::layer_settings(&ctx.api, &mut settings, &ctx.home, ctx.config_vars())?;
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

pub fn config(cmd: &ConfigCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        ConfigCmd::Export { out, include_keys } => {
            let path = out.to_string_lossy();
            let count = {
                // The document is the daemon's; the **file** is this side's,
                // because the path came from this machine's `--out`
                // (`migrate.local.md` §10.17).
                let json = ctx.api.export_config(*include_keys)?;
                share::write_config_file(&path, &json)?;
                share::provider_count(&json)
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
            // The text, not a path: the reading happened above, and the daemon
            // validates and applies what it says.
            let report = ctx.api.import_config(&json)?;
            let text = format!(
                "added {} providers, kept {}, applied {} routes",
                report.providers_added, report.providers_kept, report.routes_applied
            );
            ctx.out.emit(&report, || text);
            Ok(())
        }
    }
}

pub fn catalog(cmd: &CatalogCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        CatalogCmd::List { tag, search } => {
            let mut catalog = ctx.api.list_catalog()?;
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
            // The hub_url comes from the settings blob the daemon serves — the
            // same row it reads when it syncs. Read for the *message*: the fetch
            // is the daemon's, and it takes the URL from the same row.
            let hub_url = ctx.api.get_settings()?.hub_url.clone();
            let report = ctx.api.sync_hub()?;
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
            let linked = ctx.api.link_providers()?;
            if linked > 0 {
                ctx.out.note(format!(
                    "linked {linked} provider(s) to their catalog entry"
                ));
            }
            Ok(())
        }
        CatalogCmd::Currency => {
            let meta = ctx.api.currency_meta()?;
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
            // The rows, not the paths: those files are in this machine's home
            // and one is a SQLite database, so reading them is the client's
            // (`migrate.local.md` §10.19). The daemon applies what they say.
            let root = ctx.home.join(".cc-switch");
            let (raws, skips, mut detail) = import::read_cc_switch(
                Some(&root.join("cc-switch.db")),
                Some(&root.join("config.json")),
            );
            if raws.is_empty() && skips.is_empty() {
                detail.push("No importable CC Switch data found".into());
                let report = import::ImportReportVm {
                    imported: 0,
                    skipped: 0,
                    detail,
                };
                ctx.out
                    .emit(&report, || "imported 0, skipped 0".to_string());
                return Ok(());
            }
            let mut report = ctx.api.import_cc_switch(&raws, &skips)?;
            detail.append(&mut report.detail);
            report.detail = detail;
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
            Ok(())
        }
    }
}
