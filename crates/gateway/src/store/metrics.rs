//! Row counts for the admin `/status` endpoint.

use crate::error::Result;
use crate::store::Store;
use serde::{Deserialize, Serialize};

/// Row counts surfaced by the admin `/status` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreMetrics {
    pub providers: i64,
    pub bindings: i64,
    pub placeholder_keys: i64,
    pub usage_rows: i64,
    pub request_log_rows: i64,
}

impl Store {
    // ---- metrics ---------------------------------------------------------

    pub fn metrics(&self) -> Result<StoreMetrics> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };
        Ok(StoreMetrics {
            providers: count("SELECT COUNT(*) FROM providers")?,
            bindings: count("SELECT COUNT(*) FROM agent_bindings")?,
            placeholder_keys: count("SELECT COUNT(*) FROM placeholder_keys")?,
            usage_rows: count("SELECT COUNT(*) FROM usage")?,
            request_log_rows: count("SELECT COUNT(*) FROM request_logs")?,
        })
    }
}

#[cfg(test)]
mod tests {
    use crate::store::test_support::{sample_provider, temp_store};
    use crate::store::time::now_rfc3339;
    use crate::store::types::{Binding, Protocol, UsageRecord};

    #[test]
    fn metrics_counts() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p1", Protocol::Anthropic))
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-claude-abc", "claude")
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store
            .record_usage(&UsageRecord {
                ts: now_rfc3339(),
                agent: "claude".into(),
                provider_id: "p1".into(),
                model: None,
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
                cost_off_peak: None,
            })
            .unwrap();

        let m = store.metrics().unwrap();
        assert_eq!(m.providers, 1);
        assert_eq!(m.bindings, 1);
        assert_eq!(m.placeholder_keys, 1);
        assert_eq!(m.usage_rows, 1);
    }
}
