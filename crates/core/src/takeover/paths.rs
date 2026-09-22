//! Where each agent keeps the files a takeover rewrites: the environment
//! variables that move them, and the refusal that keeps a variable which *is*
//! set from being guessed at instead.

use crate::detect::ShellVars;
use kiwano_adapters::config::EnvDir;
use serde_json::Value;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};

/// The files a takeover rewrites for `agent`, rooted at the caller's `home`.
///
/// Path reconciliation with `kiwano_adapters::codex_config`: that module
/// resolves `~/.codex` itself as `get_codex_config_dir()`, while this file takes
/// the home directory as a parameter because every test injects a temp root and
/// the app passes `$HOME`. The split is deliberate rather than accidental: the
/// Codex *transforms* the module now owns are pure text functions with no paths
/// in them, and the one place that touches `~/.codex` on disk is
/// `enable`/`disable` below, which writes through the caller's root. A future
/// divergence shows up as a refused write, not a silent one into the wrong tree.
///
/// `vars` is where a moved directory is honored (see [`config_dir`]) — and it is
/// this function, not the adapters, that has to honor it: no adapter path
/// participates in a takeover, so a difference between what `CODEX_HOME` says
/// here and what `get_codex_config_dir()` would say there cannot strand a
/// rewrite.
pub(crate) fn takeover_paths(
    agent: &str,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<std::path::PathBuf>, String> {
    match agent {
        // CLAUDE_CONFIG_DIR moves the whole profile: settings.json, credentials
        // and the MCP file all live under it.
        "claude" => Ok(vec![config_dir(
            vars,
            "CLAUDE_CONFIG_DIR",
            ".claude",
            home,
        )?
        .join("settings.json")]),
        // Both files follow CODEX_HOME. Codex refuses to start with the variable
        // pointing at a directory that does not exist, so the root is the user's
        // to create — and a config.toml that is not there yet is refused exactly
        // as it is at the default root (the read step in `enable`).
        "codex" => {
            let dir = config_dir(vars, "CODEX_HOME", ".codex", home)?;
            Ok(vec![dir.join("config.toml"), dir.join("auth.json")])
        }
        "gemini" => Ok(vec![home.join(".gemini").join(".env")]),
        "grokbuild" => Ok(vec![home.join(".grok").join("config.toml")]),
        // claude-desktop: macOS Claude-3p configLibrary (deployment mode in
        // both claude_desktop_config.json copies + gateway profile + _meta.json)
        #[cfg(target_os = "macos")]
        "claude-desktop" => Ok(claude_desktop_paths(home)),
        #[cfg(not(target_os = "macos"))]
        "claude-desktop" => Err("claude-desktop takeover currently supports macOS only".into()),
        // additive-mode agents: the gateway entry coexists with their native
        // providers, so a missing config is fine (a fresh one gets created)
        //
        // OpenCode resolves its global config through the XDG rules, so a moved
        // XDG_CONFIG_HOME moves this file. Read from its source rather than from
        // the docs, which do not mention the variable: it composes
        // `path.join(xdgConfig, "opencode")` from the `xdg-basedir` package, and
        // that package is `env.XDG_CONFIG_HOME || path.join(os.homedir(),
        // ".config")` with no platform branch — so this one path is the same on
        // Windows, where a third-party note claims `%APPDATA%`. It is not.
        //
        // Its own two config variables are deliberately *not* honored:
        // `OPENCODE_CONFIG` names an extra file merged between the global and
        // the project config, and `OPENCODE_CONFIG_DIR` a directory searched for
        // agents, commands and plugins — neither is where the provider list
        // lives, and writing the gateway entry into one would put it where
        // OpenCode merges from rather than where it reads the user's own config.
        "opencode" => Ok(vec![config_dir(vars, "XDG_CONFIG_HOME", ".config", home)?
            .join("opencode")
            .join("opencode.json")]),
        // The second file is OpenClaw's custom model catalogue — the takeover
        // declares the selected model there because the catalogue's schema is
        // the only one that accepts the session-affinity compat flag: the main
        // config's validator rejects it and OpenClaw refuses to start on a
        // config it cannot validate (see `upsert_openclaw_models_json`).
        //
        // Where the *runtime* reads that catalogue is the per-agent directory:
        // `resolveAgentDir()` (the bundled agent-scope-config module) lands on
        // `<state>/agents/<id>/agent`, the state root defaulting to
        // `~/.openclaw`, or on the entry's own `agentDir` when the config sets
        // one. The `~/.openclaw/agent/models.json` that `OPENCLAW_AGENT_DIR`
        // and `getAgentDir()` name is what some CLI screens display, but model
        // resolution never reads it — verified end to end against 2026.6.9: a
        // catalogue written there leaves requests without the affinity
        // headers, while the per-agent one delivers them. The agent id and the
        // override live in the main config's content, so this arm reads it.
        //
        // `OPENCLAW_STATE_DIR` and `OPENCLAW_AGENT_DIR` are deliberately not
        // honored: the main config's own root is not movable either as far as
        // this code is concerned, and half-honoring a relocation would split
        // the two files across two trees.
        "openclaw" => openclaw_paths(home, vars),
        "hermes" => {
            // HERMES_HOME resolution matches hermes' own get_hermes_home()
            let dir = config_dir(vars, "HERMES_HOME", ".hermes", home)?;
            Ok(vec![dir.join("config.yaml")])
        }
        "pi" => Ok(vec![
            home.join(".pi").join("agent").join("models.json"),
            home.join(".pi").join("agent").join("settings.json"),
        ]),
        // WorkBuddy and CodeBuddy are one product family (the app embeds the
        // CLI) with separate config roots, each overridable by its own env var.
        "workbuddy" => Ok(vec![config_dir(
            vars,
            "WORKBUDDY_CONFIG_DIR",
            ".workbuddy",
            home,
        )?
        .join("models.json")]),
        "codebuddy" => Ok(vec![config_dir(
            vars,
            "CODEBUDDY_CONFIG_DIR",
            ".codebuddy",
            home,
        )?
        .join("models.json")]),
        "qwen" => Ok(vec![
            config_dir(vars, "QWEN_HOME", ".qwen", home)?.join("settings.json")
        ]),
        // Two generations of the same agent: the Node successor reads
        // ~/.kimi-code, the Python original (~/.kimi) is being retired. The
        // one that exists is the one to write; with neither, the successor —
        // that is what a fresh install is.
        "kimi" => Ok(vec![kimi_config(vars, home)?]),
        // MiMo Code follows the XDG rules for its global config, exactly like
        // OpenCode: a moved XDG_CONFIG_HOME moves this file. Its per-agent
        // auth.json is not rewritten — the takeover works on the custom
        // provider entry in the main config.
        "mimo" => Ok(vec![config_dir(vars, "XDG_CONFIG_HOME", ".config", home)?
            .join("mimocode")
            .join("mimocode.jsonc")]),
        // Cline resolves this one file through a three-level chain (read from
        // its own `sdk/packages/shared/src/storage/paths.ts`, which the docs do
        // not spell out): an exact file path, else a data directory, else a base
        // directory whose `data/` is the data directory. Each level is honored
        // because each one moves the file a takeover has to write.
        //
        // The first level names a *file*, so `config_dir` (a directory by
        // construction) cannot express it — and it is resolved here rather than
        // with a bespoke directory lookup for that reason.
        "cline" => Ok(vec![cline_provider_settings(vars, home)?]),
        other => Err(format!("unknown agent: {other}")),
    }
}

/// claude-desktop's four files under the macOS Claude-3p configLibrary: the two
/// `claude_desktop_config.json` copies (deployment mode lives in both), and
/// under the second one the gateway profile plus the `_meta.json` that records
/// which profile is applied.
#[cfg(target_os = "macos")]
fn claude_desktop_paths(home: &Path) -> Vec<PathBuf> {
    let app_support = home.join("Library").join("Application Support");
    let threep = app_support.join("Claude-3p");
    vec![
        app_support
            .join("Claude")
            .join("claude_desktop_config.json"),
        threep.join("claude_desktop_config.json"),
        threep.join("configLibrary").join(format!(
            "{}.json",
            kiwano_adapters::claude_desktop_config::PROFILE_ID
        )),
        threep.join("configLibrary").join("_meta.json"),
    ]
}

/// openclaw's files, in the order a takeover has to read them: the main config
/// first, because the catalogue's entry is rewritten against the selector that
/// config holds.
fn openclaw_paths(home: &Path, vars: &ShellVars) -> Result<Vec<PathBuf>, String> {
    let root = home.join(".openclaw");
    Ok(vec![
        root.join("openclaw.json"),
        openclaw_catalog_path(&root, vars, home)?,
    ])
}

/// Which Kimi config to write. The successor wins whenever it exists; with
/// neither, the successor is where a takeover starts writing, because that is
/// what a fresh install is.
fn kimi_config(vars: &ShellVars, home: &Path) -> Result<PathBuf, String> {
    let successor = config_dir(vars, "KIMI_CODE_HOME", ".kimi-code", home)?.join("config.toml");
    if successor.exists() {
        return Ok(successor);
    }
    let legacy = config_dir(vars, "KIMI_SHARE_DIR", ".kimi", home)?.join("config.toml");
    Ok(if legacy.exists() { legacy } else { successor })
}

/// Cline's provider settings, resolved through its own three-level chain: an
/// exact file path, else a data directory, else a base directory whose `data/`
/// is the data directory.
fn cline_provider_settings(vars: &ShellVars, home: &Path) -> Result<PathBuf, String> {
    match named_dir(vars, "CLINE_PROVIDER_SETTINGS_PATH") {
        EnvDir::Absolute(path) => Ok(path),
        EnvDir::Relative(raw) => Err(relative_env_refusal(
            "CLINE_PROVIDER_SETTINGS_PATH",
            &raw,
            &home
                .join(".cline")
                .join("data")
                .join("settings")
                .join("providers.json"),
        )),
        EnvDir::Unset => {
            // CLINE_DATA_DIR names the data directory itself and beats
            // CLINE_DIR; only when it is unset does the base come into it,
            // with `data` under it — which is also what the base defaults
            // to, so an empty environment lands on `~/.cline/data`.
            let data = match named_dir(vars, "CLINE_DATA_DIR") {
                EnvDir::Absolute(dir) => dir,
                EnvDir::Relative(raw) => {
                    return Err(relative_env_refusal(
                        "CLINE_DATA_DIR",
                        &raw,
                        &home.join(".cline").join("data"),
                    ))
                }
                EnvDir::Unset => config_dir(vars, "CLINE_DIR", ".cline", home)?.join("data"),
            };
            Ok(data.join("settings").join("providers.json"))
        }
    }
}

/// The variables that move where an agent keeps its files, for the caller that
/// has to go and ask for them ([`kiwano_core::detect::login_shell_vars`]). One
/// list here rather than a name at each use below, so the probe and the paths
/// it feeds cannot disagree about what to ask for.
pub const CONFIG_DIR_VARS: &[&str] = &[
    "CLAUDE_CONFIG_DIR",
    "CODEX_HOME",
    "XDG_CONFIG_HOME",
    "HERMES_HOME",
    "WORKBUDDY_CONFIG_DIR",
    "CODEBUDDY_CONFIG_DIR",
    "QWEN_HOME",
    "KIMI_CODE_HOME",
    "KIMI_SHARE_DIR",
    "CLINE_PROVIDER_SETTINGS_PATH",
    "CLINE_DATA_DIR",
    "CLINE_DIR",
];

/// The config root an agent resolves for itself: the environment variable that
/// overrides it, else the default directory under `home` — a user who moved
/// their config should not get a takeover written to the old place.
///
/// The shell's answer comes first and the process environment second, and that
/// order is the point: a GUI process does not inherit what the user's rc files
/// export, so `vars` is the only copy that can know about a relocated
/// directory. The process environment stays as the fallback because it is what
/// a terminal-run CLI and the tests have.
///
/// A variable that is *set* to something this cannot use is an error rather than
/// a fall-through to the default, because the two are not the same situation:
/// the default is right when nobody said otherwise, and a guess when somebody
/// did. See [`named_dir`].
fn config_dir(
    vars: &ShellVars,
    env_var: &str,
    default_dir: &str,
    home: &Path,
) -> Result<PathBuf, String> {
    match named_dir(vars, env_var) {
        EnvDir::Unset => Ok(home.join(default_dir)),
        EnvDir::Absolute(path) => Ok(path),
        EnvDir::Relative(raw) => Err(relative_env_refusal(env_var, &raw, &home.join(default_dir))),
    }
}

/// Why a *set* variable that cannot be used is refused rather than ignored, and
/// the same sentence [`config_dir`] and the readers that name a file instead of
/// a directory ([`takeover_paths`]'s Cline arm) both give. `default` is what
/// unsetting the variable would have meant, so the message can name it.
fn relative_env_refusal(env_var: &str, raw: &str, default: &Path) -> String {
    format!(
        "{env_var} is set to `{raw}` — not an absolute path. A tool resolves a relative \
         one against the directory it happens to be run in, so where its config lives is \
         not something Kiwano can know. Set {env_var} to an absolute path, or unset it to \
         use {}.",
        default.display()
    )
}

/// Where `name` points, in the environment the caller handed in — [`EnvDir`] is
/// the rule itself, shared with every other reader of these values.
///
/// The map is the *whole* environment here rather than a supplement to this
/// process's: one place decides what the user's environment is
/// ([`detect::login_shell_vars`], which folds the process's own into the shell's
/// answer), and this module then reads no ambient state at all. That is what
/// makes a takeover reproducible — and what keeps a runner's exported
/// `XDG_CONFIG_HOME` from moving the paths a test is asserting on.
fn named_dir(vars: &ShellVars, name: &str) -> EnvDir {
    kiwano_adapters::config::classify_dir(vars.get(name).map(OsStr::new))
}

/// Where OpenClaw's runtime reads the custom model catalogue:
/// `<agentDir>/models.json`, the agentDir being the default agent entry's own
/// `agentDir` when it has one, else `<state>/agents/<id>/agent`. This mirrors
/// `resolveAgentDir()`/`resolveDefaultAgentId()` in OpenClaw's bundled
/// agent-scope-config module; the state root defaults to `~/.openclaw`
/// (`NEW_STATE_DIRNAME`), and the env relocations are deliberately not honored
/// (see the openclaw arm of [`takeover_paths`]).
///
/// A main config that is missing or unparseable falls back to the `main`
/// agent's default location — what OpenClaw itself does with an empty
/// `agents.list`. Validating the config is the rewrite step's job, not this
/// path computation's.
fn openclaw_catalog_path(root: &Path, vars: &ShellVars, home: &Path) -> Result<PathBuf, String> {
    let (id, agent_dir) = std::fs::read_to_string(root.join("openclaw.json"))
        .ok()
        .and_then(|text| serde_json::from_str::<Value>(&text).ok())
        .map(|cfg| openclaw_default_agent(&cfg))
        .unwrap_or_else(|| ("main".to_string(), None));
    match agent_dir {
        Some(raw) => Ok(openclaw_user_path(&raw, vars, home)?.join("models.json")),
        None => Ok(root
            .join("agents")
            .join(id)
            .join("agent")
            .join("models.json")),
    }
}

/// OpenClaw's default agent: the first `agents.list` entry marked `default`,
/// else the first entry, else `"main"` — `resolveDefaultAgentId()`. The second
/// element is the entry's `agentDir` override, when it carries one.
fn openclaw_default_agent(cfg: &Value) -> (String, Option<String>) {
    let entries: Vec<&Value> = cfg["agents"]["list"]
        .as_array()
        .map(|list| list.iter().filter(|e| e.is_object()).collect())
        .unwrap_or_default();
    let Some(entry) = entries
        .iter()
        .find(|e| e["default"].as_bool() == Some(true))
        .or_else(|| entries.first())
    else {
        return ("main".to_string(), None);
    };
    let id = openclaw_normalize_agent_id(entry["id"].as_str().unwrap_or(""));
    let agent_dir = entry["agentDir"]
        .as_str()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string);
    (id, agent_dir)
}

