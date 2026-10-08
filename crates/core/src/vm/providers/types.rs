//! Re-exported: the wire types moved to `kiwano-api` when the API needed a home
//! both the daemon and its clients can see (`migrate.local.md` §10.5). The path
//! `crate::vm::providers::types::ProviderVm` is unchanged on purpose — the
//! move is not a refactor anyone else should have to notice.

pub use kiwano_api::providers::*;
