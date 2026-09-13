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

pub mod auxiliary;
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

/// Run a future to completion, for callers that are not async themselves.
///
/// [`sidecar`]'s endpoint probe and model listing are `async` because the
/// gateway is, but the CLI is a straight-line program: it wants the answer, not
/// a reactor. One current-thread runtime for the process, built on first use —
/// these calls are a handful per invocation, and thread-per-call would be waste.
pub fn block_on<F: std::future::Future>(future: F) -> F::Output {
    static RUNTIME: std::sync::OnceLock<tokio::runtime::Runtime> = std::sync::OnceLock::new();
    RUNTIME
        .get_or_init(|| {
            tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build a current-thread runtime")
        })
        .block_on(future)
}
