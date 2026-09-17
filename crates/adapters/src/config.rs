// This file is derived from cc-switch (https://github.com/farion1231/cc-switch),
// licensed under the MIT License.
// Source: src-tauri/src/config.rs
// Copied on 2026-09-07. Modified for Kiwano (global settings-override hooks
// from `crate::settings` / `crate::app_store` replaced by explicit
// `Option<&Path>` parameter injection on the affected getters).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::fs;
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use crate::error::AppError;

/// Kiwano's own data directory: `~/.kiwano`, beside the gateway's logs and the
/// database the app, the CLI and the gateway all open.
pub fn kiwano_data_dir() -> PathBuf {
    get_home_dir().join(".kiwano")
}

/// The SQLite file those three share: `KIWANO_DB_PATH`, else
/// `~/.kiwano/kiwano.db`.
///
/// One function because the three processes have to agree; three copies of this
/// was the shape it was in, and they had already begun to differ (`$HOME` here,
/// `.` there) — the gateway resolves it from the environment the app hands it,
/// so a disagreement is a database that exists twice, or one that appears empty.
///
/// `KIWANO_DB_PATH` is honored when it is absolute, and ignored when it is not:
/// a relative one names a *different* file to each of the three processes, since
/// each has its own working directory — the one place the value cannot mean what
/// it says. What it ignored comes back in the return value for the caller to say
/// out loud at a point where somebody can hear it ([`DbPath::ignored_note`]).
pub fn kiwano_db_path() -> DbPath {
    kiwano_db_path_from(std::env::var_os("KIWANO_DB_PATH").as_deref())
}

/// The database path, and what had to be ignored to get it.
///
/// The ignored value is *returned* rather than logged, because this is resolved
/// before the logging it would be logged to exists: the log directory is beside
/// the database (see `kiwanod::logging::log_dir`), so anything said here would be
/// said to nobody. Callers report it once their logging is up — the gateway and
/// the app through `tracing`, the CLI to stderr — which is the one ordering that
/// makes it visible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DbPath {
    pub path: PathBuf,
    /// Set when `KIWANO_DB_PATH` named something unusable — the value itself,
    /// trimmed, for the message.
    pub ignored: Option<String>,
}

impl DbPath {
    /// What to tell the user, in one wording for the three front ends.
    pub fn ignored_note(&self) -> Option<String> {
        let ignored = self.ignored.as_ref()?;
        Some(format!(
            "KIWANO_DB_PATH={ignored} is not an absolute path, so it would name a different \
             file to each process; using {} instead",
            self.path.display()
        ))
    }
}

fn kiwano_db_path_from(value: Option<&std::ffi::OsStr>) -> DbPath {
    let default = || kiwano_data_dir().join("kiwano.db");
    match classify_dir(value) {
        EnvDir::Absolute(path) => DbPath {
            path,
            ignored: None,
        },
        EnvDir::Unset => DbPath {
            path: default(),
            ignored: None,
        },
        EnvDir::Relative(relative) => DbPath {
            path: default(),
            ignored: Some(relative),
        },
    }
}

/// A directory named by an environment variable, classified by what it can
/// mean. One rule, in one place, because the readers of these values are spread
/// across crates and a second copy of the rule is a second answer.
///
/// The distinction that matters is between the first two variants and the third:
/// unset and blank are the same answer — nobody said otherwise — while a relative
/// value is somebody saying something we cannot act on. It is resolved by
/// whoever reads it against *their* working directory, and for a tool that is
/// wherever the user happened to run it. Resolving one against our own working
/// directory instead would be a confident guess at a file that, at best,
/// coincidentally shares a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EnvDir {
    /// Not set, or set to blank: the caller's default is the answer.
    Unset,
    /// An absolute path. This is the directory.
    Absolute(PathBuf),
    /// Set, but not to a path that names one place.
    Relative(String),
}

impl EnvDir {
    /// The directory, when there is one to use. `Unset` is not one — the caller
    /// has a default to apply first — so this is for the callers whose default
    /// is "nothing".
    pub fn absolute(self) -> Option<PathBuf> {
        match self {
            EnvDir::Absolute(path) => Some(path),
            _ => None,
        }
    }
}

