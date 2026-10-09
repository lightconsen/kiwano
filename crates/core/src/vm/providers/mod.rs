//! The provider read model: `ProviderVm` and everything that fills it in —
//! health, usage, limits, declared prices, and the badges a route contributes.
//!
//! Owns the billing vocabulary too (`billing_to_db`/`billing_to_ui`): the write
//! paths in `provider_edit` import from here, never the other way round.
//!
//! This is a directory rather than one file: the read model is a wire contract
//! (`types`), the route state and row assembly behind `build_provider_vms`
//! (`rows`), and the cells and vocabulary a row is made of (`billing`,
//! `display`, `health`, `money`, `usage`). Every `pub` item is re-exported
//! below, so `crate::vm::providers::X` is still the path consumers name, and
//! the helpers `provider_edit` reads back are re-exported here for the same
//! reason: it keeps one import list instead of reaching into five files.

pub mod billing;
pub mod display;
pub mod health;
pub mod money;
pub mod rows;
pub mod types;
pub mod usage;

// ── the public surface, re-exported from the file that now owns it ──

pub use billing::billing_to_ui;
pub use rows::build_provider_vms;
pub use types::{HealthVm, ProviderAdvancedVm, ProviderEndpointVm, ProviderVm, QuotaVm, UsageVm};

// The write paths' view rules moved to the daemon (`kiwanod::api::views`) —
// only the health of a live row is still built here, by the list itself.
