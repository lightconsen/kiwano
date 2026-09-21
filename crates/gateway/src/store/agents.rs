//! Agent-level configuration the user owns: the spending ceilings of a
//! route (one row per window since v19) and the custom agents they
//! defined (v16).

use crate::error::Result;
use crate::store::time::now_rfc3339;
use crate::store::types::{AgentLimit, CustomAgent};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};

impl Store {
    // ---- agent limits (migration v17, one row per window since v19) ------

    /// Every agent's ceilings, ordered by agent then window. Read whole rather
    /// than by key: the evaluator wants all of them, and the screen asks per agent
    /// off a list it already has.
    pub fn list_agent_limits(&self) -> Result<Vec<AgentLimit>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT agent, period, period_limit, limit_unit, created_at, updated_at
             FROM agent_limits ORDER BY agent ASC, period ASC",
        )?;
        let rows = stmt
            .query_map([], |r| {
                Ok(AgentLimit {
                    agent: r.get(0)?,
                    period: r.get(1)?,
                    period_limit: r.get(2)?,
                    limit_unit: r.get(3)?,
                    created_at: r.get(4)?,
                    updated_at: r.get(5)?,
                })
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        Ok(rows)
    }

    /// One agent's ceilings, in window order — the shape the screen edits.
    pub fn agent_limits_for(&self, agent: &str) -> Result<Vec<AgentLimit>> {
        Ok(self
            .list_agent_limits()?
            .into_iter()
            .filter(|l| l.agent == agent)
            .collect())
    }

    /// Replace one agent's ceilings with exactly these. An empty slice clears
    /// them, which is how the screen says "no limit".
    ///
    /// Replace rather than upsert-per-window because that is what the screen
    /// holds: a set, edited as a set. Removing a window and saving is one call
    /// either way, and this way a window the user deleted cannot be left behind by
    /// a partial write.
    pub fn replace_agent_limits(&self, agent: &str, limits: &[AgentLimit]) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        // `created_at` of a window that survives keeps saying when it was first
        // set, so the row does not lose its age to an unrelated edit.
        let prior: std::collections::HashMap<String, String> = tx
            .prepare("SELECT period, created_at FROM agent_limits WHERE agent = ?1")?
            .query_map(params![agent], |r| Ok((r.get(0)?, r.get(1)?)))?
            .collect::<rusqlite::Result<_>>()?;
        tx.execute("DELETE FROM agent_limits WHERE agent = ?1", params![agent])?;
        let now = now_rfc3339();
        for l in limits {
            tx.execute(
                "INSERT INTO agent_limits (agent, period, period_limit, limit_unit, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    agent,
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

    pub fn delete_agent_limits(&self, agent: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM agent_limits WHERE agent = ?1", params![agent])?;
        Ok(n > 0)
    }

    pub fn list_custom_agents(&self) -> Result<Vec<CustomAgent>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, label, note, protocol, created_at FROM custom_agents ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(CustomAgent {
                id: row.get(0)?,
                label: row.get(1)?,
                note: row.get(2)?,
                protocol: row.get(3)?,
                created_at: row.get(4)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    pub fn get_custom_agent(&self, id: &str) -> Result<Option<CustomAgent>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let found = conn
            .query_row(
                "SELECT id, label, note, protocol, created_at FROM custom_agents WHERE id = ?1",
                params![id],
                |row| {
                    Ok(CustomAgent {
                        id: row.get(0)?,
                        label: row.get(1)?,
                        note: row.get(2)?,
                        protocol: row.get(3)?,
                        created_at: row.get(4)?,
                    })
                },
            )
            .optional()?;
        Ok(found)
    }

    pub fn insert_custom_agent(&self, a: &CustomAgent) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO custom_agents (id, label, note, protocol, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            params![a.id, a.label, a.note, a.protocol, a.created_at],
        )?;
        Ok(())
    }

    /// Rename: only the label and the note move. The id is referenced by
    /// bindings, strategies, keys and usage rows, so it is not a thing a rename
    /// touches, and the protocol travels back unchanged for the same reason the
    /// note does — this call was not about it.
    pub fn update_custom_agent_label(
        &self,
        id: &str,
        label: &str,
        note: Option<&str>,
        protocol: Option<&str>,
    ) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "UPDATE custom_agents SET label = ?2, note = ?3, protocol = ?4 WHERE id = ?1",
            params![id, label, note, protocol],
        )?;
        Ok(n > 0)
    }

    /// Delete the agent's own row. Its route (bindings + strategy) and key are
    /// the caller's to remove — `vm::remove_custom_agent` does all three in the
    /// order that leaves nothing dangling; usage and request logs are history
    /// and are never touched.
    pub fn delete_custom_agent(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM custom_agents WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }
}
