//! Hub catalog sync (tech.md §3 Hub info sync protocol).
//!
//! Protocol (v0): `GET {hub_url}` → JSON `{ "total": N, "entries": [...] }`,
//! with entries shaped exactly like the GUI's `CatalogEntryVm`. After a
//! successful sync the normalized payload is stored in the single-row aux
//! `hub_cache`; the catalog reads the cache first and falls back to the
//! bundled static catalog.json, so it works fully offline. The Hub only
//! carries catalog metadata — API requests and keys never go through the
//! Hub (spec §6.1).
//!
//! Sync is *conditional*. `manifest.json` is published next to the artifacts
//! with a sha256 of each; when its `catalog.sha256` already matches the copy
//! we cached, the catalog download is skipped entirely. The manifest is an
//! optimisation only — if it is missing, partial, or unparseable the sync
//! degrades to the unconditional full fetch, never to an error.

use crate::vm::{self, Aux};
use sha2::{Digest, Sha256};

/// Public Hub catalog endpoint (protocol v0: plain static JSON; can later
/// upgrade smoothly to an API with version negotiation).
pub const DEFAULT_HUB_URL: &str = "https://hub.kiwano.cc/catalog.json";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// app_settings KV holding the sha256 of the catalog bytes currently cached in
/// `hub_cache`. Stored in the generic KV rather than a `hub_cache` column: the
/// Aux half of the DB has no migration framework (every table is created with
/// `CREATE TABLE IF NOT EXISTS`), so an existing install would never gain the
/// column. The sha and the payload are always written by the same code path
/// from the same response, so they cannot drift apart.
const HUB_CATALOG_SHA_KEY: &str = "hub_catalog_sha";

/// The sibling artifact URL of the catalog endpoint, derived from `hub_url`:
/// the manifest lives beside the catalog it describes. Mirrors the frontend's
/// `hubAssetUrl` (src/lib/hub.ts) so both sides resolve the same way.
///
/// `https://hub.kiwano.cc/catalog.json` → `https://hub.kiwano.cc/manifest.json`
/// `https://hub.kiwano.cc/v1/` → `https://hub.kiwano.cc/v1/manifest.json`
fn hub_asset_url(hub_url: &str, name: &str) -> Result<String, String> {
    let mut url = reqwest::Url::parse(hub_url.trim())
        .map_err(|e| format!("hub_url \"{hub_url}\" is not a valid URL: {e}"))?;
    if !matches!(url.scheme(), "http" | "https") {
        return Err(format!("hub_url must be an http(s) URL, got \"{hub_url}\""));
    }
    url.set_query(None);
    url.set_fragment(None);
    // A trailing slash means the path already names a directory; otherwise the
    // last segment is the artifact filename and gets replaced.
    let path = url.path().to_string();
    let dir = if path.ends_with('/') {
        path.trim_end_matches('/').to_string()
    } else {
        path.rsplit_once('/')
            .map(|(d, _)| d.to_string())
            .unwrap_or_default()
    };
    url.set_path(&format!("{dir}/{name}"));
    Ok(url.to_string())
}

/// sha256 as lowercase hex — the same encoding `generate.mjs` writes with
/// Node's `digest("hex")`.
fn sha256_hex(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}

/// Only a well-formed digest may arm the gate: a truncated or garbage value
/// must degrade to "no gate" rather than compare unequal forever.
fn normalize_sha(sha: Option<String>) -> Option<String> {
    let sha = sha?.trim().to_ascii_lowercase();
    (sha.len() == 64 && sha.bytes().all(|b| b.is_ascii_hexdigit())).then_some(sha)
}

/// manifest.json, reduced to what the sync needs.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct HubManifest {
    pub catalog_sha: Option<String>,
    /// Reserved for the pricing sync (remote models.json); parsed here so the
    /// type is complete and one manifest GET serves both resources.
    #[allow(dead_code)]
    pub models_version: Option<i64>,
    #[allow(dead_code)]
    pub models_sha: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ManifestWire {
    catalog: Option<ManifestCatalogWire>,
    models: Option<ManifestModelsWire>,
}

