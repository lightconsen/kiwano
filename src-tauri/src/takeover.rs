//! Agent takeover/restore (tech.md §4.3-3 takeover flow, §2.4 B key point 3).
//!
//! Takeover = back up the original config (SQLite `takeover_backups` table +
//! file copies) → rewrite base_url to point at the local gateway + inject an
//! Agent-specific placeholder key; restore = write the backup back (the
//! escape hatch). Backup semantics: re-enabling while already taken over
//! never overwrites the first backup, so the user's original config can be
//! restored no matter how many takeovers happen in a row.
//!
//! The MVP covers claude (settings.json), codex (config.toml + auth.json),
//! and gemini (~/.gemini/.env; inbound protocol in the gateway's
//! protocol.rs /v1beta branch).

use std::path::Path;

use serde_json::Value;

use crate::vm::Aux;

/// The set of backed-up files: `(absolute path, original content)`.
type Files = Vec<(String, String)>;

fn takeover_paths(agent: &str, home: &Path) -> Result<Vec<std::path::PathBuf>, String> {
    match agent {
        "claude" => Ok(vec![home.join(".claude").join("settings.json")]),
        "codex" => Ok(vec![
            home.join(".codex").join("config.toml"),
            home.join(".codex").join("auth.json"),
        ]),
        "gemini" => Ok(vec![home.join(".gemini").join(".env")]),
        other => Err(format!("unknown agent: {other}")),
    }
}

const GATEWAY_HOST: &str = "http://127.0.0.1";

/// Takeover: read the original files → perform all rewrites in memory
/// (can fail as a whole, zero side effects) → back up → atomic write.
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
            // codex's auth.json / gemini's .env are allowed to be missing (treated as empty files)
            Err(_) if p.ends_with("auth.json") || p.ends_with(".env") => String::new(),
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

    // Back up / write to disk only after every rewrite succeeds; any failure
    // fails the whole thing and keeps the escape hatch clean
    let rewritten: Files = originals
        .iter()
        .map(|(path, original)| {
            Ok((
                path.clone(),
                rewrite(agent, path, original, placeholder_key, data_port)?,
            ))
        })
        .collect::<Result<Files, String>>()?;

    // Back up only on first takeover; repeated enable keeps the original
    // text (escape-hatch semantics)
    let first_time = aux.load_takeover_backup(agent).is_none();
    if first_time {
        aux.save_takeover_backup(agent, &originals)
            .map_err(|e| e.to_string())?;
        copy_files(agent, home, &originals);
    }

    for (path, content) in &rewritten {
        // On first takeover ~/.gemini may not exist at all, so create the parent dir first
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        atomic_write(std::path::Path::new(path), content)?;
    }
    Ok(())
}

/// Restore: write the backup back verbatim and deregister it.
pub fn disable(aux: &Aux, agent: &str, home: &Path) -> Result<(), String> {
    let Some((_, files)) = aux.load_takeover_backup(agent) else {
        return Ok(()); // never taken over → idempotent success
    };
    let _ = takeover_paths(agent, home)?; // validate the agent name
    for (path, content) in &files {
        atomic_write(std::path::Path::new(path), content)?;
    }
    aux.delete_takeover_backup(agent)
        .map_err(|e| e.to_string())?;
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
        "gemini" => rewrite_gemini_env(original, base, data_port, key),
        _ => Err("unsupported agent".into()),
    }
}

/// settings.json: merge env.ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN, keeping all other fields.
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

/// config.toml: point the existing model_provider's base_url line at the
/// gateway (/v1). Refuse the takeover when there is no base_url to rewrite
/// (never fabricate codex config blindly).
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

/// auth.json: merge OPENAI_API_KEY, keeping all other fields; an empty file is treated as {}.
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

