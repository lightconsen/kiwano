//! Rule injection (Features panel, docs/request-logs-applications.md §2.2):
//! append the rules insights derived into the agent's own instruction file —
//! CLAUDE.md for Claude Code, AGENTS.md for the rest — where the agent reads
//! them every session.
//!
//! Deliberately not part of the takeover state machine: a takeover's disable
//! means "give this agent its config back", and rule injection must be
//! removable without touching routing. What it borrows is the machinery —
//! the same backup table (keyed `rules:<agent>` so the two never collide),
//! the same "existed" semantics, the same atomic write.
//!
//! The block is delimited by marker comments, so remove is a strip, not a
//! restore, whenever the markers survive; the backup is the fallback for a
//! file the user edited past recognition.

use std::path::{Path, PathBuf};

use crate::auxiliary::Aux;
use crate::detect::ShellVars;
use crate::takeover::{self, BackupFile};
use crate::vm::unix_now;

pub const MARK_BEGIN: &str = "<!-- kiwano:rules begin -->";
pub const MARK_END: &str = "<!-- kiwano:rules end -->";

/// What `kiwano rules status` reports.
#[derive(Debug, Clone, serde::Serialize)]
pub struct RulesStatus {
    pub agent: String,
    /// The instruction file, displayed the way the settings screen displays
    /// paths (`~/…` under home).
    pub file: String,
    pub applied: bool,
    /// The rules currently inside the markers.
    pub rules: Vec<String>,
    /// When the block was last (re)applied.
    pub applied_at: Option<String>,
}

/// The file an agent reads its standing instructions from, as a sibling of
/// its config: `takeover_paths` already knows where every agent keeps its
/// files (and which env vars move them), so the rules file resolves through
/// the same answer rather than a second table that could disagree.
fn rules_file(agent: &str, home: &Path, vars: &ShellVars) -> Result<PathBuf, String> {
    let name = match agent {
        "claude" => "CLAUDE.md",
        "gemini" => "GEMINI.md",
        _ => "AGENTS.md",
    };
    let paths = takeover::takeover_paths(agent, home, vars)?;
    let dir = paths
        .first()
        .and_then(|p| p.parent())
        .ok_or_else(|| format!("cannot locate {agent}'s config directory"))?;
    Ok(dir.join(name))
}

fn display(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => Path::new("~").join(rest).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

fn backup_key(agent: &str) -> String {
    format!("rules:{agent}")
}

fn applied_key(agent: &str) -> String {
    format!("rules_applied:{agent}")
}

/// Strip the marked block, returning what is left and the rules it held.
/// Absent markers mean (content unchanged, no rules) — a file the user never
/// let us write, or one they edited past recognition.
fn strip_block(content: &str) -> (String, Vec<String>) {
    let Some(begin) = content.find(MARK_BEGIN) else {
        return (content.to_string(), Vec::new());
    };
    let Some(end) = content[begin..].find(MARK_END).map(|e| begin + e) else {
        return (content.to_string(), Vec::new());
    };
    let inside = &content[begin + MARK_BEGIN.len()..end];
    let rules = inside
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- ").map(String::from))
        .collect();
    let before = content[..begin].trim_end();
    let after = content[end + MARK_END.len()..].trim_start_matches('\n');
    if after.trim().is_empty() {
        // The block was the tail: keep the file's own ending (a trailing
        // newline is part of the file the user had, not of our block).
        let mut out = before.to_string();
        if content.ends_with('\n') && !out.is_empty() {
            out.push('\n');
        }
        return (out, rules);
    }
    let mut out = before.to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(after);
    (out, rules)
}

fn read_status(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    let path = rules_file(agent, home, vars)?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let (_, rules) = strip_block(&content);
    Ok(RulesStatus {
        agent: agent.to_string(),
        file: display(&path, home),
        applied: !rules.is_empty(),
        rules,
        applied_at: aux.get_setting(&applied_key(agent)),
    })
}

/// What rules are applied to `agent`, if any. Never gated: reading is free.
pub fn status(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    read_status(aux, agent, home, vars)
}

/// Append the rules under the markers, replacing any block already there.
/// The file is backed up first — once, so a re-apply cannot overwrite the
/// pre-injection original with a later, already-injected state.
pub fn apply(
    aux: &Aux,
    agent: &str,
    rules: &[String],
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    if rules.is_empty() {
        return Err("no rules to apply".into());
    }
    let path = rules_file(agent, home, vars)?;
    let existed = path.exists();
    let content = std::fs::read_to_string(&path).unwrap_or_default();

    if aux.load_takeover_backup(&backup_key(agent)).is_none() {
        aux.save_takeover_backup(
            &backup_key(agent),
            &[BackupFile {
                path: path.display().to_string(),
                content: content.clone(),
                existed,
            }],
        )
        .map_err(|e| e.to_string())?;
    }

    let (stripped, _) = strip_block(&content);
    let block = format!(
        "{MARK_BEGIN}\n## Kiwano insights rules\n{}\n{MARK_END}\n",
        rules
            .iter()
            .map(|r| format!("- {r}"))
            .collect::<Vec<_>>()
            .join("\n")
    );
    let mut out = stripped.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&block);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    kiwano_adapters::config::atomic_write_private(&path, out.as_bytes())
        .map_err(|e| e.to_string())?;
    aux.set_setting(&applied_key(agent), &crate::vm::rfc3339(unix_now()))
        .map_err(|e| e.to_string())?;
    read_status(aux, agent, home, vars)
}