#[derive(serde::Deserialize, Default)]
struct ManifestCatalogWire {
    sha256: Option<String>,
}

#[derive(serde::Deserialize, Default)]
struct ManifestModelsWire {
    version: Option<i64>,
    sha256: Option<String>,
}

/// Every field is optional and every failure degrades to "no gate": a partial,
/// stale, or absent manifest may only cost us the optimisation.
fn parse_manifest(raw: &str) -> HubManifest {
    let Ok(wire) = serde_json::from_str::<ManifestWire>(raw) else {
        return HubManifest::default();
    };
    let catalog = wire.catalog.unwrap_or_default();
    let models = wire.models.unwrap_or_default();
    HubManifest {
        catalog_sha: normalize_sha(catalog.sha256),
        models_version: models.version,
        models_sha: normalize_sha(models.sha256),
    }
}

/// Never fails: an unreachable or malformed manifest means "fetch the catalog
/// unconditionally", exactly as before this existed.
fn fetch_manifest(client: &reqwest::blocking::Client, manifest_url: &str) -> HubManifest {
    let Ok(resp) = client
        .get(manifest_url)
        .send()
        .and_then(|r| r.error_for_status())
    else {
        return HubManifest::default();
    };
    match resp.text() {
        Ok(body) => parse_manifest(&body),
        Err(_) => HubManifest::default(),
    }
}

