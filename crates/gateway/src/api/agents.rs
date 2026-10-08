//! User-defined agents: create, rename, delete.
//!
//! Moved here from `kiwano_core::vm::agents` when the daemon started serving
//! them (see this module's parent). The registry tables and the protocol
//! catalogue stay in `kiwano-core` — they describe *this machine's* agents and
//! its config files, which the daemon never touches.
//!
//! # The id is the operation's identity
//!
//! An agent id is minted with six random hex characters, so nothing in the
//! request body identifies a create on its own — which is what
//! `migrate.local.md` §6.1 flagged as the one case needing more than a
//! value-based rule. The resolution is the one that section already chose for
//! providers: **the id comes from the client**, so a replayed request carries
//! the id it minted the first time and lands on the row that is already there.
//! `create_custom_agent` is the upsert that makes that true, and the key is
//! minted only when the row is.

use crate::store::{CustomAgent, Protocol, Store};
use kiwano_api::agents::CustomAgentVm;
use kiwano_api::error::ApiError;

/// One stored agent as the UI sees it, key included.
fn agent_vm(store: &Store, a: CustomAgent) -> Result<CustomAgentVm, ApiError> {
    // The key rides with the agent (minted at create, deleted at remove), so a
    // read of it is the same read the dialog's settings panel makes.
    let placeholder_key = store
        .list_placeholder_keys()
        .map_err(ApiError::failed)?
        .into_iter()
        .find(|k| k.agent == a.id)
        .map(|k| k.key);
    Ok(CustomAgentVm {
        id: a.id,
        label: a.label,
        note: a.note,
        protocol: a.protocol,
        placeholder_key,
    })
}

/// A protocol as it is stored: one of the three words, or None for "not said".
///
/// An unknown word is a refusal rather than a row — the field is a label the UI
/// renders and the CLI prints, so a typo in the database would be a word no
/// reader recognises. Empty trims to None: clearing the choice is how a user
/// says they would rather not say.
fn normalize_protocol(protocol: Option<&str>) -> Result<Option<String>, ApiError> {
    match protocol.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p) => match Protocol::parse_str(p) {
            Some(parsed) => Ok(Some(parsed.as_str().to_string())),
            None => Err(ApiError::invalid(format!(
                "unknown protocol: {p} — one of anthropic, openai, gemini"
            ))),
        },
    }
}

