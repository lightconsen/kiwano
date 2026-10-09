//! Hub catalog sync (tech.md §3 Hub info sync protocol).
//!
//! Moved here from `kiwano_core::sync` when the daemon took over the sync
//! (`migrate.local.md` §10.14): it is the **writer** of the `hub_cache` and
//! `hub_models_cache` tables, and those are the daemon's tables now. The caches
//! therefore have one writer and one set of accessors — which is the whole of
//! the "aux connection to the sync" that used to be.
//!
//! The `String` errors are what the CLI and the app's `sync_hub` return, and the
//! sync is `async` because the HTTP is the whole of it.

use crate::store::Store;
use kiwano_adapters::model_pricing::ModelsDoc;
use kiwano_api::providers::SyncReportVm;
use sha2::{Digest, Sha256};

/// Public Hub catalog endpoint (protocol v0: plain static JSON; can later
/// upgrade smoothly to an API with version negotiation).
pub const DEFAULT_HUB_URL: &str = "https://hub.kiwano.cc/catalog.json";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// app_settings KV holding the sha256 of the catalog bytes currently cached in
/// `hub_cache`. Stored in the generic KV rather than a `hub_cache` column: the
/// KV has no migration framework (every table is created with
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
    /// Informational: the price doc carries its own `version`, which is what
    /// the seed gate uses. Kept so the manifest type mirrors the published
    /// shape in full.
    #[allow(dead_code)]
    pub models_version: Option<i64>,
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
async fn fetch_manifest(client: &reqwest::Client, manifest_url: &str) -> HubManifest {
    let Ok(resp) = client
        .get(manifest_url)
        .send()
        .await
        .and_then(|r| r.error_for_status())
    else {
        return HubManifest::default();
    };
    match resp.text().await {
        Ok(body) => parse_manifest(&body),
        Err(_) => HubManifest::default(),
    }
}

async fn fetch_bytes(client: &reqwest::Client, url: &str) -> Result<Vec<u8>, String> {
    let bytes = client
        .get(url)
        .send()
        .await
        .map_err(|e| format!("Hub unreachable: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Hub returned an error: {e}"))?
        .bytes()
        .await
        .map_err(|e| e.to_string())?;
    Ok(bytes.to_vec())
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
fn record_catalog_sha(
    store: &Store,
    remote_sha: Option<&str>,
    verified: bool,
) -> Result<(), String> {
    match (remote_sha, verified) {
        (Some(sha), true) => store
            .set_app_setting(HUB_CATALOG_SHA_KEY, sha)
            .map_err(|e| e.to_string()),
        (Some(_), false) => store
            .delete_app_setting(HUB_CATALOG_SHA_KEY)
            .map(|_| ())
            .map_err(|e| e.to_string()),
        (None, _) => Ok(()),
    }
}

/// Validate and normalize a Hub response: every entry must deserialize as a catalog entry.
fn parse_catalog(raw: &str) -> Result<super::catalog::CatalogListVm, String> {
    serde_json::from_str(raw).map_err(|e| format!("Hub response is not a valid catalog: {e}"))
}

/// Catalog half of a sync: `(entries fetched, unchanged)`.
struct CatalogOutcome {
    fetched: i64,
    unchanged: bool,
}

/// Pricing half of a sync: the version now cached, and whether it was already
/// the cached one. `version` is None when the Hub has no pricing to offer.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct PricingOutcome {
    version: Option<i64>,
    unchanged: bool,
}

/// Sync both Hub resources. `hub_url` comes from settings (ui_settings.hub_url).
/// One manifest GET serves both gates. The resources degrade independently: a
/// pricing failure never costs the user the catalog, and vice versa.
pub async fn sync_from_hub(store: &Store, hub_url: &str) -> Result<SyncReportVm, String> {
    let manifest_url = hub_asset_url(hub_url, "manifest.json")?;
    let client = reqwest::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?;
    let mut manifest = fetch_manifest(&client, &manifest_url).await;

    let catalog = sync_catalog(&client, store, hub_url, &manifest_url, &mut manifest).await?;
    // Best-effort: the catalog is the primary resource. A bad models.json
    // leaves the previous pricing cache in place.
    let pricing = sync_pricing(&client, store, hub_url, &manifest)
        .await
        .unwrap_or_default();

    Ok(SyncReportVm {
        fetched: catalog.fetched,
        synced_at: crate::store::now_rfc3339(),
        hub_url: hub_url.into(),
        unchanged: catalog.unchanged,
        pricing_version: pricing.version,
        pricing_unchanged: pricing.unchanged,
    })
}

