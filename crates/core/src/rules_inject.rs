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

/// The block for rules the **user** wrote, kept apart from the derived block
/// above on purpose: either can be removed without the other, and a re-apply of
/// one can never touch the other's lines. Two markers rather than one block with
/// two sections because "remove the advice Kiwano derived" and "remove what I
/// wrote" are different intentions, and a single block would force them to be
/// the same one.
pub const USER_MARK_BEGIN: &str = "<!-- kiwano:user-rules begin -->";
pub const USER_MARK_END: &str = "<!-- kiwano:user-rules end -->";

/// One marked block: its markers, the heading a reader sees, and the namespace
/// its backup and its stamp live under.
///
/// The `key` is the whole reason this is a type rather than two copies of the
/// same five functions: an existing install's backups are keyed `rules:<agent>`
/// and its stamps `rules_applied:<agent>`, and those strings have to keep meaning
/// what they mean.
struct Block {
    begin: &'static str,
    end: &'static str,
    heading: &'static str,
    key: &'static str,
}

/// The rules insights derived (`kiwano rules apply`).
const INSIGHTS_BLOCK: Block = Block {
    begin: MARK_BEGIN,
    end: MARK_END,
    heading: "## Kiwano insights rules",
    key: "rules",
};

/// The rules the user wrote (`kiwano rules user`).
const USER_BLOCK: Block = Block {
    begin: USER_MARK_BEGIN,
    end: USER_MARK_END,
    heading: "## Kiwano rules (yours)",
    key: "rules_user",
};

/// The key the user's own rules are kept under in the local library — the file
/// `rules_inject` already reaches through `ctx.aux()`.
///
/// **One set per machine**, not one per agent, because that is what a rule the
/// user writes usually is: a convention they hold everywhere ("answer in
/// Chinese", "never commit without asking"). Which agents it is written *into*
/// stays a per-agent decision — that is what `rules apply <agent>` is for — so a
/// machine-level library costs nothing in control.
const USER_RULES_KEY: &str = "user_rules";

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
    /// Whether the user's own block is in the file.
    pub user_applied: bool,
    /// The user's own rules as the file holds them.
    pub user_rules: Vec<String>,
    /// When the user's block was last (re)applied.
    pub user_applied_at: Option<String>,
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

fn backup_key(block: &Block, agent: &str) -> String {
    format!("{}:{agent}", block.key)
}

fn applied_key(block: &Block, agent: &str) -> String {
    format!("{}_applied:{agent}", block.key)
}

/// The rules the user has written, in the order they wrote them. Empty when they
/// have written none — which is every install that never opens this.
pub fn user_rules(aux: &Aux) -> Vec<String> {
    aux.get_setting(USER_RULES_KEY)
        .and_then(|raw| serde_json::from_str::<Vec<String>>(&raw).ok())
        .unwrap_or_default()
}

/// Replace the user's own rules. An empty slice clears them — the library, not
/// the files: what is written into an agent's file changes on the next
/// `rules apply` (and `rules remove` takes it back out).
pub fn set_user_rules(aux: &Aux, rules: &[String]) -> Result<Vec<String>, String> {
    let cleaned: Vec<String> = rules
        .iter()
        .map(|r| r.trim().trim_start_matches("- ").trim().to_string())
        .filter(|r| !r.is_empty())
        .collect();
    let raw = serde_json::to_string(&cleaned).map_err(|e| e.to_string())?;
    aux.set_setting(USER_RULES_KEY, &raw)
        .map_err(|e| e.to_string())?;
    Ok(cleaned)
}

