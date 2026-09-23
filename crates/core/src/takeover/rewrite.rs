//! Every per-agent transform: the dispatch that picks one by name and path,
//! codex's pair (transformed together, because the gateway key has to reach
//! `config.toml` where Codex 0.149 reads it), and the text functions behind the
//! rest of the arms.

use crate::takeover::backup::{BackupFile, Files};
use crate::takeover::enable::gateway_target;
use kiwano_adapters::codex_config;
use serde_json::Value;

/// Codex's two live files, transformed together: the ported module needs both
/// (the plan judges the config text *against the key it carries*) and the
/// gateway key has to reach `config.toml` where Codex 0.149 actually reads a
/// provider's credentials.
pub(crate) fn codex_rewrites(
    originals: &[BackupFile],
    placeholder_key: &str,
    data_port: u16,
) -> Result<Files, String> {
    let config = originals
        .iter()
        .find(|o| o.path.ends_with("config.toml"))
        .expect("takeover_paths always lists codex's config.toml");
    let gateway = gateway_target("codex", data_port);
    let plan =
        codex_config::plan_codex_takeover_live_write(&config.content, placeholder_key, &gateway)
            .map_err(|e| e.to_string())?;
    let config_text = plan.config_text.unwrap_or_else(|| config.content.clone());

    originals
        .iter()
        .map(|original| {
            let content = if original.path.ends_with("config.toml") {
                config_text.clone()
            } else {
                merge_codex_placeholder_auth(&original.content, placeholder_key)?
            };
            Ok((original.path.clone(), content))
        })
        .collect()
}

/// Merge the placeholder key into Codex's `auth.json`, keeping every other
/// field.
///
/// The module only owns `config.toml` (upstream's provider-scoped token);
/// Kiwano writes the key here too because that is where every Codex release
/// before 0.48 — and any reader of the ambient login — looks for it, and
/// because the backup restores the file byte-for-byte either way.
pub(crate) fn merge_codex_placeholder_auth(original: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(original).map_err(|e| format!("auth.json is not valid JSON: {e}"))?
    };
    let obj = v
        .as_object_mut()
        .ok_or("auth.json top level is not an object")?;
    obj.insert("OPENAI_API_KEY".into(), Value::String(key.into()));
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// One file's rewritten content. `target` is the full URL the agent's base_url
/// should hold — the gateway (with the agent's own path suffix) during a
/// takeover, or a provider's own endpoint during a rebuild. `now` is only read
/// by the agents whose config records when it was written (Cline's
/// `updatedAt`); the rest ignore it. Codex never comes through here: its two
/// files are transformed together by [`codex_rewrites`] onto the ported gate
/// module.
/// `sibling_config` is the content of the agent's primary config file, for
/// the one secondary file that needs its context: openclaw's catalogue entry
/// pins the session-affinity flag to the model id the primary config selects,
/// so the two files have to be rewritten against the same selector. Every
/// other arm ignores it, and `None` is what an agent with a single file gets.
pub(crate) fn rewrite(
    agent: &str,
    path: &str,
    original: &str,
    target: &str,
    key: &str,
    now: &str,
    sibling_config: Option<&str>,
) -> Result<String, String> {
    match agent {
        "claude" => rewrite_claude(original, target, key),
        // aider is exclusive like claude: its custom endpoint *is* the three
        // global fields (`openai-api-base`/`openai-api-key`/`model`), not a
        // provider list — the rewrite overwrites them rather than upserting an
        // entry beside anything, and restore puts the user's bytes back.
        "aider" => kiwano_adapters::gateway_takeover::upsert_aider_gateway(original, target, key),
        "codex" => Err("codex files are rewritten together by codex_rewrites".into()),
        "gemini" => rewrite_gemini_env(original, target, key),
        "grokbuild" => rewrite_grok_toml(original, target, key),
        // claude-desktop: flip both config copies to 3p mode, write the full
        // gateway profile (Kiwano-owned while takeover is active) and register
        // it as the applied configLibrary profile
        "claude-desktop" => claude_desktop_rewrite(path, original, target, key),
        // additive agents: upsert a gateway provider entry and select it;
        // pre-existing provider entries survive (adapters::gateway_takeover)
        "opencode" | "openclaw" | "hermes" | "pi" | "workbuddy" | "codebuddy" | "qwen" | "kimi"
        | "cline" | "mimo" | "mcode" | "continue" | "crush" => {
            additive_rewrite(agent, path, original, target, key, now, sibling_config)
        }
        _ => Err("unsupported agent".into()),
    }
}

/// claude-desktop's four files, one rewrite each: the deployment mode lives in
/// both `claude_desktop_config.json` copies, `_meta.json` records which profile
/// is applied, and the profile document itself carries the target and the key.
fn claude_desktop_rewrite(
    path: &str,
    original: &str,
    target: &str,
    key: &str,
) -> Result<String, String> {
    if path.ends_with("claude_desktop_config.json") {
        return kiwano_adapters::claude_desktop_config::set_deployment_mode(original, "3p");
    }
    if path.ends_with("_meta.json") {
        return kiwano_adapters::claude_desktop_config::upsert_meta(original);
    }
    kiwano_adapters::claude_desktop_config::build_gateway_profile(target, key)
}

