//! Where the app's files and ports are: the database the app, the CLI and the
//! gateway all open, the home directory the agent configs live under, and the
//! data port. Resolved in one place so the readers cannot disagree.

pub(crate) fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

/// Root the agent config files live under: the tree a takeover rewrites, and
/// the one the provider list reads the live-route evidence back from. One
/// helper so the write and the read cannot end up looking at different trees —
/// and the same one the CLI uses, so the two front ends agree.
///
/// It asks the OS rather than reading `HOME`, which on Windows can be injected
/// by Git/Cygwin/MSYS and point somewhere that is not the user profile. See
/// `kiwano_adapters::config::get_home_dir`.
pub(crate) fn home_dir() -> std::path::PathBuf {
    kiwano_core::paths::home_dir()
}

/// The database the app, the CLI and the gateway all open. Resolved by the same
/// function in all three, which is what keeps them from finding different files.
pub(crate) fn db_path() -> std::path::PathBuf {
    resolved_db().path
}

/// [`db_path`] with what the resolver had to ignore — read at startup, once,
/// where there is a log to write it to.
pub(crate) fn resolved_db() -> kiwano_core::paths::DbPath {
    kiwano_core::sidecar::db_path(None)
}
