//! Where Kiwano's own files live, and where the user's do.
//!
//! The front ends (the app and the CLI) both need the user's home, because it
//! roots every agent config a takeover reads or writes. They used to read `HOME`
//! themselves, with `.` as the fallback — and `kiwano_adapters::config` warns
//! against exactly that: on Windows `HOME` can be injected by Git/MSYS, and a
//! home that is not the profile directory silently relocates every agent's
//! config. [`home_dir`] is the one that asks the OS instead.

use std::path::PathBuf;

/// The user's home directory.
pub fn home_dir() -> PathBuf {
    kiwano_adapters::config::get_home_dir()
}
