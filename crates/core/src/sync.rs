//! Hub catalog sync — served by the daemon now.
//!
//! The protocol, the gate and the caches moved to `kiwanod::api::sync` when the
//! daemon took over the sync (`migrate.local.md` §10.14): it is the writer of
//! the `hub_cache` and `hub_models_cache` tables, which are the daemon's.
//!
//! What stays here is the **client's half of the same behaviour**, and the tests
//! that pin it: the shelf the app shows, the footer's "synced today" badge, and
//! the price mirror's effect on the dashboard. They run against the daemon's
//! implementation through these re-exports, which is why they did not move with
//! the code.

// The pieces the client names, re-exported so `crate::sync::X` still resolves.
#[cfg(test)]
use kiwanod::api::sync as daemon;
pub use kiwanod::api::sync::{HubManifest, DEFAULT_HUB_URL};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::vm;
    use daemon::{
        apply_hub_documents, hub_asset_url, parse_catalog, parse_manifest, pricing_doc_error,
        record_catalog_sha, sha256_hex, sync_action, SyncAction, HUB_CATALOG_SHA_KEY,
    };
    use kiwano_adapters::model_pricing::ModelsDoc;

    #[test]
    fn catalog_roundtrip_and_reject() {
        let entry = serde_json::json!({
            "id": "deepseek", "name": "DeepSeek", "logo_char": "D",
            "logo_color": "#4D6BFE", "logo_border": false,
            "tag": "official", "tag_label": "Official",
            "rating": 4.8, "endpoint": "https://api.deepseek.com",
            "price_line": "¥1/2 per million", "billing": "per-token",
            "users": "12k", "blurb": "great value", "added": false,
            "models": ["deepseek-chat"]
        });
        // A vendor that charges two ways at one address: a tag this build knows,
        // which it keeps and hands to the add dialog to settle.
        let both = serde_json::json!({
            "id": "anthropic", "name": "Anthropic", "tag": "official",
            "rating": 4.9, "billing": "both",
            "endpoints": [{ "protocol": "anthropic", "endpoint": "https://api.anthropic.com" }]
        });
        let raw = serde_json::json!({ "total": 2, "entries": [entry, both] }).to_string();
        let list = parse_catalog(&raw).unwrap();
        assert_eq!(list.total, 2);
        assert_eq!(list.entries[0].name, "DeepSeek");
        assert_eq!(list.entries[1].billing, vm::CatalogBilling::Both);
        // An unrecognized billing tag is isolated, not rejected and not
        // coerced: one bad Hub row must not cost the user the whole catalog.
        assert_eq!(
            list.entries[0].billing,
            vm::CatalogBilling::Other("per-token".to_string())
        );
        // …and it round-trips verbatim, so the cache stays byte-stable.
        let reserialized = serde_json::to_string(&list).unwrap();
        let again = parse_catalog(&reserialized).unwrap();
        for i in 0..2 {
            assert_eq!(again.entries[i].billing, list.entries[i].billing);
        }

        assert!(parse_catalog("{not json").is_err());
        assert!(parse_catalog(r#"{"total":1,"entries":[{"id":"x"}]}"#).is_err());
    }

    /// The shelf is the Hub cache and nothing else.
    ///
    /// This used to assert a bundled fallback of 80+ entries. There is no
    /// bundled catalog any more — it was a build-time copy of another repo and
    /// had gone a whole schema migration stale — so "never synced" is now an
    /// empty shelf, which is a state the UI has to say out loud rather than
    /// dress up as "no matches".
    #[test]
    fn the_shelf_is_the_cache_and_nothing_else() {
        // The cache lives in the store now — it is the daemon's table
        // (`store::hub`), and the client reads it through the same accessor.
        let store = kiwanod::store::Store::open_in_memory().unwrap();

        // Never synced: empty, not stale.
        let never = vm::load_catalog(&store);
        assert_eq!(never.total, 0);
        assert!(never.entries.is_empty());

        // A payload carrying only the four required fields — the Hub publishes
        // the rest, and the app derives what it can. This is also the shape
        // tolerance the cache relies on: everything else is `serde(default)`.
        let payload = r#"{"total":1,"entries":[{
            "id": "x",
            "name": "X",
            "tag": "official",
            "rating": 4.5,
            "billing": "payg",
            "endpoints": [{"protocol": "openai", "endpoint": "https://x.example.com"}]
        }]}"#;
        // Parse it here first: a fixture that does not deserialize would make
        // every assertion below pass for the wrong reason (an empty shelf).
        serde_json::from_str::<vm::CatalogListVm>(payload)
            .unwrap_or_else(|e| panic!("the fixture must parse: {e}\n{payload}"));
        store
            .save_hub_cache(payload, "2026-09-07T00:00:00Z")
            .unwrap();

        let cached = vm::load_catalog(&store);
        assert_eq!(cached.total, 1);
        assert_eq!(cached.entries[0].name, "X");
        // Normalization still runs on the way in: the primary is hoisted out of
        // the endpoint list and a palette colour is derived.
        assert_eq!(cached.entries[0].protocol, "openai");
        assert_eq!(cached.entries[0].endpoint, "https://x.example.com");
        assert!(!cached.entries[0].logo_color.is_empty());

        // A cache we cannot parse is empty too — it used to fall back to the
        // bundled copy, which answered a broken cache with stale data.
        store
            .save_hub_cache("{not json", "2026-09-07T00:00:00Z")
            .unwrap();
        let broken = vm::load_catalog(&store);
        assert_eq!(broken.total, 0);
        assert!(broken.entries.is_empty());
    }

    /// `plan_query` has to survive the sync's own re-serialization.
    ///
    /// `sync_catalog` parses the Hub's payload into the view model and then
    /// serializes *that* into `hub_cache` — it caches the parsed shape, not the
    /// bytes it downloaded. So a field the view model does not name is gone by
    /// the end of the first sync, and stays gone, because the cache is the only
    /// thing the shelf reads. Nothing fails and nothing logs; the field simply
    /// never arrives. That is why this is pinned rather than left to serde's
    /// `default`: the failure mode is silence, and the fix is one line that a
    /// later cleanup could as easily delete.
    #[test]
    fn a_catalog_entrys_plan_query_survives_the_cache_roundtrip() {
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        let body = r#"{"total":2,"entries":[
            {"id": "kimi-for-coding", "name": "Kimi for Coding", "tag": "official",
             "rating": 4.5, "billing": "plan", "plan_query": {"template": "kimi"},
             "endpoints": [{"protocol": "openai", "endpoint": "https://api.example.com"}]},
            {"id": "deepseek", "name": "DeepSeek", "tag": "official",
             "rating": 4.8, "billing": "plan",
             "endpoints": [{"protocol": "openai", "endpoint": "https://api.deepseek.com"}]}
        ]}"#;

        // The exact three steps the sync takes: parse, re-serialize, cache.
        let list = parse_catalog(body).expect("the fixture must parse");
        let cached_body = serde_json::to_string(&list).unwrap();
        store
            .save_hub_cache(&cached_body, "2026-09-14T00:00:00Z")
            .unwrap();

        let cached = vm::load_catalog(&store);
        assert_eq!(cached.total, 2);
        assert_eq!(
            cached.entries[0]
                .plan_query
                .as_ref()
                .and_then(|q| q.get("template"))
                .and_then(|t| t.as_str()),
            Some("kimi"),
            "the template the add modal gates its ceiling fields on"
        );
        // The other fourteen plan providers publish none, and the modal reads
        // that absence as "do not offer a ceiling" rather than "no template
        // chosen yet" — so a missing field is a real answer, not a gap.
        assert!(cached.entries[1].plan_query.is_none());
    }

    #[test]
    fn footer_hub_synced_tracks_today() {
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        // never synced → false
        assert!(!vm::build_footer_stats(&store, "v0.0.0").unwrap().hub_synced);
        // just synced (timestamp uses the same now as production) → true
        store
            .save_hub_cache("{}", &kiwanod::store::now_rfc3339())
            .unwrap();
        assert!(vm::build_footer_stats(&store, "v0.0.0").unwrap().hub_synced);
    }

    // ── conditional sync ────────────────────────────────────────────────

    /// A parseable catalog body (pretty or compact) for the gate tests. The
    /// billing tag is deliberately unrecognized: the conditional-sync gate
    /// must tolerate it (it is preserved, not rejected).
    fn catalog_body(pretty: bool) -> String {
        let entry = serde_json::json!({
            "id": "deepseek", "name": "DeepSeek", "logo_char": "D",
            "logo_color": "#4D6BFE", "logo_border": false,
            "tag": "official", "tag_label": "Official",
            "rating": 4.8, "endpoint": "https://api.deepseek.com",
            "price_line": "¥1/2 per million", "billing": "per-token",
            "users": "12k", "blurb": "great value", "added": false,
            "models": ["deepseek-chat"]
        });
        let doc = serde_json::json!({ "total": 1, "entries": [entry] });
        if pretty {
            serde_json::to_string_pretty(&doc).unwrap()
        } else {
            doc.to_string()
        }
    }

    #[test]
    fn hub_asset_url_derivation() {
        let base = "https://hub.kiwano.cc";
        assert_eq!(
            hub_asset_url(&format!("{base}/catalog.json"), "manifest.json").unwrap(),
            format!("{base}/manifest.json")
        );
        // Directory-style endpoint (trailing slash) keeps the directory.
        assert_eq!(
            hub_asset_url(&format!("{base}/v1/"), "manifest.json").unwrap(),
            format!("{base}/v1/manifest.json")
        );
        // Nested path replaces only the filename.
        assert_eq!(
            hub_asset_url(&format!("{base}/v1/catalog.json"), "manifest.json").unwrap(),
            format!("{base}/v1/manifest.json")
        );
        assert_eq!(
            hub_asset_url(&format!("{base}/"), "manifest.json").unwrap(),
            format!("{base}/manifest.json")
        );
        // Query and fragment belong to the artifact, not the sibling path.
        assert_eq!(
            hub_asset_url(&format!("{base}/catalog.json?v=2#frag"), "manifest.json").unwrap(),
            format!("{base}/manifest.json")
        );
        // Surrounding whitespace in a hand-edited setting is tolerated.
        assert_eq!(
            hub_asset_url(&format!("  {base}/catalog.json  "), "manifest.json").unwrap(),
            format!("{base}/manifest.json")
        );

        assert!(hub_asset_url("", "manifest.json").is_err());
        assert!(hub_asset_url("catalog.json", "manifest.json").is_err());
        assert!(hub_asset_url("ftp://hub.kiwano.cc/catalog.json", "manifest.json").is_err());
    }

    #[test]
    fn manifest_wire_parse() {
        let sha = "c1c38966aadb78e2aca942cac46db3e5b3436233524a43e62507cd027c766a55";
        // The shape generate.mjs actually publishes.
        let full = format!(
            r#"{{"generated_at":"2026-09-10T11:38:37.353Z",
                 "catalog":{{"count":82,"sha256":"{sha}"}},
                 "models":{{"version":1,"sha256":"{sha}"}}}}"#
        );
        let m = parse_manifest(&full);
        assert_eq!(m.catalog_sha.as_deref(), Some(sha));
        assert_eq!(m.models_version, Some(1));
        assert_eq!(m.models_sha.as_deref(), Some(sha));

        // Uppercase hex is normalized, not rejected.
        let upper = format!(r#"{{"catalog":{{"sha256":"{}"}}}}"#, sha.to_uppercase());
        assert_eq!(parse_manifest(&upper).catalog_sha.as_deref(), Some(sha));

        // Everything else degrades to "no gate", never to an error.
        for bad in [
            "{}",
            r#"{"catalog":{}}"#,
            "not json",
            r#"{"catalog":{"sha256":"abc"}}"#,
            &format!(r#"{{"catalog":{{"sha256":"{}"}}}}"#, &sha[..63]),
            &format!(r#"{{"catalog":{{"sha256":"{sha}0"}}}}"#),
            r#"{"catalog":{"sha256":123}}"#,
        ] {
            assert_eq!(
                parse_manifest(bad),
                HubManifest::default(),
                "should degrade to no-gate: {bad}"
            );
        }
    }

    #[test]
    fn sync_action_matrix() {
        let sha = "a".repeat(64);
        let other = "b".repeat(64);
        assert_eq!(
            sync_action(Some(&sha), Some(&sha), Some("cached")),
            SyncAction::Skip
        );
        // Any missing half falls through to the full fetch.
        assert_eq!(
            sync_action(None, Some(&sha), Some("cached")),
            SyncAction::Fetch
        );
        assert_eq!(
            sync_action(Some(&sha), None, Some("cached")),
            SyncAction::Fetch
        );
        assert_eq!(sync_action(Some(&sha), Some(&sha), None), SyncAction::Fetch);
        // A changed remote means fetch.
        assert_eq!(
            sync_action(Some(&other), Some(&sha), Some("cached")),
            SyncAction::Fetch
        );
    }

    /// The step the Sync button used to skip. A sync only rewrites cache rows;
    /// what gets billed is the mirror, so a sync that cached a new price
    /// document and stopped there left the gateway on the table it had loaded at
    /// launch — and said nothing.
    ///
    /// Both halves of the DB are opened on one file here because that is what
    /// the app does: the store's migrations own `model_pricing`, the aux schema
    /// owns `hub_models_cache`, and the seed writes through one while reading
    /// the other.
    #[test]
    fn applying_cached_documents_fills_the_price_mirror_once() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = kiwanod::store::Store::open(&path).unwrap();
        let doc = serde_json::json!({
            "version": 7,
            "exchange_rates": { "USD": 1.0, "CNY": 7.1 },
            "models": [{
                "model_id": "gpt-5.2", "display_name": "GPT-5.2",
                "input": "1.75", "output": "14",
                "cache_read": "0.175", "cache_creation": "0", "currency": "USD"
            }]
        })
        .to_string();
        store
            .save_hub_models_cache(7, &doc, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        assert!(store.load_model_pricing().unwrap().is_empty());

        assert!(apply_hub_documents(&store), "the first apply writes");
        let rows = store.load_model_pricing().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].model_id, "gpt-5.2");

        // Idempotent: the gate is the version plus the content digest, so a
        // sync that brought nothing new does not rewrite the table or reload the
        // daemon.
        assert!(!apply_hub_documents(&store), "a repeat writes nothing");
    }

    #[test]
    fn sha256_hex_matches_node() {
        // Pinned literal from `printf abc | shasum -a 256`: this asserts the
        // *encoding* (lowercase hex, no `0x`, no truncation) as well as the
        // digest, since the manifest is written by Node's digest("hex").
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
    }

    /// Regression guard for the easiest way to silently defeat the gate: the
    /// cached payload is a re-serialization, and hashing that instead of the
    /// fetched bytes would never match the manifest.
    #[test]
    fn gate_hashes_raw_bytes_not_normalized() {
        let pretty = catalog_body(true);
        let reserialized = serde_json::to_string(&parse_catalog(&pretty).unwrap()).unwrap();
        assert_ne!(pretty, reserialized, "fixture must differ byte-wise");
        assert_ne!(
            sha256_hex(pretty.as_bytes()),
            sha256_hex(reserialized.as_bytes())
        );
    }

    #[test]
    fn skip_refreshes_synced_at_only() {
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        let payload = catalog_body(false);
        store
            .save_hub_cache(&payload, "2020-01-01T00:00:00Z")
            .unwrap();

        assert!(!vm::build_footer_stats(&store, "v0.0.0").unwrap().hub_synced);
        let now = kiwanod::store::now_rfc3339();
        assert!(store.touch_hub_synced_at(&now).unwrap());

        let (payload_after, synced_after) = store.hub_cache().unwrap();
        assert_eq!(payload_after, payload, "payload must stay byte-identical");
        assert_eq!(synced_after, now);
        // The point of refreshing: the footer badge stays truthful.
        assert!(vm::build_footer_stats(&store, "v0.0.0").unwrap().hub_synced);
    }

    #[test]
    fn touch_hub_synced_at_without_row() {
        // Fresh install: nothing to touch, and no panic.
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        assert!(!store
            .touch_hub_synced_at(&kiwanod::store::now_rfc3339())
            .unwrap());
    }

    // ── pricing cache / validation ──────────────────────────────────────

    #[test]
    fn hub_models_cache_roundtrip() {
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        assert_eq!(store.hub_models_cache(), None);
        store
            .save_hub_models_cache(
                7,
                "{\"version\":7}",
                &"a".repeat(64),
                "2026-01-01T00:00:00Z",
            )
            .unwrap();
        let (version, payload, sha, synced_at) = store.hub_models_cache().unwrap();
        assert_eq!(version, 7);
        assert_eq!(payload, "{\"version\":7}");
        assert_eq!(sha, "a".repeat(64));
        assert_eq!(synced_at, "2026-01-01T00:00:00Z");
        // A second write replaces the row (single-row table).
        store
            .save_hub_models_cache(
                8,
                "{\"version\":8}",
                &"b".repeat(64),
                "2026-01-02T00:00:00Z",
            )
            .unwrap();
        assert_eq!(store.hub_models_cache().unwrap().0, 8);
    }

    #[test]
    fn pricing_doc_rejects_missing_usd() {
        let parse = |rates: &str| -> ModelsDoc {
            serde_json::from_str(&format!(
                r#"{{"version":1,"exchange_rates":{rates},"models":[]}}"#
            ))
            .unwrap()
        };
        assert!(pricing_doc_error(&parse(r#"{"USD":1.0}"#)).is_none());
        assert!(pricing_doc_error(&parse(r#"{"USD":1.0,"CNY":7.1}"#)).is_none());
        assert!(pricing_doc_error(&parse("{}")).is_some());
        assert!(pricing_doc_error(&parse(r#"{"CNY":7.1}"#)).is_some());
    }

    #[test]
    fn record_catalog_sha_arms_and_disarms() {
        let store = kiwanod::store::Store::open_in_memory().unwrap();
        let sha = "c".repeat(64);

        record_catalog_sha(&store, Some(&sha), true).unwrap();
        assert_eq!(
            store.app_setting(HUB_CATALOG_SHA_KEY).as_deref(),
            Some(&sha[..])
        );

        // Unverified bytes drop the gate so the next sync re-fetches.
        record_catalog_sha(&store, Some(&sha), false).unwrap();
        assert_eq!(store.app_setting(HUB_CATALOG_SHA_KEY), None);

        // No manifest leaves whatever was stored alone.
        store.set_app_setting(HUB_CATALOG_SHA_KEY, &sha).unwrap();
        record_catalog_sha(&store, None, false).unwrap();
        assert_eq!(
            store.app_setting(HUB_CATALOG_SHA_KEY).as_deref(),
            Some(&sha[..])
        );
    }
}