/// Strip the marked block, returning what is left and the rules it held.
/// Absent markers mean (content unchanged, no rules) — a file the user never
/// let us write, or one they edited past recognition.
fn strip_block(content: &str, block: &Block) -> (String, Vec<String>) {
    let Some(begin) = content.find(block.begin) else {
        return (content.to_string(), Vec::new());
    };
    let Some(end) = content[begin..].find(block.end).map(|e| begin + e) else {
        return (content.to_string(), Vec::new());
    };
    let inside = &content[begin + block.begin.len()..end];
    let rules = inside
        .lines()
        .filter_map(|l| l.trim().strip_prefix("- ").map(String::from))
        .collect();
    let before = content[..begin].trim_end();
    let after = content[end + block.end.len()..].trim_start_matches('\n');
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
    // Each block is stripped independently, which is what makes them two: the
    // second strip sees a string the first one already edited, and neither can
    // take the other's lines with it.
    let (_, rules) = strip_block(&content, &INSIGHTS_BLOCK);
    let (_, user_rules) = strip_block(&content, &USER_BLOCK);
    Ok(RulesStatus {
        agent: agent.to_string(),
        file: display(&path, home),
        applied: !rules.is_empty(),
        rules,
        applied_at: aux.get_setting(&applied_key(&INSIGHTS_BLOCK, agent)),
        user_applied: !user_rules.is_empty(),
        user_rules,
        user_applied_at: aux.get_setting(&applied_key(&USER_BLOCK, agent)),
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
    write_block(aux, agent, rules, &INSIGHTS_BLOCK, home, vars)
}

/// Write the user's own rules into the agent's instruction file, from the library
/// they are kept in. Refuses when there are none, rather than writing an empty
/// block: an empty block is a line in the user's file that says nothing.
pub fn apply_user(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    let rules = user_rules(aux);
    if rules.is_empty() {
        return Err("no rules of your own to apply — `kiwano rules user --rule ...`".into());
    }
    write_block(aux, agent, &rules, &USER_BLOCK, home, vars)
}

fn write_block(
    aux: &Aux,
    agent: &str,
    rules: &[String],
    block: &Block,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    if rules.is_empty() {
        return Err("no rules to apply".into());
    }
    let path = rules_file(agent, home, vars)?;
    let existed = path.exists();
    let content = std::fs::read_to_string(&path).unwrap_or_default();

    if aux
        .load_takeover_backup(&backup_key(block, agent))
        .is_none()
    {
        aux.save_takeover_backup(
            &backup_key(block, agent),
            &[BackupFile {
                path: path.display().to_string(),
                content: content.clone(),
                existed,
            }],
        )
        .map_err(|e| e.to_string())?;
    }

    let (stripped, _) = strip_block(&content, block);
    let marked = format!(
        "{}\n{}\n{}\n{}\n",
        block.begin,
        block.heading,
        rules
            .iter()
            .map(|r| format!("- {r}"))
            .collect::<Vec<_>>()
            .join("\n"),
        block.end
    );
    let mut out = stripped.trim_end().to_string();
    if !out.is_empty() {
        out.push_str("\n\n");
    }
    out.push_str(&marked);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    kiwano_adapters::config::atomic_write_private(&path, out.as_bytes())
        .map_err(|e| e.to_string())?;
    aux.set_setting(&applied_key(block, agent), &crate::vm::rfc3339(unix_now()))
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
    remove_block(aux, agent, &INSIGHTS_BLOCK, home, vars)
}

/// Take the user's own block out of the file. The library keeps the rules —
/// removing them from a file is not the same act as deleting them, and the next
/// `rules apply` puts them back.
pub fn remove_user(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    remove_block(aux, agent, &USER_BLOCK, home, vars)
}

/// Take **both** blocks out — what `kiwano rules remove <agent>` means: "give me
/// my file back". Whichever is absent is skipped rather than refused, so removing
/// from a file that only ever had one block is not an error.
pub fn remove_all(
    aux: &Aux,
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    for block in [&INSIGHTS_BLOCK, &USER_BLOCK] {
        if let Err(e) = remove_block(aux, agent, block, home, vars) {
            if !e.starts_with("no rules applied") {
                return Err(e);
            }
        }
    }
    read_status(aux, agent, home, vars)
}

fn remove_block(
    aux: &Aux,
    agent: &str,
    block: &Block,
    home: &Path,
    vars: &ShellVars,
) -> Result<RulesStatus, String> {
    let path = rules_file(agent, home, vars)?;
    let content = std::fs::read_to_string(&path).unwrap_or_default();
    let (stripped, rules) = strip_block(&content, block);
    if !rules.is_empty() {
        kiwano_adapters::config::atomic_write_private(&path, stripped.as_bytes())
            .map_err(|e| e.to_string())?;
    } else if let Some((_, files)) = aux.load_takeover_backup(&backup_key(block, agent)) {
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
    let _ = aux.delete_takeover_backup(&backup_key(block, agent));
    let _ = aux.set_setting(&applied_key(block, agent), "");
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
        // Through `Path`, not `str`: this is a `String`, and comparing it as one
        // asks for the separator to be `/` — which it is everywhere except
        // Windows, where the same file is `.claude\CLAUDE.md` and CI's windows
        // leg is the only place that shows up.
        assert!(
            std::path::Path::new(&st.file).ends_with(".claude/CLAUDE.md"),
            "{}",
            st.file
        );
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

#[cfg(test)]
mod user_block_tests {
    use super::*;

    fn home() -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().to_path_buf();
        (dir, path)
    }

    fn vars() -> ShellVars {
        ShellVars::new()
    }

    fn instruction_file(home: &Path) -> PathBuf {
        home.join(".claude").join("CLAUDE.md")
    }

    fn written(home: &Path) -> String {
        std::fs::read_to_string(instruction_file(home)).unwrap_or_default()
    }

    /// The two blocks are independent in both directions: each can be written
    /// without disturbing the other, and each can be removed alone.
    ///
    /// This is the reason there are two marker pairs rather than one block with
    /// two sections — "remove the advice Kiwano derived" and "remove what I
    /// wrote" are different intentions, and a single block would make them the
    /// same one.
    #[test]
    fn the_users_block_and_the_insights_block_do_not_touch_each_other() {
        let (_d, home) = home();
        let aux = Aux::open_in_memory().unwrap();
        std::fs::create_dir_all(home.join(".claude")).unwrap();
        std::fs::write(instruction_file(&home), "# my own file\n").unwrap();

        set_user_rules(&aux, &["Answer in Chinese".into()]).unwrap();
        apply(&aux, "claude", &["cache the prefix".into()], &home, &vars()).unwrap();
        apply_user(&aux, "claude", &home, &vars()).unwrap();

        let content = written(&home);
        assert!(
            content.contains("# my own file"),
            "the user's text survives"
        );
        assert!(content.contains("cache the prefix"), "{content}");
        assert!(content.contains("Answer in Chinese"), "{content}");
        assert!(content.contains(USER_MARK_BEGIN) && content.contains(MARK_BEGIN));

        // Removing the insights half leaves the user's half exactly where it was.
        remove(&aux, "claude", &home, &vars()).unwrap();
        let content = written(&home);
        assert!(!content.contains("cache the prefix"), "{content}");
        assert!(content.contains("Answer in Chinese"), "{content}");
        assert!(content.contains("# my own file"), "{content}");

        // And re-applying insights does not take the user's block with it.
        apply(&aux, "claude", &["a second lesson".into()], &home, &vars()).unwrap();
        let content = written(&home);
        assert!(content.contains("a second lesson"), "{content}");
        assert!(content.contains("Answer in Chinese"), "{content}");

        // `remove_all` is the way out of both at once, and it keeps the library
        // so the next apply can put it back.
        let st = remove_all(&aux, "claude", &home, &vars()).unwrap();
        assert!(!st.applied && !st.user_applied);
        assert_eq!(written(&home), "# my own file\n");
        assert_eq!(user_rules(&aux), vec!["Answer in Chinese".to_string()]);

        // Removing when there is nothing to remove is an error the caller can
        // tell apart from a failure — `remove_all` swallows exactly that one.
        assert!(remove(&aux, "claude", &home, &vars()).is_err());
        assert!(remove_all(&aux, "claude", &home, &vars()).is_ok());
    }

    #[test]
    fn the_library_is_trimmed_and_an_empty_set_clears_it() {
        let (_d, _home) = home();
        let aux = Aux::open_in_memory().unwrap();
        let stored = set_user_rules(
            &aux,
            &[
                "  - Answer in Chinese  ".into(),
                "   ".into(),
                "Never commit without asking".into(),
            ],
        )
        .unwrap();
        assert_eq!(
            stored,
            vec![
                "Answer in Chinese".to_string(),
                "Never commit without asking".to_string()
            ],
            "the dash and the whitespace a user pastes are not part of the rule"
        );
        assert_eq!(user_rules(&aux), stored);

        set_user_rules(&aux, &[]).unwrap();
        assert!(user_rules(&aux).is_empty());
    }
}
