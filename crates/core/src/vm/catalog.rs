//! The hub catalog: what stays on the client.
//!
//! The parse-and-normalize rules, the types and the shelf moved to
//! `kiwanod::api::catalog` when the daemon started serving them
//! (`migrate.local.md` §10.12) — the payload they read is the daemon's table
//! now. What is left here is the client's own half: the sync report, and
//! `stored_key_for`, which answers a question about *this* machine's saved
//! configuration.
//!
//! Paths are unchanged: the types are re-exported, so `crate::vm::CatalogListVm`
//! and friends resolve exactly as before.

use kiwanod::api::catalog as daemon;
use kiwanod::store::Store;

// Re-exported, not declared here: the daemon serves these, and the client's call
// sites name them at these paths.
pub(crate) use kiwanod::api::catalog::{catalog_id_for, catalog_snapshot};
pub use kiwanod::api::catalog::{
    default_catalog_currency, CatalogBilling, CatalogEndpointVm, CatalogEntryVm, CatalogListVm,
    CatalogPriceRefVm,
};
// The endpoint-matching primitives `stored_key_for` is built from — the same
// rules the shelf matches by, so the two cannot drift.
pub(crate) use kiwanod::api::catalog::{endpoint_key, provider_endpoint_keys};

use crate::vm::e2s;

// The sync report moved to `kiwano-api` when the daemon took over the sync
// (`migrate.local.md` §10.14): the daemon produces it, so it is a wire type.
pub use kiwano_api::providers::SyncReportVm;

/// The credential stored for `provider_id`, but **only** when `endpoint` is one
/// of the endpoints that provider already answers on.
///
/// The add/edit form deliberately keeps the key out of the webview: it shows a
/// masked placeholder and sends a blank when the user has not typed a new one
/// (blank = keep the stored key). So a Test or a Fetch pressed from the edit
/// dialog arrives with nothing to authenticate with, and both used to fail —
/// the probe reported `auth`, and the model list refused outright — for a
/// provider whose key was sitting right there.
///
/// The endpoint has to match, and that is the point rather than a nicety: the
/// URL field is editable, so reading the stored key for whatever address is in
/// the box would turn a Test button into a way to post someone's credential to a
/// host of the typer's choosing. Matching the stored endpoints keeps the button
/// meaning "does my saved configuration still work".
pub fn stored_key_for(store: &Store, provider_id: &str, endpoint: &str) -> Option<String> {
    let p = store.get_provider(provider_id).ok().flatten()?;
    let wanted = endpoint_key(endpoint);
    let known = provider_endpoint_keys(&p);
    known.contains(&wanted).then_some(p.api_key)?
}

/// Fill in the catalog link on providers that have none.
///
/// The link is what prices a request at its own provider's rate, and it is
/// written when a provider is added from the shelf — and only then, so
/// everything hand-added, added from the CLI, imported, or present before the
/// column existed has none. With the Hub pricing per entry, an unlinked
/// provider is costed at *another* entry's rate, so the link is inferred from
/// the endpoint wherever that is unambiguous ([`catalog_id_for`]).
///
/// Only `NULL`s are considered: a link is a fact that decides a price, so it is
/// never re-derived, never overwritten, never cleared — the same rule
/// `update_provider` follows for an edit that carries no catalog id. The row's
/// own `updated_at` rides along, so a backfill does not read as a user edit.
///
/// Returns how many rows were linked: a caller with a running gateway needs to
/// know whether the daemon's in-memory copy just went stale.
pub fn link_providers(store: &Store) -> Result<usize, String> {
    let list = catalog_snapshot(store);
    // Never synced: nothing to infer from, and nothing to write.
    if list.entries.is_empty() {
        return Ok(0);
    }
    let mut linked = 0;
    for mut p in store.list_providers().map_err(e2s)? {
        if p.catalog_id.is_some() {
            continue;
        }
        let Some(id) = catalog_id_for(&list.entries, &p) else {
            continue; // a custom endpoint, or an ambiguous one
        };
        p.catalog_id = Some(id);
        store.update_provider(&p).map_err(e2s)?;
        linked += 1;
    }
    Ok(linked)
}

