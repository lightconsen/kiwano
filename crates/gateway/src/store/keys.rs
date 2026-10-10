//! The two key tables: the client keys that attribute a request to an agent
//! (and now scope what that client may spend and use) and a provider's extra
//! API keys (spec §4.1 P1).

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::{ApiKeyRow, ClientKey, ClientKeyLimit};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};

/// The columns every client-key read shares, in the order `client_key_from_row`
/// expects them. One constant so a read cannot quietly disagree with another.
const CLIENT_KEY_COLUMNS: &str =
    "id, key, agent, label, model_allow, provider_allow, created_at, updated_at";

fn client_key_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClientKey> {
    Ok(ClientKey {
        id: row.get(0)?,
        key: row.get(1)?,
        agent: row.get(2)?,
        label: row.get(3)?,
        model_allow: row.get(4)?,
        provider_allow: row.get(5)?,
        created_at: row.get(6)?,
        updated_at: row.get(7)?,
    })
}

fn client_key_limit_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<ClientKeyLimit> {
    Ok(ClientKeyLimit {
        key_id: row.get(0)?,
        period: row.get(1)?,
        period_limit: row.get(2)?,
        limit_unit: row.get(3)?,
        created_at: row.get(4)?,
        updated_at: row.get(5)?,
    })
}

impl Store {
    // ---- client keys -----------------------------------------------------

    /// Register a key for an agent and return its handle (`ClientKey::id`).
    ///
    /// **Idempotent on the value**: a key the store already has keeps the row it
    /// has — its handle, its label, its limits and its spend history — and only
    /// has its agent rebound. Both callers (a takeover and a user-defined agent)
    /// mint a fresh value each time, so a repeat here means the same client is
    /// registering again, not a new one. `migrate.local.md` §6.1 counts this, with
    /// `insert_api_key` and `add_custom_agent`, among the writes that must be safe
    /// to retry.
    pub fn upsert_client_key(&self, key: &str, agent: &str) -> Result<String> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let now = now_rfc3339();
        if let Some(existing) = conn
            .query_row(
                "SELECT id FROM client_keys WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?
        {
            conn.execute(
                "UPDATE client_keys SET agent = ?2, updated_at = ?3 WHERE key = ?1",
                params![key, agent, now],
            )?;
            return Ok(existing);
        }
        let id = new_client_key_id();
        conn.execute(
            "INSERT INTO client_keys (id, key, agent, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?4)",
            params![id, key, agent, now],
        )?;
        Ok(id)
    }

    /// The key a request presented. `None` for a key nobody registered, which the
    /// data plane answers 401 to — never a fallback, for the reason
    /// `Attribution::PathFallback` records.
    pub fn client_key_by_value(&self, key: &str) -> Result<Option<ClientKey>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(
                &format!("SELECT {CLIENT_KEY_COLUMNS} FROM client_keys WHERE key = ?1"),
                params![key],
                client_key_from_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn get_client_key(&self, id: &str) -> Result<Option<ClientKey>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let row = conn
            .query_row(
                &format!("SELECT {CLIENT_KEY_COLUMNS} FROM client_keys WHERE id = ?1"),
                params![id],
                client_key_from_row,
            )
            .optional()?;
        Ok(row)
    }

