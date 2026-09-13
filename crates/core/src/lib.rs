//! Kiwano's application layer — everything that is not the GUI and not the
//! gateway daemon, shared by both front ends.
//!
//! The desktop app (`kiwano-app`) and the CLI (`kiwano`) drive the same
//! operations from here, over the same SQLite file the gateway reads. Keeping
//! them in one crate is what stops the two from drifting: a provider written by
//! the CLI is the same row, with the same semantics, as one written by the app.
//!
//! - [`vm`]: view models and the operations that produce them. The serde field
//!   names mirror `src/api/types.ts` exactly — that file is the contract.
//! - [`aux`]: the second SQLite connection, holding the app-scoped tables
//!   (`app_settings`, `takeover_backups`, `hub_cache`, `hub_models_cache`).
//! - [`takeover`]: rewriting an agent's config to point at the local gateway,
//!   with backup and rollback.
//! - [`creds`]: reading the provider an agent is currently configured with.
//! - [`sync`]: the Hub catalog and pricing sync.
//! - [`pricing`]: the GUI-readable price table and currency conversion.
//! - [`csv`]: request-log export.
//!
//! Nothing here depends on Tauri, and nothing here may: the CLI links this crate
//! without a display server anywhere in sight.

pub mod aux;
pub mod creds;
pub mod csv;
pub mod detect;
pub mod import;
pub mod pricing;
pub mod share;
pub mod sidecar;
pub mod sync;
pub mod takeover;
pub mod vm;
