//! CSV for the Logs card's export — served by the daemon now.
//!
//! The renderer moved to `kiwanod::api::csv` with the rows it renders
//! (`migrate.local.md` §10.17); what stays here is the client's use of it, which
//! is writing the file. Re-exported so `crate::csv::to_csv` still resolves.

pub use kiwanod::api::csv::{to_csv, write_csv};
