//! CC Switch config import (tech.md §4.5: v3.x only).
//!
//! Data sources are tried in order: `~/.cc-switch/cc-switch.db` (v3.20+
//! SQLite, read-only) and `~/.cc-switch/config.json` (older v3.x JSON).
//! Mapping rules:
//! - app_type → Kiwano agent id is 1:1 for all nine apps (claude,
//!   claude-desktop, codex, gemini, grokbuild, opencode, openclaw, hermes, pi)
//! - protocol anthropic: claude / claude-desktop (env.ANTHROPIC_BASE_URL +
//!   ANTHROPIC_AUTH_TOKEN, ANTHROPIC_API_KEY also accepted)
//! - protocol gemini: gemini (env.GOOGLE_GEMINI_BASE_URL + GEMINI_API_KEY)
//! - protocol openai: codex (auth.OPENAI_API_KEY + config TOML base_url),
//!   grokbuild (config TOML, selected model row), opencode (provider fragment
//!   options.baseURL/apiKey), openclaw + pi (baseUrl/apiKey), hermes
//!   (base_url/api_key with camelCase fallback)
//! - base_url is split into origin (base_url) + first path segment
//!   (api_path), matching the gateway's upstream_url composition convention
//! - cc-switch's is_current → primary binding of the matching Agent
//!   (app id = agent id), delivering the "zero-cost migration" promise
//!   (spec §10.1)
//! - official placeholder / empty configs (no base_url or no key) are skipped;
//!   unknown app_types surface a skip detail line instead of being dropped
//!   silently

use std::path::Path;

use kiwano_gateway::store::{Binding, Protocol, Provider, Store, StrategyType};
use serde::Serialize;
use serde_json::Value;

use crate::vm::slug;

#[derive(Serialize)]
pub struct ImportReportVm {
    pub imported: usize,
    pub skipped: usize,
    pub detail: Vec<String>,
}

struct RawProvider {
    cc_id: String,
    app: &'static str, // one of the nine Kiwano agent ids
    name: String,
    base_url: String,
    api_path: Option<String>,
    api_key: Option<String>,
    is_current: bool,
}

