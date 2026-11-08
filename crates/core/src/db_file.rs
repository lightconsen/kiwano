//! The database file as a file: how big it is, and compacting it.
//!
//! This lives in `core` rather than in the CLI because it needs `rusqlite`, and
//! the CLI deliberately does not carry that dependency — that absence is what
//! `crates/cli/tests/one_writer.rs` is protecting. Keeping the SQL here means the
//! client crate has no way to open the daemon's database even by accident.
//!
//! Compacting is a whole-file rewrite (`VACUUM`), which needs an exclusive lock,
//! so it only runs with the daemon stopped. That is not a limitation to work
//! around: it is the one time the database is not being written, which is
//! exactly when a rewrite is safe.

use std::path::Path;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct DbSize {
    /// Bytes on disk now.
    pub file_bytes: u64,
    /// Bytes a compaction would return to the filesystem.
    pub reclaimable_bytes: u64,
}

/// Everything the database takes on disk: the file itself plus its `-wal` and
/// `-shm` siblings.
///
/// The database runs in WAL mode, so pages written since the last checkpoint
/// live in `-wal` and the main file is smaller than what the database holds.
/// Reporting the main file alone made a fresh store read as "4 KB on disk, 204
/// KB in use", which is not a footprint — it is one third of one.
pub fn footprint(path: &Path) -> u64 {
    // Appended by hand rather than with `Path::with_extension`, which would
    // replace the `.db` suffix instead of extending it.
    let sibling = |suffix: &str| {
        let mut name = path.as_os_str().to_os_string();
        name.push(suffix);
        std::path::PathBuf::from(name)
    };
    [path.to_path_buf(), sibling("-wal"), sibling("-shm")]
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum()
}

/// Why a compaction did not happen. Each one is a different thing to tell the
/// user to do, so they are distinct rather than one error string.
#[derive(Debug)]
pub enum VacuumError {
    /// Another process (normally the gateway) holds the database.
    Busy(String),
    Sql(String),
    Io(String),
}

impl std::fmt::Display for VacuumError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            VacuumError::Busy(detail) => write!(
                f,
                "the gateway still holds the database ({detail}) — stop it first \
                 (`kiwano gateway stop`), then run this again"
            ),
            VacuumError::Sql(detail) => write!(f, "vacuum failed: {detail}"),
            VacuumError::Io(detail) => write!(f, "{detail}"),
        }
    }
}

impl std::error::Error for VacuumError {}

fn open(path: &Path) -> Result<rusqlite::Connection, VacuumError> {
    let conn = rusqlite::Connection::open(path)
        .map_err(|e| VacuumError::Io(format!("cannot open {}: {e}", path.display())))?;
    // No waiting: the point is to learn *now* whether the file is free, not to
    // sit in a five-second busy timeout and then report a lock error that reads
    // like a bug.
    conn.busy_timeout(Duration::from_millis(0))
        .map_err(|e| VacuumError::Sql(e.to_string()))?;
    Ok(conn)
}

/// Take the lock `VACUUM` will need, and let go of it — the question is asked
/// before the long rewrite rather than answered by failing halfway through it.
///
/// The obvious test, `BEGIN EXCLUSIVE`, is the wrong one: the database is in WAL
/// mode, where an exclusive *transaction* only takes the write lock and another
/// process may keep reading. It succeeds against a live gateway, which is how
/// the first version of this shipped a check that never fired. `locking_mode
/// =EXCLUSIVE` is what actually takes the file lock — under it the connection
/// wants exclusive access to the whole file, and a running gateway refuses it.
fn assert_free(conn: &rusqlite::Connection) -> Result<(), VacuumError> {
    conn.execute_batch("PRAGMA locking_mode=EXCLUSIVE;")
        .map_err(|e| VacuumError::Busy(e.to_string()))?;
    // The pragma takes effect on the next transaction, not this one.
    match conn.execute_batch("BEGIN IMMEDIATE; COMMIT;") {
        Ok(()) => {}
        Err(e) => return Err(VacuumError::Busy(e.to_string())),
    }
    // Back to normal: the read-only callers share this connection and must not
    // hold the file exclusively for as long as they live.
    let _ = conn.execute_batch("PRAGMA locking_mode=NORMAL;");
    Ok(())
}

