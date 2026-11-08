//! The store file as a thing on this disk.
//!
//! Settings shows what the database costs in disk, and the answer is a fact
//! about this machine: the file, the write-ahead log beside it and the shared
//! memory index. It is read here rather than over the admin plane because a
//! size the daemon reports would be the size of a file the caller cannot see —
//! and this is the same helper `kiwano db stats` uses, so the two fronts
//! cannot disagree about it.

use crate::paths::db_path;

#[derive(serde::Serialize)]
pub struct DbUsageVm {
    /// `kiwano.db` plus its `-wal` and `-shm`, i.e. what the store occupies on
    /// disk rather than the main file alone.
    pub file_bytes: u64,
}

/// `get_db_usage` — the store's footprint, for the Settings row.
///
/// A failed metadata read counts as zero bytes, the same way
/// `kiwano_core::db_file::footprint` treats a file that is not there yet: before
/// the gateway has written anything there is nothing to report, and it is not
/// an error to show.
#[tauri::command]
pub fn get_db_usage() -> DbUsageVm {
    DbUsageVm {
        file_bytes: kiwano_core::db_file::footprint(&db_path()),
    }
}