/// [`classify_dir`] for a variable of this process.
pub fn env_dir(name: &str) -> EnvDir {
    classify_dir(std::env::var_os(name).as_deref())
}

/// Classify a variable's value as a directory.
pub fn classify_dir(value: Option<&std::ffi::OsStr>) -> EnvDir {
    let Some(value) = value else {
        return EnvDir::Unset;
    };
    let trimmed = value.to_string_lossy();
    let trimmed = trimmed.trim();
    if trimmed.is_empty() {
        return EnvDir::Unset;
    }
    let path = PathBuf::from(trimmed);
    if path.is_absolute() {
        EnvDir::Absolute(path)
    } else {
        EnvDir::Relative(trimmed.to_string())
    }
}

/// Get the user's home directory, with a fallback and logging.
///
/// ## Windows notes
///
/// - On Windows, `dirs::home_dir()` uses `SHGetKnownFolderPath(FOLDERID_Profile)`
///   and returns the real user profile directory (e.g. `C:\\Users\\Alice`),
///   matching the v3.10.2 behavior.
/// - Do not read the `HOME` environment variable directly: it can be injected by
///   third-party tools such as Git/Cygwin/MSYS, may not equal the user profile
///   directory, and could shift the `.cc-switch/cc-switch.db` path so data
///   "appears to be lost".
///
/// ## Test isolation
///
/// So that Windows CI/local tests can reliably isolate real user data, the home
/// dir can be explicitly overridden via `CC_SWITCH_TEST_HOME` (test/debug use only).
pub fn get_home_dir() -> PathBuf {
    if let Ok(home) = std::env::var("CC_SWITCH_TEST_HOME") {
        let trimmed = home.trim();
        if !trimmed.is_empty() {
            return PathBuf::from(trimmed);
        }
    }

    dirs::home_dir().unwrap_or_else(|| {
        // Fires on every call rather than once — nothing here is cached, which
        // is what gets the warning into the log: the first thing to resolve a
        // path after logging exists says it, even though the condition was
        // discovered while resolving the database path, before that.
        log::warn!(
            "cannot determine the user's home directory; using the current directory, so \
             everything Kiwano resolves relative to home lands under whichever directory \
             this process was started in"
        );
        PathBuf::from(".")
    })
}

/// Get the Claude Code config directory path
///
/// Kiwano: the cc-switch global settings override hook is replaced by explicit
/// parameter injection — callers pass their own override directory (if any).
pub fn get_claude_config_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(custom) = override_dir {
        return custom.to_path_buf();
    }

    get_home_dir().join(".claude")
}

/// Default Claude MCP config file path (~/.claude.json)
pub fn get_default_claude_mcp_path() -> PathBuf {
    get_home_dir().join(".claude.json")
}

fn normalize_path_lexically(path: &Path) -> PathBuf {
    let mut normalized = PathBuf::new();

    for component in path.components() {
        match component {
            Component::CurDir => {}
            Component::ParentDir => {
                if !normalized.pop() {
                    normalized.push(component.as_os_str());
                }
            }
            Component::Normal(part) => normalized.push(part),
            Component::RootDir | Component::Prefix(_) => normalized.push(component.as_os_str()),
        }
    }

    normalized
}

fn comparable_path_key(path: &Path) -> String {
    let mut key = normalize_path_lexically(path).to_string_lossy().to_string();

    #[cfg(windows)]
    {
        key = key.replace('\\', "/");
    }

    while key.len() > 1 && key.ends_with('/') {
        key.pop();
    }

    #[cfg(windows)]
    {
        key.make_ascii_lowercase();
    }

    key
}

fn path_eq_lexical(left: &Path, right: &Path) -> bool {
    comparable_path_key(left) == comparable_path_key(right)
}

/// Returns true when `path` is lexically contained within `base`.
///
/// Both paths are normalized lexically (without hitting the filesystem), so
/// this works for non-existent paths. It is **not** a symlink defense: a
/// symlink inside `base` can still lead a resolved path outside it. Callers
/// that go on to open the file must canonicalize the existing path and
/// re-verify containment (see `resolve_cc_switch_catalog_path`).
/// On Windows the comparison is case-insensitive.
#[allow(dead_code)] // copied alongside its siblings; future tiers use it
pub(crate) fn path_is_within(base: &Path, path: &Path) -> bool {
    let base_key = comparable_path_key(base);
    let path_key = comparable_path_key(path);

    if path_key == base_key {
        return true;
    }

    let prefix = format!("{base_key}/");
    path_key.starts_with(&prefix)
}

