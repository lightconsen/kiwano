//! Restore: put the captured files back, or degrade — rebuild from the provider
//! the gateway serves, or strip the route out of the live config — and never
//! report a restore that did not happen.

use crate::detect::ShellVars;
use crate::takeover::backup::BackupFile;
use crate::takeover::paths::takeover_paths;
use crate::takeover::rewrite::{merge_codex_placeholder_auth, rewrite};
use crate::takeover::state::{
    live_placeholder_key, placeholder_in_file, ProviderRoute, RestoreOutcome, RestoreReport,
};
use crate::takeover::strip::strip_gateway_route;
use crate::vm::Aux;
use kiwano_adapters::codex_config;
use kiwano_adapters::config::atomic_write_private;
use std::path::Path;

/// Agents whose config holds one provider slot, so a stored provider can be
/// rebuilt into it verbatim. The additive agents' `kiwano-gateway` entry and
/// Claude Desktop's configLibrary profile are kiwano-authored projections with
/// no faithful provider-side rebuild, so they fall through to the strip tier.
pub const REBUILDABLE_AGENTS: [&str; 4] = ["claude", "codex", "gemini", "grokbuild"];

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
    vars: &ShellVars,
) -> Result<RestoreReport, String> {
    // Validate the agent name before anything else; the file paths themselves
    // are resolved by the helpers below (each of which needs the same roots).
    takeover_paths(agent, home, vars)?;

    // Tier one: the backup, if it is usable.
    let (backup, warning) = usable_backup(aux, agent);
    if let Some(files) = backup {
        return restore_backup(aux, agent, &files, warning);
    }

    // Tier two: no usable backup left a question only the live files can
    // answer — whether anything of ours is there to undo at all.
    if live_placeholder_key(agent, home, vars).is_none() {
        return Ok(RestoreReport {
            outcome: RestoreOutcome::NotTakenOver,
            warning,
        });
    }

    // Tier three: rebuild the route from the provider the gateway serves.
    if let Some(report) = rebuild_tier(aux, agent, home, fallback, vars, warning) {
        return Ok(report);
    }

    // Tier four: take the route out of the live config and report the loss.
    Err(strip_tier_error(agent, home, vars))
}

/// Tier one: the backup row, vetted. A row that is unusable — one holding a
/// gateway route rather than the original config — is dropped rather than kept:
/// it can never be written back, and its presence is what would keep claiming a
/// takeover. Dropping it is reported as a warning, not an error.
fn usable_backup(aux: &Aux, agent: &str) -> (Option<Vec<BackupFile>>, Option<String>) {
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
    (backup, warning)
}

