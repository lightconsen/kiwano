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
//! `kiwano_adapters::gateway_takeover` — and claude-desktop (macOS only:
//! deploymentMode 3p + a gateway profile in the Claude-3p configLibrary via
//! `kiwano_adapters::claude_desktop_config`). Pre-existing provider
//! entries survive, and disable still restores the original bytes verbatim.
//!
//! # Honesty rules (borrowed in concept from cc-switch's takeover/codex paths;
//! this file is Kiwano-authored, so it carries no derived-from header)
//!
//! Three things can claim a takeover: this module's backup row, the
//! `placeholder_keys` table, and the agent's live config. They can disagree —
//! a rolled-back key registration over a rewritten config, a delete that
//! failed, a `~/.codex` restored by hand — so **state is derived from
//! evidence, never from the key table**: [`live_placeholder_key`] reads the
//! route out of the live file and [`restorable_backup`] validates the backup,
//! and `vm::build_settings` believes the files.
//!
//! [`enable`] refuses before it touches anything (the Codex gates in
//! `kiwano_adapters::codex_config` run first, on the text that would be
//! written) and rolls its own writes back if one of them fails, so a refused
//! or half-done takeover leaves neither a backup row nor a rewritten file to
//! mislead the reader.
//!
//! [`disable`] degrades instead of stranding: the backup if it is usable,
//! otherwise a rebuild from the provider the gateway serves, otherwise
//! stripping the gateway route out of the live config. Losing the backup never
//! leaves an agent pointed at a dead loopback port, and never reports a
//! success it did not achieve — a restore that could not give the original
//! config back says so.

use std::path::Path;

use kiwano_adapters::codex_config;
use kiwano_adapters::config::atomic_write_private;
use serde_json::Value;

use crate::vm::Aux;

/// The set of backed-up files: `(absolute path, original content)`.
/// The rewritten (path, new content) pairs a takeover writes to disk.
type Files = Vec<(String, String)>;

/// Agents whose config holds one provider slot, so a stored provider can be
/// rebuilt into it verbatim. The additive agents' `kiwano-gateway` entry and
/// Claude Desktop's configLibrary profile are kiwano-authored projections with
/// no faithful provider-side rebuild, so they fall through to the strip tier.
pub const REBUILDABLE_AGENTS: [&str; 4] = ["claude", "codex", "gemini", "grokbuild"];

/// The local gateway's origin. The port comes from the caller (the sidecar's
/// data port), and the per-agent path suffix from [`gateway_target`].
const GATEWAY_HOST: &str = "http://127.0.0.1";

/// One config file captured by a takeover.
///
/// `existed` separates "was empty" from "was not there". Without it, disabling
/// a takeover writes an empty file back for a config the agent never had —
/// leaving a 0-byte `opencode.json` behind, which the agent may read as a
/// broken config rather than as no config at all.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct BackupFile {
    pub path: String,
    pub content: String,
    pub existed: bool,
}

/// The files a takeover rewrites for `agent`, rooted at the caller's `home`.
///
/// Path reconciliation with `kiwano_adapters::codex_config`: that module
/// resolves `~/.codex` itself as `get_codex_config_dir()` (which honors the
/// `CC_SWITCH_TEST_HOME` override), while this file takes the home directory as
/// a parameter because every test injects a temp root and the app passes
/// `$HOME`. In production the two resolve to the same directory, and the split
/// is deliberate rather than accidental: the Codex *transforms* the module now
/// owns are pure text functions with no paths in them, and the one place that
/// touches `~/.codex` on disk is `enable`/`disable` below, which writes through
/// the caller's root. A future divergence shows up as a refused write, not a
/// silent one into the wrong tree.
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
                    kiwano_adapters::claude_desktop_config::PROFILE_ID
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