#[cfg(windows)]
fn derive_wsl_default_mcp_path(dir: &Path) -> Option<PathBuf> {
    use std::path::Prefix;

    let normalized = normalize_path_lexically(dir);
    let mut components = normalized.components();
    let prefix = match components.next()? {
        Component::Prefix(prefix) => prefix,
        _ => return None,
    };

    let server = match prefix.kind() {
        Prefix::UNC(server, _) | Prefix::VerbatimUNC(server, _) => server.to_string_lossy(),
        _ => return None,
    };

    if !server.eq_ignore_ascii_case("wsl$") && !server.eq_ignore_ascii_case("wsl.localhost") {
        return None;
    }

    let mut parts = Vec::new();
    for component in components {
        match component {
            Component::RootDir | Component::CurDir => {}
            Component::Normal(part) => parts.push(part.to_string_lossy().to_string()),
            Component::ParentDir | Component::Prefix(_) => return None,
        }
    }

    let is_wsl_home_default =
        parts.len() == 3 && parts[0] == "home" && !parts[1].is_empty() && parts[2] == ".claude";
    let is_wsl_root_default = parts.len() == 2 && parts[0] == "root" && parts[1] == ".claude";

    if is_wsl_home_default || is_wsl_root_default {
        return normalized
            .parent()
            .map(|parent| parent.join(".claude.json"));
    }

    None
}

fn default_mcp_path_for_config_dir(dir: &Path) -> Option<PathBuf> {
    let default_config_dir = get_home_dir().join(".claude");
    if path_eq_lexical(dir, &default_config_dir) {
        return Some(get_default_claude_mcp_path());
    }

    #[cfg(windows)]
    {
        if let Some(path) = derive_wsl_default_mcp_path(dir) {
            return Some(path);
        }
    }

    None
}

fn derive_mcp_path_from_override(dir: &Path) -> PathBuf {
    dir.join(".claude.json")
}

/// Get the Claude MCP config file path
///
/// Kiwano: override directory injected by the caller instead of global state.
pub fn get_claude_mcp_path(override_dir: Option<&Path>) -> PathBuf {
    if let Some(custom_dir) = override_dir {
        if let Some(path) = default_mcp_path_for_config_dir(custom_dir) {
            return path;
        }
        return derive_mcp_path_from_override(custom_dir);
    }
    get_default_claude_mcp_path()
}

/// Get the Claude Code main settings file path
pub fn get_claude_settings_path(override_dir: Option<&Path>) -> PathBuf {
    let dir = get_claude_config_dir(override_dir);
    let settings = dir.join("settings.json");
    if settings.exists() {
        return settings;
    }
    // Legacy naming compatibility: keep using the old file if it exists
    let legacy = dir.join("claude.json");
    if legacy.exists() {
        return legacy;
    }
    // Default for new setups: fall back to the standard settings.json name (claude.json is no longer generated)
    settings
}

/// Get the app config directory path (~/.cc-switch)
///
/// Kiwano: override directory injected by the caller instead of the
/// `crate::app_store` global hook.
pub fn get_app_config_dir(override_dir: Option<&Path>) -> PathBuf {
    if let Some(custom) = override_dir {
        return custom.to_path_buf();
    }

    let default_dir = get_home_dir().join(".cc-switch");

    // v3.10.3 compatibility: when the user's environment has a `HOME` that
    // differs from the real user profile directory, v3.10.3 may have created
    // or used a database under `HOME/.cc-switch/`. Fall back to the legacy
    // location only when the default location has no database, avoiding a
    // recurrence of the "providers disappeared" issue, while also keeping
    // fresh installs from writing to an unexpected path just because `HOME`
    // is set.
    #[cfg(windows)]
    {
        let default_db = default_dir.join("cc-switch.db");
        if !default_db.exists() {
            if let Ok(home_env) = std::env::var("HOME") {
                let trimmed = home_env.trim();
                if !trimmed.is_empty() {
                    let legacy_dir = PathBuf::from(trimmed).join(".cc-switch");
                    if legacy_dir.join("cc-switch.db").exists() {
                        log::info!(
                            "Detected v3.10.3 legacy database at {}, using it instead of {}",
                            legacy_dir.display(),
                            default_dir.display()
                        );
                        return legacy_dir;
                    }
                }
            }
        }
    }

    default_dir
}

