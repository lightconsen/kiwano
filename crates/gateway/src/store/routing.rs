//! Per-agent routing state: the strategy row and the ordered binding list
//! the strategy layer reads (tech.md §4.7).

use crate::error::Result;
use crate::store::types::{Binding, Strategy, StrategyType};
use crate::store::Store;
use rusqlite::{params, OptionalExtension};

impl Store {
    // ---- strategies & bindings (tech.md §4.7) ---------------------------

    pub fn upsert_strategy(
        &self,
        agent: &str,
        kind: StrategyType,
        config: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO agent_strategies (agent, type, config) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent) DO UPDATE SET type = ?2, config = ?3",
            params![agent, kind.as_str(), config],
        )?;
        Ok(())
    }

    pub fn get_strategy(&self, agent: &str) -> Result<Option<Strategy>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT agent, type, config FROM agent_strategies WHERE agent = ?1")?;
        let s = stmt
            .query_row(params![agent], |row| {
                let type_str: String = row.get(1)?;
                Ok(Strategy {
                    agent: row.get(0)?,
                    kind: StrategyType::parse_str(&type_str).unwrap_or(StrategyType::Single),
                    config: row.get(2)?,
                })
            })
            .optional()?;
        Ok(s)
    }

    /// Drop an agent's strategy row. Read back as the `single` default, which is
    /// what an agent with no route routes by anyway.
    pub fn delete_strategy(&self, agent: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "DELETE FROM agent_strategies WHERE agent = ?1",
            params![agent],
        )?;
        Ok(n > 0)
    }

    pub fn upsert_binding(&self, b: &Binding) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO agent_bindings (agent, provider_id, priority, weight,
                                         win_start, win_end, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(agent, provider_id) DO UPDATE SET
                priority = ?3, weight = ?4, win_start = ?5, win_end = ?6, enabled = ?7",
            params![
                b.agent,
                b.provider_id,
                b.priority,
                b.weight,
                b.win_start,
                b.win_end,
                b.enabled as i64,
            ],
        )?;
        Ok(())
    }

    /// Ordered candidate list for an agent (priority asc, enabled first).
    pub fn bindings_for_agent(&self, agent: &str) -> Result<Vec<Binding>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT agent, provider_id, priority, weight, win_start, win_end, enabled
             FROM agent_bindings WHERE agent = ?1
             ORDER BY enabled DESC, priority ASC, provider_id ASC",
        )?;
        let rows = stmt.query_map(params![agent], binding_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// The provider id chosen under the `single` strategy (primary binding).
    pub fn primary_provider_id(&self, agent: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let id = conn
            .query_row(
                "SELECT provider_id FROM agent_bindings
                 WHERE agent = ?1 AND enabled = 1
                 ORDER BY priority ASC, provider_id ASC LIMIT 1",
                params![agent],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn delete_binding(&self, agent: &str, provider_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "DELETE FROM agent_bindings WHERE agent = ?1 AND provider_id = ?2",
            params![agent, provider_id],
        )?;
        Ok(n > 0)
    }

    /// Distinct agents that have at least one binding row.
    pub fn bound_agents(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare("SELECT DISTINCT agent FROM agent_bindings ORDER BY agent")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }
}

fn binding_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
    Ok(Binding {
        agent: row.get(0)?,
        provider_id: row.get(1)?,
        priority: row.get(2)?,
        weight: row.get(3)?,
        win_start: row.get(4)?,
        win_end: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::test_support::{sample_provider, temp_store};
    use crate::store::types::Protocol;

    #[test]
    fn strategy_and_binding_selection() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("a", Protocol::Anthropic))
            .unwrap();
        store
            .insert_provider(&sample_provider("b", Protocol::OpenAI))
            .unwrap();

        store
            .upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        let s = store.get_strategy("claude").unwrap().unwrap();
        assert_eq!(s.kind, StrategyType::Single);
        assert_eq!(s.agent, "claude");

        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "b".into(),
                priority: 1,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "a".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();

        // Single strategy picks the priority-0 primary.
        assert_eq!(store.primary_provider_id("claude").unwrap().unwrap(), "a");

        let bindings = store.bindings_for_agent("claude").unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].provider_id, "a");

        // Disabling the primary demotes it; next candidate takes over.
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "a".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: false,
            })
            .unwrap();
        assert_eq!(store.primary_provider_id("claude").unwrap().unwrap(), "b");

        assert!(store.delete_binding("claude", "a").unwrap());
        assert!(!store.delete_binding("claude", "a").unwrap());

        // Unknown agent has no binding.
        assert!(store.primary_provider_id("codex").unwrap().is_none());
    }
}
