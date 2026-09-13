//! The auxiliary SQLite connection: the same database file the gateway and the
//! app share, opened a second time for the app-scoped tables and the reads the
//! gateway's `Store` does not expose.
//!
//! It is separate from `Store` because it owns tables the gateway has no reason
//! to know about — `app_settings`, `takeover_backups`, `hub_cache`,
//! `hub_models_cache` — and those carry no migrations (see `sync`). Both
//! connections are WAL with a busy timeout, so they coexist.
//!
//! Extracted from `vm.rs` when the crate was split. `vm` re-exports it as
//! `vm::Aux`, so `crate::vm::Aux` paths keep resolving.

use std::sync::Mutex;

use rusqlite::Connection;

use crate::vm::{rfc3339, unix_now};

pub struct Aux {
    pub conn: Mutex<Connection>,
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
        // Hub catalog cache (tech.md §3 Hub sync): single-row cache, payload = CatalogListVm JSON
        conn.execute(
            "CREATE TABLE IF NOT EXISTS hub_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
             )",
            [],
        )?;
        // Hub pricing cache: single-row, self-describing so the seed gate is
        // one read. A new table (not a column) because `CREATE TABLE IF NOT
        // EXISTS` reaches existing databases for free, whereas added columns
        // would need a migration framework this half of the DB does not have.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS hub_models_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 version   INTEGER NOT NULL,
                 sha256    TEXT NOT NULL,
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
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

    /// Hub catalog cache: single-row upsert (RFC3339 synced_at).
    pub fn save_hub_cache(&self, payload: &str, synced_at: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO hub_cache (id, payload, synced_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = ?1, synced_at = ?2",
            rusqlite::params![payload, synced_at],
        )?;
        Ok(())
    }

    /// `(payload, synced_at)`; None when never synced.
    pub fn load_hub_cache(&self) -> Option<(String, String)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT payload, synced_at FROM hub_cache WHERE id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()
    }

    /// Refresh only the cache timestamp, leaving the payload untouched — the
    /// conditional-sync path (manifest sha matched, nothing to re-download).
    /// Returns false when there is no cache row (never synced).
    pub fn touch_hub_synced_at(&self, synced_at: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let n = conn.execute(
            "UPDATE hub_cache SET synced_at = ?1 WHERE id = 1",
            rusqlite::params![synced_at],
        )?;
        Ok(n > 0)
    }

    /// Hub pricing cache: single-row upsert. `payload` is the remote
    /// models.json **verbatim** — re-serializing would break the sha256 that
    /// the seed gate compares against the manifest.
    pub fn save_hub_models_cache(
        &self,
        version: i64,
        payload: &str,
        sha256: &str,
        synced_at: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 version = ?1, sha256 = ?2, payload = ?3, synced_at = ?4",
            rusqlite::params![version, sha256, payload, synced_at],
        )?;
        Ok(())
    }

    /// `(version, payload, sha256, synced_at)`; None when never fetched.
    pub fn load_hub_models_cache(&self) -> Option<(i64, String, String, String)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT version, payload, sha256, synced_at FROM hub_models_cache WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .ok()
    }

    /// Average `latency_ms` over a window, optionally per provider and/or
    /// agent. `from`/`to` are RFC3339 (store ts strings compare
    /// lexicographically).
    pub fn avg_latency(
        &self,
        provider: Option<&str>,
        agent: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Option<i64> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut sql =
            String::from("SELECT AVG(latency_ms) FROM usage WHERE latency_ms IS NOT NULL");
        // Owned params: binding trait objects to borrowed &str inside `if let`
        // scopes fights the borrows (same as Store::usage_filters)
        let mut params: Vec<String> = Vec::new();
        if let Some(p) = provider {
            params.push(p.to_string());
            sql.push_str(&format!(" AND provider_id = ?{}", params.len()));
        }
        if let Some(a) = agent {
            params.push(a.to_string());
            sql.push_str(&format!(" AND agent = ?{}", params.len()));
        }
        if let Some(f) = from {
            params.push(f.to_string());
            sql.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        if let Some(t) = to {
            params.push(t.to_string());
            sql.push_str(&format!(" AND ts < ?{}", params.len()));
        }
        let mut stmt = conn.prepare(&sql).ok()?;
        let avg: Option<f64> = stmt
            .query_row(rusqlite::params_from_iter(params), |r| {
                r.get::<_, Option<f64>>(0)
            })
            .ok()?;
        avg.map(|a| a.round() as i64)
    }

    /// Per-UTC-day token totals (input+output) for one provider.
    pub fn provider_daily(&self, provider_id: &str, since: &str) -> Vec<(String, i64)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut stmt = match conn.prepare(
            "SELECT SUBSTR(ts, 1, 10) AS day,
                    SUM(input_tokens + output_tokens)
             FROM usage WHERE provider_id = ?1 AND ts >= ?2
             GROUP BY day ORDER BY day ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let map = |r: &rusqlite::Row| -> rusqlite::Result<(String, i64)> {
            Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        };
        let rows = stmt.query_map(rusqlite::params![provider_id, since], map);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}