/// .env (dotenv): rewrite the GOOGLE_GEMINI_BASE_URL and GEMINI_API_KEY
/// lines, keeping all other lines and comments; missing variables are
/// appended at the end; a missing file is treated as empty (a first
/// takeover creates the ~/.gemini directory along with it). Values are not
/// quoted — neither value contains whitespace.
fn rewrite_gemini_env(original: &str, base: &str, port: u16, key: &str) -> Result<String, String> {
    let target = format!("{base}:{port}");
    let mut found_base = false;
    let mut found_key = false;
    let mut out = Vec::with_capacity(original.lines().count() + 2);
    for line in original.lines() {
        let trimmed = line.trim_start();
        let after_export = trimmed.strip_prefix("export ").unwrap_or(trimmed).trim_start();
        let key_of = after_export.split_once('=').map(|(k, _)| k.trim());
        match key_of {
            Some("GOOGLE_GEMINI_BASE_URL") if !found_base => {
                found_base = true;
                out.push(format!("GOOGLE_GEMINI_BASE_URL={target}"));
            }
            Some("GEMINI_API_KEY") if !found_key => {
                found_key = true;
                out.push(format!("GEMINI_API_KEY={key}"));
            }
            _ => out.push(line.to_string()),
        }
    }
    if !found_base {
        out.push(format!("GOOGLE_GEMINI_BASE_URL={target}"));
    }
    if !found_key {
        out.push(format!("GEMINI_API_KEY={key}"));
    }
    let mut s = out.join("\n");
    if original.is_empty() || original.ends_with('\n') {
        s.push('\n');
    }
    Ok(s)
}

/// Atomic write: write to `*.kiwano-tmp` first, then rename, avoiding half-written states.
fn atomic_write(path: &Path, content: &str) -> Result<(), String> {
    let tmp = path.with_extension("kiwano-tmp");
    std::fs::write(&tmp, content).map_err(|e| format!("write {}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, path).map_err(|e| format!("rename {}: {e}", tmp.display()))
}

/// File-copy fallback: `~/.kiwano/backups/<agent>/<filename>` (an escape hatch beyond SQLite).
fn copy_files(agent: &str, home: &Path, files: &Files) {
    let dir = home.join(".kiwano").join("backups").join(agent);
    if std::fs::create_dir_all(&dir).is_err() {
        return; // a copy failure does not break the takeover (SQLite already has it)
    }
    for (path, content) in files {
        let name = std::path::Path::new(path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
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
        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#,
        )
        .unwrap();

        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home).unwrap();
        let rewritten = std::fs::read_to_string(&settings).unwrap();
        let v: Value = serde_json::from_str(&rewritten).unwrap();
        assert_eq!(v["model"], "opus"); // other fields preserved
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "kw-ag-claude-abcd");

        // restore = write back byte for byte
        disable(&aux, "claude", &home).unwrap();
        let restored = std::fs::read_to_string(&settings).unwrap();
        assert_eq!(
            restored,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#
        );
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
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            r#"{"original":true}"#
        );
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
        std::fs::write(
            codex_dir.join("auth.json"),
            r#"{"OPENAI_API_KEY":"sk-old"}"#,
        )
        .unwrap();

        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap();
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("wire_api = \"responses\"")); // other lines untouched
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "kw-ag-codex-abcd");

        disable(&aux, "codex", &home).unwrap();
        assert!(std::fs::read_to_string(codex_dir.join("config.toml"))
            .unwrap()
            .contains("https://api.deepseek.com/v1"));
        assert_eq!(
            serde_json::from_str::<Value>(
                &std::fs::read_to_string(codex_dir.join("auth.json")).unwrap()
            )
            .unwrap()["OPENAI_API_KEY"],
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
        // no backup left behind on failure (escape hatch stays clean)
        assert!(aux.load_takeover_backup("codex").is_none());
    }

    #[test]
    fn missing_claude_config_errors() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        assert!(enable(&aux, "claude", "k", 8317, &home).is_err());
        // disable before any takeover succeeds idempotently
        assert!(disable(&aux, "claude", &home).is_ok());
    }

    #[test]
    fn gemini_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let gemini_dir = home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join(".env"),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# 代理注释\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n",
        )
        .unwrap();

        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home).unwrap();
        let env = std::fs::read_to_string(gemini_dir.join(".env")).unwrap();
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\n"));
        assert!(env.contains("GEMINI_API_KEY=kw-ag-gemini-abcd\n"));
        assert!(env.contains("GOOGLE_GENAI_USE_VERTEXAI=false")); // other lines untouched
        assert!(env.contains("# 代理注释")); // comment preserved

        // restore = write back byte for byte
        disable(&aux, "gemini", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(gemini_dir.join(".env")).unwrap(),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# 代理注释\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n"
        );
        assert!(aux.load_takeover_backup("gemini").is_none());
    }

    #[test]
    fn gemini_takeover_creates_missing_env() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        // takeover works even when ~/.gemini/.env is missing entirely (dir + file are created automatically)
        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home).unwrap();
        let env = std::fs::read_to_string(home.join(".gemini").join(".env")).unwrap();
        assert_eq!(
            env,
            "GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\nGEMINI_API_KEY=kw-ag-gemini-abcd\n"
        );
    }
}