/// Apply the freshly cached Hub documents to everything derived from them.
///
/// A sync only rewrites two cache rows. What the app displays and the gateway
/// bills from is *derived* state: the `model_pricing` mirror the price table is
/// resolved out of, and the link from each local provider to its catalog entry.
/// Both are rebuilt here — in one function rather than as a step in each caller,
/// because that is exactly how they drifted: the startup sync did both, the
/// manual one did neither of the price half, and the Sync button therefore
/// reported success while the gateway went on billing what it had loaded at
/// launch.
///
/// Returns true when either wrote, which is precisely when the daemon's
/// in-memory copies are stale and it owes a reload.
pub fn apply_hub_documents(store: &Store) -> bool {
    let mut wrote = false;
    // A no-op when the version and content are unchanged, which is the common
    // case: most syncs bring neither a new price table nor a new link.
    match super::pricing_sync::seed_model_pricing(store) {
        Ok(r) if r.seeded > 0 => {
            tracing::info!(
                version = r.version,
                rows = r.seeded,
                "hub model pricing seeded"
            );
            wrote = true;
        }
        Err(e) => tracing::warn!(error = %e, "model pricing seed failed"),
        _ => {}
    }
    // A provider may have become linkable to a catalog entry it could not be
    // matched against before — this is where a first-run install gets its
    // catalog at all.
    match super::catalog::link_providers(store).map_err(|e| e.to_string()) {
        Ok(linked) if linked > 0 => {
            tracing::info!(linked, "providers linked to their catalog entries");
            wrote = true;
        }
        Err(e) => tracing::warn!(error = %e, "provider catalog link failed"),
        _ => {}
    }
    wrote
}

