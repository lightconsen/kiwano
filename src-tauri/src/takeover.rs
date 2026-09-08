//! Agent takeover/restore (tech.md §4.3-3 takeover flow, §2.4 B key point 3).
//!
//! Takeover = back up the original config (SQLite `takeover_backups` table +
//! file copies) → rewrite base_url to point at the local gateway + inject an
//! Agent-specific placeholder key; restore = write the backup back (the
//! escape hatch). Backup semantics: re-enabling while already taken over
//! never overwrites the first backup, so the user's original config can be
//! restored no matter how many takeovers happen in a row.
//!
//! The MVP covered claude (settings.json), codex (config.toml + auth.json),
//! and gemini (~/.gemini/.env; inbound protocol in the gateway's
//! protocol.rs /v1beta branch). The expanded registry adds grokbuild
//! (~/.grok/config.toml, selected model's base_url/api_key/api_backend), the
//! additive-mode agents opencode/openclaw/hermes/pi — whose takeover upserts
//! a `kiwano-gateway` provider entry and selects it via
//! `kiwano_cc_adapters::gateway_takeover` — and claude-desktop (macOS only:
//! deploymentMode 3p + a gateway profile in the Claude-3p configLibrary via
//! `kiwano_cc_adapters::claude_desktop_config`). Pre-existing provider
//! entries survive, and disable still restores the original bytes verbatim.

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
        "grokbuild" => Ok(vec![home.join(".grok").join("config.toml")]),
        // claude-desktop: macOS Claude-3p configLibrary (deployment mode in
        // both claude_desktop_config.json copies + gateway profile + _meta.json)
        #[cfg(target_os = "macos")]
        "claude-desktop" => {
            let app_support = home.join("Library").join("Application Support");
            let threep = app_support.join("Claude-3p");
            Ok(vec![
                app_support
                    .join("Claude")
                    .join("claude_desktop_config.json"),
                threep.join("claude_desktop_config.json"),
                threep.join("configLibrary").join(format!(
                    "{}.json",
                    kiwano_cc_adapters::claude_desktop_config::PROFILE_ID
                )),
                threep.join("configLibrary").join("_meta.json"),
            ])
        }
        #[cfg(not(target_os = "macos"))]
        "claude-desktop" => Err("claude-desktop takeover currently supports macOS only".into()),
        // additive-mode agents: the gateway entry coexists with their native
        // providers, so a missing config is fine (a fresh one gets created)
        "opencode" => Ok(vec![home
            .join(".config")
            .join("opencode")
            .join("opencode.json")]),
        "openclaw" => Ok(vec![home.join(".openclaw").join("openclaw.json")]),
        "hermes" => {
            // HERMES_HOME resolution matches hermes' own get_hermes_home()
            let dir = std::env::var_os("HERMES_HOME")
                .map(|v| v.to_string_lossy().trim().to_string())
                .filter(|v| !v.is_empty())
                .map(std::path::PathBuf::from)
                .unwrap_or_else(|| home.join(".hermes"));
            Ok(vec![dir.join("config.yaml")])
        }
        "pi" => Ok(vec![
            home.join(".pi").join("agent").join("models.json"),
            home.join(".pi").join("agent").join("settings.json"),
        ]),
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
            // codex's auth.json / gemini's .env, every additive agent's
            // config, and all claude-desktop files are allowed to be missing
            // (treated as empty files — every claude-desktop write normalizes
            // a missing/non-object document to {})
            Err(_)
                if matches!(
                    agent,
                    "opencode" | "openclaw" | "hermes" | "pi" | "claude-desktop"
                ) || p.ends_with("auth.json")
                    || p.ends_with(".env") =>
            {
                String::new()
            }
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
        "grokbuild" => rewrite_grok_toml(original, base, data_port, key),
        // claude-desktop: flip both config copies to 3p mode, write the full
        // gateway profile (Kiwano-owned while takeover is active) and register
        // it as the applied configLibrary profile
        "claude-desktop" if path.ends_with("claude_desktop_config.json") => {
            kiwano_cc_adapters::claude_desktop_config::set_deployment_mode(original, "3p")
        }
        "claude-desktop" if path.ends_with("_meta.json") => {
            kiwano_cc_adapters::claude_desktop_config::upsert_meta(original)
        }
        "claude-desktop" => kiwano_cc_adapters::claude_desktop_config::build_gateway_profile(
            &format!("{base}:{data_port}"),
            key,
        ),
        // additive agents: upsert a gateway provider entry and select it;
        // pre-existing provider entries survive (cc_adapters::gateway_takeover)
        "opencode" => kiwano_cc_adapters::gateway_takeover::upsert_opencode_gateway(
            original,
            &format!("{base}:{data_port}/v1"),
            key,
        ),
        "openclaw" => kiwano_cc_adapters::gateway_takeover::upsert_openclaw_gateway(
            original,
            &format!("{base}:{data_port}"),
            key,
        ),
        "hermes" => kiwano_cc_adapters::gateway_takeover::upsert_hermes_gateway(
            original,
            &format!("{base}:{data_port}"),
            key,
        ),
        "pi" if path.ends_with("models.json") => {
            kiwano_cc_adapters::gateway_takeover::upsert_pi_models_gateway(
                original,
                &format!("{base}:{data_port}/v1"),
                key,
            )
        }
        "pi" => kiwano_cc_adapters::gateway_takeover::select_pi_gateway(original),
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
        let after_export = trimmed
            .strip_prefix("export ")
            .unwrap_or(trimmed)
            .trim_start();
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

