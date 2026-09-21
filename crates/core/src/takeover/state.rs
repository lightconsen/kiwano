//! Restore's contract — what a restore did, and the provider a rebuild aims at
//! — plus the recognition that decides whether a takeover is live at all: the
//! placeholder key read out of the agent's own file, never out of a table that
//! can be rolled back while the file stays rewritten.

use crate::detect::ShellVars;
use crate::takeover::paths::takeover_paths;
use crate::vm::Aux;
use kiwano_adapters::codex_config;
use std::path::Path;

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
pub fn live_placeholder_key(agent: &str, home: &Path, vars: &ShellVars) -> Option<String> {
    let paths = takeover_paths(agent, home, vars).ok()?;
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
/// Claude's `settings.json`, the additive agents' JSON/YAML)
/// is a value scan, because all of them hold the key as a string value.
pub(crate) fn placeholder_in_file(path: &Path, text: &str, agent: &str) -> Option<String> {
    if agent == "codex" && path.ends_with("config.toml") {
        return codex_config::codex_config_placeholder_key(text);
    }
    gateway_token(text, agent)
}

/// The first `kw-ag-<agent>-<rand>` token in `text`. The leading boundary keeps
/// a token that merely *contains* the prefix (a URL path, a longer id) from
/// reading as ours.
pub(crate) fn gateway_token(text: &str, agent: &str) -> Option<String> {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::test_support::{no_vars, temp_home};

    #[test]
    fn live_placeholder_key_reads_the_route_out_of_the_live_file() {
        let (_dir, home) = temp_home();
        let settings = home.join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();

        // Nothing written yet: no evidence of a takeover.
        assert!(live_placeholder_key("claude", &home, &no_vars()).is_none());

        std::fs::write(
            &settings,
            r#"{"model":"opus","env":{"ANTHROPIC_BASE_URL":"http://127.0.0.1:8317","ANTHROPIC_AUTH_TOKEN":"kw-ag-claude-abcd"}}"#,
        )
        .unwrap();
        assert_eq!(
            live_placeholder_key("claude", &home, &no_vars()).as_deref(),
            Some("kw-ag-claude-abcd")
        );

        // A token that merely contains the prefix (a longer id, a URL path) is
        // not ours: recognition is by the boundary, not by `contains`.
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"sk-kw-ag-claude-notours"}}"#,
        )
        .unwrap();
        assert!(live_placeholder_key("claude", &home, &no_vars()).is_none());

        // Another agent's key is not this agent's evidence either.
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_AUTH_TOKEN":"kw-ag-codex-abcd"}}"#,
        )
        .unwrap();
        assert!(live_placeholder_key("claude", &home, &no_vars()).is_none());
    }
}
