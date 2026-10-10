//! The Hub caches: the catalog and the price table as last synced.
//!
//! # Why these are the daemon's tables now
//!
//! Both caches used to live in the app's half of the database — created and read
//! by `kiwano_core::auxiliary::Aux` — and the client was the only side that
//! touched them. That stopped being true the moment a command that *reads* them
//! had to be served by the daemon: `add_provider` infers a provider's Hub entry
//! from this payload (`catalog_snapshot`), and `list_catalog` serves the shelf
//! straight out of it. The daemon cannot depend on `kiwano-core`, so the choice
//! was to duplicate the parse-and-normalize rules, to introduce a shared crate
//! for the database layer, or to **assign ownership** — and ownership is what
//! `migrate.local.md` §9-B decided for the database as a whole.
//!
//! So the caches are the daemon's: the tables are created by the daemon's
//! migrations (v28), and the reads live here. The *writer* is still the client —
//! `sync` runs in the app until the Hub half of batch 2 moves — which is why
//! `Aux` keeps its own accessors for now. When the sync moves, they go, and the
//! table has one writer and one set of accessors.
//!
//! The migration that creates these tables is `IF NOT EXISTS` on purpose: every
//! existing install already has them (the app made them), and the columns are
//! the same. Nothing about the old data changes; what changes is who may say
//! what the schema is.

use crate::store::Store;

impl Store {
    /// The cached Hub catalog: `(payload, synced_at)`, or `None` when never
    /// synced. The payload is the remote JSON **verbatim** — the sha256 in
    /// `hub_models_cache` and the shelf's parse both depend on that.
    pub fn hub_cache(&self) -> Option<(String, String)> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.query_row(
            "SELECT payload, synced_at FROM hub_cache WHERE id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()
    }

    /// The catalog half of a sync. Upsert: the cache is one row.
    pub fn save_hub_cache(&self, payload: &str, synced_at: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO hub_cache (id, payload, synced_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = ?1, synced_at = ?2",
            rusqlite::params![payload, synced_at],
        )?;
        Ok(())
    }

    /// Refresh only the timestamp, leaving the payload untouched — the
    /// conditional-sync path (the manifest's sha matched, so nothing was
    /// re-downloaded). `false` when there is no row yet.
    pub fn touch_hub_synced_at(&self, synced_at: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "UPDATE hub_cache SET synced_at = ?1 WHERE id = 1",
            rusqlite::params![synced_at],
        )?;
        Ok(n > 0)
    }

    /// The cached price table: `(version, payload, sha256, synced_at)`, or
    /// `None` when never fetched.
    pub fn hub_models_cache(&self) -> Option<(i64, String, String, String)> {
        let conn = self.conn.lock().expect("store mutex poisoned");
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

    /// The price half of a sync. `payload` is the remote `models.json`
    /// **verbatim** — re-serializing would break the sha256 the seed gate
    /// compares against the manifest.
    pub fn save_hub_models_cache(
        &self,
        version: i64,
        payload: &str,
        sha256: &str,
        synced_at: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 version = ?1, sha256 = ?2, payload = ?3, synced_at = ?4",
            rusqlite::params![version, sha256, payload, synced_at],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trips, and the empty case is `None` rather than an error: a fresh
    /// install has never synced, and every reader treats that as "no catalog"
    /// (`catalog_snapshot` answers an empty shelf).
    #[test]
    fn the_caches_round_trip_and_start_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.hub_cache().is_none());
        assert!(store.hub_models_cache().is_none());
        assert!(!store.touch_hub_synced_at("2026-01-01T00:00:00Z").unwrap());

        store
            .save_hub_cache(r#"{"total":0,"entries":[]}"#, "2026-01-01T00:00:00Z")
            .unwrap();
        let (payload, at) = store.hub_cache().unwrap();
        assert_eq!(payload, r#"{"total":0,"entries":[]}"#);
        assert_eq!(at, "2026-01-01T00:00:00Z");

        // Upsert, not insert: the cache is one row.
        store
            .save_hub_cache(r#"{"total":1,"entries":[]}"#, "2026-01-02T00:00:00Z")
            .unwrap();
        assert_eq!(store.hub_cache().unwrap().0, r#"{"total":1,"entries":[]}"#);

        store
            .save_hub_models_cache(7, r#"{"models":[]}"#, "sha-abc", "2026-01-02T00:00:00Z")
            .unwrap();
        let (version, payload, sha, _) = store.hub_models_cache().unwrap();
        assert_eq!(
            (version, payload.as_str(), sha.as_str()),
            (7, r#"{"models":[]}"#, "sha-abc")
        );
    }

    /// The upgrade path, which is the whole risk of a table changing hands.
    ///
    /// Every install that predates this migration already has these tables —
    /// the app's `Aux` created them — and may hold a synced catalog in them. The
    /// migration is `IF NOT EXISTS` with the same columns, so opening such a
    /// database must keep the payload and still stamp the new version. A
    /// migration that dropped and recreated the table would silently empty
    /// every user's shelf.
    #[test]
    fn a_database_the_app_already_populated_survives_the_migration() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("t.db");
        {
            // An install from before v28: the app's schema, a synced payload,
            // and the version it was stamped with.
            let conn = rusqlite::Connection::open(&path).unwrap();
            // The base tables first. A real v27 install has them all — they come
            // from v1 — and the migrations that run from 27 touch some of them
            // (`client_keys` is renamed, `usage` and `providers` gain a column),
            // so a fixture carrying only the hub tables would be testing a
            // database no install ever had.
            conn.execute_batch(crate::store::migrations::BASE_SCHEMA_SQL)
                .unwrap();
            conn.execute_batch(
                "CREATE TABLE hub_cache (
                     id        INTEGER PRIMARY KEY CHECK (id = 1),
                     payload   TEXT NOT NULL,
                     synced_at TEXT NOT NULL
                 );
                 INSERT INTO hub_cache (id, payload, synced_at)
                     VALUES (1, '{\"total\":2,\"entries\":[]}', '2026-01-01T00:00:00Z');
                 PRAGMA user_version = 27;",
            )
            .unwrap();
        }

        let store = Store::open(&path).unwrap();

        let (payload, synced_at) = store.hub_cache().expect("the app's row is still there");
        assert_eq!(payload, r#"{"total":2,"entries":[]}"#);
        assert_eq!(synced_at, "2026-01-01T00:00:00Z");
        let version: i32 = rusqlite::Connection::open(&path)
            .unwrap()
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(
            version,
            crate::store::SCHEMA_VERSION,
            "stamped at the new version"
        );
    }

    /// The conditional-sync path: the payload stays, only the timestamp moves —
    /// that is what lets the footer say "checked, still current" without
    /// re-downloading the catalog.
    #[test]
    fn touching_the_timestamp_leaves_the_payload_alone() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_hub_cache("payload-1", "2026-01-01T00:00:00Z")
            .unwrap();

        assert!(store.touch_hub_synced_at("2026-01-02T00:00:00Z").unwrap());
        assert_eq!(
            store.hub_cache().unwrap(),
            ("payload-1".to_string(), "2026-01-02T00:00:00Z".to_string())
        );
    }
}
