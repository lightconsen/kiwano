//! Agent installation detection — where each supported CLI lives, on macOS,
//! Linux and Windows.
//!
//! Two sources, tried in this order, because they answer different questions:
//!
//! 1. **The user's login shell** (unix only). On macOS the GUI inherits
//!    launchd's narrow PATH and on Linux the session's, neither of which is the
//!    PATH anyone actually uses — asking the user's own shell, interactively,
//!    is the only way to see an install that lives where the user put it. The
//!    vendored approach below cannot reach those: it walks a fixed list of
//!    directories, and a user's custom directory is by definition not on it.
//!    The shell is asked for its environment rather than for a `command -v` per
//!    tool, because a shell *script* has to be written in that shell's syntax —
//!    see [`tool_paths_from_shell_env`]. Windows is skipped here: a GUI process
//!    gets the full user environment there, and [`effective_path`] merges the
//!    registry's user and machine PATH on top, so the walk below already sees
//!    everything the user would.
//! 2. **The well-known install locations** (every platform). Node version
//!    managers, Homebrew, npm prefixes, and the standalone installers' own
//!    directories — the places a tool lands when the user never put it on PATH
//!    at all, which is also the only source Windows has.
//!
//! A tool that is missing never fails the probe: it is simply not in the map.
//! Only a shell that cannot be spawned at all is an error, and even then the
//! directory walk still runs — "probe unavailable" must not be reported as
//! "nothing is installed", which is what would hide every agent tab in the UI.
//!
//! Phase 2 (`probe_agent_versions`) runs `<bin> --version` per resolved binary;
//! purely cosmetic (tooltips), never gates the installed verdict.
//!
//! Ported in part from cc-switch (https://github.com/farion1231/cc-switch),
//! licensed under the MIT License. Source: src-tauri/src/commands/misc.rs
//! (`build_tool_search_paths`, `tool_executable_candidates`,
//! `effective_path_*`, the `CommandDeadline`/`wait_child_output` pattern).
//! Reduced for Kiwano: no upgrade anchoring, no conflict diagnosis, no WSL, and
//! a first-hit search rather than a full enumeration — this module answers
//! "installed, where, which version", nothing else.

use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use serde::Serialize;

/// (agent id, CLI executable name) — the GUI-only agents (claude-desktop,
/// workbuddy) have no CLI and are checked apart.
const CLI_AGENTS: &[(&str, &str)] = &[
    ("claude", "claude"),
    ("codex", "codex"),
    ("gemini", "gemini"),
    ("grokbuild", "grok"),
    ("opencode", "opencode"),
    ("openclaw", "openclaw"),
    ("hermes", "hermes"),
    ("pi", "pi"),
    ("codebuddy", "codebuddy"),
    ("kimi", "kimi"),
    ("qwen", "qwen"),
    ("cline", "cline"),
    ("mimo", "mimo"),
    ("mcode", "mcode"),
    ("aider", "aider"),
    ("continue", "cn"),
    ("crush", "crush"),
    ("droid", "droid"),
    ("goose", "goose"),
];

/// One pass over every tool, through the login shell — whose only reader is the
/// unix half of this module, hence the cfg: the walk needs no timeout of its own.
/// Long enough for a slow interactive rc file, short enough that a wedged shell
/// cannot stall the first render.
#[cfg(unix)]
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Environment values as the user's own login shell has them — see
/// [`login_shell_vars`]. Passed around as a value so the modules that need one
/// (the takeover's config paths) stay free of the process environment, which is
/// the thing that cannot see it.
pub type ShellVars = BTreeMap<String, String>;

