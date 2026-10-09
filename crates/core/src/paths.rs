//! Where Kiwano's own files live, and where the user's do.
//!
//! The front ends (the app and the CLI) both need the user's home, because it
//! roots every agent config a takeover reads or writes. They used to read `HOME`
//! themselves, with `.` as the fallback — and `kiwano_adapters::config` warns
//! against exactly that: on Windows `HOME` can be injected by Git/MSYS, and a
//! home that is not the profile directory silently relocates every agent's
//! config. [`home_dir`] is the one that asks the OS instead.

use std::path::PathBuf;

/// The database path and whatever had to be ignored to resolve it, so a front
/// end can name the type without depending on the adapter crate directly.
pub use kiwano_adapters::config::DbPath;

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    kiwano_adapters::config::get_home_dir()
}

/// The **client's own** database and whatever had to be ignored to resolve it
/// (`migrate.local.md` §9.5 step 2).
///
/// Beside [`crate::sidecar::db_path`] because the two are a pair a front end
/// resolves together: the shared file is the daemon's and this one is the
/// client's, and which is which is the whole of the boundary.
pub fn local_db_path(shared: &std::path::Path) -> DbPath {
    kiwano_adapters::config::kiwano_local_db_path(shared)
}
