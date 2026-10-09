//! The auxiliary SQLite connection: the same database file the gateway and the
//! app share, opened a second time for the app-scoped tables and the reads the
//! gateway's `Store` does not expose.
//!
//! It is separate from `Store` because it owns tables the gateway has no reason
//! to know about — `app_settings`, `takeover_backups`, `takeover_ops` — and
//! those carry no migrations (see `sync`). Both connections are WAL with a busy
//! timeout, so they coexist.
//!
//! **The Hub caches are no longer here.** They moved to the daemon's schema
//! (`store::hub`, migration v28) when the sync became the daemon's: the client
//! was the only writer, so a second set of accessors on this side was a second
//! path to a table with one owner (`migrate.local.md` §10.14).
//!
//! Extracted from `vm.rs` when the crate was split. `vm` re-exports it as
//! `vm::Aux`, so `crate::vm::Aux` paths keep resolving.

use std::sync::Mutex;

use rusqlite::Connection;

use crate::vm::{rfc3339, unix_now};

pub struct Aux {
    pub conn: Mutex<Connection>,
}

/// What a takeover operation's row says. Two states, because the point of the
/// row is the window between them: `Pending` means the store half landed and
/// the file half has not (or the process died trying).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TakeoverOpState {
    Pending,
    Applied,
}

impl TakeoverOpState {
    /// An unknown string reads as `Pending`, the conservative side: it is the
    /// state that makes `reconcile_takeovers` look at the agent.
    fn parse(raw: &str) -> Self {
        match raw {
            "applied" => Self::Applied,
            _ => Self::Pending,
        }
    }
}

/// A takeover operation as stored: `op_id` is what makes a replayed request
/// recognisable as the same operation rather than a new one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TakeoverOp {
    pub op_id: String,
    pub state: TakeoverOpState,
    pub started_at: String,
    pub applied_at: Option<String>,
}

impl TakeoverOp {
    /// Whether the file half is recorded as having landed.
    pub fn is_applied(&self) -> bool {
        self.state == TakeoverOpState::Applied
    }
}

impl Aux {
    pub fn open(path: impl AsRef<std::path::Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // The gateway sidecar holds the same file with a 5s busy timeout; the
        // GUI writes settings concurrently, so mirror it to survive lock races.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             )",
            [],
        )?;
        // takeover backups (tech.md §4.3-3): files = JSON [[path, content], ...]
        conn.execute(
            "CREATE TABLE IF NOT EXISTS takeover_backups (
                 agent         TEXT PRIMARY KEY,
                 files         TEXT NOT NULL,
                 backed_up_at  TEXT NOT NULL
             )",
            [],
        )?;
        // A takeover operation in flight — one row per agent.
        //
        // A new table rather than two more columns on `takeover_backups`, for
        // the reason `hub_models_cache` gives below: `CREATE TABLE IF NOT
        // EXISTS` reaches existing databases for free, and this half of the DB
        // has no migration framework.
        //
        // It exists because a takeover has two halves that cannot commit
        // together — the store rows and the agent's own config files — and each
        // half looks complete on its own. The row is what makes "the store half
        // landed and the file half did not" a state someone can see:
        // `vm::takeover::reconcile_takeovers` reads it and converges. An agent
        // with no row is never touched by that pass, which is what keeps every
        // install that predates this table out of it.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS takeover_ops (
                 agent       TEXT PRIMARY KEY,
                 op_id       TEXT NOT NULL,
                 state       TEXT NOT NULL,
                 started_at  TEXT NOT NULL,
                 applied_at  TEXT
             )",
            [],
        )?;
        Ok(())
    }

    pub fn save_takeover_backup(
        &self,
        agent: &str,
        files: &[crate::takeover::BackupFile],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let json = serde_json::to_string(files).expect("serialize backup files");
        conn.execute(
            "INSERT INTO takeover_backups (agent, files, backed_up_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent) DO UPDATE SET files = ?2, backed_up_at = ?3",
            rusqlite::params![agent, json, rfc3339(unix_now())],
        )?;
        Ok(())
    }

    pub fn load_takeover_backup(
        &self,
        agent: &str,
    ) -> Option<(String, Vec<crate::takeover::BackupFile>)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let (ts, json) = conn
            .query_row(
                "SELECT backed_up_at, files FROM takeover_backups WHERE agent = ?1",
                [agent],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()?;
        // Earlier builds stored a bare [path, content] array, which cannot say
        // whether the file existed. Reading those as "existed" keeps the old
        // restore behaviour for backups already on disk. Refusing them instead
        // would make `disable` report "never taken over" and leave the agent
        // pointed at the gateway — the one outcome worth avoiding here.
        let files = serde_json::from_str::<Vec<crate::takeover::BackupFile>>(&json)
            .or_else(|_| {
                serde_json::from_str::<Vec<(String, String)>>(&json).map(|legacy| {
                    legacy
                        .into_iter()
                        .map(|(path, content)| crate::takeover::BackupFile {
                            path,
                            content,
                            existed: true,
                        })
                        .collect()
                })
            })
            .ok()?;
        Some((ts, files))
    }

    pub fn delete_takeover_backup(&self, agent: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute("DELETE FROM takeover_backups WHERE agent = ?1", [agent])?;
        Ok(())
    }

    /// Record that a takeover's store half has landed and its file half has
    /// not. Supersedes any previous operation for the same agent: one agent has
    /// at most one takeover in flight, and an abandoned one is not a reason to
    /// refuse the next.
    pub fn start_takeover_op(&self, agent: &str, op_id: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO takeover_ops (agent, op_id, state, started_at, applied_at)
             VALUES (?1, ?2, 'pending', ?3, NULL)
             ON CONFLICT(agent) DO UPDATE SET
                 op_id = ?2, state = 'pending', started_at = ?3, applied_at = NULL",
            rusqlite::params![agent, op_id, rfc3339(unix_now())],
        )?;
        Ok(())
    }

    /// Mark the operation applied — the file half landed. Answered `true` only
    /// for the operation that made the transition, so a replayed mark is not
    /// mistaken for a fresh one.
    pub fn apply_takeover_op(&self, agent: &str, op_id: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let changed = conn.execute(
            "UPDATE takeover_ops SET state = 'applied', applied_at = ?3
             WHERE agent = ?1 AND op_id = ?2 AND state = 'pending'",
            rusqlite::params![agent, op_id, rfc3339(unix_now())],
        )?;
        Ok(changed == 1)
    }

    pub fn load_takeover_op(&self, agent: &str) -> Option<TakeoverOp> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT op_id, state, started_at, applied_at FROM takeover_ops WHERE agent = ?1",
            [agent],
            |row| {
                Ok(TakeoverOp {
                    op_id: row.get(0)?,
                    state: TakeoverOpState::parse(&row.get::<_, String>(1)?),
                    started_at: row.get(2)?,
                    applied_at: row.get(3)?,
                })
            },
        )
        .ok()
    }

    pub fn clear_takeover_op(&self, agent: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute("DELETE FROM takeover_ops WHERE agent = ?1", [agent])?;
        Ok(())
    }

    pub fn load_settings_json(&self) -> Option<serde_json::Value> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = 'ui'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save_settings_json(&self, v: &serde_json::Value) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('ui', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [v.to_string()],
        )?;
        Ok(())
    }

    /// Generic KV read (app_settings table; used for cost-alert dedup etc.).
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, value],
        )?;
        Ok(())
    }

    /// Drop a KV entry (plan-limit enforcement markers, expired dedup…).
    pub fn delete_setting(&self, key: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let n = conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])?;
        Ok(n > 0)
    }
}