/// One `<bin> --version`.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Serialize)]
pub struct AgentDetectVm {
    pub agent: String,
    pub installed: bool,
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AgentVersionVm {
    pub agent: String,
    pub version: Option<String>,
}

/// Phase 1: which agents are installed, and where.
///
/// `manual` carries the directories the user declared for agents the walk
/// cannot find by itself, keyed by agent id — see [`verify_manual_dir`] for the
/// check a declaration has to pass before it gets here. It is an argument
/// rather than a store read because this module knows nothing about the
/// database, and that is what keeps it testable.
pub fn detect_agents(home: &Path, manual: &BTreeMap<String, PathBuf>) -> Vec<AgentDetectVm> {
    let cli = find_agent_binaries(home, manual);
    let mut vms: Vec<AgentDetectVm> = CLI_AGENTS
        .iter()
        .map(|(agent, cli_name)| {
            let path = cli.get(*cli_name).map(|p| p.display().to_string());
            AgentDetectVm {
                agent: (*agent).into(),
                installed: path.is_some(),
                path,
            }
        })
        .collect();
    vms.push(AgentDetectVm {
        agent: "claude-desktop".into(),
        installed: claude_desktop_installed(home),
        path: None,
    });
    vms.push(AgentDetectVm {
        agent: "workbuddy".into(),
        installed: workbuddy_installed(home),
        path: None,
    });
    vms
}

/// Phase 2: version strings for the installed CLI agents (tooltips only).
///
/// Slow by construction — one `--version` subprocess per agent — which is why
/// it is a separate call from [`detect_agents`] and why a caller should make it
/// opt-in.
pub fn probe_agent_versions(
    home: &Path,
    manual: &BTreeMap<String, PathBuf>,
) -> Vec<AgentVersionVm> {
    let cli = find_agent_binaries(home, manual);
    let mut vms: Vec<AgentVersionVm> = CLI_AGENTS
        .iter()
        .map(|(agent, cli_name)| AgentVersionVm {
            agent: (*agent).into(),
            version: cli.get(*cli_name).and_then(|p| probe_version(p)),
        })
        .collect();
    // The desktop apps have no CLI; reading their Info.plist is not worth it here.
    vms.push(AgentVersionVm {
        agent: "claude-desktop".into(),
        version: None,
    });
    vms.push(AgentVersionVm {
        agent: "workbuddy".into(),
        version: None,
    });
    vms
}

/// Every agent's binary: the shell's answer first (it is the PATH the user
/// actually has), the well-known directories second (for what the shell cannot
/// see). Keyed by executable name, the form both sources produce.
fn find_agent_binaries(
    home: &Path,
    manual: &BTreeMap<String, PathBuf>,
) -> BTreeMap<String, PathBuf> {
    let mut found = shell_probe_all();
    for (agent, cli) in CLI_AGENTS {
        if found.contains_key(*cli) {
            continue;
        }
        // Declarations arrive keyed by agent id; the walk is by executable
        // name, so this is where the two vocabularies meet.
        let declared: Vec<PathBuf> = manual.get(*agent).into_iter().cloned().collect();
        if let Some(path) = search_well_known_dirs(cli, home, &declared) {
            found.insert((*cli).to_string(), path);
        }
    }
    found
}

// ── source 1: the user's login shell (unix) ────────────────────────────────

/// Every tool the user's own shell can resolve, through the environment it
/// reports (`env` — see [`tool_paths_from_shell_env`] for why not `command -v`).
///
/// Failure is not fatal: an empty map means "the shell had nothing to say", and
/// the directory walk still runs on top of it.
#[cfg(unix)]
fn shell_probe_all() -> BTreeMap<String, PathBuf> {
    match run_login_shell("env") {
        Some(out) => tool_paths_from_shell_env(&out),
        None => BTreeMap::new(),
    }
}

/// Run one command in the user's interactive login shell and return its stdout.
///
/// Interactive (`-lic`) rather than plain login (`-lc`) because that is what
/// reads `.zshrc`/`.bashrc` — where a great many installs are actually put on
/// PATH, and where the variables that move an agent's files are usually
/// exported. It costs an interactive rc file being evaluated, so the child gets
/// a null stdin (nothing can block on a prompt), its own session (no
/// controlling terminal to be stopped by), and a deadline.
///
/// `None` covers every way of getting no answer — no spawn, a wedged shell, a
/// non-zero exit — because callers treat them the same: nothing was learned,
/// so nothing changes.
#[cfg(unix)]
fn run_login_shell(script: &str) -> Option<String> {
    use std::os::unix::process::CommandExt;

    let shell = std::env::var("SHELL")
        .ok()
        .filter(|s| is_known_shell(s))
        .unwrap_or_else(|| "sh".into());
    let mut cmd = Command::new(&shell);
    cmd.arg(login_probe_flag(&shell))
        .arg(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // SAFETY: setsid is async-signal-safe, and the child is never a session
    // leader (fork gives it the parent's group), so this cannot fail with EPERM.
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().ok()?;
    let out = wait_with_timeout(child, PROBE_TIMEOUT)?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The named variables as the user's own shell has them.
///
/// This is source 1's blindness applied to values rather than binaries: the GUI
/// inherits launchd's environment, so a variable exported from `.zshrc` — which
/// is where a relocated config directory is declared — is invisible to it, and
/// a default would be written where the user's tool never looks. The shell is
/// the only place that knows.
///
/// Absence is an answer. A variable the shell does not have is missing from the
/// map, and a shell that cannot be spawned answers with an empty map; both
/// leave every default in place, which is what a machine with no overrides
/// wants. Values come back trimmed and otherwise verbatim — whether one names a
/// usable directory is the caller's business, not this function's.
///
/// The script is `env` rather than a shell loop over `$NAME`, because the loop
/// syntax differs between the shells this can be pointed at (`for x in …; do`
/// against fish's `for x in …; end`) and `env` is one word that means the same
/// thing everywhere.
#[cfg(unix)]
pub fn login_shell_vars(names: &[&str]) -> ShellVars {
    if names.is_empty() {
        return ShellVars::new();
    }
    let mut vars = match run_login_shell("env") {
        Some(out) => parse_vars(&out, names),
        None => ShellVars::new(),
    };
    fill_from_process_env(&mut vars, names);
    vars
}

/// Fill in what the shell did not answer, from this process's own environment.
///
/// The shell's answer is a *supplement* to it rather than a replacement: a GUI
/// misses what an rc file exports (which is why the shell is asked at all), and a
/// shell that cannot be spawned has nothing to say — in both cases what this
/// process has is the answer. Doing it here, once, is what lets every reader
/// downstream take the map as the whole environment and read no ambient state.
fn fill_from_process_env(vars: &mut ShellVars, names: &[&str]) {
    for name in names {
        if !vars.contains_key(*name) {
            if let Ok(value) = std::env::var(name) {
                vars.insert((*name).to_string(), value);
            }
        }
    }
}

/// Windows: nothing to ask. A GUI process inherits the real user environment
/// there, so the process environment already is the user's — see
/// [`effective_path`] for the same reasoning on PATH.
#[cfg(not(unix))]
pub fn login_shell_vars(names: &[&str]) -> ShellVars {
    // No shell to ask, and none needed: this process's environment is the
    // user's — the same reason [`shell_probe_all`] does not run here.
    let mut vars = ShellVars::new();
    fill_from_process_env(&mut vars, names);
    vars
}

/// `NAME=value` lines from `env` output, keeping only the names asked for.
///
/// Split at the first `=`, since a value may contain one. A name that appears
/// twice keeps the *last* occurrence, and that is the one rule this shares with
/// the tool probe: an interactive rc file's own output — a banner, a version
/// notice — lands on the same stdout ahead of the command's, so anything that
/// looks like the line we want is more likely noise the earlier it appears.
#[cfg(unix)]
fn parse_vars(out: &str, names: &[&str]) -> ShellVars {
    let mut vars = ShellVars::new();
    for line in out.lines() {
        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if names.contains(&name) {
            vars.insert(name.to_string(), value.to_string());
        }
    }
    vars
}

/// Windows: nothing to ask. A GUI process inherits the real user environment
/// and [`effective_path`] merges the registry PATH into it, so the directory
/// walk below already sees what a shell would report.
#[cfg(not(unix))]
fn shell_probe_all() -> BTreeMap<String, PathBuf> {
    BTreeMap::new()
}

/// The flag that makes a shell run one command as an interactive login shell.
/// `sh`/`dash` take `-c` (they have no interactive rc worth reading); `fish`
/// spells the combination `-lc`; the rest accept `-lic`.
#[cfg(unix)]
fn login_probe_flag(shell: &str) -> &'static str {
    match Path::new(shell)
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("sh")
    {
        "sh" | "dash" => "-c",
        "fish" => "-lc",
        _ => "-lic",
    }
}

#[cfg(unix)]
fn is_known_shell(path: &str) -> bool {
    matches!(
        Path::new(path).file_name().and_then(OsStr::to_str),
        Some("sh" | "bash" | "zsh" | "fish" | "dash")
    )
}

/// Every agent's binary, resolved against the PATH the user's own shell reports.
///
/// The tools are resolved here rather than by `command -v` inside the shell, and
/// the reason is syntax: a loop is the one construct these shells do not share —
/// fish ends a block with `end`, POSIX shells with `done` — so a script written
/// for one silently reports nothing under the other. (It did: a fish user never
/// got this source at all, and the directory walk below was their only one.)
/// `env` is one word that means the same thing in every shell, and the PATH it
/// prints is the whole of what the shell had to say.
///
/// Nothing is lost by resolving it here. `command -v` also answers with aliases
/// and functions, but that answer is not a path — the old probe required an
/// absolute one — so those were discarded before they reached the walk.
///
/// A PATH entry that is not absolute is skipped, the rule the walk applies for
/// the same reason: it would resolve against whatever directory the process
/// happened to be in.
///
/// The last `PATH=` line wins rather than the first, for the same reason
/// [`parse_vars`] does: an interactive rc file's output reaches this same stdout
/// ahead of the command's, and `env` prints each variable once.
#[cfg(unix)]
fn tool_paths_from_shell_env(env: &str) -> BTreeMap<String, PathBuf> {
    let path = env
        .lines()
        .filter_map(|line| line.strip_prefix("PATH="))
        .next_back()
        .unwrap_or_default();
    let dirs: Vec<PathBuf> = std::env::split_paths(OsStr::new(path))
        .filter(|dir| dir.is_absolute())
        .collect();

    let mut found = BTreeMap::new();
    for (_, cli) in CLI_AGENTS {
        for dir in &dirs {
            if let Some(hit) = executable_candidates(cli, dir)
                .into_iter()
                .find(|candidate| candidate.is_file())
            {
                found.insert((*cli).to_string(), hit);
                break;
            }
        }
    }
    found
}

// ── source 2: the well-known install locations (every platform) ─────────────

/// An environment-variable lookup, injected so a test can decide what the walk
/// sees.
pub(crate) type VarLookup = Box<dyn Fn(&str) -> Option<OsString>>;

/// The environment the walk reads: the home directory, and the PATH the child
/// probes are given. A struct rather than direct `std::env` reads so a test can
/// point the walk at a temp directory and see it work.
pub(crate) struct SearchEnv {
    pub(crate) home: PathBuf,
    pub(crate) path: OsString,
    /// Env-var lookups the location list needs (`$PNPM_HOME`, `$XDG_BIN_DIR`,
    /// …). Injected so tests do not depend on the ambient environment.
    pub(crate) var: VarLookup,
    /// Directories the user declared for the agent being looked for
    /// (`Store::manual_agent_dirs`). Searched after the well-known locations
    /// and before PATH: a declaration is the user's word, but the well-known
    /// locations are what this module is sure of, and the sure thing goes
    /// first.
    pub(crate) manual: Vec<PathBuf>,
}

impl SearchEnv {
    /// The real environment of this process.
    fn from_process(home: &Path, manual: &[PathBuf]) -> SearchEnv {
        SearchEnv {
            home: home.to_path_buf(),
            path: effective_path(),
            var: Box::new(|name| std::env::var_os(name)),
            manual: manual.to_vec(),
        }
    }

    /// A variable's value as a directory to search, or nothing.
    ///
    /// `Unset` is nothing because there is no default to apply here — the walk
    /// has its own list of directories — and `Relative` is nothing because
    /// searching our own working directory for it could only ever find a file
    /// that happens to share the name. The rule itself is
    /// `kiwano_adapters::config::classify_dir`, shared with the writes
    /// (`takeover::named_dir`) so the two cannot disagree about a value.
    fn var_str(&self, name: &str) -> Option<PathBuf> {
        kiwano_adapters::config::classify_dir((self.var)(name).as_deref()).absolute()
    }
}

/// First hit across the well-known directories, in priority order.
fn search_well_known_dirs(name: &str, home: &Path, manual: &[PathBuf]) -> Option<PathBuf> {
    search_binary_in(name, &SearchEnv::from_process(home, manual))
}

/// The walk itself, against a caller-supplied environment — which is what lets
/// a test point it at a temp directory instead of the machine.
fn search_binary_in(name: &str, env: &SearchEnv) -> Option<PathBuf> {
    for dir in search_paths(name, env) {
        for candidate in executable_candidates(name, &dir) {
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}

/// The candidate directories for one tool, most specific first. The order is
/// the priority: a native install beats a stale package-manager shim.
fn search_paths(name: &str, env: &SearchEnv) -> Vec<PathBuf> {
    let home = env.home.as_path();
    let mut paths: Vec<PathBuf> = Vec::new();

    // Tool-specific locations, because these installers do not use a shared
    // prefix: `$GROK_BIN_DIR`, and OpenCode's documented list.
    if name == "grok" {
        if let Some(dir) = env.var_str("GROK_BIN_DIR") {
            push_unique(&mut paths, dir);
        }
        push_unique(&mut paths, home.join(".grok").join("bin"));
    }
    if name == "opencode" {
        for var in ["OPENCODE_INSTALL_DIR", "XDG_BIN_DIR"] {
            if let Some(dir) = env.var_str(var) {
                push_unique(&mut paths, dir);
            }
        }
        push_unique(&mut paths, home.join("bin"));
        push_unique(&mut paths, home.join(".opencode").join("bin"));
        push_unique(&mut paths, home.join(".bun").join("bin"));
        push_unique(&mut paths, home.join("go").join("bin"));
        if let Some(gopath) = env.var_str("GOPATH") {
            for entry in std::env::split_paths(&gopath) {
                push_unique(&mut paths, entry.join("bin"));
            }
        }
    }

    // The shared per-user prefixes: where `pip install --user`, `npm -g` with a
    // configured prefix, `n`/`volta`, and the version managers put binaries.
    push_unique(&mut paths, home.join(".local").join("bin"));
    push_unique(&mut paths, home.join(".npm-global").join("bin"));
    push_unique(&mut paths, home.join("n").join("bin"));
    push_unique(&mut paths, home.join(".volta").join("bin"));
    push_unique(
        &mut paths,
        home.join(".local").join("share").join("mise").join("shims"),
    );
    push_children_bin(&mut paths, &home.join(".local/share/mise/installs/node"));
    push_children_bin(&mut paths, &home.join(".local/state/fnm_multishells"));
    push_children_bin(&mut paths, &home.join(".nvm").join("versions").join("node"));

    #[cfg(target_os = "macos")]
    {
        push_unique(&mut paths, PathBuf::from("/opt/homebrew/bin"));
        push_unique(&mut paths, PathBuf::from("/usr/local/bin"));
        if name == "hermes" {
            push_children_bin(&mut paths, &home.join("Library").join("Python"));
        }
    }

    #[cfg(target_os = "linux")]
    {
        push_unique(&mut paths, PathBuf::from("/usr/local/bin"));
        push_unique(&mut paths, PathBuf::from("/usr/bin"));
    }

    #[cfg(target_os = "windows")]
    {
        // The standalone (non-npm) installers, ahead of the package-manager
        // prefixes so a native install wins over a stale npm shim (#4701).
        if let Some(local) = dirs::data_local_dir() {
            if name == "codex" {
                push_unique(
                    &mut paths,
                    local
                        .join("Programs")
                        .join("OpenAI")
                        .join("Codex")
                        .join("bin"),
                );
            }
            if name == "claude" {
                push_unique(&mut paths, local.join("Programs").join("claude"));
            }
            push_unique(&mut paths, local.join("pnpm"));
            push_unique(&mut paths, local.join("Volta").join("bin"));
            push_unique(&mut paths, local.join("Yarn").join("bin"));
            if name == "hermes" {
                push_children_bin(&mut paths, &local.join("Programs/Python"));
            }
        }
        if let Some(appdata) = dirs::data_dir() {
            push_unique(&mut paths, appdata.join("npm"));
            if name == "hermes" {
                push_children_bin(&mut paths, &appdata.join("Python"));
            }
            push_children(&mut paths, &appdata.join("nvm"));
        }
        if let Some(pnpm_home) = env.var_str("PNPM_HOME") {
            push_unique(&mut paths, pnpm_home);
        }
        if let Some(volta_home) = env.var_str("VOLTA_HOME") {
            push_unique(&mut paths, volta_home.join("bin"));
        }
        if let Some(nvm_symlink) = env.var_str("NVM_SYMLINK") {
            push_unique(&mut paths, nvm_symlink);
        }
        for var in ["SCOOP", "SCOOP_GLOBAL"] {
            if let Some(root) = env.var_str(var) {
                push_unique(&mut paths, root.join("shims"));
            }
        }
        if let Some(program_data) = env.var_str("ProgramData") {
            push_unique(&mut paths, program_data.join("scoop").join("shims"));
        }
        push_unique(&mut paths, home.join("scoop").join("shims"));
        push_unique(&mut paths, PathBuf::from("C:\\Program Files\\nodejs"));
    }

    // The directory the user declared for this tool, if the locations above
    // missed it — the case this whole path exists for.
    for dir in &env.manual {
        push_unique(&mut paths, dir.clone());
    }

    // Last: whatever PATH says. On unix this is narrow (the GUI's inherited
    // PATH, not the user's) — that is what source 1 is for — and on Windows it
    // is the merged user+machine PATH, which is a real answer.
    //
    // Only the absolute entries: a relative one means "the current directory",
    // and the current directory of a GUI app is wherever it was launched from,
    // which is not a place the user's tools live. A literal `~` is not expanded
    // by anyone, so it is dead weight too — it never names a directory that
    // exists. Both come from someone's dotfile and both are skipped here.
    for entry in std::env::split_paths(&env.path) {
        if entry.is_absolute() {
            push_unique(&mut paths, entry);
        }
    }

    paths
}

/// The executable names to try in one directory.
///
/// Windows: `.cmd` first, then `.exe`, then the bare name — and the bare name
/// only when no `.cmd`/`.exe` sibling exists, because npm leaves a POSIX
/// `#!/bin/sh` shim beside its `.cmd` on Windows and running that shim fails
/// with a shell error rather than a version string.
fn executable_candidates(name: &str, dir: &Path) -> Vec<PathBuf> {
    #[cfg(windows)]
    {
        let extensionless = dir.join(name);
        let mut candidates = vec![
            dir.join(format!("{name}.cmd")),
            dir.join(format!("{name}.exe")),
        ];
        if !candidates.iter().any(|c| c.is_file()) {
            candidates.push(extensionless);
        }
        candidates
    }

    #[cfg(not(windows))]
    {
        vec![dir.join(name)]
    }
}

fn push_unique(paths: &mut Vec<PathBuf>, path: PathBuf) {
    if path.as_os_str().is_empty() {
        return;
    }
    if !paths.iter().any(|existing| existing == &path) {
        paths.push(path);
    }
}

/// Every existing child of `base`, with `bin` appended (version managers keep
/// one directory per version).
fn push_children_bin(paths: &mut Vec<PathBuf>, base: &Path) {
    if !base.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            let bin = entry.path().join("bin");
            if bin.is_dir() {
                push_unique(paths, bin);
            }
        }
    }
}

/// Every existing child of `base` itself (nvm on Windows keeps the binaries
/// directly under each version directory).
#[cfg(windows)]
fn push_children(paths: &mut Vec<PathBuf>, base: &Path) {
    if !base.is_dir() {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(base) {
        for entry in entries.flatten() {
            if entry.path().is_dir() {
                push_unique(paths, entry.path());
            }
        }
    }
}

// ── the effective PATH ──────────────────────────────────────────────────────

/// The PATH the walk ends with.
#[cfg(unix)]
fn effective_path() -> OsString {
    // Kept as an `OsString` on purpose: a unix environment value may hold
    // arbitrary non-NUL bytes, and one non-UTF-8 segment converted lossily
    // would drop or corrupt every directory after it.
    std::env::var_os("PATH").unwrap_or_default()
}

/// Windows: the process PATH merged with the registry's user and machine PATH.
///
/// A process started by the MSI/WiX updater inherits only the machine-level
/// PATH and loses the user-level one, so a freshly self-updated app would stop
/// seeing the tools the user installed for themselves. Merging the registry
/// values back in restores what a newly logged-in shell would see.
#[cfg(windows)]
fn effective_path() -> OsString {
    use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
    use winreg::RegKey;

    let process = std::env::var("PATH").unwrap_or_default();
    let user = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey("Environment")
        .and_then(|k| k.get_value::<String, _>("Path"))
        .map(|raw| expand_env_chars(&raw))
        .unwrap_or_default();
    let machine = RegKey::predef(HKEY_LOCAL_MACHINE)
        .open_subkey("SYSTEM\\CurrentControlSet\\Control\\Session Manager\\Environment")
        .and_then(|k| k.get_value::<String, _>("Path"))
        .map(|raw| expand_env_chars(&raw))
        .unwrap_or_default();
    OsString::from(merge_path_segments(&[&process, &user, &machine]))
}

/// Expand `%VAR%` references the way the shell would, leaving an unknown name
/// as the literal text (which is what Windows itself does).
#[cfg(windows)]
fn expand_env_chars(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let bytes: Vec<char> = raw.chars().collect();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == '%' {
            if let Some(end) = bytes[i + 1..].iter().position(|c| *c == '%') {
                let name: String = bytes[i + 1..i + 1 + end].iter().collect();
                let known =
                    !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_');
                if known {
                    match std::env::var(&name) {
                        Ok(value) => out.push_str(&value),
                        Err(_) => out.push_str(&raw[i..i + end + 2]),
                    }
                    i += end + 2;
                    continue;
                }
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    out
}

/// Merge Windows PATH segments: process first, then user, then machine; blanks
/// dropped; duplicates removed case-insensitively (Windows paths are).
#[cfg(windows)]
fn merge_path_segments(parts: &[&str]) -> String {
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    let mut merged: Vec<&str> = Vec::new();
    for part in parts {
        for segment in part.split(';') {
            let segment = segment.trim();
            if segment.is_empty() || !seen.insert(segment.to_ascii_lowercase()) {
                continue;
            }
            merged.push(segment);
        }
    }
    merged.join(";")
}

// ── running a tool ──────────────────────────────────────────────────────────

/// Run `<bin> --version` and return its first non-empty output line.
fn probe_version(bin: &Path) -> Option<String> {
    let out = run_tool(bin, &["--version"], VERSION_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    parse_version_output(&decode_command_output(&out.stdout))
}

/// Run a tool with a deadline, in its own session so a wedged child can be
/// killed as a group.
fn run_tool(bin: &Path, args: &[&str], timeout: Duration) -> Option<std::process::Output> {
    let mut cmd = tool_command(bin, args);
    cmd.stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: as in `shell_probe_all` — async-signal-safe, never a group
        // leader, so it cannot fail.
        unsafe {
            cmd.pre_exec(|| {
                if libc::setsid() == -1 {
                    return Err(std::io::Error::last_os_error());
                }
                Ok(())
            });
        }
    }
    let child = cmd.spawn().ok()?;
    wait_with_timeout(child, timeout)
}

/// The command that runs one tool. On Windows a `.cmd`/`.bat` is a script, not
/// an executable — `CreateProcess` cannot start it, so it goes through
/// `cmd /C call`, and a canonicalized path arrives with a `\\?\` prefix that
/// `cmd` rejects until it is stripped.
fn tool_command(bin: &Path, args: &[&str]) -> Command {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;

        if is_windows_script(bin) {
            let shell_path = windows_shell_compatible_path(bin);
            let quoted = shell_path.to_string_lossy();
            let quoted = if quoted.contains(' ') || quoted.contains('%') {
                format!("\"{}\"", quoted.replace('"', "\\\""))
            } else {
                quoted.into_owned()
            };
            let mut command = format!("call {quoted}");
            for arg in args {
                command.push(' ');
                command.push_str(arg);
            }
            let mut cmd = Command::new("cmd");
            cmd.args(["/D", "/S", "/C"])
                .raw_arg(command)
                .creation_flags(CREATE_NO_WINDOW);
            return cmd;
        }

        let mut cmd = Command::new(bin);
        cmd.args(args).creation_flags(CREATE_NO_WINDOW);
        cmd
    }

    #[cfg(not(windows))]
    {
        let mut cmd = Command::new(bin);
        cmd.args(args);
        cmd
    }
}

#[cfg(windows)]
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[cfg(windows)]
fn is_windows_script(path: &Path) -> bool {
    path.extension()
        .and_then(OsStr::to_str)
        .is_some_and(|ext| ext.eq_ignore_ascii_case("cmd") || ext.eq_ignore_ascii_case("bat"))
}

/// `std::fs::canonicalize` prefixes local paths with `\\?\` (and UNC paths with
/// `\\?\UNC\`), which `cmd.exe` cannot `call` a batch file through. Keep the
/// canonical form everywhere else — it is the install's identity — and strip it
/// only here, at the shell boundary.
#[cfg(windows)]
fn windows_shell_compatible_path(path: &Path) -> PathBuf {
    let raw = path.to_string_lossy();
    if let Some(unc) = raw.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{unc}"))
    } else if let Some(local) = raw.strip_prefix(r"\\?\") {
        PathBuf::from(local)
    } else {
        path.to_path_buf()
    }
}

/// Text from a child process. Windows CLIs still emit their OEM code page, so
/// a plain UTF-8 decode mangles them; the fallback decodes with the active code
/// page.
fn decode_command_output(bytes: &[u8]) -> String {
    #[cfg(windows)]
    {
        if let Ok(text) = std::str::from_utf8(bytes) {
            return text.to_string();
        }
        decode_windows_command_output(bytes)
    }

    #[cfg(not(windows))]
    {
        String::from_utf8_lossy(bytes).into_owned()
    }
}

#[cfg(windows)]
fn decode_windows_command_output(bytes: &[u8]) -> String {
    use windows_sys::Win32::Globalization::{GetACP, GetOEMCP, MultiByteToWideChar};

    let mut best = String::from_utf8_lossy(bytes).into_owned();
    // SAFETY: neither takes an argument or touches memory — each returns the
    // number of a code page in this process. The conversion below, which does
    // the pointer work, has its own block.
    for codepage in unsafe { [GetOEMCP(), GetACP()] } {
        let wide_len = unsafe {
            MultiByteToWideChar(
                codepage,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                std::ptr::null_mut(),
                0,
            )
        };
        if wide_len <= 0 {
            continue;
        }
        let mut wide = vec![0u16; wide_len as usize];
        let written = unsafe {
            MultiByteToWideChar(
                codepage,
                0,
                bytes.as_ptr(),
                bytes.len() as i32,
                wide.as_mut_ptr(),
                wide_len,
            )
        };
        if written <= 0 {
            continue;
        }
        wide.truncate(written as usize);
        if let Ok(text) = String::from_utf16(&wide) {
            if !text.contains('\u{fffd}') {
                best = text;
                break;
            }
        }
    }
    best
}

fn parse_version_output(out: &str) -> Option<String> {
    out.lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .map(String::from)
}

/// Poll `try_wait` until exit or deadline; on timeout the child's whole process
/// group is killed and `None` returned.
///
/// The group kill matters more than it looks: the probe runs the user's
/// interactive shell, and a shell that spawned something of its own — a version
/// manager, a wrapper script — leaves a child holding the pipe open. Killing
/// only the direct child would leave the read hanging on it.
fn wait_with_timeout(
    mut child: std::process::Child,
    timeout: Duration,
) -> Option<std::process::Output> {
    let deadline = Instant::now() + timeout;
    loop {
        if let Some(status) = child.try_wait().ok()? {
            let mut stdout = Vec::new();
            let mut stderr = Vec::new();
            if let Some(mut out) = child.stdout.take() {
                let _ = out.read_to_end(&mut stdout);
            }
            if let Some(mut err) = child.stderr.take() {
                let _ = err.read_to_end(&mut stderr);
            }
            return Some(std::process::Output {
                status,
                stdout,
                stderr,
            });
        }
        if Instant::now() >= deadline {
            terminate_child_tree(&mut child);
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Kill the child and everything it started.
#[cfg(unix)]
fn terminate_child_tree(child: &mut std::process::Child) {
    let group = -(child.id() as libc::pid_t);
    // SAFETY: the child was placed in its own session (and therefore its own
    // process group) before it was spawned, so this signal reaches that group
    // and nothing of ours.
    if unsafe { libc::kill(group, libc::SIGKILL) } != 0 {
        let _ = child.kill();
    }
    let _ = child.wait();
}

#[cfg(windows)]
fn terminate_child_tree(child: &mut std::process::Child) {
    use std::os::windows::process::CommandExt;

    // /T kills the tree: a `.cmd` shim runs as `cmd` with children of its own.
    let killed = Command::new("taskkill")
        .args(["/PID", &child.id().to_string(), "/T", "/F"])
        .creation_flags(CREATE_NO_WINDOW)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success());
    if !killed {
        let _ = child.kill();
    }
    let _ = child.wait();
}

// ── a directory the user declared ───────────────────────────────────────────

/// Every directory the walk tries for `agent`, in the order it tries them.
///
/// This is the walk's own list ([`search_paths`]) rather than a description of
/// it, which is the point: a dialog that says "we looked and it was not there"
/// should be showing what the detector did, not a hand-kept second copy that
/// drifts from it. For an agent the walk *did* find, the list is still every
/// place it looked — so it answers "where did you look", not "where was it".
///
/// A declaration is passed the same way the walk takes it, so a declared
/// directory is in the list too: it is a place the walk consults.
pub fn agent_search_dirs(agent: &str, home: &Path, manual: &[PathBuf]) -> Vec<PathBuf> {
    let Some((_, cli)) = CLI_AGENTS.iter().find(|(id, _)| *id == agent) else {
        return Vec::new();
    };
    search_paths(cli, &SearchEnv::from_process(home, manual))
}

/// Check a directory the user pointed at for `agent`, using the same search
/// and the same version probe the walk uses — there is no second opinion about
/// what counts as this tool's executable.
///
/// What it can confirm: the directory holds a file with the agent's expected
/// name, and that file runs (so the returned string is its version). What it
/// cannot: that the file *is* that agent's CLI. `--version` output has no
/// shape to match across tools — `codex-cli 0.42.0`, `gemini, version 1.2.3`,
/// and bare numbers all occur — so any name check would reject real tools more
/// often than it caught impostors. The UI says as much rather than claiming a
/// verification this cannot make.
pub fn verify_manual_dir(agent: &str, dir: &Path) -> Result<ManualHit, String> {
    let Some((_, cli)) = CLI_AGENTS.iter().find(|(id, _)| *id == agent) else {
        return Err(format!("{agent} has no command-line tool to point at"));
    };
    let found = executable_candidates(cli, dir)
        .into_iter()
        .find(|candidate| candidate.is_file());
    let Some(bin) = found else {
        return Err(format!("no `{cli}` in {}", dir.display()));
    };
    match probe_version(&bin) {
        Some(version) => Ok(ManualHit {
            path: bin.display().to_string(),
            version,
        }),
        None => Err(format!("found {}, but it would not run", bin.display())),
    }
}

/// What [`verify_manual_dir`] found: the file, and the version it printed.
///
/// The file is half the answer for a user who is looking at a directory and
/// wondering which of its files Kiwano picked — the version alone would leave
/// them comparing paths by eye.
#[derive(Debug, Serialize)]
pub struct ManualHit {
    pub path: String,
    pub version: String,
}

// ── the GUI-only agents ─────────────────────────────────────────────────────

/// Claude Desktop marker: the app-support directory (either flavor) exists.
#[cfg(target_os = "macos")]
fn claude_desktop_installed(home: &Path) -> bool {
    let base = home.join("Library").join("Application Support");
    base.join("Claude").is_dir() || base.join("Claude-3p").is_dir()
}

/// Windows and Linux have no marker this module can check yet — Claude Desktop
/// does ship for Windows, so this is a known gap rather than a "not supported".
#[cfg(not(target_os = "macos"))]
fn claude_desktop_installed(_home: &Path) -> bool {
    false
}

/// WorkBuddy is a desktop app with no CLI. Its config root is the stronger
/// signal of the two — it only exists once the app has run — with the app
/// bundle as the fallback for a fresh install that has not started yet.
///
/// The config root honors `WORKBUDDY_CONFIG_DIR`, the same override the
/// takeover writes through (`takeover::config_dir`); reading `$HOME` here would
/// make the two disagree about where the app's files are.
fn workbuddy_installed(home: &Path) -> bool {
    // A relative override is not resolved: it would name a directory under
    // *our* working directory, and reporting an installation because of a file
    // there would be a claim about a place the app never reads.
    let config_dir = kiwano_adapters::config::env_dir("WORKBUDDY_CONFIG_DIR")
        .absolute()
        .unwrap_or_else(|| home.join(".workbuddy"));
    if config_dir.is_dir() {
        return true;
    }
    #[cfg(target_os = "macos")]
    {
        Path::new("/Applications/WorkBuddy.app").exists()
    }
    #[cfg(not(target_os = "macos"))]
    {
        false
    }
}

#[cfg(test)]
mod tests;
