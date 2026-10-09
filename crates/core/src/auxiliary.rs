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

    /// The `app_settings` keys that are **this machine's**, and therefore come
    /// across from the file the two halves used to share.
    ///
    /// Named by prefix rather than as "everything the daemon does not use", so a
    /// key it adopts later cannot be swept in by accident: this list is what the
    /// client owns, and growing it is a decision (`migrate.local.md` §9.4).
    ///
    /// The daemon's keys are deliberately absent — `dlp_finding_acked`,
    /// `pricing_seeded_*`, `hub_catalog_sha`, `alert_sent:*`,
    /// `plan_quota_cache:*` and the `ui` blob are read and written by *it*, and
    /// copying them here would be copying rows nothing on this side can serve.
    const CLIENT_KEY_PREFIXES: [&str; 3] = ["detect_dir:", "rules:", "rules_applied:"];

    /// The marker that says this client has already been brought across.
    const ADOPTED_MARKER: &str = "adopted_from_shared";

    /// The tables that come across whole. Rows the daemon never wrote — it has
    /// no connection to this side's file and no call to make to these.
    const ADOPTED_TABLES: [&str; 2] = ["takeover_backups", "takeover_ops"];

    /// Whether a database has a table at all.
    fn table_exists(conn: &Connection, schema: &str, table: &str) -> rusqlite::Result<bool> {
        let count: i64 = conn.query_row(
            &format!(
                "SELECT COUNT(*) FROM {schema}.sqlite_master WHERE type = 'table' AND name = ?1"
            ),
            rusqlite::params![table],
            |r| r.get(0),
        )?;
        Ok(count > 0)
    }

    /// Bring this client's own rows across from the database it used to share
    /// with the daemon (`migrate.local.md` §9.5 step 2).
    ///
    /// Runs once. The copy is `INSERT OR IGNORE`, so a second pass would be
    /// harmless anyway — but the marker is what makes "has this install been
    /// adopted" *answerable*, which is what a later step needs before the old
    /// copies can be considered for deletion.
    ///
    /// **The shared rows are left where they are** — §9.5's "先别删". An older
    /// build still pointed at that file finds what it wrote, and the copies cost
    /// a few rows. Nothing on this side reads them again.
    ///
    /// A shared file that is missing, unreadable or not a database is **not an
    /// error**: a client that never shared one starts with none, and this runs on
    /// the way up where a hard failure would be a client that cannot start.
    pub fn adopt_from(&self, shared: &std::path::Path) -> rusqlite::Result<usize> {
        if !shared.is_file() {
            return Ok(0);
        }
        let conn = self.conn.lock().expect("the aux connection");
        if Self::read_setting(&conn, Self::ADOPTED_MARKER)?.is_some() {
            return Ok(0);
        }
        // Attached rather than opened: one connection, one transaction's worth of
        // work, and nothing that outlives this call.
        if conn
            .execute(
                "ATTACH DATABASE ?1 AS shared",
                rusqlite::params![shared.to_string_lossy()],
            )
            .is_err()
        {
            return Ok(0);
        }

        let mut adopted = 0usize;
        let result = (|| -> rusqlite::Result<usize> {
            for table in Self::ADOPTED_TABLES {
                // The shared file may not have it: those tables were created by
                // `Aux::init_tables`, so a database that only the daemon ever
                // opened has `app_settings` (its migration makes one) and
                // neither of these. Absent means nothing to bring across, not a
                // failure.
                if !Self::table_exists(&conn, "shared", table)? {
                    continue;
                }
                adopted += conn.execute(
                    &format!("INSERT OR IGNORE INTO main.{table} SELECT * FROM shared.{table}"),
                    [],
                )?;
            }
            for prefix in Self::CLIENT_KEY_PREFIXES {
                adopted += conn.execute(
                    "INSERT OR IGNORE INTO main.app_settings (key, value)
                     SELECT key, value FROM shared.app_settings WHERE key LIKE ?1",
                    rusqlite::params![format!("{prefix}%")],
                )?;
            }
            // Counted, not assumed: a copy that silently dropped rows would
            // leave the marker behind and never try again.
            for (table, key_filter) in Self::ADOPTED_TABLES
                .iter()
                .map(|t| (*t, None))
                .chain([("app_settings", Some(Self::CLIENT_KEY_PREFIXES.as_slice()))])
            {
                if !Self::table_exists(&conn, "shared", table)? {
                    continue;
                }
                let source: i64 = match key_filter {
                    Some(prefixes) => {
                        let clause = prefixes
                            .iter()
                            .map(|p| format!("key LIKE '{p}%'"))
                            .collect::<Vec<_>>()
                            .join(" OR ");
                        conn.query_row(
                            &format!("SELECT COUNT(*) FROM shared.{table} WHERE {clause}"),
                            [],
                            |r| r.get(0),
                        )?
                    }
                    None => {
                        conn.query_row(&format!("SELECT COUNT(*) FROM shared.{table}"), [], |r| {
                            r.get(0)
                        })?
                    }
                };
                let here: i64 = match key_filter {
                    Some(prefixes) => {
                        let clause = prefixes
                            .iter()
                            .map(|p| format!("key LIKE '{p}%'"))
                            .collect::<Vec<_>>()
                            .join(" OR ");
                        conn.query_row(
                            &format!("SELECT COUNT(*) FROM main.{table} WHERE {clause}"),
                            [],
                            |r| r.get(0),
                        )?
                    }
                    None => {
                        conn.query_row(&format!("SELECT COUNT(*) FROM main.{table}"), [], |r| {
                            r.get(0)
                        })?
                    }
                };
                if here < source {
                    return Err(rusqlite::Error::ExecuteReturnedResults);
                }
            }
            Ok(adopted)
        })();

        let _ = conn.execute("DETACH DATABASE shared", []);
        if result.is_ok() {
            conn.execute(
                "INSERT OR REPLACE INTO app_settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![Self::ADOPTED_MARKER, "1"],
            )?;
        }
        result
    }

    /// [`Aux::get_setting`] on a connection that is already held — the shape
    /// `adopt_from` needs, because locking twice would deadlock.
    fn read_setting(conn: &Connection, key: &str) -> rusqlite::Result<Option<String>> {
        Ok(conn
            .query_row(
                "SELECT value FROM app_settings WHERE key = ?1",
                rusqlite::params![key],
                |row| row.get(0),
            )
            .ok())
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

    /// The prefix a declared agent directory is stored under.
    ///
    /// Spelled the same as the constant these rows lived under while they were
    /// in the shared database: [`Aux::adopt_from`] matches on it, so the two are
    /// one spelling or the copy finds nothing.
    pub const MANUAL_AGENT_DIR_PREFIX: &str = "detect_dir:";

    /// The install directory a user declared for a built-in agent the detector
    /// could not find on its own, keyed by agent id.
    ///
    /// Moved here from `kiwanod::store::Store` (`migrate.local.md` §9.5 step 2).
    /// It lived there by **layering** — the accessor's code sits in the daemon's
    /// crate — while every runtime caller was a client, which is the distinction
    /// §9.3 had to be written to record. A declaration is a fact about this
    /// machine: which `claude` binary the user pointed at is not something
    /// another machine's daemon can answer, and not something it should store.
    ///
    /// One row per agent rather than a JSON blob: the value is a path, the key
    /// names its owner, and a row that cannot be read can only ever lose itself.
    /// Nothing here fails — a declaration is a hint, and a client that cannot
    /// read one behaves exactly like one that never had it.
    pub fn manual_agent_dirs(&self) -> std::collections::BTreeMap<String, std::path::PathBuf> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let Ok(mut stmt) = conn.prepare("SELECT key, value FROM app_settings WHERE key LIKE ?1")
        else {
            return Default::default();
        };
        let rows = stmt.query_map(
            rusqlite::params![format!("{}%", Self::MANUAL_AGENT_DIR_PREFIX)],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        );
        let Ok(rows) = rows else {
            return Default::default();
        };
        rows.filter_map(|row| {
            let (key, value) = row.ok()?;
            let agent = key.strip_prefix(Self::MANUAL_AGENT_DIR_PREFIX)?;
            let dir = value.trim();
            (!agent.is_empty() && !dir.is_empty())
                .then(|| (agent.to_string(), std::path::PathBuf::from(dir)))
        })
        .collect()
    }

    /// Record an agent's declared directory, replacing whatever was there.
    pub fn set_manual_agent_dir(&self, agent: &str, dir: &std::path::Path) -> rusqlite::Result<()> {
        self.set_setting(
            &format!("{}{agent}", Self::MANUAL_AGENT_DIR_PREFIX),
            &dir.to_string_lossy(),
        )
    }

    /// Forget it. `true` when there was one to forget.
    pub fn clear_manual_agent_dir(&self, agent: &str) -> rusqlite::Result<bool> {
        self.delete_setting(&format!("{}{agent}", Self::MANUAL_AGENT_DIR_PREFIX))
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    /// The declared directories round-trip, and only their own bad rows drop
    /// out. Moved here with the accessors (`migrate.local.md` §9.5 step 2): a
    /// declaration is a fact about *this machine*, so it lives in this machine's
    /// file.
    #[test]
    fn manual_agent_dirs_round_trip_and_lose_only_their_own_bad_rows() {
        let aux = Aux::open_in_memory().unwrap();
        assert!(aux.manual_agent_dirs().is_empty());

        aux.set_manual_agent_dir("gemini", Path::new("/opt/custom/bin"))
            .unwrap();
        aux.set_manual_agent_dir("qwen", Path::new("/srv/qwen/bin"))
            .unwrap();
        let dirs = aux.manual_agent_dirs();
        assert_eq!(
            dirs.get("gemini"),
            Some(&std::path::PathBuf::from("/opt/custom/bin"))
        );
        assert_eq!(
            dirs.get("qwen"),
            Some(&std::path::PathBuf::from("/srv/qwen/bin"))
        );

        // Replacing one leaves its neighbour where it was.
        aux.set_manual_agent_dir("gemini", Path::new("/elsewhere"))
            .unwrap();
        let dirs = aux.manual_agent_dirs();
        assert_eq!(
            dirs.get("gemini"),
            Some(&std::path::PathBuf::from("/elsewhere"))
        );
        assert_eq!(
            dirs.get("qwen"),
            Some(&std::path::PathBuf::from("/srv/qwen/bin"))
        );

        // A row with nothing usable in it drops out; the others stand.
        aux.set_setting(&format!("{}broken", Aux::MANUAL_AGENT_DIR_PREFIX), "   ")
            .unwrap();
        let dirs = aux.manual_agent_dirs();
        assert!(!dirs.contains_key("broken"));
        assert!(dirs.contains_key("qwen"));

        // Clearing reports whether there was one, and is idempotent after.
        assert!(aux.clear_manual_agent_dir("gemini").unwrap());
        assert!(!aux.clear_manual_agent_dir("gemini").unwrap());
        assert!(!aux.manual_agent_dirs().contains_key("gemini"));
    }

    /// A shared file with rows in it, as a build that shared the database left
    /// one: this machine's keys, plus two the daemon owns.
    fn shared_file(dir: &Path) -> std::path::PathBuf {
        let path = dir.join("kiwano.db");
        let conn = Connection::open(&path).unwrap();
        conn.execute(
            "CREATE TABLE app_settings (key TEXT PRIMARY KEY, value TEXT NOT NULL)",
            [],
        )
        .unwrap();
        for (key, value) in [
            ("detect_dir:gemini", "/opt/bin"),
            ("rules:claude", "[{\"text\":\"x\"}]"),
            ("rules_applied:claude", "2026-01-01T00:00:00Z"),
            // The daemon's, and must **not** come across: it reads and writes
            // these itself, and a client with a stale copy would be a second
            // answer to a question only the daemon is asked.
            ("dlp_finding_acked", "7"),
            ("hub_catalog_sha", "abc"),
        ] {
            conn.execute(
                "INSERT INTO app_settings (key, value) VALUES (?1, ?2)",
                rusqlite::params![key, value],
            )
            .unwrap();
        }
        conn.execute(
            "CREATE TABLE takeover_backups (
                 agent TEXT PRIMARY KEY, files TEXT NOT NULL, backed_up_at TEXT NOT NULL)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO takeover_backups VALUES ('claude', '[]', '2026-01-01T00:00:00Z')",
            [],
        )
        .unwrap();
        conn.execute(
            "CREATE TABLE takeover_ops (
                 agent TEXT PRIMARY KEY, op_id TEXT NOT NULL, state TEXT NOT NULL,
                 started_at TEXT NOT NULL, applied_at TEXT)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO takeover_ops VALUES ('claude', 'op-1', 'applied', '2026-01-01T00:00:00Z', NULL)",
            [],
        )
        .unwrap();
        drop(conn);
        path
    }

    /// An upgraded install ends up with the rows a fresh one would have had —
    /// and with none of the daemon's.
    ///
    /// The rehearsal `migrate.local.md` §3.3b asks for, in the shape §10's
    /// fourth test class describes: the two arrival paths have to converge. The
    /// second half is the one that matters more — a copy that swept in
    /// `dlp_finding_acked` would look like success and be a second source for a
    /// row the daemon is the authority on.
    #[test]
    fn an_upgraded_client_ends_up_with_what_a_fresh_one_would_have() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_file(dir.path());

        let upgraded = Aux::open(dir.path().join("local.db")).unwrap();
        let adopted = upgraded.adopt_from(&shared).unwrap();
        assert_eq!(adopted, 5, "two tables' rows and three client keys");

        let fresh = Aux::open(dir.path().join("fresh.db")).unwrap();
        fresh
            .set_manual_agent_dir("gemini", Path::new("/opt/bin"))
            .unwrap();
        fresh
            .set_setting("rules:claude", "[{\"text\":\"x\"}]")
            .unwrap();
        fresh
            .set_setting("rules_applied:claude", "2026-01-01T00:00:00Z")
            .unwrap();
        fresh.save_takeover_backup("claude", &[]).unwrap();

        // Table by table, and key by key: the two files answer the same.
        assert_eq!(upgraded.manual_agent_dirs(), fresh.manual_agent_dirs());
        assert_eq!(
            upgraded.get_setting("rules:claude"),
            fresh.get_setting("rules:claude")
        );
        // The row's presence and its files, not the timestamp:
        // `save_takeover_backup` stamps "now", so the two were written seconds
        // apart by construction.
        let (up_files, fresh_files) = (
            upgraded
                .load_takeover_backup("claude")
                .map(|(_, f)| f.len()),
            fresh.load_takeover_backup("claude").map(|(_, f)| f.len()),
        );
        assert_eq!(up_files, fresh_files);
        assert!(upgraded.load_takeover_backup("claude").is_some());
        assert_eq!(
            upgraded.load_takeover_op("claude").map(|o| o.op_id),
            Some("op-1".to_string())
        );

        // And the daemon's rows stayed where they are.
        for key in ["dlp_finding_acked", "hub_catalog_sha"] {
            assert_eq!(upgraded.get_setting(key), None, "{key} came across");
        }
    }

    /// Adopting twice is one adoption.
    ///
    /// Not merely harmless — the marker is what makes "has this install been
    /// adopted" answerable, which is what a later step needs before the shared
    /// copies could be considered for deletion.
    #[test]
    fn adopting_twice_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let shared = shared_file(dir.path());
        let aux = Aux::open(dir.path().join("local.db")).unwrap();

        assert!(aux.adopt_from(&shared).unwrap() > 0);
        assert_eq!(aux.adopt_from(&shared).unwrap(), 0, "the marker held");

        // A client that changes one afterwards keeps its own answer.
        aux.set_manual_agent_dir("gemini", Path::new("/mine"))
            .unwrap();
        assert_eq!(aux.adopt_from(&shared).unwrap(), 0);
        assert_eq!(
            aux.manual_agent_dirs().get("gemini"),
            Some(&std::path::PathBuf::from("/mine"))
        );
    }

    /// A file that is not a database, and one that does not exist, are both
    /// "nothing to adopt" — this runs on the way up, where a hard failure would
    /// be a client that cannot start.
    #[test]
    fn a_shared_file_that_cannot_be_read_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let aux = Aux::open(dir.path().join("local.db")).unwrap();

        assert_eq!(aux.adopt_from(&dir.path().join("absent.db")).unwrap(), 0);

        let junk = dir.path().join("junk.db");
        std::fs::write(&junk, b"not a database").unwrap();
        assert_eq!(aux.adopt_from(&junk).unwrap(), 0);
    }
}
