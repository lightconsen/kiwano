//! API key rotation: the key list, masked for display, plus add and delete.
//!
//! Moved here from `kiwano_core::vm::keys` when the daemon started serving it
//! (see this module's parent for why the logic, and not just the types, had to
//! move). The bodies are unchanged except for the error type: an
//! [`ApiError`] carries the kind the server needs to pick a status, so the
//! server no longer re-queries the store to discover what went wrong.

use crate::store::{now_rfc3339, Store};
use kiwano_api::error::ApiError;
use kiwano_api::keys::{mask_key, ApiKeyVm};

// ── Multi-key rotation (spec §4.1 P1: auto-rotate multiple API keys per provider) ──

pub fn list_api_keys(store: &Store, provider_id: &str) -> Result<Vec<ApiKeyVm>, ApiError> {
    store
        .list_api_keys(provider_id)
        .map(|rows| {
            rows.into_iter()
                .map(|r| ApiKeyVm {
                    id: r.id,
                    masked: mask_key(&r.api_key),
                    label: r.label,
                    enabled: r.enabled,
                    created_at: r.created_at,
                })
                .collect()
        })
        .map_err(ApiError::failed)
}

/// Append a rotation key (providers.api_key is the primary key, always first in
/// the pool).
///
/// Idempotent on the key's value — the store decides that, so every caller
/// agrees — which is what makes a retry answer with the row that is already
/// there instead of a second one (`migrate.local.md` §6.1).
pub fn add_api_key(
    store: &Store,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
) -> Result<ApiKeyVm, ApiError> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err(ApiError::invalid("API key must not be empty"));
    }
    store
        .get_provider(provider_id)
        .map_err(ApiError::failed)?
        .ok_or_else(|| ApiError::not_found(format!("provider `{provider_id}` not found")))?;
    let label = label.map(str::trim).filter(|s| !s.is_empty());
    let id = store
        .insert_api_key(provider_id, key, label)
        .map_err(ApiError::failed)?;
    Ok(ApiKeyVm {
        id,
        masked: mask_key(key),
        label: label.map(String::from),
        enabled: true,
        created_at: now_rfc3339(),
    })
}

/// Remove a rotation key. `false` when there was no such row, which is the
/// state a retry finds and is not an error.
pub fn delete_api_key(store: &Store, id: i64) -> Result<bool, ApiError> {
    store.delete_api_key(id).map_err(ApiError::failed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::{Billing, Protocol, Provider};

    fn provider(id: &str) -> Provider {
        Provider {
            id: id.into(),
            name: id.into(),
            catalog_id: None,
            protocol: Protocol::OpenAI,
            openai_wire: crate::store::OpenAiWire::Both,
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
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    /// The kinds, which is the whole reason this function returns an
    /// [`ApiError`]: the server picks a status from the kind, so a wrong kind is
    /// a wrong status on the wire. This is the test that would fail if someone
    /// "simplified" the empty-key check into the failed branch.
    #[test]
    fn the_failure_kinds_are_the_ones_a_status_can_be_picked_from() {
        let store = Store::open_in_memory().unwrap();

        let empty = add_api_key(&store, "p-1", "   ", None).unwrap_err();
        assert_eq!(empty.kind(), kiwano_api::error::ApiErrorKind::Invalid);
        assert_eq!(empty.message(), "API key must not be empty");

        let unknown = add_api_key(&store, "p-1", "sk-x", None).unwrap_err();
        assert_eq!(unknown.kind(), kiwano_api::error::ApiErrorKind::NotFound);
        assert_eq!(unknown.message(), "provider `p-1` not found");
    }

    /// The mask is applied where the plaintext is held, and the store's
    /// idempotency holds through this entry point too — both properties used to
    /// be asserted only through `vm::`, which is now the same code.
    #[test]
    fn a_key_is_masked_on_the_way_out_and_added_once() {
        let store = Store::open_in_memory().unwrap();
        store.insert_provider(&provider("p-1")).unwrap();

        let added = add_api_key(&store, "p-1", "sk-live-abcdefghijklmnop", Some("backup")).unwrap();
        assert_eq!(added.masked, "sk-liv…mnop");
        assert_eq!(added.label.as_deref(), Some("backup"));

        add_api_key(&store, "p-1", "sk-live-abcdefghijklmnop", Some("backup")).unwrap();
        assert_eq!(list_api_keys(&store, "p-1").unwrap().len(), 1);

        assert!(delete_api_key(&store, added.id).unwrap());
        assert!(
            !delete_api_key(&store, added.id).unwrap(),
            "gone is not an error"
        );
    }
}