/// config.toml (Grok Build): point the selected model's base_url at the
/// gateway (/v1), inject the placeholder api_key and pin api_backend to
/// "responses" — the xAI Responses wire reaches the gateway's OpenAI family.
/// Refuse when there is no [models].default / [model."…"] base_url to rewrite
/// (official xAI OAuth setup) — same semantics as codex without base_url.
fn rewrite_grok_toml(original: &str, base: &str, port: u16, key: &str) -> Result<String, String> {
    let target = format!("{base}:{port}/v1");

    // Pass 1: the selected profile name from the [models] table (`default = "…"`)
    let mut profile: Option<String> = None;
    let mut section = String::new();
    for line in original.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            continue;
        }
        if section == "models" {
            if let Some((k, v)) = t.split_once('=') {
                if k.trim() == "default" {
                    let v = v.trim().trim_matches('"').trim();
                    if !v.is_empty() {
                        profile = Some(v.to_string());
                    }
                }
            }
        }
    }
    let Some(profile) = profile else {
        return Err(
            "config.toml 中没有 [models] default —— Grok Build 可能还在使用官方 xAI 登录，请先配置自定义模型再接管"
                .into(),
        );
    };

    // Pass 2: rewrite base_url / api_key / api_backend inside the selected
    // [model."<profile>"] table; other tables and rows stay untouched
    let selected = format!("model.\"{profile}\"");
    let unquoted = format!("model.{profile}");
    let mut found_base = false;
    let mut found_key = false;
    let mut found_backend = false;
    let mut base_idx: Option<usize> = None;
    let mut section = String::new();
    let mut out: Vec<String> = Vec::new();
    for line in original.lines() {
        let t = line.trim();
        if t.starts_with('[') && t.ends_with(']') {
            section = t[1..t.len() - 1].trim().to_string();
            out.push(line.to_string());
            continue;
        }
        if section == selected || section == unquoted {
            if let Some((k, _)) = t.split_once('=') {
                match k.trim() {
                    "base_url" => {
                        found_base = true;
                        base_idx = Some(out.len());
                        out.push(format!("base_url = \"{target}\""));
                        continue;
                    }
                    "api_key" => {
                        found_key = true;
                        out.push(format!("api_key = \"{key}\""));
                        continue;
                    }
                    "api_backend" => {
                        found_backend = true;
                        out.push("api_backend = \"responses\"".into());
                        continue;
                    }
                    _ => {}
                }
            }
        }
        out.push(line.to_string());
    }
    if !found_base {
        return Err(format!(
            "config.toml 中没有 [model.\"{profile}\"] 的 base_url —— 请先为 Grok Build 配置一个自定义模型再接管"
        ));
    }
    // Missing api_key / api_backend rows are inserted right after base_url
    // (they are optional in xAI's config when the profile relies on env_key)
    let mut extra = Vec::new();
    if !found_key {
        extra.push(format!("api_key = \"{key}\""));
    }
    if !found_backend {
        extra.push("api_backend = \"responses\"".into());
    }
    if let Some(i) = base_idx {
        out.splice(i + 1..i + 1, extra);
    }

    let mut s = out.join("\n");
    if original.ends_with('\n') {
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

    #[test]
    fn grok_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(
            grok_dir.join("config.toml"),
            r#"theme = "dark"
[models]
default = "custom-grok"

[model."custom-grok"]
name = "My Grok"
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "xai-old"
api_backend = "chat"
context_window = 131072

[model."other"]
name = "Other"
model = "grok-3"
base_url = "https://relay.example.com/v1"
"#,
        )
        .unwrap();

        enable(&aux, "grokbuild", "kw-ag-grokbuild-abcd", 8317, &home).unwrap();
        let toml = std::fs::read_to_string(grok_dir.join("config.toml")).unwrap();
        // selected model points at the gateway; backend pinned to responses
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("api_key = \"kw-ag-grokbuild-abcd\""));
        assert!(toml.contains("api_backend = \"responses\""));
        // rows outside the selected model table stay untouched
        assert!(toml.contains("https://relay.example.com/v1"));
        assert!(toml.contains("theme = \"dark\""));

        disable(&aux, "grokbuild", &home).unwrap();
        // restore = write back byte for byte
        assert_eq!(
            std::fs::read_to_string(grok_dir.join("config.toml")).unwrap(),
            r#"theme = "dark"
[models]
default = "custom-grok"

[model."custom-grok"]
name = "My Grok"
model = "grok-4.5"
base_url = "https://api.x.ai/v1"
api_key = "xai-old"
api_backend = "chat"
context_window = 131072

[model."other"]
name = "Other"
model = "grok-3"
base_url = "https://relay.example.com/v1"
"#
        );
        assert!(aux.load_takeover_backup("grokbuild").is_none());
    }

    #[test]
    fn grok_missing_api_key_and_backend_are_inserted() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        std::fs::write(
            grok_dir.join("config.toml"),
            "[models]\ndefault = \"p\"\n\n[model.p]\nname = \"P\"\nmodel = \"grok-4.5\"\nbase_url = \"https://api.x.ai/v1\"\nenv_key = \"XAI_API_KEY\"\ncontext_window = 131072\n",
        )
        .unwrap();

        enable(&aux, "grokbuild", "kw-ag-grokbuild-abcd", 8317, &home).unwrap();
        let toml = std::fs::read_to_string(grok_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("api_key = \"kw-ag-grokbuild-abcd\""));
        assert!(toml.contains("api_backend = \"responses\""));
        assert!(toml.contains("env_key = \"XAI_API_KEY\"")); // untouched row preserved
    }

    #[test]
    fn grok_official_oauth_config_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let grok_dir = home.join(".grok");
        std::fs::create_dir_all(&grok_dir).unwrap();
        // official xAI login: no [models]/[model.*] tables at all
        std::fs::write(grok_dir.join("config.toml"), "theme = \"dark\"\n").unwrap();
        assert!(enable(&aux, "grokbuild", "kw-ag-grokbuild-abcd", 8317, &home).is_err());
        // no backup left behind on failure (escape hatch stays clean)
        assert!(aux.load_takeover_backup("grokbuild").is_none());
    }

    #[test]
    fn opencode_takeover_roundtrip_additive() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        let original = r#"{
  "theme": "dark",
  "model": "deepseek/deepseek-chat",
  "provider": { "deepseek": { "npm": "@ai-sdk/openai", "options": { "apiKey": "sk-old" } } }
}"#;
        std::fs::write(dir.join("opencode.json"), original).unwrap();

        enable(&aux, "opencode", "kw-ag-opencode-abcd", 8317, &home).unwrap();
        let v: Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join("opencode.json")).unwrap())
                .unwrap();
        assert_eq!(v["model"], "kiwano-gateway/deepseek-chat");
        assert_eq!(
            v["provider"]["kiwano-gateway"]["options"]["baseURL"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(
            v["provider"]["kiwano-gateway"]["options"]["apiKey"],
            "kw-ag-opencode-abcd"
        );
        assert!(v["provider"]["deepseek"].is_object()); // additive: entry survives

        disable(&aux, "opencode", &home).unwrap();
        // restore = write back byte for byte
        assert_eq!(
            std::fs::read_to_string(dir.join("opencode.json")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("opencode").is_none());
    }

    #[test]
    fn pi_takeover_roundtrip_writes_both_files() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        // pi's files may not exist yet (agent dir is created on takeover)
        enable(&aux, "pi", "kw-ag-pi-abcd", 8317, &home).unwrap();
        let agent = home.join(".pi").join("agent");
        let models: Value =
            serde_json::from_str(&std::fs::read_to_string(agent.join("models.json")).unwrap())
                .unwrap();
        assert_eq!(
            models["providers"]["kiwano-gateway"]["baseUrl"],
            "http://127.0.0.1:8317/v1"
        );
        assert_eq!(
            models["providers"]["kiwano-gateway"]["apiKey"],
            "kw-ag-pi-abcd"
        );
        let settings: Value =
            serde_json::from_str(&std::fs::read_to_string(agent.join("settings.json")).unwrap())
                .unwrap();
        assert_eq!(settings["defaultProvider"], "kiwano-gateway");

        disable(&aux, "pi", &home).unwrap();
        // originals were missing → restored as empty files (backup-verbatim
        // semantics, same as gemini's .env); the CLI recreates its own state
        assert_eq!(
            std::fs::read_to_string(agent.join("models.json")).unwrap(),
            ""
        );
        assert_eq!(
            std::fs::read_to_string(agent.join("settings.json")).unwrap(),
            ""
        );
        assert!(aux.load_takeover_backup("pi").is_none());
    }

    #[test]
    fn hermes_takeover_roundtrip_preserves_untouched_sections() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".hermes");
        std::fs::create_dir_all(&dir).unwrap();
        let original = "agent:\n  max_turns: 50\ncustom_providers:\n  - name: openrouter\n    base_url: https://openrouter.ai/api/v1\n";
        std::fs::write(dir.join("config.yaml"), original).unwrap();

        enable(&aux, "hermes", "kw-ag-hermes-abcd", 8317, &home).unwrap();
        let out = std::fs::read_to_string(dir.join("config.yaml")).unwrap();
        let v: serde_yaml::Value = serde_yaml::from_str(&out).unwrap();
        assert_eq!(v["model"]["provider"], "kiwano-gateway");
        assert_eq!(v["agent"]["max_turns"], 50);
        let providers = v["custom_providers"].as_sequence().unwrap();
        assert_eq!(providers.len(), 2);

        disable(&aux, "hermes", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(dir.join("config.yaml")).unwrap(),
            original
        );
        assert!(aux.load_takeover_backup("hermes").is_none());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn claude_desktop_takeover_roundtrip_writes_profile() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let app_support = home.join("Library").join("Application Support");
        let normal_config = app_support
            .join("Claude")
            .join("claude_desktop_config.json");
        std::fs::create_dir_all(normal_config.parent().unwrap()).unwrap();
        let original = r#"{"deploymentMode":"1p","autoUpdater":true}"#;
        std::fs::write(&normal_config, original).unwrap();
        // Claude-3p side (config, profile, _meta.json) is absent: takeover
        // must create it from scratch

        enable(
            &aux,
            "claude-desktop",
            "kw-ag-claude-desktop-abcd",
            8317,
            &home,
        )
        .unwrap();

        let normal: Value =
            serde_json::from_str(&std::fs::read_to_string(&normal_config).unwrap()).unwrap();
        assert_eq!(normal["deploymentMode"], "3p");
        assert_eq!(normal["autoUpdater"], true); // untouched key survives

        let threep: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("claude_desktop_config.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(threep["deploymentMode"], "3p");

        let profile: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("configLibrary")
                    .join("00000000-0000-4000-8000-000000157210.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(profile["inferenceProvider"], "gateway");
        assert_eq!(profile["inferenceGatewayBaseUrl"], "http://127.0.0.1:8317");
        assert_eq!(
            profile["inferenceGatewayApiKey"],
            "kw-ag-claude-desktop-abcd"
        );
        assert_eq!(profile["inferenceModels"].as_array().unwrap().len(), 4);

        let meta: Value = serde_json::from_str(
            &std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("configLibrary")
                    .join("_meta.json"),
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(meta["appliedId"], "00000000-0000-4000-8000-000000157210");

        disable(&aux, "claude-desktop", &home).unwrap();
        // restore = write back byte for byte (absent originals → empty files)
        assert_eq!(std::fs::read_to_string(&normal_config).unwrap(), original);
        assert_eq!(
            std::fs::read_to_string(
                app_support
                    .join("Claude-3p")
                    .join("claude_desktop_config.json")
            )
            .unwrap(),
            ""
        );
        assert!(aux.load_takeover_backup("claude-desktop").is_none());
    }
}