/// Takeover: read the original files → perform all rewrites in memory
/// (can fail as a whole, zero side effects) → back up → atomic write.
///
/// The rewrites come first so a refusal (the Codex gates live in the rewrite)
/// costs nothing; if a *write* then fails part-way, every file this call
/// replaced is put back and the backup row it created is dropped, so neither a
/// half-rewritten config nor a backup row can describe a takeover that did not
/// happen.
pub fn enable(
    aux: &Aux,
    agent: &str,
    placeholder_key: &str,
    data_port: u16,
    home: &Path,
) -> Result<(), String> {
    let paths = takeover_paths(agent, home)?;
    let mut originals: Vec<BackupFile> = Vec::new();
    for p in &paths {
        // codex's auth.json / gemini's .env, every additive agent's config, and
        // all claude-desktop files are allowed to be missing (treated as empty
        // files — every claude-desktop write normalizes a missing/non-object
        // document to {}). `existed` records which of the two it was, so
        // disabling can put the directory back the way it found it.
        let (content, existed) = match std::fs::read_to_string(p) {
            Ok(c) => (c, true),
            Err(_)
                if matches!(
                    agent,
                    "opencode" | "openclaw" | "hermes" | "pi" | "claude-desktop"
                ) || p.ends_with("auth.json")
                    || p.ends_with(".env") =>
            {
                (String::new(), false)
            }
            Err(_) => {
                return Err(format!(
                    "{} not found — run {} at least once before takeover",
                    p.display(),
                    agent
                ))
            }
        };
        originals.push(BackupFile {
            path: p.to_string_lossy().into_owned(),
            content,
            existed,
        });
    }

    let rewritten = compute_rewrites(agent, &originals, placeholder_key, data_port)?;

    // Back up only on first takeover; repeated enable keeps the original
    // text (escape-hatch semantics)
    let first_time = aux.load_takeover_backup(agent).is_none();
    if first_time {
        aux.save_takeover_backup(agent, &originals)
            .map_err(|e| e.to_string())?;
        copy_files(agent, home, &originals);
    }

    let mut written: Vec<usize> = Vec::with_capacity(rewritten.len());
    for (index, (path, content)) in rewritten.iter().enumerate() {
        // On first takeover ~/.gemini may not exist at all, so create the parent dir first
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Err(e) = atomic_write_private(Path::new(path), content.as_bytes()) {
            rollback_writes(&rewritten, &written, &originals);
            if first_time {
                let _ = aux.delete_takeover_backup(agent);
            }
            return Err(format!(
                "{}: {e} — the takeover was not applied",
                rewritten[index].0
            ));
        }
        written.push(index);
    }
    Ok(())
}

/// The new contents of every file a takeover touches, computed entirely in
/// memory. Codex goes through the ported gate module (which may refuse the
/// config before anything on disk has changed); every other agent keeps its
/// line-level rewriter, now handed the full target URL rather than a host+port
/// pair so the same code can later point an agent at an upstream provider.
fn compute_rewrites(
    agent: &str,
    originals: &[BackupFile],
    placeholder_key: &str,
    data_port: u16,
) -> Result<Files, String> {
    if agent == "codex" {
        return codex_rewrites(originals, placeholder_key, data_port);
    }
    let target = gateway_target(agent, data_port);
    originals
        .iter()
        .map(|original| {
            Ok((
                original.path.clone(),
                rewrite(
                    agent,
                    &original.path,
                    &original.content,
                    &target,
                    placeholder_key,
                )?,
            ))
        })
        .collect()
}

/// The URL a takeover writes for `agent`: the gateway origin plus the path
/// suffix that agent's base_url carries (the Responses-speaking agents append
/// `/v1`; the Anthropic/Gemini-shaped ones take the origin bare).
fn gateway_target(agent: &str, data_port: u16) -> String {
    let suffix = match agent {
        "codex" | "grokbuild" | "opencode" | "pi" => "/v1",
        _ => "",
    };
    format!("{GATEWAY_HOST}:{data_port}{suffix}")
}

/// Put the files this call already replaced back the way they were. Best
/// effort by nature — it runs on the failure path — but it is what keeps a
/// partial write from leaving the reader a config that half-exists.
fn rollback_writes(rewritten: &Files, written: &[usize], originals: &[BackupFile]) {
    for index in written.iter().rev() {
        let Some((path, _)) = rewritten.get(*index) else {
            continue;
        };
        let Some(original) = originals.iter().find(|o| &o.path == path) else {
            continue;
        };
        let path = Path::new(path);
        if original.existed {
            let _ = atomic_write_private(path, original.content.as_bytes());
        } else {
            let _ = std::fs::remove_file(path);
        }
    }
}