/// The additive agents, whose gateway entry coexists with their native
/// providers (so a missing config is fine — a fresh one gets created). Each
/// delegates to the adapter that knows the agent's own schema; for openclaw,
/// pi and kimi one secondary file differs from the main config, and the path is
/// the only thing that tells the two apart.
fn additive_rewrite(
    agent: &str,
    path: &str,
    original: &str,
    target: &str,
    key: &str,
    now: &str,
    sibling_config: Option<&str>,
) -> Result<String, String> {
    match agent {
        "opencode" => {
            kiwano_adapters::gateway_takeover::upsert_opencode_gateway(original, target, key)
        }
        "openclaw" if path.ends_with("models.json") => {
            kiwano_adapters::gateway_takeover::upsert_openclaw_models_json(
                original,
                sibling_config.unwrap_or(""),
                target,
                key,
            )
        }
        "openclaw" => {
            kiwano_adapters::gateway_takeover::upsert_openclaw_gateway(original, target, key)
        }
        "hermes" => kiwano_adapters::gateway_takeover::upsert_hermes_gateway(original, target, key),
        "pi" if path.ends_with("models.json") => {
            kiwano_adapters::gateway_takeover::upsert_pi_models_gateway(original, target, key)
        }
        "pi" => kiwano_adapters::gateway_takeover::select_pi_gateway(original),
        // WorkBuddy's model list is a bare array; CodeBuddy's is an object with
        // a picker list beside it. Both name their provider by URL per model
        // row, so the transforms take over one row rather than adding a second
        // with the same id (see adapters::gateway_takeover).
        "workbuddy" => {
            kiwano_adapters::gateway_takeover::upsert_workbuddy_gateway(original, target, key)
        }
        "codebuddy" => kiwano_adapters::gateway_takeover::upsert_codebuddy_models_gateway(
            original, target, key,
        ),
        "qwen" => kiwano_adapters::gateway_takeover::upsert_qwen_gateway(original, target, key),
        // MiMo Code pins its custom provider id to `custom` (its own docs), so
        // the upsert fills that slot rather than `kiwano-gateway` — see the
        // adapter for the why.
        "mimo" => kiwano_adapters::gateway_takeover::upsert_mimo_gateway(original, target, key),
        // MiniMax Code's shape is read off its shipped CLI (the `mcode provider
        // add` fields, YAML), not its docs — see the adapter for the source.
        "mcode" => kiwano_adapters::gateway_takeover::upsert_mcode_gateway(original, target, key),
        // Continue has no selector field — its active model is the first
        // chat-role one — so the upsert unshifts the gateway entry at the head
        // of `models` scoped to `roles: [chat]` (see the adapter for the why).
        "continue" => {
            kiwano_adapters::gateway_takeover::upsert_continue_gateway(original, target, key)
        }
        // Crush selects through the `models.large` slot its own `model large`
        // command persists, so the upsert fills that slot rather than adding a
        // second (the `small` summarization slot stays).
        "crush" => kiwano_adapters::gateway_takeover::upsert_crush_gateway(original, target, key),
        // The two Kimi generations spell the same OpenAI shape differently:
        // the successor calls it `openai`, the Python original `kimi` — the
        // type whose dispatcher attaches the conversation's prompt_cache_key
        // (its session marker on the wire; `openai_legacy` would speak the
        // identical protocol but send no session). The path is the only thing
        // that tells them apart.
        "kimi" => {
            let provider_type = if path.contains(".kimi-code") {
                "openai"
            } else {
                "kimi"
            };
            kiwano_adapters::gateway_takeover::upsert_kimi_gateway(
                original,
                target,
                key,
                provider_type,
            )
        }
        // Cline keeps one entry per provider slot and selects by provider id,
        // so the takeover replaces the slot its own custom-endpoint option
        // writes to rather than adding one beside it (see the adapter).
        "cline" => {
            kiwano_adapters::gateway_takeover::upsert_cline_gateway(original, target, key, now)
        }
        _ => Err("unsupported agent".into()),
    }
}

/// settings.json: merge env.ANTHROPIC_BASE_URL / ANTHROPIC_AUTH_TOKEN, keeping all other fields.
fn rewrite_claude(original: &str, target: &str, key: &str) -> Result<String, String> {
    let mut v: Value = if original.trim().is_empty() {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_str(original)
            .map_err(|e| format!("settings.json is not valid JSON: {e}"))?
    };
    let obj = v
        .as_object_mut()
        .ok_or("settings.json top level is not an object")?;
    let env = obj
        .entry("env")
        .or_insert_with(|| Value::Object(serde_json::Map::new()));
    let env = env
        .as_object_mut()
        .ok_or("settings.json env is not an object")?;
    env.insert("ANTHROPIC_BASE_URL".into(), Value::String(target.into()));
    env.insert("ANTHROPIC_AUTH_TOKEN".into(), Value::String(key.into()));
    serde_json::to_string_pretty(&v).map_err(|e| e.to_string())
}

/// .env (dotenv): rewrite the `GOOGLE_GEMINI_BASE_URL` and `GEMINI_API_KEY`
/// lines, keeping every other line and comment where it was; a variable the
/// file does not have is appended at the end, and a missing file is treated as
/// empty (a first takeover creates `~/.gemini` along with it). Values are not
/// quoted — neither contains whitespace.
fn rewrite_gemini_env(original: &str, target: &str, key: &str) -> Result<String, String> {
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
fn rewrite_grok_toml(original: &str, target: &str, key: &str) -> Result<String, String> {
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
            "no [models] default in config.toml — Grok Build may still be signed in to official xAI; configure a custom model before takeover"
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
            "no base_url under [model.\"{profile}\"] in config.toml — configure a custom model for Grok Build before takeover"
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