/// Default import entry: pass candidate paths explicitly so tests never read real user data.
pub fn run_import(
    store: &Store,
    db_path: Option<&Path>,
    json_path: Option<&Path>,
) -> ImportReportVm {
    let (mut raws, mut detail, mut skips) = (Vec::new(), Vec::new(), Vec::new());
    if let Some(p) = db_path {
        match read_db(p) {
            Ok((mut r, mut s)) => {
                detail.push(format!("source: {} ({} rows)", p.display(), r.len()));
                raws.append(&mut r);
                skips.append(&mut s);
            }
            Err(e) => detail.push(format!("db skipped: {e}")),
        }
    }
    if raws.is_empty() {
        if let Some(p) = json_path {
            match read_json(p) {
                Ok((mut r, mut s)) => {
                    detail.push(format!("source: {} ({} rows)", p.display(), r.len()));
                    raws.append(&mut r);
                    skips.append(&mut s);
                }
                Err(e) => detail.push(format!("json skipped: {e}")),
            }
        }
    }
    if raws.is_empty() && skips.is_empty() {
        detail.push("No importable CC Switch data found".into());
        return ImportReportVm {
            imported: 0,
            skipped: 0,
            detail,
        };
    }

    let (mut imported, mut skipped) = (0usize, 0usize);
    for line in &skips {
        skipped += 1;
        detail.push(line.clone());
    }
    for raw in raws {
        let (name, protocol) = match raw.app {
            "claude" | "claude-desktop" => (raw.name.clone(), Protocol::Anthropic),
            "gemini" => (raw.name.clone(), Protocol::Gemini),
            _ => (raw.name.clone(), Protocol::OpenAI),
        };
        // Skip official placeholders / empty configs
        if raw.base_url.is_empty() || raw.api_key.as_deref().unwrap_or("").is_empty() {
            skipped += 1;
            detail.push(format!("skip {}/{}: missing endpoint or key", raw.app, raw.cc_id));
            continue;
        }
        let id = format!("ccs-{}-{}", raw.app, slug(&raw.cc_id));
        let now = crate::vm::rfc3339(crate::vm::unix_now());
        let provider = Provider {
            id: id.clone(),
            name,
            protocol,
            base_url: raw.base_url.clone(),
            api_path: raw.api_path.clone(),
            endpoints: Vec::new(),
            api_key: raw.api_key.clone(),
            billing: kiwano_gateway::store::Billing::Metered,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };
        let existed = store
            .get_provider(&id)
            .map_err(|e| e.to_string())
            .ok()
            .flatten()
            .is_some();
        let up = if existed {
            store.update_provider(&provider)
        } else {
            store.insert_provider(&provider)
        };
        if let Err(e) = up {
            skipped += 1;
            detail.push(format!("skip {id}: {e}"));
            continue;
        }
        imported += 1;
        detail.push(format!(
            "import {id}: {} ({}){}",
            provider.name,
            provider.protocol.as_str(),
            if existed { " · updated" } else { "" },
        ));
        if raw.is_current {
            let agent = raw.app; // app_type == Kiwano agent id (1:1 for all nine)
            if store
                .upsert_strategy(agent, StrategyType::Single, None)
                .is_ok()
            {
                let prev = store.primary_provider_id(agent).ok().flatten();
                let _ = store.upsert_binding(&Binding {
                    agent: agent.to_string(),
                    provider_id: id.clone(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                });
                if let Some(prev) = prev {
                    if prev != id {
                        let _ = store.upsert_binding(&Binding {
                            agent: agent.to_string(),
                            provider_id: prev,
                            priority: 1,
                            weight: 1,
                            win_start: None,
                            win_end: None,
                            enabled: true,
                        });
                    }
                }
            }
        }
    }
    detail.truncate(30);
    ImportReportVm {
        imported,
        skipped,
        detail,
    }
}

/// origin + first-path-segment split: `https://api.deepseek.com/anthropic` →
/// (`https://api.deepseek.com`, Some(`/anthropic`)).
fn split_url(url: &str) -> (String, Option<String>) {
    let url = url.trim_end_matches('/');
    let Some(scheme_end) = url.find("://") else {
        return (url.to_string(), None);
    };
    let rest = &url[scheme_end + 3..];
    match rest.find('/') {
        Some(i) => {
            let base = format!("{}://{}", &url[..scheme_end], &rest[..i]);
            let path = rest[i..].trim_end_matches('/').to_string();
            let path = if path.is_empty() { None } else { Some(path) };
            (base, path)
        }
        None => (url.to_string(), None),
    }
}

fn str_at(v: Option<&Value>) -> String {
    v.and_then(Value::as_str).unwrap_or("").trim().to_string()
}

fn claude_creds(v: &Value) -> (String, Option<String>) {
    let env = v.get("env");
    let base = str_at(env.and_then(|e| e.get("ANTHROPIC_BASE_URL")));
    let key = ["ANTHROPIC_AUTH_TOKEN", "ANTHROPIC_API_KEY"]
        .iter()
        .map(|k| str_at(env.and_then(|e| e.get(*k))))
        .find(|s| !s.is_empty());
    (base, key)
}

/// gemini settings_config: same `{env: {...}}` shape as claude, with Gemini's
/// own key names (GOOGLE_GEMINI_BASE_URL + GEMINI_API_KEY).
fn gemini_creds(v: &Value) -> (String, Option<String>) {
    let env = v.get("env");
    let base = str_at(env.and_then(|e| e.get("GOOGLE_GEMINI_BASE_URL")));
    let key = ["GEMINI_API_KEY", "GOOGLE_API_KEY"]
        .iter()
        .map(|k| str_at(env.and_then(|e| e.get(*k))))
        .find(|s| !s.is_empty());
    (base, key)
}

/// codex settings_config: `auth.OPENAI_API_KEY` plus the `base_url = "…"`
/// line in the config TOML (scanned line by line to avoid a toml dependency).
fn codex_creds(v: &Value) -> (String, Option<String>) {
    let key = str_at(v.get("auth").and_then(|a| a.get("OPENAI_API_KEY")));
    let toml_src = v.get("config").and_then(Value::as_str).unwrap_or("");
    let mut base = String::new();
    for line in toml_src.lines() {
        let Some((k, val)) = line.trim().split_once('=') else {
            continue;
        };
        if k.trim() != "base_url" {
            continue;
        }
        let val = val.trim().trim_matches('"').trim_matches('\'');
        if val.starts_with("http") {
            base = val.to_string();
            break;
        }
    }
    (base, (!key.is_empty()).then_some(key))
}

/// grokbuild settings_config: `{"config": "<toml>"}` whose `[models] default`
/// selects a `[model."profile"]` row carrying base_url/api_key (two-pass scan,
/// mirroring takeover.rs's rewrite_grok_toml).
fn grok_creds(v: &Value) -> (String, Option<String>) {
    let toml_src = v.get("config").and_then(Value::as_str).unwrap_or("");
    // pass 1: the [models] table's `default = "<profile>"` selector
    let mut section = "";
    let mut default_profile = String::new();
    for line in toml_src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed;
            continue;
        }
        if section == "[models]" {
            if let Some((k, val)) = trimmed.split_once('=') {
                if k.trim() == "default" {
                    default_profile = val.trim().trim_matches('"').trim_matches('\'').to_string();
                }
            }
        }
    }
    if default_profile.is_empty() {
        return (String::new(), None);
    }
    // pass 2: the selected [model."profile"] row's base_url / api_key
    let selected = format!("[model.\"{default_profile}\"]");
    let (mut base, mut key) = (String::new(), String::new());
    section = "";
    for line in toml_src.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            section = trimmed;
            continue;
        }
        if section != selected {
            continue;
        }
        if let Some((k, val)) = trimmed.split_once('=') {
            let val = val.trim().trim_matches('"').trim_matches('\'');
            match k.trim() {
                "base_url" if val.starts_with("http") => base = val.to_string(),
                "api_key" => key = val.to_string(),
                _ => {}
            }
        }
    }
    (base, (!key.is_empty()).then_some(key))
}

