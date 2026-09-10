//! Agent installation detection, reduced from cc-switch's
//! `probe_tool_installations` (commands/misc.rs) into two phases:
//!
//! Phase 1 (`detect_agents`): existence only — one login-shell subprocess
//! runs `command -v` for every CLI agent, plus a directory check for Claude
//! Desktop. Fast enough to run before the first render of the Apps filter.
//! Phase 2 (`probe_agent_versions`): async `--version` per resolved binary;
//! purely cosmetic (tooltips), never gates the installed verdict.
//!
//! The GUI inherits launchd's narrow PATH on macOS, so both phases go through
//! the user's login shell (`$SHELL -lc`) to see the real user PATH. Windows
//! is not handled here yet (the takeover layer is macOS-leaning too).

use std::collections::BTreeMap;
use std::io::Read;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// (agent id, CLI executable name) — claude-desktop has no CLI and is checked apart.
const CLI_AGENTS: &[(&str, &str)] = &[
    ("claude", "claude"),
    ("codex", "codex"),
    ("gemini", "gemini"),
    ("grokbuild", "grok"),
    ("opencode", "opencode"),
    ("openclaw", "openclaw"),
    ("hermes", "hermes"),
    ("pi", "pi"),
];

const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Serialize)]
pub struct AgentDetectVm {
    pub agent: String,
    pub installed: bool,
    /// Resolved CLI binary path (null for claude-desktop).
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentVersionVm {
    pub agent: String,
    /// First non-empty line of `--version`; null when not probed / failed.
    pub version: Option<String>,
}

/// Phase 1: which agents are installed on this machine.
#[tauri::command(async)]
pub fn detect_agents() -> Vec<AgentDetectVm> {
    let cli = detect_cli_agents().unwrap_or_default();
    let mut vms: Vec<AgentDetectVm> = CLI_AGENTS
        .iter()
        .map(|(agent, cli_name)| {
            let path = cli.get(*cli_name).cloned();
            AgentDetectVm {
                agent: (*agent).into(),
                installed: path.is_some(),
                path,
            }
        })
        .collect();
    vms.push(AgentDetectVm {
        agent: "claude-desktop".into(),
        installed: claude_desktop_installed(),
        path: None,
    });
    vms
}

/// Phase 2: version strings for the installed CLI agents (async; tooltips only).
#[tauri::command(async)]
pub fn probe_agent_versions() -> Vec<AgentVersionVm> {
    let cli = detect_cli_agents().unwrap_or_default();
    let mut vms: Vec<AgentVersionVm> = CLI_AGENTS
        .iter()
        .map(|(agent, cli_name)| AgentVersionVm {
            agent: (*agent).into(),
            version: cli.get(*cli_name).and_then(|p| probe_version(p)),
        })
        .collect();
    // The desktop app has no CLI; reading its Info.plist is not worth it here.
    vms.push(AgentVersionVm {
        agent: "claude-desktop".into(),
        version: None,
    });
    vms
}

/// Run the `command -v` loop through the user's login shell and map
/// executable name → resolved path. Errors mean "probe unavailable", not
/// "nothing installed" — callers degrade to empty (UI shows every agent).
fn detect_cli_agents() -> Result<BTreeMap<String, String>, String> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/sh".into());
    let mut child = Command::new(shell)
        .arg("-lc")
        .arg(probe_script())
        // stdin off: interactive rc files must never block the probe
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("agent probe spawn failed: {e}"))?;
    if wait_with_timeout(&mut child, PROBE_TIMEOUT)
        .filter(|st| st.success())
        .is_none()
    {
        return Err("agent probe timed out or failed".into());
    }
    let mut buf = String::new();
    if let Some(mut out) = child.stdout.take() {
        out.read_to_string(&mut buf).map_err(|e| e.to_string())?;
    }
    Ok(parse_probe_output(&buf))
}