/// The shelf — served by the daemon (`kiwanod::api::catalog::load_catalog`), so
/// what it reads and what the daemon infers a link from cannot disagree.
pub fn load_catalog(store: &Store) -> CatalogListVm {
    daemon::load_catalog(store)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm::test_support::{catalog_store, provider, store, stored_catalog_id};
    use kiwanod::api::catalog::endpoint_matches;
    use kiwanod::store::Billing;

    #[test]
    fn catalog_billing_roundtrip_known_and_unknown() {
        // Known tags map onto their variants and serialize back lowercase.
        for (raw, variant) in [
            ("plan", CatalogBilling::Plan),
            ("payg", CatalogBilling::Payg),
            ("unl", CatalogBilling::Unl),
            ("both", CatalogBilling::Both),
        ] {
            assert_eq!(CatalogBilling::parse_str(raw), Some(variant.clone()));
            assert_eq!(CatalogBilling::from(raw.to_string()), variant);
            assert_eq!(variant.as_str(), raw);
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{raw}\"")
            );
        }

        // Unknown tags survive verbatim instead of being coerced to payg.
        let other = CatalogBilling::from("per-token".to_string());
        assert_eq!(other, CatalogBilling::Other("per-token".into()));
        assert_eq!(CatalogBilling::parse_str("per-token"), None);
        assert_eq!(serde_json::to_string(&other).unwrap(), "\"per-token\"");
        assert_eq!(other.as_str(), "per-token");
        let back: CatalogBilling = serde_json::from_str("\"per-token\"").unwrap();
        assert_eq!(back, other);
    }

    #[test]
    fn catalog_entry_unknown_billing_survives_json_roundtrip() {
        // A hub cache payload with a bad row must deserialize and re-serialize
        // byte-identically (the sync gate compares bytes).
        let raw = r##"{"id":"x","name":"X","logo_char":"X","logo_color":"#000",
            "tag":"official","tag_label":"Official","rating":1.0,
            "endpoint":"https://x.example","price_line":"p","billing":"per-token",
            "users":"1","blurb":"b","added":false,"models":[]}"##;
        let entry: CatalogEntryVm = serde_json::from_str(raw).unwrap();
        assert_eq!(
            entry.billing,
            CatalogBilling::Other("per-token".to_string())
        );
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["billing"], "per-token");

        let known: CatalogEntryVm =
            serde_json::from_str(&raw.replace("\"per-token\"", "\"payg\"")).unwrap();
        assert_eq!(known.billing, CatalogBilling::Payg);
        assert_eq!(
            serde_json::to_value(&known).unwrap()["billing"],
            serde_json::json!("payg")
        );
    }

    /// The boundary the matching rule turns on. A future "simplify to
    /// `starts_with`" has to fail here rather than silently link a lookalike
    /// host to a vendor's prices.
    #[test]
    fn endpoint_matches_takes_a_path_boundary() {
        // Equal keys, and either direction of the path prefix: a bare host
        // typed by the user, or a path an imported config carries.
        assert!(endpoint_matches("api.deepseek.com", "api.deepseek.com"));
        assert!(endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com/anthropic"
        ));
        assert!(endpoint_matches(
            "api.deepseek.com/anthropic",
            "api.deepseek.com"
        ));
        assert!(endpoint_matches("api.deepseek.com/v1", "api.deepseek.com"));

        // …and what must not match: a neighbour host, another port, and two
        // different sub-services under one host.
        assert!(!endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com.evil.com"
        ));
        assert!(!endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com:8443"
        ));
        assert!(!endpoint_matches(
            "api.deepseek.com/v1",
            "api.deepseek.com/anthropic"
        ));
        // An empty key would otherwise be a prefix of everything.
        assert!(!endpoint_matches("", "api.deepseek.com"));
        assert!(!endpoint_matches("api.deepseek.com", ""));
    }

    /// The backfill, on rows that predate the column: only `NULL`s are touched,
    /// an existing link is left alone down to its `updated_at` (a backfill is
    /// not a user edit), and a second run has nothing left to do.
    #[test]
    fn link_providers_fills_only_nulls() {
        let s = catalog_store();

        let mut ds = provider("ds", "DeepSeek", Billing::Metered);
        ds.base_url = "https://api.deepseek.com".into();
        let mut kfc = provider("kfc", "Kimi For Coding", Billing::Metered);
        kfc.base_url = "https://api.kimi.com/coding".into();
        kfc.catalog_id = Some("kimi-for-coding".into());
        let mut ambiguous = provider("amb", "Ambiguous", Billing::Metered);
        ambiguous.base_url = "https://api.example.com".into();
        for p in [&ds, &kfc, &ambiguous] {
            s.insert_provider(p).unwrap();
        }

        assert_eq!(link_providers(&s).unwrap(), 1);
        assert_eq!(stored_catalog_id(&s, "ds").as_deref(), Some("deepseek"));
        assert_eq!(
            stored_catalog_id(&s, "kfc").as_deref(),
            Some("kimi-for-coding")
        );
        assert_eq!(stored_catalog_id(&s, "amb"), None);
        assert_eq!(
            s.get_provider("kfc").unwrap().unwrap().updated_at,
            kfc.updated_at,
            "a link already made is a fact, not something to re-derive"
        );

        // Idempotent: the second pass has nothing to write.
        assert_eq!(link_providers(&s).unwrap(), 0);
    }

    /// Never synced: there is nothing to infer from, so nothing is written —
    /// and nothing is cleared either.
    #[test]
    fn link_providers_is_a_noop_without_a_catalog() {
        let s = store();
        let mut p = provider("ds", "DeepSeek", Billing::Metered);
        p.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&p).unwrap();

        assert_eq!(link_providers(&s).unwrap(), 0);
        assert_eq!(stored_catalog_id(&s, "ds"), None);
    }

    /// The shelf's badge asks the same question as the link — "is this entry
    /// one the user already has?" — so it answers the same way. Otherwise a
    /// linked provider would still be offered for adding a second time.
    #[test]
    fn catalog_added_matches_the_link_rule() {
        let s = catalog_store();
        let mut p = provider("ds", "DeepSeek", Billing::Metered);
        p.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&p).unwrap();

        let added: Vec<String> = load_catalog(&s)
            .entries
            .into_iter()
            .filter(|e| e.added)
            .map(|e| e.id)
            .collect();
        assert_eq!(added, ["deepseek"]);
    }

    #[test]
    fn catalog_added_derives_from_provider_endpoints() {
        let s = store(); // never synced: no cache is an empty shelf
                         // Seeded from the Hub cache, which is the only source now — this used to
                         // lean on the bundled catalog, and the two entries below are the ones it
                         // asserted against: DeepSeek on its primary, Kimi For Coding reachable on
                         // a second protocol at a different path.
        let payload = r#"{"total":2,"entries":[
            {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
             "billing":"payg","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com"}]},
            {"id":"kimi-for-coding","name":"Kimi For Coding","tag":"official","rating":4,
             "billing":"plan","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.kimi.com/coding/v1"},
                          {"protocol":"anthropic","endpoint":"https://api.kimi.com/coding"}]}
        ]}"#;
        s.save_hub_cache(payload, "2026-09-07T00:00:00Z").unwrap();

        // nothing added yet: the flags a payload may carry are ignored
        let empty = load_catalog(&s);
        assert!(!empty.entries.iter().any(|e| e.added));

        // add a provider on the merged Kimi entry's anthropic additional
        // endpoint (trailing slash variant) — the whole entry counts as added
        let mut p = provider("kfc", "Kimi For Coding", Billing::Metered);
        p.base_url = "https://api.kimi.com/coding/".into();
        s.insert_provider(&p).unwrap();

        let list = load_catalog(&s);
        let added: Vec<&str> = list
            .entries
            .iter()
            .filter(|e| e.added)
            .map(|e| e.id.as_str())
            .collect();
        // exactly the merged entry matches (via its alt endpoint); everything
        // else — including DeepSeek at a different endpoint — stays addable
        assert_eq!(added, ["kimi-for-coding"]);

        // a provider on the primary endpoint also marks the entry added.
        // Same store, not a fresh one: `added` is derived from the providers in
        // it, and a fresh store would have neither cache nor providers — an
        // empty shelf that passes for the
        // wrong reason.
        let mut d = provider("ds", "DeepSeek", Billing::Metered);
        d.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&d).unwrap();
        let list2 = load_catalog(&s);
        let added2: Vec<&str> = list2
            .entries
            .iter()
            .filter(|e| e.added)
            .map(|e| e.id.as_str())
            .collect();
        // catalog order as published: DeepSeek first
        assert_eq!(added2, ["deepseek", "kimi-for-coding"]);
    }
}