/// opencode settings_config: a provider fragment `{npm, options: {baseURL,
/// apiKey}, models}` (cc-switch's OpenCodeProviderConfig).
fn opencode_creds(v: &Value) -> (String, Option<String>) {
    let options = v.get("options");
    let base = str_at(options.and_then(|o| o.get("baseURL")));
    let key = str_at(options.and_then(|o| o.get("apiKey")));
    (base, (!key.is_empty()).then_some(key))
}

/// openclaw + pi settings_config: a provider fragment `{baseUrl, apiKey, …}`
/// (camelCase, openclaw.json / models.json conventions).
fn openclaw_creds(v: &Value) -> (String, Option<String>) {
    let base = str_at(v.get("baseUrl"));
    let key = str_at(v.get("apiKey"));
    (base, (!key.is_empty()).then_some(key))
}

/// hermes settings_config: a custom_providers entry `{base_url, api_key}`
/// (snake_case; camelCase accepted for historical DeepLink imports).
fn hermes_creds(v: &Value) -> (String, Option<String>) {
    let base = ["base_url", "baseUrl"]
        .iter()
        .map(|k| str_at(v.get(*k)))
        .find(|s| !s.is_empty());
    let key = ["api_key", "apiKey"]
        .iter()
        .map(|k| str_at(v.get(*k)))
        .find(|s| !s.is_empty());
    let (base, key) = (base.unwrap_or_default(), key.unwrap_or_default());
    (base, (!key.is_empty()).then_some(key))
}

fn raw_from(
    app: &'static str,
    cc_id: &str,
    name: &str,
    sc: &Value,
    is_current: bool,
) -> RawProvider {
    let (url, key) = match app {
        "claude" | "claude-desktop" => claude_creds(sc),
        "gemini" => gemini_creds(sc),
        "codex" => codex_creds(sc),
        "grokbuild" => grok_creds(sc),
        "opencode" => opencode_creds(sc),
        "openclaw" | "pi" => openclaw_creds(sc),
        "hermes" => hermes_creds(sc),
        _ => (String::new(), None),
    };
    let (base_url, api_path) = split_url(&url);
    RawProvider {
        cc_id: cc_id.to_string(),
        app,
        name: if name.is_empty() {
            cc_id.to_string()
        } else {
            name.to_string()
        },
        base_url,
        api_path,
        api_key: key,
        is_current,
    }
}

