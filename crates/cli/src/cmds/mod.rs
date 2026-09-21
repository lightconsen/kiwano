//! The commands.
//!
//! Each is thin on purpose: resolve what it needs, call into `kiwano_core`, and
//! render. The behaviour lives in the core crate, so the app and the CLI cannot
//! drift into writing different rows for the same operation.
//!
//! The module is split by subcommand, one file per banner this file used to
//! carry. Every `pub` item keeps the path it had when this was one file
//! (`cmds::status`): the facade below re-exports it, because `lib.rs` dispatches
//! on those names and `mcp.rs` reads `build_insights_report` by that name.
//!
//! Two of the modules are layers rather than commands, and they are the ones
//! that cross. `input` turns `--flag`s into `vm::…Input` values and serves both
//! `providers` and `settings` (which owns `import`); `render` turns a view model
//! into the text a shell prints and every command module calls into it. The
//! helpers that cross a module boundary are `pub(crate)` and nothing wider.
//!
//! `runtime` lives here rather than in a submodule because it has no domain — it
//! is the error helper every module maps with, the way `vm::e2s` is.

use crate::CliError;

pub mod agents;
pub mod cache_experiment;
pub mod dashboard;
pub mod gateway;
pub mod input;
pub mod insights;
pub mod keys;
pub mod logs;
pub mod providers;
pub mod render;
pub mod routes;
pub mod rules;
pub mod settings;
pub mod status;
pub mod usage;

// ── the public surface, re-exported so every `cmds::x` path still resolves ──

pub use agents::agents;
pub use cache_experiment::cache_experiment;
pub use dashboard::{alerts, dashboard};
pub use gateway::gateway;
pub(crate) use insights::build_insights_report;
pub use insights::insights;
pub use keys::keys;
pub use logs::logs;
pub use providers::providers;
pub use routes::routes;
pub use rules::rules;
pub use settings::{catalog, config, import, settings};
pub use status::{reload, status};
pub use usage::usage;

/// The runtime error every command maps a failure into.
fn runtime(e: impl std::fmt::Display) -> CliError {
    CliError::runtime(e.to_string())
}
