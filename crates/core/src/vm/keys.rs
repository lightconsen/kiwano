//! API key rotation: the key list, masked for display, plus add and delete.

use crate::vm::e2s;
use crate::vm::time::{rfc3339, unix_now};
use kiwanod::store::Store;
use serde::Serialize;

// ── Multi-key rotation (spec §4.1 P1: auto-rotate multiple API keys per provider) ──

#[derive(Serialize)]
pub struct ApiKeyVm {
    pub id: i64,
    /// Masked for display — never the key itself. The UI only ever needs to
    /// tell two entries apart, and every copy of the plaintext we do not hand
    /// out is one less copy sitting in a webview heap.
    pub masked: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// Display form of a key: a recognisable head and tail, never a usable secret.
///
/// The previous frontend-side masker printed the key verbatim when it was 12
/// characters or shorter, which is exactly the case where a mask matters most.
/// There is no length at which this one returns the whole key.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let n = chars.len();
    if n > 12 {
        format!(
            "{}…{}",
            chars[..6].iter().collect::<String>(),
            chars[n - 4..].iter().collect::<String>()
        )
    } else if n > 4 {
        format!("…{}", chars[n - 4..].iter().collect::<String>())
    } else {
        "•".repeat(n)
    }
}

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

    /// A masked key is a display string, not a credential — there is no input
    /// length at which it hands the key back.
    #[test]
    fn mask_key_never_returns_the_whole_key() {
        assert_eq!(mask_key("sk-live-abcdefghijklmnop"), "sk-liv…mnop");
        // The frontend masker this replaced printed keys of 12 characters or
        // fewer verbatim — the case where masking matters most.
        assert_eq!(mask_key("123456789012"), "…9012");
        assert_eq!(mask_key("sk-short"), "…hort");
        assert_eq!(mask_key("abc"), "•••");
        assert_eq!(mask_key(""), "");
    }

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
