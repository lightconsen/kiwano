//! Agent 接管/还原（tech.md §4.3-3 接管流、§2.4 B 要点三）。
//!
//! 接管 = 备份原配置（SQLite `takeover_backups` 表 + 文件副本）→ 改写
//! base_url 指向本地网关 + 注入该 Agent 专属占位 Key；还原 = 写回备份
//! （逃生门）。备份语义：已接管时再次 enable 不覆盖最初备份，保证
//! 无论连续接管多少次都能还原到用户原始配置。
//!
//! MVP 覆盖 claude（settings.json）与 codex（config.toml + auth.json）；
//! gemini 接管为 P1（tech.md §2.4 B：入站协议多一种，与原型状态一致）。

use std::path::Path;

use serde_json::Value;

use crate::vm::Aux;

/// 备份的文件集合：`(绝对路径, 原始内容)`。
type Files = Vec<(String, String)>;

fn takeover_paths(agent: &str, home: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    match agent {
        "claude" => Ok(vec![home.join(".claude").join("settings.json")]),
        "codex" => Ok(vec![
            home.join(".codex").join("config.toml"),
            home.join(".codex").join("auth.json"),
        ]),
        other => Err(format!("`{other}` 接管为 P1（Gemini 入站协议未支持）")),
    }
}

const GATEWAY_HOST: &str = "http://127.0.0.1";

/// 接管：读原文件 → 内存中完成全部改写（可整体失败、零副作用）→
/// 备份 → 原子写盘。
pub fn enable(
    aux: &Aux,
    agent: &str,
    placeholder_key: &str,
    data_port: u16,
    home: &Path,
) -> Result<(), String> {
    let paths = takeover_paths(agent, home)?;
    let mut originals: Files = Vec::new();
    for p in &paths {
        let content = match std::fs::read_to_string(p) {
            Ok(c) => c,
            // codex 的 auth.json 允许不存在（视为空对象）
            Err(_) if p.ends_with("auth.json") => String::new(),
            Err(_) => {
                return Err(format!(
                    "未找到 {} —— 请先运行过 {} 再接管",
                    p.display(),
                    agent
                ))
            }
        };
        originals.push((p.to_string_lossy().into_owned(), content));
    }

    // 全部改写成功才落备份/写盘；任一失败即整体失败，不污染逃生门
    let rewritten: Files = originals
        .iter()
        .map(|(path, original)| {
            Ok((path.clone(), rewrite(agent, path, original, placeholder_key, data_port)?))
        })
        .collect::<Result<Files, String>>()?;

    // 首次接管才落备份；重复 enable 保持最初原文（逃生门语义）
    let first_time = aux.load_takeover_backup(agent).is_none();
    if first_time {
        aux.save_takeover_backup(agent, &originals)
            .map_err(|e| e.to_string())?;
        copy_files(agent, home, &originals);
    }

    for (path, content) in &rewritten {
        atomic_write(std::path::Path::new(path), content)?;
    }
    Ok(())
}

/// 还原：写回备份原文并注销备份。
pub fn disable(aux: &Aux, agent: &str, home: &Path) -> Result<(), String> {
    let Some((_, files)) = aux.load_takeover_backup(agent) else {
        return Ok(()); // 未接管过 → 幂等成功
    };
    let _ = takeover_paths(agent, home)?; // 校验 agent 合法性
    for (path, content) in &files {
        atomic_write(std::path::Path::new(path), content)?;
    }
    aux.delete_takeover_backup(agent).map_err(|e| e.to_string())?;
    Ok(())
}

fn rewrite(
    agent: &str,
    path: &str,
    original: &str,
    key: &str,
    data_port: u16,
) -> Result<String, String> {
    let base = GATEWAY_HOST;
    match agent {
        "claude" => rewrite_claude(original, base, data_port, key),
        "codex" if path.ends_with("config.toml") => rewrite_codex_toml(original, base, data_port),
        "codex" => rewrite_codex_auth(original, key),
        _ => Err("unsupported agent".into()),
    }
}

