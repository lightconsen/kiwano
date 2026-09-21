//! The latest health-probe verdict per provider (`provider_health`), which
//! the prober writes and the Apps screen reads whole.

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::ProviderHealth;
use crate::store::Store;
use rusqlite::params;

impl Store {
    // ---- provider health (P1 failover groundwork) ------------------------

    /// Record one health-probe verdict (the gateway's prober → `provider_health`).
    ///
    /// A row per provider, replaced in place: the table holds the *latest*
    /// answer, and nothing reads a history of them.
    pub fn upsert_provider_health(
        &self,
        provider_id: &str,
        status: &str,
        latency_ms: i64,
        source: &str,
        error: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO provider_health (provider_id, status, latency_ms, checked_at, source, error)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(provider_id) DO UPDATE SET
                status = ?2, latency_ms = ?3, checked_at = ?4, source = ?5, error = ?6",
            params![provider_id, status, latency_ms, now_rfc3339(), source, error],
        )?;
        Ok(())
    }

    /// Every health-probe verdict on file, for the view that shows them.
    ///
    /// Read whole rather than per provider: the Apps screen renders every row
    /// from one pass, so a lookup per row would be the same query N times.
    pub fn list_provider_health(&self) -> Result<Vec<ProviderHealth>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT provider_id, status, latency_ms, checked_at, source, error
             FROM provider_health ORDER BY provider_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProviderHealth {
                    provider_id: row.get(0)?,
                    status: row.get(1)?,
                    latency_ms: row.get(2)?,
                    checked_at: row.get(3)?,
                    source: row.get(4)?,
                    error: row.get(5)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::test_support::{sample_provider, temp_store};
    use crate::store::types::Protocol;

    /// The prober's verdict lands in `provider_health`, is replaced in place,
    /// and goes with the provider it describes.
    #[test]
    fn provider_health_roundtrip() {
        let (_dir, store) = temp_store();
        let p = sample_provider("p-probe", Protocol::OpenAI);
        store.insert_provider(&p).unwrap();
        store
            .insert_provider(&sample_provider("p-other", Protocol::OpenAI))
            .unwrap();
        assert!(store.list_provider_health().unwrap().is_empty());

        store
            .upsert_provider_health("p-probe", "reachable", 42, "probe", None)
            .unwrap();
        let rows = store.list_provider_health().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].provider_id, "p-probe");
        assert_eq!(rows[0].status, "reachable");
        assert_eq!(rows[0].latency_ms, Some(42));
        assert!(!rows[0].checked_at.is_empty(), "the verdict is dated");

        // A second round replaces the verdict rather than adding one: the table
        // holds the latest answer, and a provider going down is one row moving.
        store
            .upsert_provider_health("p-probe", "down", 0, "test", Some("invalid API key"))
            .unwrap();
        let rows = store.list_provider_health().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].status, "down");
        assert_eq!(rows[0].source, "test", "the verdict says who took it");
        assert_eq!(rows[0].error.as_deref(), Some("invalid API key"));
        assert_eq!(rows[0].latency_ms, Some(0));

        // The row is the provider's: deleting it takes the verdict with it (the
        // FK cascade, which is why the table can be read without a join).
        store.delete_provider("p-probe").unwrap();
        assert!(store.list_provider_health().unwrap().is_empty());
    }

    #[test]
    fn health_upsert_and_get() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p1", Protocol::OpenAI))
            .unwrap();
    }
}
