//! Endpoint display rules — one implementation, in the daemon's crate.
//!
//! These were defined **twice** for a while: the originals here, and a copy in
//! `kiwanod::api::views` made when `add_provider` moved. Both compiled and both
//! had passing tests, which is exactly how a duplicated rule survives — nothing
//! fails, the two just drift the first time one of them is edited.
//!
//! `migrate.local.md` §10.7's rule settles it: the rule that produces a shape
//! follows the shape. The daemon produces these fields now (`add_provider`'s
//! return, and the provider list once it moves), so the implementation is there
//! and this is a re-export.

pub(crate) use kiwanod::api::views::{display_endpoint, endpoint_note, vm_endpoints};