/// What a compaction would reclaim, without doing it.
pub fn reclaimable(path: &Path) -> Result<DbSize, VacuumError> {
    let file_bytes = footprint(path);
    let conn = open(path)?;
    assert_free(&conn)?;
    let (page_size, freelist): (i64, i64) = conn
        .query_row(
            "SELECT (SELECT page_size FROM pragma_page_size),
                    (SELECT freelist_count FROM pragma_freelist_count)",
            [],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )
        .map_err(|e| VacuumError::Sql(e.to_string()))?;
    Ok(DbSize {
        file_bytes,
        reclaimable_bytes: (freelist * page_size).max(0) as u64,
    })
}

/// Rewrite the file to only the pages in use. Returns the size after.
///
/// Refuses rather than waiting when another process holds the database: a
/// compaction that ran anyway would be wrong, and one that waited would look
/// like a hang.
pub fn compact(path: &Path) -> Result<DbSize, VacuumError> {
    let before = reclaimable(path)?;
    let conn = open(path)?;
    assert_free(&conn)?;
    conn.execute_batch("VACUUM")
        .map_err(|e| VacuumError::Sql(e.to_string()))?;
    // `VACUUM` writes through the WAL, so the result is not on disk until the
    // log is folded back into the file. Without this the "after" size is the
    // pre-compaction file plus a full-size WAL.
    conn.execute_batch("PRAGMA wal_checkpoint(TRUNCATE);")
        .map_err(|e| VacuumError::Sql(e.to_string()))?;
    drop(conn);

    let after = footprint(path);
    Ok(DbSize {
        file_bytes: after,
        reclaimable_bytes: before.reclaimable_bytes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A database with a page on the free list: insert a large value, delete it,
    /// and SQLite keeps the page for reuse.
    fn db_with_free_pages() -> (tempfile::TempDir, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE t (v BLOB);
             INSERT INTO t VALUES (randomblob(1000000));
             DELETE FROM t;",
        )
        .unwrap();
        drop(conn);
        (dir, path)
    }

    #[test]
    fn footprint_counts_the_log_beside_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        std::fs::write(&path, vec![0u8; 8192]).unwrap();

        assert_eq!(footprint(&path), 8192, "no siblings, just the file");

        // A `-wal` that is really there must be counted: pages written since
        // the last checkpoint live in it, and it can be the larger of the two.
        let mut wal = path.as_os_str().to_os_string();
        wal.push("-wal");
        std::fs::write(&wal, vec![0u8; 4096]).unwrap();
        assert_eq!(footprint(&path), 12_288);
    }

    #[test]
    fn reclaimable_reports_the_freed_pages() {
        let (_dir, path) = db_with_free_pages();
        let size = reclaimable(&path).unwrap();
        assert!(size.reclaimable_bytes > 0, "{size:?}");
        assert!(size.file_bytes > size.reclaimable_bytes);
    }

    #[test]
    fn compact_returns_the_freed_pages_to_the_file() {
        let (_dir, path) = db_with_free_pages();
        let before = std::fs::metadata(&path).unwrap().len();
        let after = compact(&path).unwrap();
        assert!(
            after.file_bytes < before,
            "{before} -> {}",
            after.file_bytes
        );
    }

    #[test]
    fn a_held_database_is_refused_rather_than_waited_on() {
        let (_dir, path) = db_with_free_pages();

        // Hold the lock the way the gateway would, from a second connection.
        let holder = rusqlite::Connection::open(&path).unwrap();
        holder.execute_batch("BEGIN EXCLUSIVE").unwrap();

        match compact(&path) {
            Err(VacuumError::Busy(_)) => {}
            other => panic!("expected Busy, got {other:?}"),
        }
    }
}
