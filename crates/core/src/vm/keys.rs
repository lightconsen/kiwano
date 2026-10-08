//! API key rotation: the key list, masked for display, plus add and delete.

use crate::vm::e2s;
use crate::vm::time::{rfc3339, unix_now};
use kiwanod::store::Store;

// ── Multi-key rotation (spec §4.1 P1: auto-rotate multiple API keys per provider) ──

// Re-exported, not declared here: the wire types moved to `kiwano-api`, which
// the daemon can depend on and `kiwano-core` cannot be reached from
// (`migrate.local.md` §10.5). The path `crate::vm::keys::ApiKeyVm` is unchanged
// on purpose.
pub use kiwano_api::keys::ApiKeyVm;
// The mask moved with the type it shapes: the daemon has to produce the same
// masked string the app does, and two copies of a rule like this is how the two
// would come to disagree (`migrate.local.md` §10.6).
pub use kiwano_api::keys::mask_key;

pub fn list_api_keys(store: &Store, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
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
        .map_err(e2s)
}

/// Append a rotation key (providers.api_key is the primary key, always first in the pool).
pub fn add_api_key(
    store: &Store,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
) -> Result<ApiKeyVm, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key must not be empty".into());
    }
    store
        .get_provider(provider_id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider `{provider_id}` not found"))?;
    let id = store
        .insert_api_key(
            provider_id,
            key,
            label.map(str::trim).filter(|s| !s.is_empty()),
        )
        .map_err(e2s)?;
    Ok(ApiKeyVm {
        id,
        masked: mask_key(key),
        label: label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        enabled: true,
        created_at: rfc3339(unix_now()),
    })
}

pub fn delete_api_key(store: &Store, id: i64) -> Result<bool, String> {
    store.delete_api_key(id).map_err(e2s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{provider, store};
    use kiwanod::store::Billing;

    /// The provider's rotating keys travel to the webview for the edit form.
    /// They used to arrive in the clear; assert the plaintext is gone from the
    /// serialized VM, since that is the payload the IPC layer actually sends.
    #[test]
    fn api_keys_do_not_reach_the_frontend_in_the_clear() {
        let s = store();
        s.insert_provider(&provider("p1", "P", Billing::Metered))
            .unwrap();
        let key = "sk-live-abcdefghijklmnop";
        add_api_key(&s, "p1", key, Some("backup")).unwrap();

        let keys = list_api_keys(&s, "p1").unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].masked, "sk-liv…mnop");
        assert_eq!(keys[0].label.as_deref(), Some("backup"));

        let json = serde_json::to_string(&keys).unwrap();
        assert!(!json.contains(key), "plaintext key in VM payload: {json}");

        // add_api_key echoes the new row back to the UI; it must mask too.
        let added = add_api_key(&s, "p1", key, None).unwrap();
        assert!(!serde_json::to_string(&added).unwrap().contains(key));
    }
}