/// Get the app config file path
pub fn get_app_config_path(override_dir: Option<&Path>) -> PathBuf {
    get_app_config_dir(override_dir).join("config.json")
}

/// Sanitize a provider name to make it safe for file names
#[allow(dead_code)]
pub fn sanitize_provider_name(name: &str) -> String {
    name.chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '-',
            _ => c,
        })
        .collect::<String>()
        .to_lowercase()
}

/// Get the provider config file path
#[allow(dead_code)]
pub fn get_provider_config_path(provider_id: &str, provider_name: Option<&str>) -> PathBuf {
    let base_name = provider_name
        .map(sanitize_provider_name)
        .unwrap_or_else(|| sanitize_provider_name(provider_id));

    get_claude_config_dir(None).join(format!("settings-{base_name}.json"))
}

/// Read a JSON config file
pub fn read_json_file<T: for<'a> Deserialize<'a>>(path: &Path) -> Result<T, AppError> {
    if !path.exists() {
        return Err(AppError::Config(format!(
            "the file does not exist: {}",
            path.display()
        )));
    }

    let content = fs::read_to_string(path).map_err(|e| AppError::io(path, e))?;

    serde_json::from_str(&content).map_err(|e| AppError::json(path, e))
}

/// Recursively sort JSON object keys (alphabetically) so serialized output is deterministic
fn sort_json_keys(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut sorted_map = Map::new();
            let mut keys: Vec<_> = map.keys().collect();
            keys.sort();
            for key in keys {
                sorted_map.insert(key.clone(), sort_json_keys(&map[key]));
            }
            Value::Object(sorted_map)
        }
        Value::Array(arr) => Value::Array(arr.iter().map(sort_json_keys).collect()),
        other => other.clone(),
    }
}

/// Write a JSON config file and return the bytes actually written.
pub fn write_json_file_with_contents<T: Serialize>(
    path: &Path,
    data: &T,
) -> Result<Vec<u8>, AppError> {
    // Ensure the directory exists
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let value = serde_json::to_value(data).map_err(|e| AppError::JsonSerialize { source: e })?;
    let sorted_value = sort_json_keys(&value);
    let json = serde_json::to_string_pretty(&sorted_value)
        .map_err(|e| AppError::JsonSerialize { source: e })?;

    let contents = json.into_bytes();
    atomic_write(path, &contents)?;
    Ok(contents)
}

/// Write a JSON config file (keys sorted alphabetically for deterministic output)
pub fn write_json_file<T: Serialize>(path: &Path, data: &T) -> Result<(), AppError> {
    write_json_file_with_contents(path, data).map(|_| ())
}

/// Atomically write a text file (used for TOML/plain text)
pub fn write_text_file(path: &Path, data: &str) -> Result<(), AppError> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }
    atomic_write(path, data.as_bytes())
}

/// Atomic write: write to a temp file, then rename it into place to avoid a half-written state
pub fn atomic_write(path: &Path, data: &[u8]) -> Result<(), AppError> {
    atomic_write_with_unix_mode(path, data, None)
}

/// Atomically write a file containing credentials. On Unix, both new and replaced files always use 0600.
pub fn atomic_write_private(path: &Path, data: &[u8]) -> Result<(), AppError> {
    atomic_write_with_unix_mode(path, data, Some(0o600))
}