fn read_db(path: &Path) -> Result<(Vec<RawProvider>, Vec<String>), String> {
    let conn =
        rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
            .map_err(|e| format!("{}: {e}", path.display()))?;
    let mut stmt = conn
        .prepare("SELECT id, app_type, name, settings_config, is_current FROM providers")
        .map_err(|e| e.to_string())?;
    let rows = stmt
        .query_map([], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
                row.get::<_, String>(3)?,
                row.get::<_, i64>(4)? != 0,
            ))
        })
        .map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut skips = Vec::new();
    for row in rows {
        let (cc_id, app_type, name, sc, current) = row.map_err(|e| e.to_string())?;
        let app: &'static str = match app_type.as_str() {
            "claude" => "claude",
            "claude-desktop" => "claude-desktop",
            "codex" => "codex",
            "gemini" => "gemini",
            "grokbuild" => "grokbuild",
            "opencode" => "opencode",
            "openclaw" => "openclaw",
            "hermes" => "hermes",
            "pi" => "pi",
            other => {
                // unknown app types surface as explicit skip lines
                skips.push(format!("skip {other}/{cc_id}: unsupported app type"));
                continue;
            }
        };
        let Ok(sc) = serde_json::from_str::<Value>(&sc) else {
            skips.push(format!(
                "skip {app}/{cc_id}: settings_config is not valid JSON"
            ));
            continue;
        };
        out.push(raw_from(app, &cc_id, &name, &sc, current));
    }
    Ok((out, skips))
}

