//! CC Switch 配置导入（tech.md §4.5：仅 v3.x）。
//!
//! 数据源按序尝试：`~/.cc-switch/cc-switch.db`（v3.20+ SQLite，只读）、
//! `~/.cc-switch/config.json`（旧 v3.x JSON）。映射规则：
//! - app_type claude → protocol anthropic，取 env.ANTHROPIC_BASE_URL /
//!   ANTHROPIC_AUTH_TOKEN（兼容 ANTHROPIC_API_KEY）
//! - app_type codex  → protocol openai，取 auth.OPENAI_API_KEY + config TOML
//!   里的 base_url 行
//! - base_url 拆为 origin（base_url）+ 首段路径（api_path），与网关
//!   upstream_url 的拼接约定一致
//! - cc-switch 的 is_current → 对应 Agent 的主选绑定（claude→claude、
//!   codex→codex），实现「零成本迁移」承诺（spec §10.1）
//! - 官方占位/空配置（无 base_url 或无 Key）跳过

use std::path::Path;

use kiwano_gateway::store::{Binding, Protocol, Provider, StrategyType, Store};
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
    app: &'static str, // "claude" | "codex"
    name: String,
    base_url: String,
    api_path: Option<String>,
    api_key: Option<String>,
    is_current: bool,
}

/// 默认导入入口：显式传入候选路径，避免测试读到真实用户数据。
pub fn run_import(store: &Store, db_path: Option<&Path>, json_path: Option<&Path>) -> ImportReportVm {
    let (mut raws, mut detail) = (Vec::new(), Vec::new());
    if let Some(p) = db_path {
        match read_db(p) {
            Ok(mut r) => {
                detail.push(format!("source: {} ({} rows)", p.display(), r.len()));
                raws.append(&mut r);
            }
            Err(e) => detail.push(format!("db skipped: {e}")),
        }
    }
    if raws.is_empty() {
        if let Some(p) = json_path {
            match read_json(p) {
                Ok(mut r) => {
                    detail.push(format!("source: {} ({} rows)", p.display(), r.len()));
                    raws.append(&mut r);
                }
                Err(e) => detail.push(format!("json skipped: {e}")),
            }
        }
    }
    if raws.is_empty() {
        detail.push("未找到可导入的 CC Switch 数据".into());
        return ImportReportVm { imported: 0, skipped: 0, detail };
    }

    let (mut imported, mut skipped) = (0usize, 0usize);
    for raw in raws {
        let (name, protocol) = match raw.app {
            "claude" => (raw.name.clone(), Protocol::Anthropic),
            _ => (raw.name.clone(), Protocol::OpenAI),
        };
        // 跳过官方占位/空配置
        if raw.base_url.is_empty() || raw.api_key.as_deref().unwrap_or("").is_empty() {
            skipped += 1;
            detail.push(format!("skip {}/{}: 缺少端点或 Key", raw.app, raw.cc_id));
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
            api_key: raw.api_key.clone(),
            billing: kiwano_gateway::store::Billing::Metered,
            period_limit: None,
            reset_period: None,
            enabled: true,
            created_at: now.clone(),
            updated_at: now,
        };
        let existed = store.get_provider(&id).map_err(|e| e.to_string()).ok().flatten().is_some();
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
            if existed { " · 已更新" } else { "" },
        ));
        if raw.is_current {
            let agent = raw.app; // claude→claude, codex→codex
            if store.upsert_strategy(agent, StrategyType::Single, None).is_ok() {
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
    ImportReportVm { imported, skipped, detail }
}

/// origin + 首段路径拆分：`https://api.deepseek.com/anthropic` →
/// (`https://api.deepseek.com`, Some(`/anthropic`))。
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

/// codex settings_config：`auth.OPENAI_API_KEY` + config TOML 里的
/// `base_url = "…"` 行（按行扫描，避免引入 toml 依赖）。
fn codex_creds(v: &Value) -> (String, Option<String>) {
    let key = str_at(v.get("auth").and_then(|a| a.get("OPENAI_API_KEY")));
    let toml_src = v.get("config").and_then(Value::as_str).unwrap_or("");
    let mut base = String::new();
    for line in toml_src.lines() {
        let Some((k, val)) = line.trim().split_once('=') else { continue };
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

fn raw_from(app: &'static str, cc_id: &str, name: &str, sc: &Value, is_current: bool) -> RawProvider {
    let (url, key) = match app {
        "claude" => claude_creds(sc),
        _ => codex_creds(sc),
    };
    let (base_url, api_path) = split_url(&url);
    RawProvider {
        cc_id: cc_id.to_string(),
        app,
        name: if name.is_empty() { cc_id.to_string() } else { name.to_string() },
        base_url,
        api_path,
        api_key: key,
        is_current,
    }
}

fn read_db(path: &Path) -> Result<Vec<RawProvider>, String> {
    let conn = rusqlite::Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY,
    )
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
    for row in rows {
        let (cc_id, app_type, name, sc, current) = row.map_err(|e| e.to_string())?;
        let app: &'static str = match app_type.as_str() {
            "claude" => "claude",
            "codex" => "codex",
            _ => continue, // claude-desktop / gemini 等 MVP 不支持
        };
        let Ok(sc) = serde_json::from_str::<Value>(&sc) else { continue };
        out.push(raw_from(app, &cc_id, &name, &sc, current));
    }
    Ok(out)
}

/// 旧版 v3.x JSON：MultiAppConfig，apps 以 flatten 形式在顶层
/// `{ "claude": { "providers": { id: {...} }, "current": id }, ... }`。
fn read_json(path: &Path) -> Result<Vec<RawProvider>, String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let root: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let mut out = Vec::new();
    for app in ["claude", "codex"] {
        let Some(mgr) = root.get(app) else { continue };
        let current = mgr.get("current").and_then(Value::as_str).unwrap_or("");
        let Some(providers) = mgr.get("providers").and_then(Value::as_object) else { continue };
        for (cc_id, p) in providers {
            let Some(sc) = p.get("settingsConfig") else { continue };
            let name = p.get("name").and_then(Value::as_str).unwrap_or("");
            out.push(raw_from(app, cc_id, name, sc, cc_id == current));
        }
    }
    Ok(out)
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
        assert_eq!(report.skipped, 1); // 官方空配置

        let p = store.get_provider("ccs-claude-p1").unwrap().unwrap();
        assert_eq!(p.base_url, "https://api.deepseek.com");
        assert_eq!(p.api_path.as_deref(), Some("/anthropic"));
        assert_eq!(p.api_key.as_deref(), Some("sk-a"));
        // is_current → 主选绑定
        assert_eq!(store.primary_provider_id("claude").unwrap().as_deref(), Some("ccs-claude-p1"));
        assert_eq!(store.primary_provider_id("codex").unwrap().as_deref(), Some("ccs-codex-c1"));
        // 重复导入 → 更新而非报错
        let again = run_import(&store, None, Some(&path));
        assert_eq!(again.imported, 2);
        assert!(again.detail.iter().any(|d| d.contains("已更新")));
    }
}
