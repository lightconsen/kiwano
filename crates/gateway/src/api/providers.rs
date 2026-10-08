//! Provider writes the daemon owns: switching one off, and deleting one.
//!
//! Moved here from `kiwano_core::vm::provider_edit` when the daemon started
//! serving them (see this module's parent).
//!
//! **`add_provider` and `update_provider` are not here, and the reason is worth
//! writing down.** The plan put `add_provider` in batch 1 as pure state, but it
//! reads the app-scoped half of the database twice — `catalog_snapshot(aux)` to
//! infer a provider's Hub entry, and `health_vm(aux, …)` for the verdict it
//! returns (`migrate.local.md` §10.9). That half is batch 2's subject (the Hub
//! cache and the probes), so those two commands move with it rather than
//! dragging it in here. What is left is genuinely state-only.

use crate::store::{Binding, Store};
use kiwano_api::error::ApiError;

/// Switch a provider on or off.
///
/// The way to stop using one for a while. The old `enable_provider` did
/// something else with the word: it promoted a provider to primary everywhere it
/// was bound. `providers use --agent` is that operation, one agent at a time, and
/// it keeps its name.
///
/// Idempotent by nature: the body is the state the caller wants, so setting it
/// twice leaves the same row — and the early return means a no-op write does not
/// even touch `updated_at`.
pub fn set_provider_enabled(store: &Store, id: &str, enabled: bool) -> Result<(), ApiError> {
    let mut p = store
        .get_provider(id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("provider not found: {id}")))?;
    if p.enabled == enabled {
        return Ok(());
    }
    p.enabled = enabled;
    p.updated_at = crate::store::now_rfc3339();
    store.update_provider(&p).map_err(ApiError::failed)
}

/// Delete a provider. If it is some Agent's primary, the next-best candidate in
/// that Agent's list is promoted automatically.
///
/// `false` when there was no such row, which is what a retry finds and is not an
/// error — the state converged with the first call.
pub fn delete_provider(store: &Store, id: &str) -> Result<bool, ApiError> {
    let mut affected: Vec<String> = Vec::new();
    for a in store.bound_agents().map_err(ApiError::failed)? {
        if store
            .primary_provider_id(&a)
            .map_err(ApiError::failed)?
            .as_deref()
            == Some(id)
        {
            affected.push(a);
        }
    }
    let deleted = store.delete_provider(id).map_err(ApiError::failed)?;
    if !deleted {
        return Ok(false);
    }
    // The promotion is the point of the loop above: a route whose primary is
    // gone must not be left pointing at a row that no longer exists.
    for agent in affected {
        let remaining = store.bindings_for_agent(&agent).map_err(ApiError::failed)?;
        if let Some(next) = remaining.first() {
            let next_id = next.provider_id.clone();
            store
                .upsert_binding(&Binding {
                    agent: agent.clone(),
                    provider_id: next_id.clone(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(ApiError::failed)?;
            for (i, b) in remaining
                .iter()
                .filter(|b| b.provider_id != next_id)
                .enumerate()
            {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: b.provider_id.clone(),
                        priority: i as i64 + 1,
                        weight: b.weight,
                        win_start: b.win_start.clone(),
                        win_end: b.win_end.clone(),
                        enabled: b.enabled,
                    })
                    .map_err(ApiError::failed)?;
            }
        }
    }
    Ok(true)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Billing, Protocol, Provider, StrategyType};

    fn provider(id: &str) -> Provider {
        Provider {
            id: id.into(),
            name: id.into(),
            catalog_id: None,
            protocol: Protocol::OpenAI,
            base_url: "http://127.0.0.1:1".into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some(format!("sk-{id}")),
            model_default: None,
            billing: Billing::Metered,
            period_limit: None,
            limit_unit: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            prices: None,
            reset_period: None,
            enabled: true,
            created_at: crate::store::now_rfc3339(),
            updated_at: crate::store::now_rfc3339(),
        }
    }

    fn bind(store: &Store, agent: &str, provider_id: &str, priority: i64) {
        store
            .upsert_binding(&Binding {
                agent: agent.into(),
                provider_id: provider_id.into(),
                priority,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
    }

    /// Off and back on, and the no-op case: a provider already in the state the
    /// caller asked for is not rewritten, so its `updated_at` does not move.
    #[test]
    fn enabling_is_a_state_the_caller_sets_not_a_toggle() {
        let store = Store::open_in_memory().unwrap();
        store.insert_provider(&provider("p-1")).unwrap();

        set_provider_enabled(&store, "p-1", false).unwrap();
        let off = store.get_provider("p-1").unwrap().unwrap();
        assert!(!off.enabled);

        set_provider_enabled(&store, "p-1", false).unwrap();
        assert_eq!(
            store.get_provider("p-1").unwrap().unwrap().updated_at,
            off.updated_at,
            "a no-op write does not touch the row"
        );

        set_provider_enabled(&store, "p-1", true).unwrap();
        assert!(store.get_provider("p-1").unwrap().unwrap().enabled);

        let missing = set_provider_enabled(&store, "ghost", true).unwrap_err();
        assert_eq!(missing.kind(), kiwano_api::error::ApiErrorKind::NotFound);
        assert_eq!(missing.message(), "provider not found: ghost");
    }

    /// Deleting the primary promotes the next candidate — the whole reason the
    /// delete looks at bindings first. A route left pointing at a deleted row
    /// would fail every request until someone noticed.
    #[test]
    fn deleting_a_primary_promotes_the_next_candidate() {
        let store = Store::open_in_memory().unwrap();
        for id in ["p-1", "p-2", "p-3"] {
            store.insert_provider(&provider(id)).unwrap();
        }
        bind(&store, "claude", "p-1", 0);
        bind(&store, "claude", "p-2", 1);
        bind(&store, "claude", "p-3", 2);
        store
            .upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();

        assert!(delete_provider(&store, "p-1").unwrap());

        let after = store.bindings_for_agent("claude").unwrap();
        assert_eq!(
            after
                .iter()
                .map(|b| (b.provider_id.as_str(), b.priority))
                .collect::<Vec<_>>(),
            vec![("p-2", 0), ("p-3", 1)],
            "the queue closes up behind the deleted primary"
        );
        assert_eq!(
            store.primary_provider_id("claude").unwrap().as_deref(),
            Some("p-2")
        );

        // Deleting a provider nobody binds leaves the route alone.
        assert!(delete_provider(&store, "p-3").is_ok());
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);

        // And a second delete of the same id is `false`, not an error.
        assert!(!delete_provider(&store, "p-1").unwrap());
    }
}