/// Codex's two live files, transformed together: the ported module needs both
/// (the plan judges the config text *against the key it carries*) and the
/// gateway key has to reach `config.toml` where Codex 0.149 actually reads a
/// provider's credentials.
fn codex_rewrites(
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
fn merge_codex_placeholder_auth(original: &str, key: &str) -> Result<String, String> {
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

/// What a restore actually did.
#[derive(Debug, PartialEq, Eq)]
pub enum RestoreOutcome {
    /// Nothing of ours was found in the live config: the agent was not taken
    /// over (or something put it back by hand).
    NotTakenOver,
    /// The original files were written back byte for byte from the backup.
    RestoredFromBackup,
    /// No usable backup: the live config was rebuilt from the provider the
    /// gateway serves for this agent.
    RebuiltFromProvider,
}

/// The result of a restore, plus the one thing that is a warning rather than a
/// failure: the restore itself succeeded but a follow-up cleanup step did not.
#[derive(Debug)]
pub struct RestoreReport {
    pub outcome: RestoreOutcome,
    /// Never an error the caller should fail on — the agent is already whole.
    pub warning: Option<String>,
}

/// The provider the gateway routes for an agent, used to rebuild a live config
/// whose backup is gone.
#[derive(Debug, Clone)]
pub struct ProviderRoute {
    pub base_url: String,
    pub api_key: String,
}

/// Whether a takeover backup exists that restore can actually write back.
///
/// A backup captured while the config was *already* taken over is a projection
/// of the loopback route, not the user's original config; writing it back
/// would re-install what restore exists to remove. Those rows do not count as
/// evidence of a healthy takeover.
pub fn restorable_backup(aux: &Aux, agent: &str) -> bool {
    let Some((_, files)) = aux.load_takeover_backup(agent) else {
        return false;
    };
    !files
        .iter()
        .any(|file| placeholder_in_file(Path::new(&file.path), &file.content, agent).is_some())
}

/// The placeholder key the agent's live config actually carries, if any.
///
/// This is the evidence half of takeover state: recognition is by the
/// `kw-ag-<agent>-<rand>` shape (or, for a Codex config, the parsed token
/// slots), never by a lookup in the `placeholder_keys` table, which can be
/// rolled back while the files stay rewritten.
pub fn live_placeholder_key(agent: &str, home: &Path) -> Option<String> {
    let paths = takeover_paths(agent, home).ok()?;
    for path in &paths {
        let Ok(text) = std::fs::read_to_string(path) else {
            continue;
        };
        if let Some(key) = placeholder_in_file(path, &text, agent) {
            return Some(key);
        }
    }
    None
}

/// Our placeholder key inside one file's text. A Codex `config.toml` is read
/// through the module's parsed detector; everything else (a Codex `auth.json`,
/// Claude's `settings.json`, gemini's `.env`, the additive agents' JSON/YAML)
/// is a value scan, because all of them hold the key as a string value.
fn placeholder_in_file(path: &Path, text: &str, agent: &str) -> Option<String> {
    if agent == "codex" && path.ends_with("config.toml") {
        return codex_config::codex_config_placeholder_key(text);
    }
    gateway_token(text, agent)
}

/// The first `kw-ag-<agent>-<rand>` token in `text`. The leading boundary keeps
/// a token that merely *contains* the prefix (a URL path, a longer id) from
/// reading as ours.
fn gateway_token(text: &str, agent: &str) -> Option<String> {
    let needle = format!("{}{agent}-", codex_config::GATEWAY_PLACEHOLDER_PREFIX);
    let mut rest = text;
    while let Some(at) = rest.find(&needle) {
        let at_boundary = rest[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !c.is_ascii_alphanumeric() && c != '-' && c != '_' && c != '.');
        let token: String = rest[at..]
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric() || *c == '-')
            .collect();
        if at_boundary && token.len() > needle.len() {
            return Some(token);
        }
        rest = &rest[at + needle.len()..];
    }
    None
}

/// Restore: put every captured file back the way it was, then deregister the
/// backup. A file that did not exist before the takeover is removed rather than
/// written empty.
///
/// Degradation chain when the backup is missing or itself a loopback
/// projection: rebuild the route from `fallback` (the provider the gateway
/// serves), and failing that strip the gateway route out of the live config so
/// the agent is never left pointed at a dead loopback port.
///
/// `Err` means the agent could not be put back: either a file write failed
/// mid-restore, or no original configuration could be recovered at all (the
/// route is removed first either way, so "could not restore" never also means
/// "still stranded"). The caller must not report success on it.
pub fn disable(
    aux: &Aux,
    agent: &str,
    home: &Path,
    fallback: Option<&ProviderRoute>,
) -> Result<RestoreReport, String> {
    // Validate the agent name before anything else; the file paths themselves
    // are resolved by the helpers below (each of which needs the same home).
    takeover_paths(agent, home)?;

    // An unusable backup row is dropped rather than kept: it can never be
    // written back, and its presence is what would keep claiming a takeover.
    let mut warning = None;
    let backup = match aux.load_takeover_backup(agent) {
        Some((_, files))
            if files
                .iter()
                .any(|f| placeholder_in_file(Path::new(&f.path), &f.content, agent).is_some()) =>
        {
            warning = Some(format!(
                "the {agent} takeover backup held a gateway route, not the original config, and was discarded"
            ));
            let _ = aux.delete_takeover_backup(agent);
            None
        }
        other => other.map(|(_, files)| files),
    };

    if let Some(files) = backup {
        for file in &files {
            let path = Path::new(&file.path);
            if file.existed {
                atomic_write_private(path, file.content.as_bytes())
                    .map_err(|e| format!("could not restore {}: {e}", file.path))?;
            } else {
                match std::fs::remove_file(path) {
                    Ok(()) => {}
                    // Already gone: the takeover created it, so this is the same
                    // end state. Anything else (permissions, a directory in the
                    // way) is reported rather than swallowed.
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(e) => return Err(format!("{}: {e}", file.path)),
                }
            }
        }
        // The restore is done; a row that will not go away is untidy, not
        // broken — the next takeover overwrites it and the next restore is
        // idempotent.
        if let Err(e) = aux.delete_takeover_backup(agent) {
            warning = Some(format!(
                "the {agent} config was restored but its backup row could not be dropped: {e}"
            ));
        }
        return Ok(RestoreReport {
            outcome: RestoreOutcome::RestoredFromBackup,
            warning,
        });
    }

    // No usable backup. Only the live files can say whether anything is there
    // to undo.
    if live_placeholder_key(agent, home).is_none() {
        return Ok(RestoreReport {
            outcome: RestoreOutcome::NotTakenOver,
            warning,
        });
    }

    if REBUILDABLE_AGENTS.contains(&agent) {
        if let Some(route) = fallback {
            if rebuild_from_provider(agent, home, route).is_ok() {
                if let Err(e) = aux.delete_takeover_backup(agent) {
                    warning = Some(format!(
                        "the {agent} config was rebuilt but its backup row could not be dropped: {e}"
                    ));
                }
                return Ok(RestoreReport {
                    outcome: RestoreOutcome::RebuiltFromProvider,
                    warning,
                });
            }
        }
    }

    // Last resort: take our route out of the live config, then report the loss
    // honestly. This must not fail silently — the user's upstream credentials
    // are gone even though their agent is no longer pointed at the gateway.
    strip_gateway_route(agent, home).map_err(|e| {
        format!(
            "the {agent} takeover backup is gone and its gateway route could not be removed: {e}"
        )
    })?;
    if live_placeholder_key(agent, home).is_some() {
        return Err(format!(
            "the {agent} gateway route could not be removed from its live config — remove the {} route by hand before using {agent}",
            codex_config::GATEWAY_PLACEHOLDER_PREFIX
        ));
    }
    Err(format!(
        "the {agent} takeover backup is gone: the gateway route was removed so {agent} no longer points at the local gateway, but the original configuration could not be recovered — re-enter {agent}'s provider settings"
    ))
}

/// Rebuild an agent's live config from the provider the gateway serves it:
/// the same rewrite a takeover performs, aimed at the provider's own endpoint
/// and key instead of the gateway and a placeholder.
fn rebuild_from_provider(agent: &str, home: &Path, route: &ProviderRoute) -> Result<(), String> {
    for path in takeover_paths(agent, home)? {
        let Ok(current) = std::fs::read_to_string(&path) else {
            continue;
        };
        let content = if agent == "codex" && path.ends_with("config.toml") {
            let rebuilt = codex_config::rebuild_codex_live_from_provider(
                &current,
                &route.base_url,
                &route.api_key,
            )
            .map_err(|e| e.to_string())?;
            // The provider's table declares its own credential source, so the
            // real key could not replace the placeholder: stripping is the only
            // honest option left.
            if codex_config::codex_config_placeholder_key(&rebuilt).is_some() {
                return Err("the active provider declares its own credentials".into());
            }
            rebuilt
        } else if agent == "codex" {
            merge_codex_placeholder_auth(&current, &route.api_key)?
        } else {
            rewrite(
                agent,
                &path.to_string_lossy(),
                &current,
                &route.base_url,
                &route.api_key,
            )?
        };
        atomic_write_private(&path, content.as_bytes())?;
    }
    Ok(())
}

/// Take the gateway route out of every live file of `agent`, leaving the agent
/// on its own defaults. Reached only when there is no backup and no provider to
/// rebuild from, so it cannot restore credentials — it exists so the agent is
/// not left pointed at a loopback port nothing is listening on.
fn strip_gateway_route(agent: &str, home: &Path) -> Result<(), String> {
    for path in takeover_paths(agent, home)? {
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

/// Drop dotenv lines whose value is one of ours.
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

/// One file's rewritten content. `target` is the full URL the agent's base_url
/// should hold — the gateway (with the agent's own path suffix) during a
/// takeover, or a provider's own endpoint during a rebuild. Codex never comes
/// through here: its two files are transformed together by
/// [`codex_rewrites`] onto the ported gate module.
fn rewrite(
    agent: &str,
    path: &str,
    original: &str,
    target: &str,
    key: &str,
) -> Result<String, String> {
    match agent {
        "claude" => rewrite_claude(original, target, key),
        "codex" => Err("codex files are rewritten together by codex_rewrites".into()),
        "gemini" => rewrite_gemini_env(original, target, key),
        "grokbuild" => rewrite_grok_toml(original, target, key),
        // claude-desktop: flip both config copies to 3p mode, write the full
        // gateway profile (Kiwano-owned while takeover is active) and register
        // it as the applied configLibrary profile
        "claude-desktop" if path.ends_with("claude_desktop_config.json") => {
            kiwano_adapters::claude_desktop_config::set_deployment_mode(original, "3p")
        }
        "claude-desktop" if path.ends_with("_meta.json") => {
            kiwano_adapters::claude_desktop_config::upsert_meta(original)
        }
        "claude-desktop" => {
            kiwano_adapters::claude_desktop_config::build_gateway_profile(target, key)
        }
        // additive agents: upsert a gateway provider entry and select it;
        // pre-existing provider entries survive (adapters::gateway_takeover)
        "opencode" => {
            kiwano_adapters::gateway_takeover::upsert_opencode_gateway(original, target, key)
        }
        "openclaw" => {
            kiwano_adapters::gateway_takeover::upsert_openclaw_gateway(original, target, key)
        }
        "hermes" => kiwano_adapters::gateway_takeover::upsert_hermes_gateway(original, target, key),
        "pi" if path.ends_with("models.json") => {
            kiwano_adapters::gateway_takeover::upsert_pi_models_gateway(original, target, key)
        }
        "pi" => kiwano_adapters::gateway_takeover::select_pi_gateway(original),
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

/// .env (dotenv): rewrite the GOOGLE_GEMINI_BASE_URL and GEMINI_API_KEY
/// lines, keeping all other lines and comments; missing variables are
/// appended at the end; a missing file is treated as empty (a first
/// takeover creates the ~/.gemini directory along with it). Values are not
/// quoted — neither value contains whitespace.
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

// (The local mode-less `atomic_write` this file used to carry is gone: every
// takeover file it wrote holds a credential — an auth token, an API key, a
// placeholder — so all of them go through
// `kiwano_adapters::config::atomic_write_private`, which forces 0600 on Unix.)

/// File-copy fallback: `~/.kiwano/backups/<agent>/<filename>` (an escape hatch beyond SQLite).
///
/// Written with `atomic_write_private` like every other file here: a backup copy
/// of `auth.json` or `settings.json` holds the same credential as the original,
/// so it must not land with the process umask's permissions. (Failure is
/// ignored — the copy is a convenience beside the SQLite row, never the
/// authority, so a copy that cannot be written does not fail the takeover.)
fn copy_files(agent: &str, home: &Path, files: &[BackupFile]) {
    let dir = home.join(".kiwano").join("backups").join(agent);
    for file in files {
        // Nothing to copy for a file the agent never had; an empty copy would
        // read as "their config was empty".
        if !file.existed {
            continue;
        }
        let name = std::path::Path::new(&file.path)
            .file_name()
            .map(|n| n.to_string_lossy().into_owned());
        if let Some(name) = name {
            let _ = atomic_write_private(&dir.join(name), file.content.as_bytes());
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

    /// Restore with no provider to rebuild from. This is the plain "put my
    /// config back" call; the degradation chain's middle tier is exercised by
    /// the tests that pass a [`ProviderRoute`] explicitly.
    fn restore(aux: &Aux, agent: &str, home: &Path) -> Result<RestoreReport, String> {
        disable(aux, agent, home, None)
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
        restore(&aux, "claude", &home).unwrap();
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

        restore(&aux, "claude", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(&settings).unwrap(),
            r#"{"original":true}"#
        );
    }

    /// A Codex config the gates accept: the active provider is named by a
    /// top-level `model_provider` **string** and its table lives under
    /// `[model_providers.<id>]`. The pre-0.48 `[model_provider.custom]` singular
    /// table several older Kiwano fixtures used is not a shape Codex 0.149 reads
    /// at all — there is no active provider for the gateway to route, so the
    /// takeover is refused — which is why the fixture is written this way.
    const CODEX_ORIGINAL: &str = r#"model = "m"
model_provider = "custom"

[model_providers.custom]
name = "DeepSeek"
base_url = "https://api.deepseek.com/v1"
wire_api = "responses"
"#;

    fn write_codex_config(home: &Path, config: &str, auth: &str) -> std::path::PathBuf {
        let codex_dir = home.join(".codex");
        std::fs::create_dir_all(&codex_dir).unwrap();
        std::fs::write(codex_dir.join("config.toml"), config).unwrap();
        std::fs::write(codex_dir.join("auth.json"), auth).unwrap();
        codex_dir
    }

    #[test]
    fn codex_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, r#"{"OPENAI_API_KEY":"sk-old"}"#);

        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap();
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("base_url = \"http://127.0.0.1:8317/v1\""));
        assert!(toml.contains("wire_api = \"responses\"")); // other lines untouched
        assert!(toml.contains("name = \"DeepSeek\"")); // the table keeps its name
        assert!(toml.contains("experimental_bearer_token = \"kw-ag-codex-abcd\""));
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "kw-ag-codex-abcd");

        let report = restore(&aux, "codex", &home).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RestoredFromBackup);
        assert!(report.warning.is_none());
        // Byte for byte, including the `model_provider` selection.
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            CODEX_ORIGINAL
        );
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("auth.json")).unwrap(),
            r#"{"OPENAI_API_KEY":"sk-old"}"#
        );
        assert!(aux.load_takeover_backup("codex").is_none());

        // Restoring again finds nothing of ours and says so instead of failing.
        let again = restore(&aux, "codex", &home).unwrap();
        assert_eq!(again.outcome, RestoreOutcome::NotTakenOver);
    }

    #[test]
    fn codex_without_base_url_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, "model = \"m\"\n", "{}");
        let err = enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap_err();
        assert!(err.contains("custom"), "{err}");
        // The switch did not happen: no backup row, and the live config is the
        // one the user wrote (the gates run before any write).
        assert!(aux.load_takeover_backup("codex").is_none());
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            "model = \"m\"\n"
        );
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("auth.json")).unwrap(),
            "{}"
        );
    }

    #[test]
    fn codex_write_failure_rolls_the_other_file_back() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, "{}");
        // Make the second file unwritable by handing the writer a directory
        // where the file belongs: config.toml is written first and must be put
        // back when auth.json cannot be.
        std::fs::remove_file(codex_dir.join("auth.json")).unwrap();
        std::fs::create_dir(codex_dir.join("auth.json")).unwrap();

        let err = enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap_err();
        assert!(err.contains("the takeover was not applied"), "{err}");
        assert!(aux.load_takeover_backup("codex").is_none());
        assert_eq!(
            std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
            CODEX_ORIGINAL,
            "a half-applied takeover must leave no rewritten config behind"
        );
    }

    #[test]
    fn codex_lost_backup_strips_the_route_and_reports_a_failure() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, r#"{"OPENAI_API_KEY":"sk-old"}"#);
        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap();
        // The SQLite row is the backup restore reads; losing it is the case
        // that used to be reported as a successful "Not taken over".
        aux.delete_takeover_backup("codex").unwrap();
        assert!(!restorable_backup(&aux, "codex"));

        // No provider to rebuild from: strip, and never claim success.
        let err = disable(&aux, "codex", &home, None).unwrap_err();
        assert!(err.contains("backup is gone"), "{err}");
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(
            !toml.contains("127.0.0.1") && !toml.contains("kw-ag-"),
            "the agent must not be left pointing at loopback: {toml}"
        );
        assert!(!toml.contains("model_provider ="), "{toml}");
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap())
                .unwrap();
        assert!(auth.get("OPENAI_API_KEY").is_none(), "{auth}");
    }

    #[test]
    fn codex_lost_backup_rebuilds_from_the_provider() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, r#"{"OPENAI_API_KEY":"sk-old"}"#);
        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home).unwrap();
        aux.delete_takeover_backup("codex").unwrap();

        let route = ProviderRoute {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "codex", &home, Some(&route)).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RebuiltFromProvider);
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(
            toml.contains("base_url = \"https://api.deepseek.com/v1\""),
            "{toml}"
        );
        assert!(
            toml.contains("experimental_bearer_token = \"sk-real\""),
            "{toml}"
        );
        assert!(
            !toml.contains("kw-ag-") && !toml.contains("127.0.0.1"),
            "{toml}"
        );
        let auth: Value =
            serde_json::from_str(&std::fs::read_to_string(codex_dir.join("auth.json")).unwrap())
                .unwrap();
        assert_eq!(auth["OPENAI_API_KEY"], "sk-real");
    }

    #[test]
    fn a_backup_that_holds_the_gateway_route_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        // A config that is *already* taken over (a hand-restored ~/.codex, or a
        // takeover whose backup row vanished): taking it over again captures the
        // loopback route as the "original", and writing that back would
        // re-install exactly what restore exists to remove.
        let codex_dir = write_codex_config(
            &home,
            "model = \"m\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"DeepSeek\"\nbase_url = \"http://127.0.0.1:8317/v1\"\nwire_api = \"responses\"\nexperimental_bearer_token = \"kw-ag-codex-old\"\n",
            r#"{"OPENAI_API_KEY":"kw-ag-codex-old"}"#,
        );
        enable(&aux, "codex", "kw-ag-codex-new", 8317, &home).unwrap();
        assert!(
            !restorable_backup(&aux, "codex"),
            "a loopback projection is not the user's original config"
        );
        assert_eq!(
            live_placeholder_key("codex", &home).as_deref(),
            Some("kw-ag-codex-new")
        );

        let route = ProviderRoute {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "codex", &home, Some(&route)).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RebuiltFromProvider);
        assert!(
            report.warning.unwrap().contains("not the original config"),
            "a discarded backup is a warning, not a silent success"
        );
        assert!(aux.load_takeover_backup("codex").is_none());
        let toml = std::fs::read_to_string(codex_dir.join("config.toml")).unwrap();
        assert!(toml.contains("https://api.deepseek.com/v1"), "{toml}");
        assert!(!toml.contains("127.0.0.1"), "{toml}");
    }

    #[test]
    fn claude_lost_backup_rebuilds_from_the_provider() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","SOMETHING":"kept"}}"#,
        )
        .unwrap();
        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home).unwrap();
        aux.delete_takeover_backup("claude").unwrap();

        let route = ProviderRoute {
            base_url: "https://relay.example.com".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "claude", &home, Some(&route)).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::RebuiltFromProvider);
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(v["env"]["ANTHROPIC_BASE_URL"], "https://relay.example.com");
        assert_eq!(v["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-real");
        assert_eq!(v["env"]["SOMETHING"], "kept"); // user fields survive the rebuild
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn claude_lost_backup_without_a_provider_strips_our_route() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","SOMETHING":"kept"}}"#,
        )
        .unwrap();
        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home).unwrap();
        aux.delete_takeover_backup("claude").unwrap();

        let err = disable(&aux, "claude", &home, None).unwrap_err();
        assert!(err.contains("backup is gone"), "{err}");
        let v: Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert!(v["env"].get("ANTHROPIC_AUTH_TOKEN").is_none(), "{v}");
        assert!(v["env"].get("ANTHROPIC_BASE_URL").is_none(), "{v}");
        assert_eq!(v["env"]["SOMETHING"], "kept"); // strip is surgical
        assert_eq!(v["model"], "opus");
    }

    #[test]
    fn additive_agent_without_a_backup_cannot_be_stripped_and_says_so() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let dir = home.join(".config").join("opencode");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("opencode.json"),
            r#"{"provider":{"deepseek":{"options":{"apiKey":"sk-old"}}}}"#,
        )
        .unwrap();
        enable(&aux, "opencode", "kw-ag-opencode-abcd", 8317, &home).unwrap();
        aux.delete_takeover_backup("opencode").unwrap();

        // No faithful rebuild exists for the additive agents' kiwano-authored
        // entry, and stripping one would need to know which provider the user
        // selected before: the honest answer is a failure, not a silent success.
        let err = disable(&aux, "opencode", &home, None).unwrap_err();
        assert!(err.contains("no fallback route"), "{err}");
    }

    #[test]
    fn live_placeholder_key_reads_the_route_out_of_the_live_file() {
        let (_dir, home) = temp_home();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

        // Nothing written yet: no evidence of a takeover.
        assert!(live_placeholder_key("claude", &home).is_none());

        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8317","ANTHROPIC_AUTH_TOKEN":"kw-ag-claude-abcd"}}"#,
        )
        .unwrap();
        assert_eq!(
            live_placeholder_key("claude", &home).as_deref(),
            Some("kw-ag-claude-abcd")
        );

        // A token that merely contains the prefix (a longer id, a URL path) is
        // not ours: recognition is by the boundary, not by `contains`.
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-kw-ag-claude-notours"}}"#,
        )
        .unwrap();
        assert!(live_placeholder_key("claude", &home).is_none());

        // Another agent's key is not this agent's evidence either.
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"kw-ag-codex-abcd"}}"#,
        )
        .unwrap();
        assert!(live_placeholder_key("claude", &home).is_none());
    }

    #[test]
    fn missing_claude_config_errors() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        assert!(enable(&aux, "claude", "k", 8317, &home).is_err());
        // disable before any takeover succeeds idempotently, and reports that
        // there was nothing of ours to undo rather than a restore it did not do
        let report = restore(&aux, "claude", &home).unwrap();
        assert_eq!(report.outcome, RestoreOutcome::NotTakenOver);
        assert!(report.warning.is_none());
    }

    #[test]
    fn gemini_takeover_roundtrip() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let gemini_dir = home.join(".gemini");
        std::fs::create_dir_all(&gemini_dir).unwrap();
        std::fs::write(
            gemini_dir.join(".env"),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# proxy comment\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n",
        )
        .unwrap();

        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home).unwrap();
        let env = std::fs::read_to_string(gemini_dir.join(".env")).unwrap();
        assert!(env.contains("GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\n"));
        assert!(env.contains("GEMINI_API_KEY=kw-ag-gemini-abcd\n"));
        assert!(env.contains("GOOGLE_GENAI_USE_VERTEXAI=false")); // other lines untouched
        assert!(env.contains("# proxy comment")); // comment preserved

        // restore = write back byte for byte
        restore(&aux, "gemini", &home).unwrap();
        assert_eq!(
            std::fs::read_to_string(gemini_dir.join(".env")).unwrap(),
            "GOOGLE_GENAI_USE_VERTEXAI=false\nGEMINI_API_KEY=AIzaSy-old\n# proxy comment\nGOOGLE_GEMINI_BASE_URL=https://generativelanguage.googleapis.com\n"
        );
        assert!(aux.load_takeover_backup("gemini").is_none());
    }

    #[test]
    fn gemini_takeover_creates_missing_env() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let env_path = home.join(".gemini").join(".env");
        // takeover works even when ~/.gemini/.env is missing entirely (dir + file are created automatically)
        enable(&aux, "gemini", "kw-ag-gemini-abcd", 8317, &home).unwrap();
        let env = std::fs::read_to_string(&env_path).unwrap();
        assert_eq!(
            env,
            "GOOGLE_GEMINI_BASE_URL=http://127.0.0.1:8317\nGEMINI_API_KEY=kw-ag-gemini-abcd\n"
        );

        // Disabling puts the directory back the way it found it: the file we
        // created is removed, not left behind as an empty config (which an
        // agent may read as a broken one).
        restore(&aux, "gemini", &home).unwrap();
        assert!(
            !env_path.exists(),
            "a file created by the takeover must not survive disable"
        );
        assert!(aux.load_takeover_backup("gemini").is_none());
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

        restore(&aux, "grokbuild", &home).unwrap();
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

        restore(&aux, "opencode", &home).unwrap();
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

        restore(&aux, "pi", &home).unwrap();
        // The originals were missing, so restore removes what the takeover
        // created instead of leaving two 0-byte JSON files for the CLI to
        // choke on; the agent recreates its own state.
        assert!(!agent.join("models.json").exists());
        assert!(!agent.join("settings.json").exists());
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

        restore(&aux, "hermes", &home).unwrap();
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

        restore(&aux, "claude-desktop", &home).unwrap();
        // Restore is byte-for-byte for files that existed and removal for files
        // the takeover created: here the whole Claude-3p side was absent.
        assert_eq!(std::fs::read_to_string(&normal_config).unwrap(), original);
        let threep = app_support.join("Claude-3p");
        assert!(!threep.join("claude_desktop_config.json").exists());
        assert!(!threep
            .join("configLibrary")
            .join("00000000-0000-4000-8000-000000157210.json")
            .exists());
        assert!(!threep.join("configLibrary").join("_meta.json").exists());
        assert!(aux.load_takeover_backup("claude-desktop").is_none());
    }
}