/// Older v3.x JSON: MultiAppConfig with apps flattened at the top level as
/// `{ "claude": { "providers": { id: {...} }, "current": id }, ... }`.
fn read_json(path: &Path) -> Result<(Vec<RawProvider>, Vec<String>), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    let mut skips = Vec::new();
    let Some(managers) = root.as_object() else {
        return Ok((out, skips));
    };
    for (app_key, mgr) in managers {
        let Some(providers) = mgr.get("providers").and_then(Value::as_object) else {
            continue; // version / mcp / other non-manager sections
        };
        let app = match app_key.as_str() {
            "claude" => "claude",
            "claude-desktop" => "claude-desktop",
            "codex" => "codex",
            "gemini" => "gemini",
            "grokbuild" => "grokbuild",
            "opencode" => "opencode",
            "openclaw" => "openclaw",
            "hermes" => "hermes",
            "pi" => "pi",
            _ => {
                for cc_id in providers.keys() {
                    skips.push(format!("skip {app_key}/{cc_id}: unsupported app type"));
                }
                continue;
            }
        };
        let current = mgr.get("current").and_then(Value::as_str).unwrap_or("");
        for (cc_id, p) in providers {
            let Some(sc) = p.get("settingsConfig") else {
                continue;
            };
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            out.push(raw_from(app, cc_id, name, sc, cc_id == current));
        }
    }
    Ok((out, skips))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_url_origin_and_prefix() {
        assert_eq!(
            split_url("https://api.deepseek.com/anthropic"),
            ("https://api.deepseek.com".into(), Some("/anthropic".into()))
        );
        assert_eq!(
            split_url("https://api.groq.com/openai/v1/"),
            ("https://api.groq.com".into(), Some("/openai/v1".into()))
        );
        assert_eq!(
            split_url("http://localhost:11434"),
            ("http://localhost:11434".into(), None)
        );
    }

    #[test]
    fn claude_and_codex_extraction() {
        let claude = serde_json::json!({
            "env": { "ANTHROPIC_AUTH_TOKEN": "sk-a", "ANTHROPIC_BASE_URL": "https://x.com/anthropic" }
        });
        let (url, key) = claude_creds(&claude);
        assert_eq!(url, "https://x.com/anthropic");
        assert_eq!(key.as_deref(), Some("sk-a"));

        let codex = serde_json::json!({
            "auth": { "OPENAI_API_KEY": "sk-b" },
            "config": "model = \"m\"\n[model_provider.p]\nbase_url = \"https://y.com/v1\"\n"
        });
        let (url, key) = codex_creds(&codex);
        assert_eq!(url, "https://y.com/v1");
        assert_eq!(key.as_deref(), Some("sk-b"));
    }

    #[test]
    fn per_app_extraction() {
        let gemini = serde_json::json!({
            "env": { "GEMINI_API_KEY": "g-k", "GOOGLE_GEMINI_BASE_URL": "https://g.com/v1beta" }
        });
        assert_eq!(
            gemini_creds(&gemini),
            ("https://g.com/v1beta".into(), Some("g-k".into()))
        );

        let grok = serde_json::json!({ "config": "theme = \"dark\"\n[models]\ndefault = \"p\"\n\n[model.\"p\"]\nname = \"P\"\nbase_url = \"https://api.x.ai/v1\"\napi_key = \"xai-k\"\n" });
        assert_eq!(
            grok_creds(&grok),
            ("https://api.x.ai/v1".into(), Some("xai-k".into()))
        );

        let grok_no_default = serde_json::json!({ "config": "theme = \"dark\"\n" });
        assert_eq!(grok_creds(&grok_no_default), (String::new(), None));

        let opencode = serde_json::json!({ "npm": "@ai-sdk/openai", "options": { "baseURL": "https://z.com/v1", "apiKey": "oz" } });
        assert_eq!(
            opencode_creds(&opencode),
            ("https://z.com/v1".into(), Some("oz".into()))
        );

        let openclaw = serde_json::json!({ "baseUrl": "https://oc.com/v1", "apiKey": "ok" });
        assert_eq!(
            openclaw_creds(&openclaw),
            ("https://oc.com/v1".into(), Some("ok".into()))
        );

        let hermes = serde_json::json!({ "base_url": "https://h.com", "api_key": "hk" });
        assert_eq!(
            hermes_creds(&hermes),
            ("https://h.com".into(), Some("hk".into()))
        );
        let hermes_camel = serde_json::json!({ "baseUrl": "https://h2.com", "apiKey": "hk2" });
        assert_eq!(
            hermes_creds(&hermes_camel),
            ("https://h2.com".into(), Some("hk2".into()))
        );
    }

    #[test]
    fn import_from_json_maps_and_binds() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
  "version": 2,
  "claude": {
    "current": "p1",
    "providers": {
      "p1": { "id": "p1", "name": "DeepSeek", "settingsConfig": { "env": { "ANTHROPIC_AUTH_TOKEN": "sk-a", "ANTHROPIC_BASE_URL": "https://api.deepseek.com/anthropic" } } },
      "p2": { "id": "p2", "name": "Claude Official", "settingsConfig": { "env": {} } }
    }
  },
  "codex": {
    "current": "c1",
    "providers": {
      "c1": { "id": "c1", "name": "Groq", "settingsConfig": { "auth": { "OPENAI_API_KEY": "sk-b" }, "config": "base_url = \"https://api.groq.com/openai\"" } }
    }
  }
}"#,
        )
        .unwrap();
        let store = Store::open_in_memory().unwrap();
        let report = run_import(&store, None, Some(&path));
        assert_eq!(report.imported, 2, "detail={:?}", report.detail);
        assert_eq!(report.skipped, 1); // official empty config

        let p = store.get_provider("ccs-claude-p1").unwrap().unwrap();
        assert_eq!(p.base_url, "https://api.deepseek.com");
        assert_eq!(p.api_path.as_deref(), Some("/anthropic"));
        assert_eq!(p.api_key.as_deref(), Some("sk-a"));
        // is_current → primary binding
        assert_eq!(
            store.primary_provider_id("claude").unwrap().as_deref(),
            Some("ccs-claude-p1")
        );
        assert_eq!(
            store.primary_provider_id("codex").unwrap().as_deref(),
            Some("ccs-codex-c1")
        );
        // re-import → update, not an error
        let again = run_import(&store, None, Some(&path));
        assert_eq!(again.imported, 2);
        assert!(again.detail.iter().any(|d| d.contains("updated")));
    }

    #[test]
    fn import_from_json_covers_all_nine_apps() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(
            &path,
            r#"{
  "claude": { "current": "a", "providers": { "a": { "id": "a", "name": "C", "settingsConfig": { "env": { "ANTHROPIC_AUTH_TOKEN": "k1", "ANTHROPIC_BASE_URL": "https://c.com" } } } } },
  "claude-desktop": { "current": "d", "providers": { "d": { "id": "d", "name": "D", "settingsConfig": { "env": { "ANTHROPIC_AUTH_TOKEN": "k2", "ANTHROPIC_BASE_URL": "https://d.com" } } } } },
  "codex": { "current": "x", "providers": { "x": { "id": "x", "name": "X", "settingsConfig": { "auth": { "OPENAI_API_KEY": "k3" }, "config": "base_url = \"https://x.com/v1\"" } } } },
  "gemini": { "current": "g", "providers": { "g": { "id": "g", "name": "G", "settingsConfig": { "env": { "GEMINI_API_KEY": "k4", "GOOGLE_GEMINI_BASE_URL": "https://g.com/v1beta" } } } } },
  "grokbuild": { "current": "r", "providers": { "r": { "id": "r", "name": "R", "settingsConfig": { "config": "[models]\ndefault = \"p\"\n\n[model.\"p\"]\nbase_url = \"https://r.com/v1\"\napi_key = \"k5\"\n" } } } },
  "opencode": { "current": "o", "providers": { "o": { "id": "o", "name": "O", "settingsConfig": { "npm": "@ai-sdk/openai", "options": { "baseURL": "https://o.com/v1", "apiKey": "k6" } } } } },
  "openclaw": { "current": "l", "providers": { "l": { "id": "l", "name": "L", "settingsConfig": { "baseUrl": "https://l.com/v1", "apiKey": "k7" } } } },
  "hermes": { "current": "h", "providers": { "h": { "id": "h", "name": "H", "settingsConfig": { "base_url": "https://h.com", "api_key": "k8" } } } },
  "pi": { "current": "p", "providers": { "p": { "id": "p", "name": "P", "settingsConfig": { "baseUrl": "https://p.com/v1", "apiKey": "k9" } } } }
}"#,
        )
        .unwrap();
        let store = Store::open_in_memory().unwrap();
        let report = run_import(&store, None, Some(&path));
        assert_eq!(report.imported, 9, "detail={:?}", report.detail);
        assert_eq!(report.skipped, 0);

        // protocol mapping per family
        let d = store.get_provider("ccs-claude-desktop-d").unwrap().unwrap();
        assert_eq!(d.protocol.as_str(), "anthropic");
        let g = store.get_provider("ccs-gemini-g").unwrap().unwrap();
        assert_eq!(g.protocol.as_str(), "gemini");
        let r = store.get_provider("ccs-grokbuild-r").unwrap().unwrap();
        assert_eq!(r.protocol.as_str(), "openai");
        assert_eq!(r.base_url, "https://r.com");
        assert_eq!(r.api_path.as_deref(), Some("/v1"));
        let o = store.get_provider("ccs-opencode-o").unwrap().unwrap();
        assert_eq!(o.protocol.as_str(), "openai");
        assert_eq!(o.api_key.as_deref(), Some("k6"));

        // every agent got its primary binding from is_current
        for agent in [
            "claude",
            "claude-desktop",
            "codex",
            "gemini",
            "grokbuild",
            "opencode",
            "openclaw",
            "hermes",
            "pi",
        ] {
            let expected = format!("ccs-{agent}-");
            let got = store.primary_provider_id(agent).unwrap();
            assert!(
                got.as_deref().is_some_and(|id| id.starts_with(&expected)),
                "agent {agent}: expected {expected}*, got {got:?}"
            );
        }
    }

    #[test]
    fn import_from_db_reports_unknown_app_type() {
        let dir = tempfile::tempdir().unwrap();
        let db_path = dir.path().join("cc-switch.db");
        let conn = rusqlite::Connection::open(&db_path).unwrap();
        conn.execute(
            "CREATE TABLE providers (id TEXT PRIMARY KEY, app_type TEXT NOT NULL, name TEXT NOT NULL, settings_config TEXT NOT NULL, is_current INTEGER NOT NULL DEFAULT 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO providers VALUES ('u1', 'mysteryapp', 'Mystery', '{}', 0)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO providers VALUES ('g1', 'gemini', 'Gem', '{\"env\":{\"GEMINI_API_KEY\":\"kg\",\"GOOGLE_GEMINI_BASE_URL\":\"https://g.com/v1beta\"}}', 1)",
            [],
        )
        .unwrap();

        let store = Store::open_in_memory().unwrap();
        let report = run_import(&store, Some(&db_path), None);
        assert_eq!(report.imported, 1, "detail={:?}", report.detail);
        assert_eq!(report.skipped, 1);
        assert!(report
            .detail
            .iter()
            .any(|d| d.contains("skip mysteryapp/u1")));
        assert_eq!(
            store.primary_provider_id("gemini").unwrap().as_deref(),
            Some("ccs-gemini-g1")
        );
    }
}