/// settings.json：合并 env.ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN，保留其余字段。
fn rewrite_claude(original: &str, base: &str, port: u16, key: &str) -> Result<String, String> {
    let mut v: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(original).map_err(|e| format!("settings.json 不是合法 JSON: {e}"))?
    };
    let obj = v.as_object_mut().ok_or("settings.json 顶层不是对象")?;
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let env = env.as_object_mut().ok_or("settings.json 的 env 不是对象")?;
    env.insert(
        "ANTHROPIC_BASE_URL".into(),
        Value::String(format!("{base}:{port}")),
    );
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), Value::String(key.into()));
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// config.toml：把已有 model_provider 的 base_url 行改指向网关（/v1）。
/// 没有可改的 base_url 时拒绝接管（不盲目生成 codex 配置）。
fn rewrite_codex_toml(original: &str, base: &str, port: u16) -> Result<String, String> {
    let target = format!("{base}:{port}/v1");
    let mut found = false;
    let mut out = Vec::with_capacity(original.lines().count());
    for line in original.lines() {
        match line.split_once('=') {
            Some((k, _)) if k.trim() == "base_url" => {
                found = true;
                out.push(format!("base_url = \"{target}\""));
            }
            _ => out.push(line.to_string()),
        }
    }
    if !found {
        return Err(
            "config.toml 中没有 base_url —— 请先为 Codex 配置一个自定义 Provider 再接管".into(),
        );
    }
    let mut s = out.join("\n");
    if original.ends_with('\n') {
        s.push('\n');
    }
    Ok(s)
}

/// auth.json：合并 OPENAI_API_KEY，保留其余字段；空文件视为 {}。
fn rewrite_codex_auth(original: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(original).map_err(|e| format!("auth.json 不是合法 JSON: {e}"))?
    };
    let obj = v.as_object_mut().ok_or("auth.json 顶层不是对象")?;
    obj.insert("OPENAI_API_KEY".into(), Value::String(key.into()));
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// 原子写：先写 `*.kiwano-tmp` 再 rename，避免半写状态。
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("kiwano-tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", tmp.display()))
}

/// 文件副本兜底：`~/.kiwano/backups/<agent>/<文件名>`（SQLite 之外的逃生门）。
fn copy_files(agent: &str, home: &Path, files: &Files) {
    let dir = home.join(".kiwano").join("backups").join(agent);
    if std::fs::create_dir_all(&dir).is_err() {
        return; // 副本失败不影响接管（SQLite 里已有）
    }
    for (path, content) in files {
        let name = std::path::Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned());
        if let Some(name) = name {
            let _ = std::fs::write(dir.join(name), content);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_home() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().to_path_buf();
        (dir, home)
    }

    #[test]
    fn claude_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#).unwrap();

        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home).unwrap();
        let rewritten = std::fs::read_to_string(&settings).unwrap();
        let v: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(v["model"], "opus"); // 其余字段保留
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "kw-ag-claude-abcd");

        // 还原 = 逐字节写回
        disable(&aux, "claude", &home).unwrap();
        let restored = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(restored, r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#);
        assert!(aux.load_takeover_backup("claude").is_none());
    }

    #[test]
    fn repeated_enable_keeps_first_backup() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, r#"{"original":true}"#).unwrap();

        enable(&aux, "claude", "kw-ag-claude-1111", 8317, &home).unwrap();
        enable(&aux, "claude", "kw-ag-claude-2222", 8317, &home).unwrap();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "kw-ag-claude-2222");

        disable(&aux, "claude", &home).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), r#"{"original":true}"#);
    }

    #[test]
    fn codex_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(
            codex_dir.join("config.toml"),
            "model = \"m\"\n[model_provider.custom]\nname = \"x\"\nbase_url = \"https://api.deepseek.com/v1\"\nwire_api = \"responses\"\n",
        )
        .unwrap();
        std::fs::write(codex_dir.join("auth.json"), r#"{"OPENAI_API_KEY":"sk-old"}"#).unwrap();

        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap();
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("wire_api = \"responses\"")); // 其余行不动
        let auth: Value = serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap()).unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "kw-ag-codex-abcd");

        disable(&aux, "codex", &home).unwrap();
        assert!(std::fs::read_to_string(codex_dir.join("config.toml")).unwrap().contains("https://api.deepseek.com/v1"));
        assert_eq!(
            serde_json::from_str::<Value>(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap()).unwrap()["OPENAI_API_KEY"],
            "sk-old"
        );
    }

    #[test]
    fn codex_without_base_url_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(codex_dir.join("config.toml"), "model = \"m\"\n").unwrap();
        std::fs::write(codex_dir.join("auth.json"), "{}").unwrap();
        assert!(enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).is_err());
        // 失败后不留备份（逃生门不被污染）
        assert!(aux.load_takeover_backup("codex").is_none());
    }

    #[test]
    fn missing_claude_config_errors_and_gemini_unsupported() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        assert!(enable(&aux, "claude", "k", 8317, &home).is_err());
        assert!(enable(&aux, "gemini", "k", 8317, &home).is_err());
        // 未接管时 disable 幂等成功
        assert!(disable(&aux, "claude", &home).is_ok());
    }
}
