//! The resources this daemon serves: one module per resource, each holding the
//! logic that used to sit in `kiwano_core::vm`.
//!
//! # Why the logic is here and not in `kiwano-core`
//!
//! The migration's plan said the daemon's endpoints would call the existing
//! `vm::` functions, "the logic not changing a single line". That is impossible,
//! and the first endpoint written is what proved it: `kiwano-core` depends on
//! `kiwanod` (it needs `Store`), so `kiwanod` cannot depend on `kiwano-core` —
//! the daemon cannot call back into the layer above it. Writing the keys
//! endpoints therefore meant writing a *second copy* of `vm::add_api_key`'s
//! body, down to the strings the UI shows (`crates/core/src/vm/keys.rs` and
//! `crates/gateway/src/server/admin.rs` each had their own "API key must not be
//! empty"). That is the shape `migrate.local.md` §10.5 considered and rejected
//! *for the types* — and `migrate.local.md` §10.8 records the same reasoning
//! being applied to the logic.
//!
//! The rule that decides it is already written down twice, once in each
//! neighbouring decision: §10.6 says a type goes to the side that *serves* it,
//! and §10.7 says the rule that produces a shape follows the shape
//! (`mask_key` moved to `kiwano-api` for exactly this reason). The daemon
//! serves these resources now, and it owns the `Store` they are read from, so
//! the logic that builds them belongs here — and `kiwano-core` re-exports it,
//! so its call sites (the CLI, the app's remaining direct callers, the contract
//! tests) keep the paths and the signatures they had.
//!
//! # What is *not* here
//!
//! Transport. These functions take a `&Store` and return a resource or an
//! [`ApiError`]; the HTTP status, the JSON envelope and the token are
//! `server::admin`'s business. That split is what lets the same function answer
//! a unix-socket request today and a TCP one after the cross-machine work
//! (`migrate.local.md` §6) without a line changing.
//!
//! [`ApiError`]: kiwano_api::error::ApiError

pub mod agents;
pub mod catalog;
pub mod csv;
pub mod import;
pub mod keys;
pub mod limits;
pub mod logs;
pub mod pricing;
pub mod pricing_sync;
pub mod probe;
pub mod providers;
pub mod providers_add;
pub mod routes;
pub mod settings;
pub mod share;
pub mod sync;
pub mod views;
