//! The last tier of a restore: take our route out of the live config when there
//! is no backup to write back and no provider to rebuild from. It cannot
//! restore credentials — it exists so the agent is not left pointed at a
//! loopback port nothing is listening on.

use crate::detect::ShellVars;
use crate::takeover::paths::takeover_paths;
use crate::takeover::state::gateway_token;
use kiwano_adapters::codex_config;
use kiwano_adapters::config::atomic_write_private;
use serde_json::Value;
use std::path::Path;

/// Take the gateway route out of every live file of `agent`, leaving the agent
/// on its own defaults. Reached only when there is no backup and no provider to
/// rebuild from, so it cannot restore credentials — it exists so the agent is
/// not left pointed at a loopback port nothing is listening on.
pub(crate) fn strip_gateway_route(
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<(), String> {
    for path in takeover_paths(agent, home, vars)? {
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(content) = strip_file(agent, &path, &text)? {
            atomic_write_private(&path, content.as_bytes())?;
        }
    }
    Ok(())
}

/// The route-removal transform for one file. `None` = nothing of ours in it.
fn strip_file(agent: &str, path: &Path, text: &str) -> Result<Option<String>, String> {
    if agent == "codex" && path.ends_with("config.toml") {
        return codex_config::remove_codex_gateway_route(text).map_err(|e| e.to_string());
    }
    if agent == "codex" {
        // auth.json: drop the placeholder key, keep the user's other fields.
        return json_without_gateway_key(text, &["OPENAI_API_KEY"], agent);
    }
    match agent {
        "claude" => json_without_gateway_key(
            text,
            &["env.ANTHROPIC_AUTH_TOKEN", "env.ANTHROPIC_BASE_URL"],
            agent,
        ),
        "gemini" => {
            dotenv_without_gateway_keys(text, &["GEMINI_API_KEY", "GOOGLE_GEMINI_BASE_URL"])
        }
        "grokbuild" => Ok(drop_gateway_toml_lines(text, agent)),
        other => Err(format!(
            "no fallback route for {other}: its gateway entry cannot be removed without a backup"
        )),
    }
}

/// Remove the named JSON paths when they hold one of our values. Each entry is
/// either a top-level `key` or a nested `object.key`; a value counts as ours
/// when it is a gateway placeholder token or a loopback gateway URL.
/// Drop dotenv lines whose value is one of ours.
///
/// Line-level and lossless elsewhere: comments, blank lines and the user's own
/// variables stay exactly where they were. Handles `export ` prefixes, and
/// treats a quoted value as the value.
fn dotenv_without_gateway_keys(text: &str, keys: &[&str]) -> Result<Option<String>, String> {
    let mut changed = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let after_export = line
                .trim_start()
                .strip_prefix("export ")
                .unwrap_or(line.trim_start());
            let Some((key, value)) = after_export.split_once('=') else {
                return true;
            };
            let ours = keys.contains(&key.trim())
                && value_is_ours(value.trim().trim_matches(['"', '\'']), "");
            if ours {
                changed = true;
            }
            !ours
        })
        .collect();
    if !changed {
        return Ok(None);
    }
    Ok(Some(kept.join("\n")))
}

fn json_without_gateway_key(
    text: &str,
    paths: &[&str],
    agent: &str,
) -> Result<Option<String>, String> {
    if text.trim().is_empty() {
        return Ok(None);
    }
    let mut v: Value = serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
    let mut changed = false;
    for path in paths {
        let (head, tail) = match path.split_once('.') {
            Some((head, tail)) => (head, Some(tail)),
            None => (*path, None),
        };
        let target = match tail {
            Some(_) => match v.get_mut(head).and_then(|v| v.as_object_mut()) {
                Some(obj) => obj,
                None => continue,
            },
            None => match v.as_object_mut() {
                Some(obj) => obj,
                None => continue,
            },
        };
        let key = tail.unwrap_or(head);
        let is_ours = target
            .get(key)
            .and_then(|v| v.as_str())
            .is_some_and(|value| value_is_ours(value, agent));
        if is_ours {
            target.remove(key);
            changed = true;
        }
    }
    if !changed {
        return Ok(None);
    }
    // Re-serialized rather than edited in place: this is the last resort for a
    // config we can no longer reconstruct, and a well-formed file beats a
    // preserved layout. (The backup path never reformats anything.)
    serde_json::to_string_pretty(&v)
        .map(Some)
        .map_err(|e| e.to_string())
}

/// Drop TOML lines whose value is one of ours (the grok strip: the selected
/// model's `base_url` / `api_key` rows). Line-level on purpose — the file is
/// otherwise left exactly as the user wrote it, comments included.
fn drop_gateway_toml_lines(text: &str, agent: &str) -> Option<String> {
    let mut changed = false;
    let kept: Vec<&str> = text
        .lines()
        .filter(|line| {
            let Some((key, value)) = line.split_once('=') else {
                return true;
            };
            let key = key.trim();
            if key != "base_url" && key != "api_key" {
                return true;
            }
            let ours = value_is_ours(value.trim().trim_matches(['"', '\'']).trim(), agent);
            if ours {
                changed = true;
            }
            !ours
        })
        .collect();
    changed.then(|| kept.join("\n"))
}

/// Whether a config value is one of ours: a gateway placeholder token, or a
/// URL pointing at the loopback gateway a takeover writes.
fn value_is_ours(value: &str, agent: &str) -> bool {
    codex_config::is_loopback_gateway_url(value)
        || (!agent.is_empty() && gateway_token(value, agent).is_some())
        || value.starts_with(codex_config::GATEWAY_PLACEHOLDER_PREFIX)
}