fn atomic_write_with_unix_mode(
    path: &Path,
    data: &[u8],
    unix_mode: Option<u32>,
) -> Result<(), AppError> {
    #[cfg(not(unix))]
    let _ = unix_mode;

    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| AppError::io(parent, e))?;
    }

    let parent = path
        .parent()
        .ok_or_else(|| AppError::Config("the path has no directory".to_string()))?;
    let file_name = path
        .file_name()
        .ok_or_else(|| AppError::Config("the path has no file name".to_string()))?
        .to_string_lossy()
        .to_string();
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    static TEMP_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let (tmp, mut file) = (|| -> Result<(PathBuf, fs::File), AppError> {
        let mut last_collision = None;
        for _ in 0..16 {
            let counter = TEMP_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let candidate = parent.join(format!(
                "{file_name}.tmp.{}.{ts}.{counter}",
                std::process::id()
            ));
            let mut options = fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            if let Some(mode) = unix_mode {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(mode);
            }
            match options.open(&candidate) {
                Ok(file) => return Ok((candidate, file)),
                Err(source) if source.kind() == std::io::ErrorKind::AlreadyExists => {
                    last_collision = Some((candidate, source));
                }
                Err(source) => return Err(AppError::io(&candidate, source)),
            }
        }

        let (candidate, source) = last_collision.expect("temporary filename loop must run");
        Err(AppError::io(&candidate, source))
    })()?;

    if let Err(source) = file.write_all(data).and_then(|_| file.flush()) {
        drop(file);
        let _ = fs::remove_file(&tmp);
        return Err(AppError::io(&tmp, source));
    }
    drop(file);

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if let Some(mode) = unix_mode {
            if let Err(source) = fs::set_permissions(&tmp, fs::Permissions::from_mode(mode)) {
                let _ = fs::remove_file(&tmp);
                return Err(AppError::io(&tmp, source));
            }
        } else if let Ok(meta) = fs::metadata(path) {
            let perm = meta.permissions().mode();
            let _ = fs::set_permissions(&tmp, fs::Permissions::from_mode(perm));
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows_sys::Win32::{
            Foundation::ERROR_NOT_SUPPORTED, Storage::FileSystem::ReplaceFileW,
        };

        let replaced: Vec<u16> = path
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let replacement: Vec<u16> = tmp
            .as_os_str()
            .encode_wide()
            .chain(std::iter::once(0))
            .collect();
        let mut completed = false;
        let mut last_error = None;

        for _ in 0..3 {
            // SAFETY: both path buffers are NUL-terminated UTF-16 and remain alive for the
            // duration of the call. Backup, exclusion, and reserved pointers are intentionally null.
            let replaced_ok = unsafe {
                ReplaceFileW(
                    replaced.as_ptr(),
                    replacement.as_ptr(),
                    std::ptr::null(),
                    0,
                    std::ptr::null(),
                    std::ptr::null(),
                )
            };
            if replaced_ok != 0 {
                completed = true;
                break;
            }

            let replace_error = std::io::Error::last_os_error();
            // WSL UNC paths reject ReplaceFileW with ERROR_NOT_SUPPORTED (50).
            // std::fs::rename uses a different replace-existing API on Windows.
            let replace_not_supported =
                replace_error.raw_os_error() == Some(ERROR_NOT_SUPPORTED as i32);
            if replace_error.kind() != std::io::ErrorKind::NotFound && !replace_not_supported {
                last_error = Some(replace_error);
                break;
            }

            match fs::rename(&tmp, path) {
                Ok(()) => {
                    completed = true;
                    break;
                }
                Err(source)
                    if matches!(
                        source.kind(),
                        std::io::ErrorKind::AlreadyExists | std::io::ErrorKind::PermissionDenied
                    ) =>
                {
                    last_error = Some(source);
                }
                Err(source) => {
                    last_error = Some(source);
                    break;
                }
            }
        }

        if !completed {
            let source = last_error.unwrap_or_else(std::io::Error::last_os_error);
            let _ = fs::remove_file(&tmp);
            return Err(AppError::IoContext {
                context: format!(
                    "atomic replace failed: {} -> {}",
                    tmp.display(),
                    path.display()
                ),
                source,
            });
        }
    }

    #[cfg(not(windows))]
    {
        if let Err(source) = fs::rename(&tmp, path) {
            let _ = fs::remove_file(&tmp);
            return Err(AppError::IoContext {
                context: format!(
                    "atomic replace failed: {} -> {}",
                    tmp.display(),
                    path.display()
                ),
                source,
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The database path is one answer for three processes, so a value that
    /// would mean something different to each of them is not used.
    #[test]
    fn the_database_path_is_resolved_in_one_place() {
        use std::ffi::OsStr;

        // Built rather than written as a literal: `/srv/kiwano.db` is absolute on
        // unix and *relative* on Windows, where the rule under test would refuse
        // it — the test would then be asserting the opposite of what it reads.
        let absolute = std::env::temp_dir().join("kiwano-db-path-test.db");
        let absolute_arg = absolute.to_string_lossy().into_owned();
        assert_eq!(
            kiwano_db_path_from(Some(OsStr::new(&absolute_arg))),
            DbPath {
                path: absolute.clone(),
                ignored: None
            }
        );
        // Unset, blank, and relative all land on the same default — the last
        // one because a relative path names a different file per process.
        for value in [None, Some(OsStr::new("")), Some(OsStr::new("./kiwano.db"))] {
            let resolved = kiwano_db_path_from(value);
            assert!(
                resolved.path.ends_with(".kiwano/kiwano.db"),
                "{value:?} resolved to {}",
                resolved.path.display()
            );
        }

        // The ignored value comes back for the caller to report — this is
        // resolved before any logger exists, so saying it here would say it to
        // nobody.
        let relative = kiwano_db_path_from(Some(OsStr::new("./kiwano.db")));
        assert_eq!(relative.ignored.as_deref(), Some("./kiwano.db"));
        let note = relative
            .ignored_note()
            .expect("a note for the ignored value");
        assert!(note.contains("./kiwano.db"), "{note}");
        assert!(kiwano_db_path_from(None).ignored_note().is_none());
    }

    /// The three answers, and the one that is not a path: unset and blank are
    /// the caller's default, an absolute value is the directory, and a relative
    /// one is reported as such rather than joined onto our working directory.
    #[test]
    fn a_directory_from_the_environment_is_classified() {
        use std::ffi::OsStr;

        assert_eq!(classify_dir(None), EnvDir::Unset);
        assert_eq!(classify_dir(Some(OsStr::new(""))), EnvDir::Unset);
        assert_eq!(classify_dir(Some(OsStr::new("   "))), EnvDir::Unset);
        // Built, not written as a literal: `/srv/tools` is absolute on unix and
        // *relative* on Windows, where this test would be asserting the opposite
        // of what it reads.
        let absolute = std::env::temp_dir().join("kiwano-classify-test");
        let padded = format!(" {} ", absolute.display());
        assert_eq!(
            classify_dir(Some(OsStr::new(&padded))),
            EnvDir::Absolute(absolute),
            "padding is not part of a path"
        );
        assert_eq!(
            classify_dir(Some(OsStr::new("tools/bin"))),
            EnvDir::Relative("tools/bin".to_string())
        );

        // And the same thing with the next source left out of it.
        assert_eq!(EnvDir::Unset.absolute(), None);
        assert_eq!(EnvDir::Relative("x".into()).absolute(), None);
        assert_eq!(
            EnvDir::Absolute(PathBuf::from("/srv")).absolute(),
            Some(PathBuf::from("/srv"))
        );
    }

    fn assert_atomic_write_replaces_existing_file(dir: &Path) {
        let path = dir.join("atomic-write-contract.json");
        std::fs::write(&path, b"old contents").unwrap();

        atomic_write(&path, b"new contents").unwrap();

        assert_eq!(std::fs::read(&path).unwrap(), b"new contents");
        let tmp_prefix = "atomic-write-contract.json.tmp.";
        let leftovers: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|entry| entry.unwrap())
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(tmp_prefix))
            .map(|entry| entry.path())
            .collect();
        assert!(
            leftovers.is_empty(),
            "temporary files remain: {leftovers:?}"
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn atomic_write_replaces_existing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert_atomic_write_replaces_existing_file(dir.path());
    }

    #[cfg(windows)]
    #[test]
    fn atomic_write_preserves_destination_when_windows_replace_fails() {
        use std::os::windows::fs::OpenOptionsExt;
        use windows_sys::Win32::Storage::FileSystem::FILE_SHARE_READ;

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.json");
        std::fs::write(&path, b"old contents").unwrap();
        let held_file = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(FILE_SHARE_READ)
            .open(&path)
            .unwrap();

        let result = atomic_write(&path, b"new contents");

        assert!(result.is_err());
        drop(held_file);
        assert_eq!(std::fs::read(&path).unwrap(), b"old contents");
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 1);
    }

    #[cfg(windows)]
    #[test]
    #[ignore = "requires CC_SWITCH_WSL_TEST_DIR to point to a WSL2 UNC directory"]
    fn atomic_write_replaces_existing_wsl_unc_file() {
        let root = PathBuf::from(
            std::env::var_os("CC_SWITCH_WSL_TEST_DIR").expect("CC_SWITCH_WSL_TEST_DIR must be set"),
        );
        let home = get_home_dir();
        let temp = std::env::temp_dir();
        for (name, path) in [
            ("test root", root.as_path()),
            ("test home", home.as_path()),
            ("temporary directory", temp.as_path()),
        ] {
            let unc = path.to_string_lossy();
            assert!(
                unc.starts_with(r"\\wsl.localhost\") || unc.starts_with(r"\\wsl$\"),
                "expected {name} to be a WSL UNC path, got {unc}"
            );
            assert!(
                path.starts_with(&root),
                "expected {name} to be under {}, got {unc}",
                root.display()
            );
        }

        let dir = tempfile::Builder::new()
            .prefix("atomic-write-contract-")
            .tempdir_in(&root)
            .unwrap();
        assert_atomic_write_replaces_existing_file(dir.path());
    }

    #[test]
    fn derive_mcp_path_from_override_uses_config_dir_for_custom_path() {
        let override_dir = PathBuf::from("/tmp/profile/.claude");
        let derived = derive_mcp_path_from_override(&override_dir);
        assert_eq!(derived, PathBuf::from("/tmp/profile/.claude/.claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_override_uses_config_dir_for_non_hidden_folder() {
        let override_dir = PathBuf::from("/data/claude-config");
        let derived = derive_mcp_path_from_override(&override_dir);
        assert_eq!(derived, PathBuf::from("/data/claude-config/.claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_override_supports_relative_rootless_dir() {
        let override_dir = PathBuf::from("claude");
        let derived = derive_mcp_path_from_override(&override_dir);
        assert_eq!(derived, PathBuf::from("claude/.claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_root_like_dir_uses_root_file() {
        let override_dir = PathBuf::from("/");
        let derived = derive_mcp_path_from_override(&override_dir);
        assert_eq!(derived, PathBuf::from("/.claude.json"));
    }

    #[test]
    fn derive_mcp_path_from_override_preserves_leading_parent_dirs() {
        let override_dir = PathBuf::from("../../profiles/work/.claude");
        let derived = derive_mcp_path_from_override(&override_dir);
        assert_eq!(derived, override_dir.join(".claude.json"));
    }

    #[cfg(windows)]
    #[test]
    fn wsl_unc_home_default_uses_split_mcp_path() {
        let override_dir = PathBuf::from(r"\\wsl$\Ubuntu\home\travis\.claude");
        let derived = default_mcp_path_for_config_dir(&override_dir)
            .expect("WSL home default should use split MCP path");
        assert_eq!(
            derived,
            PathBuf::from(r"\\wsl$\Ubuntu\home\travis\.claude.json")
        );
    }

    #[cfg(windows)]
    #[test]
    fn wsl_unc_root_default_uses_split_mcp_path() {
        let override_dir = PathBuf::from(r"\\wsl.localhost\Ubuntu\root\.claude");
        let derived = default_mcp_path_for_config_dir(&override_dir)
            .expect("WSL root default should use split MCP path");
        assert_eq!(
            derived,
            PathBuf::from(r"\\wsl.localhost\Ubuntu\root\.claude.json")
        );
    }

    #[cfg(windows)]
    #[test]
    fn wsl_unc_custom_dir_uses_nested_mcp_path() {
        let override_dir = PathBuf::from(r"\\wsl$\Ubuntu\opt\claude\.claude");
        assert!(default_mcp_path_for_config_dir(&override_dir).is_none());
        assert_eq!(
            derive_mcp_path_from_override(&override_dir),
            PathBuf::from(r"\\wsl$\Ubuntu\opt\claude\.claude\.claude.json")
        );
    }

    #[test]
    fn sort_json_keys_sorts_top_level_object() {
        let input = serde_json::json!({
            "z": 1,
            "a": 2,
            "m": 3,
        });
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, r#"{"a":2,"m":3,"z":1}"#);
    }

    #[test]
    fn sort_json_keys_recurses_into_nested_objects() {
        let input = serde_json::json!({
            "outer_b": {"z": 1, "a": 2},
            "outer_a": {"y": 3, "b": 4},
        });
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(
            serialized,
            r#"{"outer_a":{"b":4,"y":3},"outer_b":{"a":2,"z":1}}"#
        );
    }

    #[test]
    fn sort_json_keys_preserves_array_order() {
        let input = serde_json::json!([3, 1, 2]);
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, "[3,1,2]");
    }

    #[test]
    fn sort_json_keys_sorts_objects_inside_arrays_but_keeps_array_order() {
        let input = serde_json::json!([
            {"z": 1, "a": 2},
            {"y": 3, "b": 4},
        ]);
        let sorted = sort_json_keys(&input);
        let serialized = serde_json::to_string(&sorted).unwrap();
        assert_eq!(serialized, r#"[{"a":2,"z":1},{"b":4,"y":3}]"#);
    }

    #[test]
    fn sort_json_keys_passes_through_primitives() {
        let cases = vec![
            serde_json::json!("hello"),
            serde_json::json!(42),
            serde_json::json!(3.5),
            serde_json::json!(true),
            serde_json::json!(null),
        ];
        for value in cases {
            let sorted = sort_json_keys(&value);
            assert_eq!(sorted, value);
        }
    }

    #[test]
    fn sort_json_keys_handles_empty_collections() {
        let empty_obj = serde_json::json!({});
        assert_eq!(
            serde_json::to_string(&sort_json_keys(&empty_obj)).unwrap(),
            "{}"
        );

        let empty_arr = serde_json::json!([]);
        assert_eq!(
            serde_json::to_string(&sort_json_keys(&empty_arr)).unwrap(),
            "[]"
        );
    }

    #[test]
    fn sort_json_keys_produces_identical_output_for_different_insertion_orders() {
        // Core guarantee: the same logical config must serialize to identical
        // bytes regardless of key insertion order.
        let mut a = Map::new();
        a.insert("env".to_string(), serde_json::json!({"PATH": "/usr/bin"}));
        a.insert("model".to_string(), serde_json::json!("claude-sonnet-4-5"));
        a.insert("permissions".to_string(), serde_json::json!({"allow": []}));

        let mut b = Map::new();
        b.insert("permissions".to_string(), serde_json::json!({"allow": []}));
        b.insert("model".to_string(), serde_json::json!("claude-sonnet-4-5"));
        b.insert("env".to_string(), serde_json::json!({"PATH": "/usr/bin"}));

        let sorted_a = sort_json_keys(&Value::Object(a));
        let sorted_b = sort_json_keys(&Value::Object(b));

        assert_eq!(
            serde_json::to_string(&sorted_a).unwrap(),
            serde_json::to_string(&sorted_b).unwrap(),
        );
    }
}

/// Copy a file
pub fn copy_file(from: &Path, to: &Path) -> Result<(), AppError> {
    fs::copy(from, to).map_err(|e| AppError::IoContext {
        context: format!("cannot copy {} to {}", from.display(), to.display()),
        source: e,
    })?;
    Ok(())
}

/// Delete a file
pub fn delete_file(path: &Path) -> Result<(), AppError> {
    if path.exists() {
        fs::remove_file(path).map_err(|e| AppError::io(path, e))?;
    }
    Ok(())
}

/// Claude Code config status
#[derive(Serialize, Deserialize)]
pub struct ConfigStatus {
    pub exists: bool,
    pub path: String,
}

/// Get the Claude Code config status
pub fn get_claude_config_status(override_dir: Option<&Path>) -> ConfigStatus {
    let path = get_claude_settings_path(override_dir);
    ConfigStatus {
        exists: path.exists(),
        path: path.to_string_lossy().to_string(),
    }
}