    pub fn list_client_keys(&self) -> Result<Vec<ClientKey>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT {CLIENT_KEY_COLUMNS} FROM client_keys ORDER BY created_at ASC, id ASC"
        ))?;
        let rows = stmt
            .query_map([], client_key_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn list_client_keys_for_agent(&self, agent: &str) -> Result<Vec<ClientKey>> {
        Ok(self
            .list_client_keys()?
            .into_iter()
            .filter(|k| k.agent == agent)
            .collect())
    }

    /// Drop a key by its **handle**. The secret is not an identifier here: it is
    /// what the caller presents, and asking a management surface to name the row
    /// by a live credential is how credentials end up in shell history.
    pub fn delete_client_key(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM client_keys WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    /// Give a key a new secret, keeping everything that identifies it: the
    /// handle, the agent, the label, the allowlists, and the spend windows (and
    /// so the spend already recorded against those windows). Rotation is what a
    /// leaked key is answered with; it must not also be a way to clear a budget.
    ///
    /// Returns the new secret, or `None` when no such handle exists.
    pub fn rotate_client_key(&self, id: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        // Read the agent under the lock already held rather than through
        // `get_client_key`: the connection mutex is not reentrant, so the tidy
        // version of this deadlocks.
        let agent: Option<String> = conn
            .query_row(
                "SELECT agent FROM client_keys WHERE id = ?1",
                params![id],
                |row| row.get(0),
            )
            .optional()?;
        let Some(agent) = agent else {
            return Ok(None);
        };
        let key = format!("kw-ag-{agent}-{}", short_rand());
        conn.execute(
            "UPDATE client_keys SET key = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, key, now_rfc3339()],
        )?;
        Ok(Some(key))
    }

    /// Set the human name of a key. `None` clears it.
    pub fn set_client_key_label(&self, id: &str, label: Option<&str>) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "UPDATE client_keys SET label = ?2, updated_at = ?3 WHERE id = ?1",
            params![id, label, now_rfc3339()],
        )?;
        Ok(n > 0)
    }

    /// Replace both allowlists. `None` means no restriction; the caller passes
    /// `None` rather than `[]` for a cleared list, because a stored `[]` would
    /// read as "allow nothing".
    pub fn set_client_key_allowlists(
        &self,
        id: &str,
        model_allow: Option<&str>,
        provider_allow: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "UPDATE client_keys SET model_allow = ?2, provider_allow = ?3, updated_at = ?4
             WHERE id = ?1",
            params![id, model_allow, provider_allow, now_rfc3339()],
        )?;
        Ok(n > 0)
    }

    // ---- a client key's spend windows ------------------------------------

    pub fn list_client_key_limits(&self) -> Result<Vec<ClientKeyLimit>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key_id, period, period_limit, limit_unit, created_at, updated_at
             FROM client_key_limits ORDER BY key_id ASC, period ASC",
        )?;
        let rows = stmt
            .query_map([], client_key_limit_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    pub fn client_key_limits_for(&self, key_id: &str) -> Result<Vec<ClientKeyLimit>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key_id, period, period_limit, limit_unit, created_at, updated_at
             FROM client_key_limits WHERE key_id = ?1 ORDER BY period ASC",
        )?;
        let rows = stmt
            .query_map(params![key_id], client_key_limit_from_row)?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// Replace one key's windows with exactly these; an empty slice clears them.
    ///
    /// Replace rather than upsert-per-window, for the reason
    /// [`Store::replace_agent_limits`] gives: the set is what the caller holds,
    /// and a window that was deleted cannot be left behind by a partial write.
    /// A window that survives keeps its `created_at`.
    pub fn replace_client_key_limits(&self, key_id: &str, limits: &[ClientKeyLimit]) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        let prior: std::collections::HashMap<String, String> = tx
            .prepare("SELECT period, created_at FROM client_key_limits WHERE key_id = ?1")?
            .query_map(params![key_id], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute(
            "DELETE FROM client_key_limits WHERE key_id = ?1",
            params![key_id],
        )?;
        let now = now_rfc3339();
        for l in limits {
            tx.execute(
                "INSERT INTO client_key_limits
                     (key_id, period, period_limit, limit_unit, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    key_id,
                    l.period,
                    l.period_limit,
                    l.limit_unit,
                    prior.get(&l.period).cloned().unwrap_or_else(|| now.clone()),
                    now
                ],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    pub fn delete_client_key_limits(&self, key_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "DELETE FROM client_key_limits WHERE key_id = ?1",
            params![key_id],
        )?;
        Ok(n > 0)
    }
}

/// A handle: `ck-` and 48 bits. Not a credential, so it needs no more than
/// enough width that two rows minted in the same millisecond differ.
fn new_client_key_id() -> String {
    format!("ck-{}", short_rand_hex(6))
}

/// The `xxxx` in `kw-ag-<agent>-xxxx`: what distinguishes two keys of one agent.
fn short_rand() -> String {
    uuid::Uuid::new_v4().simple().to_string()[..4].to_string()
}

fn short_rand_hex(bytes: usize) -> String {
    uuid::Uuid::new_v4().simple().to_string()[..bytes * 2].to_string()
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
    /// Add a key to a provider's pool. **Idempotent on the key itself**: a key
    /// the provider already has is returned rather than inserted again.
    ///
    /// The identity of "this rotation key" is its value, so a request delivered
    /// twice is one key — which is what makes the API's `add_api_key` safe to
    /// retry (`migrate.local.md` §6.1: of batch 1's writes, this one and
    /// `add_custom_agent` and `add_provider` are the three that mint something).
    /// The check lives here rather than in one caller because both callers —
    /// the daemon's endpoint and the `vm::` path the app used to take — must
    /// agree about it.
    ///
    /// The label of an already-present key is left alone: the row exists, and a
    /// retry is not an edit.
    pub fn insert_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let existing: Option<i64> = conn
            .query_row(
                "SELECT id FROM api_keys WHERE provider_id = ?1 AND api_key = ?2",
                params![provider_id, api_key],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = existing {
            return Ok(id);
        }
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
    use crate::store::types::ClientKeyLimit;

    fn window(key_id: &str, period: &str, limit: f64, unit: Option<&str>) -> ClientKeyLimit {
        let now = crate::store::now_rfc3339();
        ClientKeyLimit {
            key_id: key_id.to_string(),
            period: period.to_string(),
            period_limit: limit,
            limit_unit: unit.map(str::to_string),
            created_at: now.clone(),
            updated_at: now,
        }
    }

    #[test]
    fn client_key_lookup() {
        let (_dir, store) = temp_store();
        store
            .upsert_client_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .upsert_client_key("kw-ag-codex-xyz789", "codex")
            .unwrap();

        let claude = store
            .client_key_by_value("kw-ag-claude-abc123")
            .unwrap()
            .expect("registered");
        assert_eq!(claude.agent, "claude");
        assert!(claude.id.starts_with("ck-"));
        // Nothing was said about limits or allowlists, so nothing restricts it —
        // which is also what every row that predates the columns reads as.
        assert_eq!(claude.model_allow, None);
        assert_eq!(claude.provider_allow, None);
        assert_eq!(claude.label, None);

        assert_eq!(store.client_key_by_value("kw-ag-unknown").unwrap(), None);

        // Upsert rebinds the agent and keeps the handle: same client, same
        // budget, new route.
        let again = store
            .upsert_client_key("kw-ag-claude-abc123", "gemini")
            .unwrap();
        assert_eq!(again, claude.id);
        assert_eq!(
            store
                .client_key_by_value("kw-ag-claude-abc123")
                .unwrap()
                .unwrap()
                .agent,
            "gemini"
        );

        // Two values may name one agent: they are two clients, two handles.
        let second = store
            .upsert_client_key("kw-ag-claude-zzz999", "gemini")
            .unwrap();
        assert_ne!(second, claude.id);
        assert_eq!(store.list_client_keys_for_agent("gemini").unwrap().len(), 2);

        assert_eq!(store.list_client_keys().unwrap().len(), 3);
        assert!(store.delete_client_key(&claude.id).unwrap());
        assert!(!store.delete_client_key(&claude.id).unwrap());
    }

    #[test]
    fn rotation_keeps_everything_but_the_secret() {
        let (_dir, store) = temp_store();
        let id = store
            .upsert_client_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .set_client_key_label(&id, Some("office laptop"))
            .unwrap();
        store
            .set_client_key_allowlists(&id, Some(r#"["gpt-5.5"]"#), None)
            .unwrap();
        store
            .replace_client_key_limits(&id, &[window(&id, "day", 100.0, Some("requests"))])
            .unwrap();

        let fresh = store.rotate_client_key(&id).unwrap().expect("exists");
        assert!(fresh.starts_with("kw-ag-claude-"));
        assert_ne!(fresh, "kw-ag-claude-abc123");

        // The old value is gone and the new one works under the same handle.
        assert!(store
            .client_key_by_value("kw-ag-claude-abc123")
            .unwrap()
            .is_none());
        let rotated = store.client_key_by_value(&fresh).unwrap().expect("exists");
        assert_eq!(rotated.id, id);
        assert_eq!(rotated.label.as_deref(), Some("office laptop"));
        assert_eq!(rotated.model_allow.as_deref(), Some(r#"["gpt-5.5"]"#));
        assert_eq!(store.client_key_limits_for(&id).unwrap().len(), 1);

        assert!(store.rotate_client_key("ck-nope").unwrap().is_none());
    }

    #[test]
    fn replacing_windows_keeps_a_survivors_age_and_clears_the_rest() {
        let (_dir, store) = temp_store();
        let id = store
            .upsert_client_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .replace_client_key_limits(
                &id,
                &[
                    window(&id, "day", 100.0, Some("requests")),
                    window(&id, "monthly", 20.0, Some("USD")),
                ],
            )
            .unwrap();
        let first_day = store
            .client_key_limits_for(&id)
            .unwrap()
            .into_iter()
            .find(|l| l.period == "day")
            .unwrap()
            .created_at;

        // A surviving window keeps when it was first set; the dropped one is gone.
        store
            .replace_client_key_limits(&id, &[window(&id, "day", 250.0, Some("wan_tokens"))])
            .unwrap();
        let after = store.client_key_limits_for(&id).unwrap();
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].period, "day");
        assert_eq!(after[0].period_limit, 250.0);
        assert_eq!(after[0].created_at, first_day);

        // Clearing is an empty set, not a window with no ceiling.
        store.replace_client_key_limits(&id, &[]).unwrap();
        assert!(store.client_key_limits_for(&id).unwrap().is_empty());
    }

    #[test]
    fn deleting_a_key_takes_its_windows_with_it() {
        let (_dir, store) = temp_store();
        let id = store
            .upsert_client_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .replace_client_key_limits(&id, &[window(&id, "day", 100.0, None)])
            .unwrap();

        assert!(store.delete_client_key(&id).unwrap());
        // The FK cascade is the reason a deleted key cannot leave a ceiling
        // behind that a re-created handle would inherit.
        assert!(store.list_client_key_limits().unwrap().is_empty());
    }
}
