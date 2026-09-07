//! Hub 目录同步（tech.md §三 Hub 信息同步协议）。
//!
//! 协议（v0）：`GET {hub_url}` → JSON `{ "total": N, "entries": [...] }`，
//! 条目与 GUI 的 `CatalogEntryVm` 同形。同步成功后把规范化 payload 存入
//! aux `hub_cache` 单行缓存；货架读取缓存优先、随包静态 catalog.json 兜底，
//! 离线完全可用。Hub 只承载目录元信息——API 请求与 Key 永不经 Hub（spec §6.1）。

use crate::vm::{self, Aux};

/// Hub 公开目录端点（协议 v0：纯静态 JSON，后续可平滑升级为带版本协商的 API）。
pub const DEFAULT_HUB_URL: &str = "https://hub.kiwano.app/catalog.json";

const TIMEOUT: std::time::Duration = std::time::Duration::from_secs(10);

/// 校验并规范化 Hub 响应：条目必须能反序列化为目录条目。
fn parse_catalog(raw: &str) -> Result<vm::CatalogListVm, String> {
    serde_json::from_str(raw).map_err(|e| format!("Hub 响应不是合法目录: {e}"))
}

/// 拉取 Hub 目录并落缓存。`hub_url` 来自设置（ui_settings.hub_url）。
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
    aux.save_hub_cache(&payload, &synced_at).map_err(|e| e.to_string())?;
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
        // 未同步 → 随包兜底
        let fallback = vm::load_catalog(&aux);
        assert_eq!(fallback.total, 42);
        assert!(!fallback.entries.is_empty());

        // 落缓存后 → 缓存优先
        let payload = serde_json::to_string(&vm::CatalogListVm {
            total: 1,
            entries: vec![fallback.entries[0].clone()],
        })
        .unwrap();
        aux.save_hub_cache(&payload, "2026-09-07T00:00:00Z").unwrap();
        let cached = vm::load_catalog(&aux);
        assert_eq!(cached.total, 1);
    }

    #[test]
    fn footer_hub_synced_tracks_today() {
        let aux = Aux::open_in_memory().unwrap();
        let store = kiwano_gateway::store::Store::open_in_memory().unwrap();
        // 未同步 → false
        assert!(!vm::build_footer_stats(&store, &aux).unwrap().hub_synced);
        // 刚同步（时间戳用与生产一致的 now）→ true
        aux.save_hub_cache("{}", &vm::rfc3339(vm::unix_now())).unwrap();
        assert!(vm::build_footer_stats(&store, &aux).unwrap().hub_synced);
    }
}