/// `for t in …; do command -v … && printf 'tool path'; done` — one shell, all tools.
fn probe_script() -> String {
    let names: Vec<&str> = CLI_AGENTS.iter().map(|(_, cli)| *cli).collect();
    format!(
        "for t in {}; do p=$(command -v \"$t\" 2>/dev/null) && printf '%s %s\\n' \"$t\" \"$p\"; done",
        names.join(" ")
    )
}

/// Parse `"<tool> <path>"` lines; rc-file noise (welcome banners etc.) and
/// non-absolute paths are ignored. Later duplicates keep the first hit.
fn parse_probe_output(out: &str) -> BTreeMap<String, String> {
    let mut found = BTreeMap::new();
    for line in out.lines() {
        let mut it = line.split_whitespace();
        let (Some(tool), Some(path)) = (it.next(), it.next()) else {
            continue;
        };
        if !CLI_AGENTS.iter().any(|(_, cli)| *cli == tool) || !path.starts_with('/') {
            continue;
        }
        found
            .entry(tool.to_string())
            .or_insert_with(|| path.to_string());
    }
    found
}

/// Run `<bin> --version` and return its first non-empty output line.
fn probe_version(bin: &str) -> Option<String> {
    let mut child = Command::new(bin)
        .arg("--version")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    wait_with_timeout(&mut child, PROBE_TIMEOUT).filter(|st| st.success())?;
    let mut buf = String::new();
    child.stdout.take()?.read_to_string(&mut buf).ok()?;
    parse_version_output(&buf)
}

fn parse_version_output(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(String::from)
}

/// Poll `try_wait` until exit or deadline; on timeout the child is killed and
/// None returned (cc-switch's CommandDeadline pattern, std-only).
fn wait_with_timeout(
    child: &mut std::process::Child,
    timeout: Duration,
) -> Option<std::process::ExitStatus> {
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(st)) => return Some(st),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// Claude Desktop marker: the app-support directory (either flavor) exists.
#[cfg(target_os = "macos")]
fn claude_desktop_installed() -> bool {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let base = std::path::PathBuf::from(home).join("Library/Application Support");
    base.join("Claude").is_dir() || base.join("Claude-3p").is_dir()
}

#[cfg(not(target_os = "macos"))]
fn claude_desktop_installed() -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_output_parses_tools_and_ignores_noise() {
        let out = concat!(
            "\u{1f680} welcome back\n",
            "claude /opt/homebrew/bin/claude\n",
            "codex /usr/local/bin/codex\n",
            "grok /Users/x/.local/bin/grok\n",
            "some random rc line\n",
            "pi /usr/bin/pi\n",
            "gemini relative/gemini\n",
        );
        let m = parse_probe_output(out);
        assert_eq!(m.get("claude").unwrap(), "/opt/homebrew/bin/claude");
        assert_eq!(m.get("grok").unwrap(), "/Users/x/.local/bin/grok");
        assert_eq!(m.get("pi").unwrap(), "/usr/bin/pi");
        // relative path → ignored
        assert!(!m.contains_key("gemini"));
        assert_eq!(m.len(), 4);
    }

    #[test]
    fn probe_output_keeps_first_hit_on_duplicates() {
        let m = parse_probe_output("codex /a/codex\ncodex /b/codex\n");
        assert_eq!(m.get("codex").unwrap(), "/a/codex");
    }

    #[test]
    fn probe_script_lists_every_cli() {
        let s = probe_script();
        for (_, cli) in CLI_AGENTS {
            assert!(s.contains(cli), "script misses {cli}");
        }
    }

    #[test]
    fn version_output_takes_first_non_empty_line() {
        assert_eq!(
            parse_version_output("2.1.83 (Claude Code)"),
            Some("2.1.83 (Claude Code)".into())
        );
        assert_eq!(
            parse_version_output("\n  \n0.5.12\nmore"),
            Some("0.5.12".into())
        );
        assert_eq!(parse_version_output(""), None);
    }
}
