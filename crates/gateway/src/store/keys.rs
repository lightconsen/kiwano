//! The two key tables: the placeholder keys that attribute a request to an
//! agent (tech.md §4.6) and a provider's extra API keys (spec §4.1 P1).

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::{ApiKeyRow, PlaceholderKey};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};

impl Store {
    // ---- placeholder keys (tech.md §4.6) --------------------------------

    pub fn upsert_placeholder_key(&self, key: &str, agent: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO placeholder_keys (key, agent, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET agent = ?2",
            params![key, agent, now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn agent_for_key(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let agent = conn
            .query_row(
                "SELECT agent FROM placeholder_keys WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(agent)
    }

    pub fn delete_placeholder_key(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM placeholder_keys WHERE key = ?1", params![key])?;
        Ok(n > 0)
    }

    pub fn list_placeholder_keys(&self) -> Result<Vec<PlaceholderKey>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key, agent, created_at FROM placeholder_keys ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PlaceholderKey {
                key: row.get(0)?,
                agent: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

impl Store {
    // ---- extra API keys (spec §4.1 P1 multi-key rotation) -----------------------

    pub fn list_api_keys(&self, provider_id: &str) -> Result<Vec<ApiKeyRow>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, provider_id, api_key, label, enabled, created_at
             FROM api_keys WHERE provider_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![provider_id], |row| {
            Ok(ApiKeyRow {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                api_key: row.get(2)?,
                label: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Insert an extra key; returns its row id.
    pub fn insert_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO api_keys (provider_id, api_key, label, enabled, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)",
            params![provider_id, api_key, label, now_rfc3339()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_api_key(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM api_keys WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use crate::store::test_support::temp_store;

    #[test]
    fn placeholder_key_lookup() {
        let (_dir, store) = temp_store();
        store
            .upsert_placeholder_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-codex-xyz789", "codex")
            .unwrap();

        assert_eq!(
            store.agent_for_key("kw-ag-claude-abc123").unwrap(),
            Some("claude".to_string())
        );
        assert_eq!(
            store.agent_for_key("kw-ag-codex-xyz789").unwrap(),
            Some("codex".to_string())
        );
        assert_eq!(store.agent_for_key("kw-ag-unknown").unwrap(), None);

        // Upsert rebinds the agent.
        store
            .upsert_placeholder_key("kw-ag-claude-abc123", "gemini")
            .unwrap();
        assert_eq!(
            store.agent_for_key("kw-ag-claude-abc123").unwrap(),
            Some("gemini".to_string())
        );

        assert_eq!(store.list_placeholder_keys().unwrap().len(), 2);
        assert!(store.delete_placeholder_key("kw-ag-claude-abc123").unwrap());
        assert!(!store.delete_placeholder_key("kw-ag-claude-abc123").unwrap());
    }
}