/// Tier one continued: write the captured files back — a file that did not
/// exist before the takeover is removed rather than written empty — then
/// deregister the backup.
fn restore_backup(
    aux: &Aux,
    agent: &str,
    files: &[BackupFile],
    mut warning: Option<String>,
) -> Result<RestoreReport, String> {
    for file in files {
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
    // The restore is done; a row that will not go away is untidy, not broken —
    // the next takeover overwrites it and the next restore is idempotent.
    if let Err(e) = aux.delete_takeover_backup(agent) {
        warning = Some(format!(
            "the {agent} config was restored but its backup row could not be dropped: {e}"
        ));
    }
    Ok(RestoreReport {
        outcome: RestoreOutcome::RestoredFromBackup,
        warning,
    })
}

/// Tier three: the same rewrite a takeover performs, aimed at the provider's
/// own endpoint and key. `None` means the tier does not apply — the agent has
/// no provider-shaped config to rebuild, no provider was handed in, or the
/// rebuild itself failed — and the caller moves on to stripping. A rebuild that
/// did land also drops the backup row, which is untidy to keep but not a
/// failure to report as one.
fn rebuild_tier(
    aux: &Aux,
    agent: &str,
    home: &Path,
    fallback: Option<&ProviderRoute>,
    vars: &ShellVars,
    mut warning: Option<String>,
) -> Option<RestoreReport> {
    if !REBUILDABLE_AGENTS.contains(&agent) {
        return None;
    }
    let route = fallback?;
    rebuild_from_provider(agent, home, route, vars).ok()?;
    if let Err(e) = aux.delete_takeover_backup(agent) {
        warning = Some(format!(
            "the {agent} config was rebuilt but its backup row could not be dropped: {e}"
        ));
    }
    Some(RestoreReport {
        outcome: RestoreOutcome::RebuiltFromProvider,
        warning,
    })
}

/// Tier four: take our route out of the live config, then report the loss. The
/// message is the answer either way, because this must not fail silently — the
/// user's upstream credentials are gone even though their agent is no longer
/// pointed at the gateway.
fn strip_tier_error(agent: &str, home: &Path, vars: &ShellVars) -> String {
    if let Err(e) = strip_gateway_route(agent, home, vars) {
        return format!(
            "the {agent} takeover backup is gone and its gateway route could not be removed: {e}"
        );
    }
    if live_placeholder_key(agent, home, vars).is_some() {
        return format!(
            "the {agent} gateway route could not be removed from its live config — remove the {} route by hand before using {agent}",
            codex_config::GATEWAY_PLACEHOLDER_PREFIX
        );
    }
    format!(
        "the {agent} takeover backup is gone: the gateway route was removed so {agent} no longer points at the local gateway, but the original configuration could not be recovered — re-enter {agent}'s provider settings"
    )
}

/// Rebuild an agent's live config from the provider the gateway serves it:
/// the same rewrite a takeover performs, aimed at the provider's own endpoint
/// and key instead of the gateway and a placeholder.
fn rebuild_from_provider(
    agent: &str,
    home: &Path,
    route: &ProviderRoute,
    vars: &ShellVars,
) -> Result<(), String> {
    // openclaw's catalogue is rewritten against the main config's selector, and
    // `takeover_paths` puts the main config first, so one pass carries its
    // content to the second file. Whatever the main config holds here — the
    // user's original selector or one a previous takeover already repointed —
    // the model id after the slash is the same.
    let mut openclaw_config: Option<String> = None;
    for path in takeover_paths(agent, home, vars)? {
        let Ok(current) = std::fs::read_to_string(&path) else {
            continue;
        };
        if path.ends_with("openclaw.json") {
            openclaw_config = Some(current.clone());
        }
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
                &crate::vm::rfc3339(crate::vm::unix_now()),
                openclaw_config.as_deref(),
            )?
        };
        atomic_write_private(&path, content.as_bytes())?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::backup::BackupFile;
    use crate::takeover::enable;
    use crate::takeover::restorable_backup;
    use crate::takeover::state::{ProviderRoute, RestoreOutcome};
    use crate::takeover::test_support::{no_vars, temp_home, write_codex_config, CODEX_ORIGINAL};
    use serde_json::Value;

    #[test]
    fn codex_lost_backup_strips_the_route_and_reports_a_failure() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(&home, CODEX_ORIGINAL, r#"{"OPENAI_API_KEY":"sk-old"}"#);
        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home, &no_vars()).unwrap();
        // The SQLite row is the backup restore reads; losing it is the case
        // that used to be reported as a successful "Not taken over".
        aux.delete_takeover_backup("codex").unwrap();
        assert!(!restorable_backup(&aux, "codex"));

        // No provider to rebuild from: strip, and never claim success.
        let err = disable(&aux, "codex", &home, None, &no_vars()).unwrap_err();
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
        enable(&aux, "codex", "kw-ag-codex-abcd", 8317, &home, &no_vars()).unwrap();
        aux.delete_takeover_backup("codex").unwrap();

        let route = ProviderRoute {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "codex", &home, Some(&route), &no_vars()).unwrap();
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

    /// A row an older build could have written: a backup captured while the
    /// config was already routed. Restore writes it back only to re-install the
    /// loopback route it exists to remove, so `disable` discards it — loudly.
    #[test]
    fn a_legacy_backup_that_holds_the_gateway_route_is_rejected() {
        let (_dir, home) = temp_home();
        let aux = Aux::open_in_memory().unwrap();
        let codex_dir = write_codex_config(
            &home,
            "model = \"m\"\nmodel_provider = \"custom\"\n\n[model_providers.custom]\nname = \"DeepSeek\"\nbase_url = \"http://127.0.0.1:8317/v1\"\nwire_api = \"responses\"\nexperimental_bearer_token = \"kw-ag-codex-old\"\n",
            r#"{"OPENAI_API_KEY":"kw-ag-codex-old"}"#,
        );
        aux.save_takeover_backup(
            "codex",
            &[BackupFile {
                path: codex_dir.join("config.toml").display().to_string(),
                content: std::fs::read_to_string(codex_dir.join("config.toml")).unwrap(),
                existed: true,
            }],
        )
        .unwrap();
        assert!(
            !restorable_backup(&aux, "codex"),
            "a loopback projection is not the user's original config"
        );

        let route = ProviderRoute {
            base_url: "https://api.deepseek.com/v1".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "codex", &home, Some(&route), &no_vars()).unwrap();
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
        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home, &no_vars()).unwrap();
        aux.delete_takeover_backup("claude").unwrap();

        let route = ProviderRoute {
            base_url: "https://relay.example.com".into(),
            api_key: "sk-real".into(),
        };
        let report = disable(&aux, "claude", &home, Some(&route), &no_vars()).unwrap();
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
        enable(&aux, "claude", "kw-ag-claude-abcd", 8317, &home, &no_vars()).unwrap();
        aux.delete_takeover_backup("claude").unwrap();

        let err = disable(&aux, "claude", &home, None, &no_vars()).unwrap_err();
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
        enable(
            &aux,
            "opencode",
            "kw-ag-opencode-abcd",
            8317,
            &home,
            &no_vars(),
        )
        .unwrap();
        aux.delete_takeover_backup("opencode").unwrap();

        // No faithful rebuild exists for the additive agents' kiwano-authored
        // entry, and stripping one would need to know which provider the user
        // selected before: the honest answer is a failure, not a silent success.
        let err = disable(&aux, "opencode", &home, None, &no_vars()).unwrap_err();
        assert!(err.contains("no fallback route"), "{err}");
    }
}