/// Remove the block. A strip when the markers are intact; a byte-level
/// restore from the backup when they are not (the user edited them away, and
/// stripping by pattern would then remove nothing while the backup still
/// says we wrote something). Never gated: the way out must always work.
pub fn remove(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    let path = rules_file(agent, home, vars)?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let (stripped, rules) = strip_block(&content);
    if !rules.is_empty() {
        kiwano_adapters::config::atomic_write_private(&path, stripped.as_bytes())
            .map_err(|e| e.to_string())?;
    } else if let Some((_, files)) = aux.load_takeover_backup(&backup_key(agent)) {
        for f in files {
            let p = PathBuf::from(&f.path);
            if f.existed {
                kiwano_adapters::config::atomic_write_private(&p, f.content.as_bytes())
                    .map_err(|e| e.to_string())?;
            } else if p.exists() {
                std::fs::remove_file(&p).map_err(|e| e.to_string())?;
            }
        }
    } else {
        return Err(format!("no rules applied to {agent}"));
    }
    let _ = aux.delete_takeover_backup(&backup_key(agent));
    let _ = aux.set_setting(&applied_key(agent), "");
    read_status(aux, agent, home, vars)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::auxiliary::Aux;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    fn vars() -> ShellVars {
        ShellVars::new()
    }

    #[test]
    fn apply_writes_a_marked_block_and_status_reads_it() {
        let (_d, home) = home();
        let aux = Aux::open_in_memory().unwrap();
        let st = apply(
            &aux,
            "claude",
            &["Keep the prompt prefix stable.".into()],
            &home,
            &vars(),
        )
        .unwrap();
        assert!(st.applied);
        assert_eq!(st.rules, ["Keep the prompt prefix stable."]);
        assert!(st.file.ends_with(".claude/CLAUDE.md"), "{}", st.file);
        assert!(st.applied_at.is_some());

        let on_disk = std::fs::read_to_string(home.join(".claude/CLAUDE.md")).unwrap();
        assert!(on_disk.contains(MARK_BEGIN));
        // User content is not clobbered by a re-apply.
        let st = apply(&aux, "claude", &["Another rule.".into()], &home, &vars()).unwrap();
        assert_eq!(st.rules, ["Another rule."]);
    }

    #[test]
    fn apply_preserves_existing_content_and_remove_restores_it() {
        let (_d, home) = home();
        let aux = Aux::open_in_memory().unwrap();
        let claude_dir = home.join(".claude");
        std::fs::create_dir_all(&claude_dir).unwrap();
        std::fs::write(claude_dir.join("CLAUDE.md"), "My own rules.\n").unwrap();

        apply(&aux, "claude", &["Injected.".into()], &home, &vars()).unwrap();
        let mid = std::fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap();
        assert!(mid.starts_with("My own rules.\n"));
        assert!(mid.contains("- Injected."));

        remove(&aux, "claude", &home, &vars()).unwrap();
        assert_eq!(
            std::fs::read_to_string(claude_dir.join("CLAUDE.md")).unwrap(),
            "My own rules.\n"
        );
        assert!(!status(&aux, "claude", &home, &vars()).unwrap().applied);
        // Removing twice is an honest error, not a silent no-op.
        assert!(remove(&aux, "claude", &home, &vars()).is_err());
    }

    #[test]
    fn remove_without_markers_falls_back_to_the_backup() {
        let (_d, home) = home();
        let aux = Aux::open_in_memory().unwrap();
        apply(&aux, "codex", &["Rule.".into()], &home, &vars()).unwrap();
        let file = home.join(".codex/AGENTS.md");
        // The user rewrites the file, markers and all.
        std::fs::write(&file, "hand-written\n").unwrap();

        remove(&aux, "codex", &home, &vars()).unwrap();
        // The backup says the file did not exist before the injection — so it
        // is gone again, not left behind as an empty file.
        assert!(!file.exists());
    }
}
