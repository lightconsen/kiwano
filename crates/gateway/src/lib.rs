//! Kiwano gateway — local proxy data plane for AI Provider management.
//!
//! Layout (tech.md §4.4): `store/` (SQLite), `router/` (route table + single
//! strategy), `server/` (data plane :8317 + admin plane :8310), `meter/`
//! (usage capture), `main.rs` (binary entry).

pub mod error;
pub mod forward;
pub mod meter;
pub mod protocol;
pub mod router;
pub mod server;
pub mod store;