/// Create a user-defined agent: a row, a placeholder key, and a `single`
/// strategy — which is all an agent *is* to the gateway, whose route table is
/// built from those tables and never from the registry.
///
/// **Idempotent on the id**, which the caller mints
/// (`kiwano_api::ids::mint_agent_id`). An id that is already there *is* this
/// operation's result: the request is answered with the agent that exists, and
/// nothing is written — no second row, and no second key. That is the whole of
/// §6.1's requirement for this command, and it needs no operation-id table,
/// because the id the caller minted already is one.
///
/// A label the user repeats still gets its own agent: it is a *different*
/// intent, so it mints a different id. The suffix is what makes that possible.
pub fn create_custom_agent(
    store: &Store,
    id: &str,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, ApiError> {
    if let Some(existing) = store.get_custom_agent(id).map_err(ApiError::failed)? {
        return agent_vm(store, existing);
    }
    let label = label.trim();
    if label.is_empty() {
        return Err(ApiError::invalid("an agent needs a name"));
    }
    let protocol = normalize_protocol(protocol)?;
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    store
        .insert_custom_agent(&CustomAgent {
            id: id.to_string(),
            label: label.to_string(),
            note: note.map(str::to_string),
            protocol: protocol.clone(),
            created_at: crate::store::now_rfc3339(),
        })
        .map_err(ApiError::failed)?;
    // Its key, in the same shape a takeover mints — the gateway's attribution
    // does not care where the row came from.
    let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
    let key = format!("kw-ag-{id}-{rand}");
    store
        .upsert_placeholder_key(&key, id)
        .map_err(ApiError::failed)?;
    // A route with no strategy still routes (the engine reads `single` for a
    // missing row), but writing it here is what makes the agent's tab show the
    // strategy it actually has rather than an implicit default.
    store
        .upsert_strategy(id, crate::store::StrategyType::Single, None)
        .map_err(ApiError::failed)?;
    agent_vm(
        store,
        CustomAgent {
            id: id.to_string(),
            label: label.to_string(),
            note: note.map(str::to_string),
            protocol,
            created_at: crate::store::now_rfc3339(),
        },
    )
}

/// Rename a user-defined agent.
///
/// Only its label, note and protocol move: the id is what bindings, routes, keys
/// and usage rows point at, so a rename must not touch it — the agent keeps its
/// route and its history, and only the name its user reads changes.
///
/// Idempotent by nature: a `PUT` whose body is the resulting state, so sending
/// it twice leaves the same agent.
pub fn update_custom_agent(
    store: &Store,
    id: &str,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, ApiError> {
    let label = label.trim();
    if label.is_empty() {
        return Err(ApiError::invalid("an agent needs a name"));
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    let protocol = normalize_protocol(protocol)?;
    if !store
        .update_custom_agent_label(id, label, note, protocol.as_deref())
        .map_err(ApiError::failed)?
    {
        return Err(ApiError::not_found(format!("no such custom agent: {id}")));
    }
    // The same shape `create_custom_agent` returns, key included: the dialog
    // that shows an agent's settings reads it from here too.
    agent_vm(
        store,
        CustomAgent {
            id: id.to_string(),
            label: label.to_string(),
            note: note.map(str::to_string),
            protocol,
            created_at: String::new(),
        },
    )
}

/// Delete a user-defined agent, and everything that was only about it: its
/// bindings, its strategy, its key, its row. `usage` and `request_logs` are
/// history and stay — the same line the provider deletion draws.
///
/// A second delete is refused rather than treated as success: the caller has to
/// be able to tell "it is gone" from "it never was" (`migrate.local.md` §6.1
/// accepts that a retried delete answers differently, as long as the state
/// converges — and it does, because the first one did the work).
pub fn remove_custom_agent(store: &Store, id: &str) -> Result<(), ApiError> {
    if store
        .get_custom_agent(id)
        .map_err(ApiError::failed)?
        .is_none()
    {
        return Err(ApiError::not_found(format!("no such custom agent: {id}")));
    }
    for b in store.bindings_for_agent(id).map_err(ApiError::failed)? {
        store
            .delete_binding(id, &b.provider_id)
            .map_err(ApiError::failed)?;
    }
    store.delete_strategy(id).map_err(ApiError::failed)?;
    for k in store.list_placeholder_keys().map_err(ApiError::failed)? {
        if k.agent == id {
            store
                .delete_placeholder_key(&k.key)
                .map_err(ApiError::failed)?;
        }
    }
    store.delete_custom_agent(id).map_err(ApiError::failed)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Billing;

    fn store() -> Store {
        Store::open_in_memory().unwrap()
    }

    /// §6.1: **a replayed create mints one agent and one key.**
    ///
    /// This is the acceptance gate for the id-in-the-body decision, and it fails
    /// against an insert that does not look first — the shape this had before the
    /// daemon served it, when the only thing a second delivery could do was
    /// write a second row.
    #[test]
    fn a_replayed_create_mints_one_agent_and_one_key() {
        let store = store();
        let id = kiwano_api::ids::mint_agent_id("Long Tasks");

        let first = create_custom_agent(&store, &id, "Long Tasks", Some("night"), None).unwrap();
        let again = create_custom_agent(&store, &id, "Long Tasks", Some("night"), None).unwrap();

        assert_eq!(first.id, again.id);
        assert_eq!(
            first.placeholder_key, again.placeholder_key,
            "the replay reuses the key it minted"
        );
        assert_eq!(store.list_custom_agents().unwrap().len(), 1);
        assert_eq!(
            store.list_placeholder_keys().unwrap().len(),
            1,
            "one agent, one key"
        );
        assert!(
            store.get_strategy(&id).unwrap().is_some(),
            "and the default route was written once"
        );
    }

    /// A label the user repeats is a *different intent*, so it gets its own id
    /// and therefore its own agent — the property the suffix exists for, and the
    /// one that would break if the id were derived from the label alone.
    #[test]
    fn a_repeated_label_is_a_different_intent_and_a_different_agent() {
        let store = store();
        let a = create_custom_agent(
            &store,
            &kiwano_api::ids::mint_agent_id("Long Tasks"),
            "Long Tasks",
            None,
            None,
        )
        .unwrap();
        let b = create_custom_agent(
            &store,
            &kiwano_api::ids::mint_agent_id("Long Tasks"),
            "Long Tasks",
            None,
            None,
        )
        .unwrap();
        assert_ne!(a.id, b.id);
        assert_ne!(a.placeholder_key, b.placeholder_key);
        assert_eq!(store.list_custom_agents().unwrap().len(), 2);
        assert_eq!(store.list_placeholder_keys().unwrap().len(), 2);
    }

    /// The refusals carry the kind the status is picked from, and the sentences
    /// the UI already shows.
    #[test]
    fn the_refusals_are_kinds_and_keep_their_wording() {
        let store = store();

        let blank = create_custom_agent(
            &store,
            &kiwano_api::ids::mint_agent_id("x"),
            "   ",
            None,
            None,
        )
        .unwrap_err();
        assert_eq!(blank.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert_eq!(blank.message(), "an agent needs a name");
        assert_eq!(
            store.list_custom_agents().unwrap().len(),
            0,
            "a refusal writes nothing"
        );

        let typo = create_custom_agent(
            &store,
            &kiwano_api::ids::mint_agent_id("Typo"),
            "Typo",
            None,
            Some("opemai"),
        )
        .unwrap_err();
        assert_eq!(typo.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert_eq!(
            typo.message(),
            "unknown protocol: opemai — one of anthropic, openai, gemini"
        );

        let missing = update_custom_agent(&store, "no-such", "X", None, None).unwrap_err();
        assert_eq!(missing.kind(), kiwano_api::error::ApiErrorKind::NotFound);
        assert_eq!(missing.message(), "no such custom agent: no-such");

        let gone = remove_custom_agent(&store, "no-such").unwrap_err();
        assert_eq!(gone.kind(), kiwano_api::error::ApiErrorKind::NotFound);
    }

    /// A rename moves the label and nothing else: the id, the route and the key
    /// belong to the agent, not to the name its user gave it.
    #[test]
    fn a_rename_keeps_the_id_and_the_key() {
        let store = store();
        let id = kiwano_api::ids::mint_agent_id("Long Tasks");
        let created =
            create_custom_agent(&store, &id, "Long Tasks", Some("at night"), None).unwrap();

        let renamed =
            update_custom_agent(&store, &id, "Nightly batch", Some("moved"), Some("gemini"))
                .unwrap();
        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.label, "Nightly batch");
        assert_eq!(renamed.note.as_deref(), Some("moved"));
        assert_eq!(renamed.protocol.as_deref(), Some("gemini"));
        assert_eq!(renamed.placeholder_key, created.placeholder_key);

        // A blank note clears it rather than storing whitespace, and clearing the
        // protocol is how a user says they would rather not say.
        let cleared =
            update_custom_agent(&store, &id, "Nightly batch", Some("  "), Some("")).unwrap();
        assert_eq!(cleared.note, None);
        assert_eq!(cleared.protocol, None);

        // The row on disk agrees with what was returned.
        let stored = store.get_custom_agent(&id).unwrap().unwrap();
        assert_eq!(stored.label, "Nightly batch");
        assert_eq!(stored.protocol, None);
    }

    /// Deleting takes the route and the key, and leaves everything the user did
    /// not create here: the provider, and the usage history.
    #[test]
    fn a_delete_takes_the_route_and_the_key_and_leaves_the_history() {
        let store = store();
        store
            .insert_provider(&crate::store::Provider {
                id: "p-1".into(),
                name: "P".into(),
                catalog_id: None,
                protocol: Protocol::OpenAI,
                base_url: "http://127.0.0.1:1".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk-p".into()),
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
            })
            .unwrap();
        let id = kiwano_api::ids::mint_agent_id("Long Tasks");
        create_custom_agent(&store, &id, "Long Tasks", None, None).unwrap();
        store
            .upsert_binding(&crate::store::Binding {
                agent: id.clone(),
                provider_id: "p-1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();

        remove_custom_agent(&store, &id).unwrap();

        assert!(store.get_custom_agent(&id).unwrap().is_none());
        assert!(store.bindings_for_agent(&id).unwrap().is_empty());
        assert!(store.get_strategy(&id).unwrap().is_none());
        assert!(store
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != id));
        assert!(
            store.get_provider("p-1").unwrap().is_some(),
            "not ours to remove"
        );
    }
}
