//! API key rotation: the key list, masked for display, plus add and delete.
//!
//! The bodies live in `kiwanod::api::keys` now: the daemon serves this resource,
//! and `kiwano-core` depends on `kiwanod` rather than the other way round, so a
//! copy here would be the second one (`migrate.local.md` §10.8 explains the
//! finding; `crates/gateway/src/api/mod.rs` explains the rule).
//!
//! What stays is the *client-facing* signature. These functions return
//! `Result<_, String>` because that is what the app's IPC and the CLI take, and
//! because the strings are pinned by the contract fixtures — the daemon's
//! [`ApiError`] carries the same sentence plus a kind, and this is where the two
//! part company. Paths and signatures are unchanged, so no call site moved.
//!
//! [`ApiError`]: kiwano_api::error::ApiError

use kiwanod::api::keys as daemon;
use kiwanod::store::Store;

// Re-exported, not declared here: the wire types moved to `kiwano-api`, which
// the daemon can depend on and `kiwano-core` cannot be reached from
// (`migrate.local.md` §10.5). The path `crate::vm::keys::ApiKeyVm` is unchanged
// on purpose — and so is `mask_key`'s: the daemon has to produce the same
// masked string the app does, and two copies of a rule like this is how the two
// would come to disagree (`migrate.local.md` §10.6).
pub use kiwano_api::keys::{mask_key, ApiKeyVm};

// ── Multi-key rotation (spec §4.1 P1: auto-rotate multiple API keys per provider) ──

pub fn list_api_keys(store: &Store, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
    daemon::list_api_keys(store, provider_id).map_err(|e| e.to_string())
}

/// Append a rotation key (providers.api_key is the primary key, always first in the pool).
pub fn add_api_key(
    store: &Store,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
) -> Result<ApiKeyVm, String> {
    daemon::add_api_key(store, provider_id, api_key, label).map_err(|e| e.to_string())
}

pub fn delete_api_key(store: &Store, id: i64) -> Result<bool, String> {
    daemon::delete_api_key(store, id).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{provider, store};
    use kiwanod::store::Billing;

    /// The provider's rotating keys travel to the webview for the edit form.
    /// They used to arrive in the clear; assert the plaintext is gone from the
    /// serialized VM, since that is the payload the IPC layer actually sends.
    ///
    /// Kept on this side on purpose: it is the *app-facing* promise (the string
    /// error, the masked payload the webview receives), and it now runs against
    /// the daemon's implementation through the wrapper above.
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

        // And the string error the UI shows is unchanged by the move — the
        // contract fixtures pin these two exactly.
        assert_eq!(
            add_api_key(&s, "p1", "   ", None).unwrap_err(),
            "API key must not be empty"
        );
    }
}