fn fetch_bytes(client: &reqwest::blocking::Client, url: &str) -> Result<Vec<u8>, String> {
    client
        .get(url)
        .send()
        .map_err(|e| format!("Hub unreachable: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Hub returned an error: {e}"))?
        .bytes()
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

#[derive(Debug, PartialEq, Eq)]
enum SyncAction {
    /// The manifest sha matched what we already cached — skip the download.
    Skip,
    Fetch,
}

/// Gate for the conditional sync, factored out so the decision is fully
/// testable without HTTP (same posture as `lib.rs`'s `watchdog_decision`).
///
/// `cache_payload` is `Some` only when a *parseable* catalog is cached: a skip
/// also needs to report `fetched`, and a corrupt payload must fall through to
/// the full fetch so it heals instead of reporting a stale count forever.
fn sync_action(
    remote_sha: Option<&str>,
    cached_sha: Option<&str>,
    cache_payload: Option<&str>,
) -> SyncAction {
    match (remote_sha, cached_sha, cache_payload) {
        (Some(remote), Some(cached), Some(_)) if remote == cached => SyncAction::Skip,
        _ => SyncAction::Fetch,
    }
}

/// Arm or disarm the gate after a fetch. Verified bytes keep the gate armed;
/// bytes we cannot vouch for (manifest skew during a publish, truncation) drop
/// it so the next sync re-fetches and converges. No manifest at all leaves
/// whatever was stored alone — we have nothing to invalidate it with.
fn record_catalog_sha(aux: &Aux, remote_sha: Option<&str>, verified: bool) -> Result<(), String> {
    match (remote_sha, verified) {
        (Some(sha), true) => aux
            .set_setting(HUB_CATALOG_SHA_KEY, sha)
            .map_err(|e| e.to_string()),
        (Some(_), false) => aux
            .delete_setting(HUB_CATALOG_SHA_KEY)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        (None, _) => Ok(()),
    }
}

/// Validate and normalize a Hub response: every entry must deserialize as a catalog entry.
fn parse_catalog(raw: &str) -> Result<vm::CatalogListVm, String> {
    serde_json::from_str(raw).map_err(|e| format!("Hub response is not a valid catalog: {e}"))
}

/// Fetch the Hub catalog and cache it. `hub_url` comes from settings
/// (ui_settings.hub_url). When the manifest says the remote catalog is the one
/// we already cached, nothing is downloaded and `unchanged` comes back true.
pub fn sync_from_hub(aux: &Aux, hub_url: &str) -> Result<vm::SyncReportVm, String> {
    let manifest_url = hub_asset_url(hub_url, "manifest.json")?;
    let client = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut manifest = fetch_manifest(&client, &manifest_url);

    // ── gate ──
    // Skipping requires all three of: a remote sha, the sha we cached, and a
    // cache payload that still parses. A stray sha with no usable payload must
    // not skip — `fetched` would be unknown, and load_catalog would quietly
    // serve the bundled catalog instead.
    let cached_list = aux
        .load_hub_cache()
        .and_then(|(payload, _)| parse_catalog(&payload).ok());
    let cached_sha = aux.get_setting(HUB_CATALOG_SHA_KEY);
    if sync_action(
        manifest.catalog_sha.as_deref(),
        cached_sha.as_deref(),
        cached_list.as_ref().map(|_| ""),
    ) == SyncAction::Skip
    {
        let synced_at = vm::rfc3339(vm::unix_now());
        // A skip still counts as "synced today": the app just confirmed it is
        // current, which is exactly what the footer badge claims.
        aux.touch_hub_synced_at(&synced_at)
            .map_err(|e| e.to_string())?;
        return Ok(vm::SyncReportVm {
            fetched: cached_list.map_or(0, |l| l.entries.len() as i64),
            synced_at,
            hub_url: hub_url.into(),
            unchanged: true,
        });
    }

    // ── full fetch ──
    // The manifest and the artifacts are uploaded separately, so a hash
    // mismatch usually just means we raced a publish. One re-read of both
    // absorbs the common case; if it still mismatches we cache the parsed
    // catalog but drop the gate so the next sync re-fetches. This guards
    // against races and truncated responses, not against a hostile Hub —
    // TLS already covers that.
    let mut attempt = 0;
    let (list, remote_sha, verified) = loop {
        // Hash the raw bytes. The cached payload is a re-serialization
        // (different key order, spacing, omitted None fields) and would never
        // match the manifest, silently defeating the whole gate.
        let bytes = fetch_bytes(&client, hub_url)?;
        let remote_sha = manifest.catalog_sha.clone();
        let verified = remote_sha
            .as_deref()
            .is_some_and(|sha| sha256_hex(&bytes) == sha);
        let body =
            std::str::from_utf8(&bytes).map_err(|e| format!("Hub response is not UTF-8: {e}"))?;
        let list = parse_catalog(body)?;
        if !verified && remote_sha.is_some() && attempt == 0 {
            attempt += 1;
            manifest = fetch_manifest(&client, &manifest_url);
            continue;
        }
        break (list, remote_sha, verified);
    };

    let payload = serde_json::to_string(&list).map_err(|e| e.to_string())?;
    let synced_at = vm::rfc3339(vm::unix_now());
    aux.save_hub_cache(&payload, &synced_at)
        .map_err(|e| e.to_string())?;
    // Only arm the gate once the payload is safely cached.
    record_catalog_sha(aux, remote_sha.as_deref(), verified)?;
    Ok(vm::SyncReportVm {
        fetched: list.entries.len() as i64,
        synced_at,
        hub_url: hub_url.into(),
        unchanged: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
        let raw = serde_json::json!({ "total": 1, "entries": [entry] }).to_string();
        let list = parse_catalog(&raw).unwrap();
        assert_eq!(list.total, 1);
        assert_eq!(list.entries[0].name, "DeepSeek");

        assert!(parse_catalog("{not json").is_err());
        assert!(parse_catalog(r#"{"total":1,"entries":[{"id":"x"}]}"#).is_err());
    }

    #[test]
    fn cache_preferred_and_fallback_bundled() {
        let aux = Aux::open_in_memory().unwrap();
        let store = kiwano_gateway::store::Store::open_in_memory().unwrap();
        // never synced → bundled fallback
        let fallback = vm::load_catalog(&store, &aux);
        assert_eq!(fallback.total as usize, fallback.entries.len());
        // Multi-protocol merge puts one row per brand (openai/anthropic/gemini
        // siblings folded in), so the entry count shrinks while the endpoint
        // count keeps the catalog's real size
        let endpoint_count = fallback
            .entries
            .iter()
            .map(|e| 1 + e.endpoints.len())
            .sum::<usize>();
        assert!(fallback.entries.len() > 80);
        assert!(endpoint_count > 100);
        // every bundled entry carries its protocol fingerprint
        assert!(fallback
            .entries
            .iter()
            .all(|e| ["openai", "anthropic", "gemini"].contains(&e.protocol.as_str())));

        // cache written → cache wins
        let payload = serde_json::to_string(&vm::CatalogListVm {
            total: 1,
            entries: vec![fallback.entries[0].clone()],
        })
        .unwrap();
        aux.save_hub_cache(&payload, "2026-09-07T00:00:00Z")
            .unwrap();
        let cached = vm::load_catalog(&store, &aux);
        assert_eq!(cached.total, 1);
    }

    #[test]
    fn footer_hub_synced_tracks_today() {
        let aux = Aux::open_in_memory().unwrap();
        let store = kiwano_gateway::store::Store::open_in_memory().unwrap();
        // never synced → false
        assert!(
            !vm::build_footer_stats(&store, &aux, "v0.0.0")
                .unwrap()
                .hub_synced
        );
        // just synced (timestamp uses the same now as production) → true
        aux.save_hub_cache("{}", &vm::rfc3339(vm::unix_now()))
            .unwrap();
        assert!(
            vm::build_footer_stats(&store, &aux, "v0.0.0")
                .unwrap()
                .hub_synced
        );
    }

    // ── conditional sync ────────────────────────────────────────────────

    /// A parseable catalog body (pretty or compact) for the gate tests.
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
        let aux = Aux::open_in_memory().unwrap();
        let store = kiwano_gateway::store::Store::open_in_memory().unwrap();
        let payload = catalog_body(false);
        aux.save_hub_cache(&payload, "2020-01-01T00:00:00Z")
            .unwrap();

        assert!(
            !vm::build_footer_stats(&store, &aux, "v0.0.0")
                .unwrap()
                .hub_synced
        );
        let now = vm::rfc3339(vm::unix_now());
        assert!(aux.touch_hub_synced_at(&now).unwrap());

        let (payload_after, synced_after) = aux.load_hub_cache().unwrap();
        assert_eq!(payload_after, payload, "payload must stay byte-identical");
        assert_eq!(synced_after, now);
        // The point of refreshing: the footer badge stays truthful.
        assert!(
            vm::build_footer_stats(&store, &aux, "v0.0.0")
                .unwrap()
                .hub_synced
        );
    }

    #[test]
    fn touch_hub_synced_at_without_row() {
        let aux = Aux::open_in_memory().unwrap();
        // Fresh install: nothing to touch, and no panic.
        assert!(!aux
            .touch_hub_synced_at(&vm::rfc3339(vm::unix_now()))
            .unwrap());
    }

    #[test]
    fn record_catalog_sha_arms_and_disarms() {
        let aux = Aux::open_in_memory().unwrap();
        let sha = "c".repeat(64);

        record_catalog_sha(&aux, Some(&sha), true).unwrap();
        assert_eq!(
            aux.get_setting(HUB_CATALOG_SHA_KEY).as_deref(),
            Some(&sha[..])
        );

        // Unverified bytes drop the gate so the next sync re-fetches.
        record_catalog_sha(&aux, Some(&sha), false).unwrap();
        assert_eq!(aux.get_setting(HUB_CATALOG_SHA_KEY), None);

        // No manifest leaves whatever was stored alone.
        aux.set_setting(HUB_CATALOG_SHA_KEY, &sha).unwrap();
        record_catalog_sha(&aux, None, false).unwrap();
        assert_eq!(
            aux.get_setting(HUB_CATALOG_SHA_KEY).as_deref(),
            Some(&sha[..])
        );
    }
}