/// OpenClaw's `normalizeAgentId()`: trim, lowercase, runs of anything outside
/// `[a-z0-9_-]` become a single `-`, leading and trailing `-` are stripped,
/// and an empty result is `main`.
fn openclaw_normalize_agent_id(raw: &str) -> String {
    let mut out = String::new();
    for c in raw.trim().to_lowercase().chars() {
        if c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-' {
            out.push(c);
        } else if !out.ends_with('-') {
            out.push('-');
        }
    }
    let trimmed = out.trim_matches('-');
    if trimmed.is_empty() {
        "main".to_string()
    } else {
        trimmed.to_string()
    }
}

/// OpenClaw's `resolveUserPath()` for an `agentDir` from the config: a leading
/// `~` expands against home, `$VAR` and `${VAR}` against the environment the
/// caller handed in. What remains relative is refused rather than resolved
/// against a working directory Kiwano cannot know — the same stance
/// [`relative_env_refusal`] takes for environment variables.
fn openclaw_user_path(raw: &str, vars: &ShellVars, home: &Path) -> Result<PathBuf, String> {
    let mut text = raw.trim().to_string();
    if text == "~" || text.starts_with("~/") {
        text = home
            .join(text.trim_start_matches('~').trim_start_matches('/'))
            .to_string_lossy()
            .into_owned();
    }
    let mut expanded = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] != '$' {
            expanded.push(chars[i]);
            i += 1;
            continue;
        }
        let (name, next) = if chars.get(i + 1) == Some(&'{') {
            match chars[i + 2..].iter().position(|c| *c == '}') {
                Some(end) => (
                    chars[i + 2..i + 2 + end].iter().collect::<String>(),
                    i + 3 + end,
                ),
                None => (String::new(), i + 1),
            }
        } else {
            let mut end = i + 1;
            while end < chars.len() && (chars[end].is_ascii_alphanumeric() || chars[end] == '_') {
                end += 1;
            }
            (chars[i + 1..end].iter().collect::<String>(), end)
        };
        if name.is_empty() {
            expanded.push('$');
            i += 1;
            continue;
        }
        match vars.get(&name) {
            Some(value) => expanded.push_str(value),
            None => {
                return Err(format!(
                    "openclaw's agents.list agentDir `{raw}` references ${name}, which is not set \
                     in the environment Kiwano can see. Set it, or make agentDir an absolute path."
                ))
            }
        }
        i = next;
    }
    match kiwano_adapters::config::classify_dir(Some(OsStr::new(&expanded))) {
        EnvDir::Absolute(path) => Ok(path),
        _ => Err(format!(
            "openclaw's agents.list agentDir `{raw}` is not an absolute path — OpenClaw resolves a \
             relative one against the directory it happens to run in, so where the model catalogue \
             lives is not something Kiwano can know. Make agentDir absolute, or remove it to use \
             the default location under {}.",
            home.join(".openclaw").display()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::takeover::test_support::{abs_dir, no_vars, temp_home};
    use crate::test_env::EnvGuard;

    // ── where an agent keeps its files ──

    /// The map the caller passes *is* the environment: nothing in this module
    /// reads this process's own, so a variable exported outside the map cannot
    /// move a path — which is what keeps a takeover reproducible, and what kept a
    /// CI runner's exported `XDG_CONFIG_HOME` from moving the paths these tests
    /// assert on.
    #[test]
    fn only_the_environment_it_is_handed_moves_a_config_root() {
        let tmp = tempfile::tempdir().unwrap();
        let home = abs_dir(&tmp, "home");
        let home = home.as_path();
        let exported = abs_dir(&tmp, "exported");

        // Exported by this process, absent from the map: not consulted.
        let _guard = EnvGuard::set("QWEN_HOME", Some(&exported.to_string_lossy()));
        assert_eq!(
            takeover_paths("qwen", home, &no_vars()).unwrap(),
            vec![home.join(".qwen").join("settings.json")],
            "the ambient environment is not a second source"
        );

        // In the map: that is the answer.
        let from_map = abs_dir(&tmp, "from-map");
        let vars = ShellVars::from([(
            "QWEN_HOME".to_string(),
            from_map.to_string_lossy().into_owned(),
        )]);
        assert_eq!(
            takeover_paths("qwen", home, &vars).unwrap(),
            vec![from_map.join("settings.json")]
        );
    }

    /// The three agents a variable can move, and what it moves them to.
    #[test]
    fn a_config_root_variable_moves_the_files_it_names() {
        let _guards = (
            EnvGuard::set("CLAUDE_CONFIG_DIR", None),
            EnvGuard::set("CODEX_HOME", None),
            EnvGuard::set("XDG_CONFIG_HOME", None),
        );
        let tmp = tempfile::tempdir().unwrap();
        let (claude, codex, xdg) = (
            abs_dir(&tmp, "claude"),
            abs_dir(&tmp, "codex"),
            abs_dir(&tmp, "xdg"),
        );
        let home = abs_dir(&tmp, "home");
        let home = home.as_path();
        let vars = ShellVars::from([
            (
                "CLAUDE_CONFIG_DIR".to_string(),
                claude.to_string_lossy().into_owned(),
            ),
            (
                "CODEX_HOME".to_string(),
                codex.to_string_lossy().into_owned(),
            ),
            (
                "XDG_CONFIG_HOME".to_string(),
                xdg.to_string_lossy().into_owned(),
            ),
        ]);

        // The whole Claude profile moves with the variable, settings included.
        assert_eq!(
            takeover_paths("claude", home, &vars).unwrap(),
            vec![claude.join("settings.json")]
        );
        // Codex's two files move together — a config pointing at the gateway
        // with the auth left behind is a half-takeover.
        assert_eq!(
            takeover_paths("codex", home, &vars).unwrap(),
            vec![codex.join("config.toml"), codex.join("auth.json")]
        );
        // OpenCode reaches its global config through the XDG rules.
        assert_eq!(
            takeover_paths("opencode", home, &vars).unwrap(),
            vec![xdg.join("opencode").join("opencode.json")]
        );
        // MiMo Code follows the same XDG rules for its global config.
        assert_eq!(
            takeover_paths("mimo", home, &vars).unwrap(),
            vec![xdg.join("mimocode").join("mimocode.jsonc")]
        );
        // Unset, the XDG location is `~/.config/opencode/opencode.json`: the
        // default's sibling, not a replacement for it.
        let unset = ShellVars::from([("XDG_CONFIG_HOME".to_string(), "  ".to_string())]);
        assert_eq!(
            takeover_paths("opencode", home, &unset).unwrap(),
            vec![home.join(".config").join("opencode").join("opencode.json")],
            "a blank value is nobody saying otherwise"
        );
        // Same default for MiMo, beside the OpenCode one.
        assert_eq!(
            takeover_paths("mimo", home, &unset).unwrap(),
            vec![home.join(".config").join("mimocode").join("mimocode.jsonc")]
        );
    }

    /// OpenCode's own config variables are not relocations, and honoring them as
    /// if they were would write the gateway entry somewhere OpenCode only merges
    /// from — or searches for agents — rather than the user's global config.
    #[test]
    fn opencodes_custom_config_is_not_its_global_config() {
        let _guards = (
            EnvGuard::set("XDG_CONFIG_HOME", None),
            EnvGuard::set("OPENCODE_CONFIG", None),
            EnvGuard::set("OPENCODE_CONFIG_DIR", None),
        );
        let home = Path::new("/tmp/kiwano-config-home");
        let vars = ShellVars::from([
            (
                "OPENCODE_CONFIG".to_string(),
                "/srv/extra/opencode.json".to_string(),
            ),
            ("OPENCODE_CONFIG_DIR".to_string(), "/srv/extra".to_string()),
        ]);

        assert_eq!(
            takeover_paths("opencode", home, &vars).unwrap(),
            vec![home.join(".config").join("opencode").join("opencode.json")]
        );
    }

    /// A variable that is set to something unusable is not the same as one that
    /// is unset, and the difference is a refusal rather than a guess: the
    /// default is right when nobody said otherwise, and a guess when somebody
    /// did. A relative path names no one place — the tool resolves it against
    /// whatever directory it is run in — so there is no file here to write.
    #[test]
    fn a_relative_value_is_refused_rather_than_guessed_at() {
        let _unset = EnvGuard::set("CODEBUDDY_CONFIG_DIR", None);
        let home = Path::new("/tmp/kiwano-config-home");
        let vars = ShellVars::from([(
            "CODEBUDDY_CONFIG_DIR".to_string(),
            "relative/dir".to_string(),
        )]);

        let err = takeover_paths("codebuddy", home, &vars).unwrap_err();
        // The message names the variable, what is wrong with the value, and
        // both ways out.
        assert!(err.contains("CODEBUDDY_CONFIG_DIR"), "{err}");
        assert!(err.contains("relative/dir"), "{err}");
        assert!(err.contains("absolute path"), "{err}");

        // Set to an absolute path instead, the same variable is honored — and
        // padded whitespace is not part of the path.
        let tmp = tempfile::tempdir().unwrap();
        let buddy = abs_dir(&tmp, "buddy");
        let padded = format!("{} ", buddy.display());
        let vars = ShellVars::from([("CODEBUDDY_CONFIG_DIR".to_string(), padded)]);
        assert_eq!(
            takeover_paths("codebuddy", home, &vars).unwrap(),
            vec![buddy.join("models.json")]
        );
    }

    #[test]
    fn openclaw_catalogue_follows_the_agents_list() {
        let (_dir, home) = temp_home();
        let root = home.join(".openclaw");
        std::fs::create_dir_all(&root).unwrap();
        let catalog_of =
            |vars: &ShellVars| takeover_paths("openclaw", &home, vars).unwrap()[1].clone();
        let write_cfg = |text: &str| std::fs::write(root.join("openclaw.json"), text).unwrap();

        // No config on disk: the default agent's runtime directory.
        assert_eq!(
            catalog_of(&no_vars()),
            root.join("agents")
                .join("main")
                .join("agent")
                .join("models.json")
        );

        // The entry marked default wins over the first, and its id goes
        // through OpenClaw's normalization (spaces are not id characters).
        write_cfg(r#"{"agents":{"list":[{"id":"Work Bot"},{"id":"Second One","default":true}]}}"#);
        assert_eq!(
            catalog_of(&no_vars()),
            root.join("agents")
                .join("second-one")
                .join("agent")
                .join("models.json")
        );

        // Without a default mark the first entry is the default agent.
        write_cfg(r#"{"agents":{"list":[{"id":"Work Bot"}]}}"#);
        assert_eq!(
            catalog_of(&no_vars()),
            root.join("agents")
                .join("work-bot")
                .join("agent")
                .join("models.json")
        );

        // An agentDir override moves the catalogue: `~` expands against home…
        write_cfg(r#"{"agents":{"list":[{"id":"main","agentDir":"~/oc-agent"}]}}"#);
        assert_eq!(
            catalog_of(&no_vars()),
            home.join("oc-agent").join("models.json")
        );

        // …and `$VAR`/`${VAR}` against the environment Kiwano was handed. The
        // value sits under `home` so it is absolute on every platform — a
        // literal `/tmp/...` is relative on Windows, where the rule under
        // test refuses exactly that (see `abs_dir`).
        let oc_root = home.join("oc-root");
        let mut vars = no_vars();
        vars.insert("OC_ROOT".into(), oc_root.to_string_lossy().into_owned());
        write_cfg(r#"{"agents":{"list":[{"id":"main","agentDir":"${OC_ROOT}/agent"}]}}"#);
        assert_eq!(catalog_of(&vars), oc_root.join("agent").join("models.json"));
        write_cfg(r#"{"agents":{"list":[{"id":"main","agentDir":"$OC_ROOT/agent"}]}}"#);
        assert_eq!(catalog_of(&vars), oc_root.join("agent").join("models.json"));

        // A relative override or an unknown variable is refused rather than
        // guessed at — both would write the catalogue where the runtime never
        // looks.
        write_cfg(r#"{"agents":{"list":[{"id":"main","agentDir":"somewhere"}]}}"#);
        assert!(takeover_paths("openclaw", &home, &no_vars()).is_err());
        write_cfg(r#"{"agents":{"list":[{"id":"main","agentDir":"$NOPE/agent"}]}}"#);
        assert!(takeover_paths("openclaw", &home, &no_vars()).is_err());
    }

    /// Cline resolves its provider settings through three levels (its own
    /// `sdk/…/storage/paths.ts`), and each one has to be honored: a level this
    /// ignored is a takeover written where the tool does not read.
    #[test]
    fn cline_paths_follow_its_three_level_chain() {
        let _guards = (
            EnvGuard::set("CLINE_PROVIDER_SETTINGS_PATH", None),
            EnvGuard::set("CLINE_DATA_DIR", None),
            EnvGuard::set("CLINE_DIR", None),
        );
        let tmp = tempfile::tempdir().unwrap();
        let home = abs_dir(&tmp, "home");
        let home = home.as_path();
        let suffix = Path::new("settings").join("providers.json");

        // Nobody said otherwise: the base is `~/.cline`, and the data directory
        // is the `data` under it.
        assert_eq!(
            takeover_paths("cline", home, &no_vars()).unwrap(),
            vec![home.join(".cline").join("data").join(&suffix)]
        );

        let base = abs_dir(&tmp, "base");
        let vars =
            ShellVars::from([("CLINE_DIR".to_string(), base.to_string_lossy().into_owned())]);
        assert_eq!(
            takeover_paths("cline", home, &vars).unwrap(),
            vec![base.join("data").join(&suffix)]
        );

        // CLINE_DATA_DIR moves the data directory itself and beats the base
        // when both are set — the order Cline resolves them in.
        let data = abs_dir(&tmp, "data");
        let vars = ShellVars::from([
            ("CLINE_DIR".to_string(), base.to_string_lossy().into_owned()),
            (
                "CLINE_DATA_DIR".to_string(),
                data.to_string_lossy().into_owned(),
            ),
        ]);
        assert_eq!(
            takeover_paths("cline", home, &vars).unwrap(),
            vec![data.join(&suffix)]
        );

        // The exact file wins over both, and it is a *file* rather than a
        // directory — the one level `config_dir` cannot express.
        let file = abs_dir(&tmp, "elsewhere").join("providers.json");
        let vars = ShellVars::from([
            (
                "CLINE_DATA_DIR".to_string(),
                data.to_string_lossy().into_owned(),
            ),
            (
                "CLINE_PROVIDER_SETTINGS_PATH".to_string(),
                file.to_string_lossy().into_owned(),
            ),
        ]);
        assert_eq!(takeover_paths("cline", home, &vars).unwrap(), vec![file]);

        // A relative value is refused rather than fallen through to the
        // default: the default is right when nobody said otherwise, and a guess
        // when somebody did.
        let vars = ShellVars::from([(
            "CLINE_PROVIDER_SETTINGS_PATH".to_string(),
            "rel/providers.json".to_string(),
        )]);
        let err = takeover_paths("cline", home, &vars).unwrap_err();
        assert!(
            err.contains("CLINE_PROVIDER_SETTINGS_PATH is set to `rel/providers.json`"),
            "{err}"
        );
    }
}
