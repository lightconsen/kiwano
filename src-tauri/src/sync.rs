//! Hub catalog sync (tech.md §3 Hub info sync protocol).
//!
//! Protocol (v0): `GET {hub_url}` → JSON `{ "total": N, "entries": [...] }`,
//! with entries shaped exactly like the GUI's `CatalogEntryVm`. After a
//! successful sync the normalized payload is stored in the single-row aux
//! `hub_cache`; the catalog reads the cache first and falls back to the
//! bundled static catalog.json, so it works fully offline. The Hub only
//! carries catalog metadata — API requests and keys never go through the
//! Hub (spec §6.1).

use crate::vm::{self, Aux};

/// Public Hub catalog endpoint (protocol v0: plain static JSON; can later
/// upgrade smoothly to an API with version negotiation).
pub const DEFAULT_HUB_URL: &str = "https://hub.kiwano.app/catalog.json";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// Validate and normalize a Hub response: every entry must deserialize as a catalog entry.
fn parse_catalog(raw: &str) -> Result<vm::CatalogListVm, String> {
    serde_json::from_str(raw).map_err(|e| format!("Hub 响应不是合法目录: {e}"))
}

/// Fetch the Hub catalog and cache it. `hub_url` comes from settings (ui_settings.hub_url).
pub fn sync_from_hub(aux: &Aux, hub_url: &str) -> Result<vm::SyncReportVm, String> {
    let body = reqwest::blocking::Client::builder()
        .timeout(TIMEOUT)
        .build()
        .map_err(|e| e.to_string())?
        .get(hub_url)
        .send()
        .map_err(|e| format!("Hub 不可达: {e}"))?
        .error_for_status()
        .map_err(|e| format!("Hub 返回错误: {e}"))?
        .text()
        .map_err(|e| e.to_string())?;

    let list = parse_catalog(&body)?;
    let payload = serde_json::to_string(&list).map_err(|e| e.to_string())?;
    let synced_at = vm::rfc3339(vm::unix_now());
    aux.save_hub_cache(&payload, &synced_at)
        .map_err(|e| e.to_string())?;
    Ok(vm::SyncReportVm {
        fetched: list.entries.len() as i64,
        synced_at,
        hub_url: hub_url.into(),
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
            "tag": "official", "tag_label": "官方",
            "rating": 4.8, "endpoint": "https://api.deepseek.com",
            "price_line": "¥1/2 每百万", "billing": "per-token",
            "users": "12k", "blurb": "性价比高", "added": false,
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
        // never synced → bundled fallback
        let fallback = vm::load_catalog(&aux);
        assert_eq!(fallback.total, 42);
        assert!(!fallback.entries.is_empty());

        // cache written → cache wins
        let payload = serde_json::to_string(&vm::CatalogListVm {
            total: 1,
            entries: vec![fallback.entries[0].clone()],
        })
        .unwrap();
        aux.save_hub_cache(&payload, "2026-09-07T00:00:00Z")
            .unwrap();
        let cached = vm::load_catalog(&aux);
        assert_eq!(cached.total, 1);
    }

    #[test]
    fn footer_hub_synced_tracks_today() {
        let aux = Aux::open_in_memory().unwrap();
        let store = kiwano_gateway::store::Store::open_in_memory().unwrap();
        // never synced → false
        assert!(!vm::build_footer_stats(&store, &aux).unwrap().hub_synced);
        // just synced (timestamp uses the same now as production) → true
        aux.save_hub_cache("{}", &vm::rfc3339(vm::unix_now()))
            .unwrap();
        assert!(vm::build_footer_stats(&store, &aux).unwrap().hub_synced);
    }
}