async fn sync_catalog(
    client: &reqwest::Client,
    store: &Store,
    hub_url: &str,
    manifest_url: &str,
    manifest: &mut HubManifest,
) -> Result<CatalogOutcome, String> {
    // ── gate ──
    // Skipping requires all three of: a remote sha, the sha we cached, and a
    // cache payload that still parses. A stray sha with no usable payload must
    // not skip — `fetched` would be unknown, and the shelf would be empty.
    let cached_list = store
        .hub_cache()
        .and_then(|(payload, _)| parse_catalog(&payload).ok());
    let cached_sha = store.app_setting(HUB_CATALOG_SHA_KEY);
    if sync_action(
        manifest.catalog_sha.as_deref(),
        cached_sha.as_deref(),
        cached_list.as_ref().map(|_| ""),
    ) == SyncAction::Skip
    {
        let synced_at = crate::store::now_rfc3339();
        // A skip still counts as "synced today": the app just confirmed it is
        // current, which is exactly what the footer badge claims.
        store
            .touch_hub_synced_at(&synced_at)
            .map_err(|e| e.to_string())?;
        return Ok(CatalogOutcome {
            fetched: cached_list.map_or(0, |l| l.entries.len() as i64),
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
        let bytes = fetch_bytes(client, hub_url).await?;
        let remote_sha = manifest.catalog_sha.clone();
        let verified = remote_sha
            .as_deref()
            .is_some_and(|sha| sha256_hex(&bytes) == sha);
        let body =
            std::str::from_utf8(&bytes).map_err(|e| format!("Hub response is not UTF-8: {e}"))?;
        let list = parse_catalog(body)?;
        if !verified && remote_sha.is_some() && attempt == 0 {
            attempt += 1;
            *manifest = fetch_manifest(client, manifest_url).await;
            continue;
        }
        break (list, remote_sha, verified);
    };

    let payload = serde_json::to_string(&list).map_err(|e| e.to_string())?;
    let synced_at = crate::store::now_rfc3339();
    store
        .save_hub_cache(&payload, &synced_at)
        .map_err(|e| e.to_string())?;
    // Only arm the gate once the payload is safely cached.
    record_catalog_sha(store, remote_sha.as_deref(), verified)?;
    Ok(CatalogOutcome {
        fetched: list.entries.len() as i64,
        unchanged: false,
    })
}

/// Why a Hub price doc is unusable, if it is.
///
/// A doc without USD would silently mis-denominate every displayed cost:
/// `convert_amount` passes an unknown currency through unchanged rather than
/// failing, so a missing rate shows up as a plausible-looking wrong number. It
/// is rejected whole rather than half-applied.
fn pricing_doc_error(doc: &ModelsDoc) -> Option<&'static str> {
    if !doc.exchange_rates.contains_key("USD") {
        return Some("Hub pricing has no USD exchange rate");
    }
    None
}

/// Fetch the Hub price table and cache it verbatim.
///
/// The cache row carries the sha256 of the bytes it holds, so the gate needs
/// nothing else: a stale manifest simply fails to match and the next sync
/// re-reads. Storing the computed digest (rather than the manifest's) keeps the
/// row self-describing and the column non-nullable.
async fn sync_pricing(
    client: &reqwest::Client,
    store: &Store,
    hub_url: &str,
    manifest: &HubManifest,
) -> Result<PricingOutcome, String> {
    let cached = store.hub_models_cache();
    if let (Some(remote), Some((version, _, cached_sha, _))) =
        (manifest.models_sha.as_deref(), cached.as_ref())
    {
        if remote == cached_sha {
            return Ok(PricingOutcome {
                version: Some(*version),
                unchanged: true,
            });
        }
    }

    let bytes = fetch_bytes(client, &hub_asset_url(hub_url, "models.json")?).await?;
    let body = std::str::from_utf8(&bytes).map_err(|e| format!("Hub pricing is not UTF-8: {e}"))?;
    let doc: ModelsDoc = serde_json::from_str(body)
        .map_err(|e| format!("Hub pricing is not a valid models doc: {e}"))?;
    if let Some(why) = pricing_doc_error(&doc) {
        return Err(why.into());
    }

    store
        .save_hub_models_cache(
            doc.version,
            body,
            &sha256_hex(&bytes),
            &crate::store::now_rfc3339(),
        )
        .map_err(|e| e.to_string())?;
    Ok(PricingOutcome {
        version: Some(doc.version),
        unchanged: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Store;

    /// The conditional gate, on a store with a cache in it: a manifest whose
    /// sha matches what is cached skips the fetch, and a sha that does not —
    /// or no manifest at all — fetches. The gate is the optimisation the whole
    /// conditional protocol exists for, and a wrong answer is either a wasted
    /// download or a shelf that silently never updates.
    #[test]
    fn the_sync_gate_skips_only_when_everything_matches() {
        let store = Store::open_in_memory().unwrap();
        let sha = "a".repeat(64);
        store
            .save_hub_cache(r#"{"total":1,"entries":[]}"#, "2026-01-01T00:00:00Z")
            .unwrap();
        store.set_app_setting(HUB_CATALOG_SHA_KEY, &sha).unwrap();

        // Match: skip. The manifest offers nothing the cache does not already have.
        let manifest = HubManifest {
            catalog_sha: Some(sha.clone()),
            models_version: None,
            models_sha: None,
        };
        assert_eq!(
            sync_action(
                manifest.catalog_sha.as_deref(),
                store.app_setting(HUB_CATALOG_SHA_KEY).as_deref(),
                Some("")
            ),
            SyncAction::Skip
        );

        // A different sha, and no sha at all: fetch. A partial manifest must
        // never cost the shelf an update.
        assert_eq!(
            sync_action(
                Some(&"b".repeat(64)),
                store.app_setting(HUB_CATALOG_SHA_KEY).as_deref(),
                Some("")
            ),
            SyncAction::Fetch
        );
        assert_eq!(
            sync_action(
                None,
                store.app_setting(HUB_CATALOG_SHA_KEY).as_deref(),
                Some("")
            ),
            SyncAction::Fetch
        );
    }

    /// `apply_hub_documents` is a no-op when the Hub has published nothing new —
    /// the common case — and reports "wrote" only when a document actually
    /// changed what the mirror holds. A caller reloads its route table on that
    /// signal, so a false positive would re-read on every sync.
    #[test]
    fn applying_unchanged_documents_writes_nothing() {
        let store = Store::open_in_memory().unwrap();
        // No cache at all: nothing to seed from, nothing to link against.
        assert!(!apply_hub_documents(&store));
    }
}
