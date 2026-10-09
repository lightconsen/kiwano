//! The KV cache for plan quota reports (5-minute TTL) and the refresh step that
//! fills it.
//!
//! **The cache is moving from the app's KV to the daemon's** (`migrate.local.md`
//! §9.2.1). Both sides read it — the daemon enforces the ceiling from it, and
//! the app's alert path reads it to describe a limit — and the two are upgraded
//! independently, so the write goes to both places and the read prefers the new
//! one. Step 4 of that plan deletes the old row once nothing falls back.

use crate::plan_quota::dispatch::run_template;
use crate::plan_quota::types::{PlanQuotaReport, QuotaOutcome};
use crate::store::Store;
use std::collections::HashMap;

/// `app_settings` KV cache TTL for one provider's plan quota report.
const CACHE_TTL_MS: i64 = 5 * 60 * 1000;

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

fn cache_key(provider_id: &str) -> String {
    format!("plan_quota_cache:{provider_id}")
}

/// The still-fresh cached report for a provider, if there is one. No network:
/// the enforcement path reads what the refresh step put here, so a slow or
/// dead provider endpoint cannot stall a routing decision.
pub fn cached_report(store: &Store, provider_id: &str) -> Option<PlanQuotaReport> {
    cache_read(store, provider_id)
}

fn cache_read(store: &Store, provider_id: &str) -> Option<PlanQuotaReport> {
    // New place first, old second — see the module's note. A daemon from before
    // the move wrote only the old row, and a client from before it reads only
    // that one; both have to work against whichever peer is running.
    let key = cache_key(provider_id);
    let raw = store
        .gateway_setting(&key)
        .or_else(|| store.app_setting(&key))?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let ts = v.get("ts")?.as_i64()?;
    if now_millis() - ts > CACHE_TTL_MS {
        return None;
    }
    let mut report: PlanQuotaReport = serde_json::from_value(v.get("report")?.clone()).ok()?;
    report.cached = true;
    Some(report)
}

/// Store a report for a provider — the write half of [`cached_report`], which
/// the refresh step calls. Public so the app's alert path can be exercised
/// against a cached report without a provider endpoint to talk to.
pub fn cache_write(store: &Store, provider_id: &str, report: &PlanQuotaReport) {
    let v = serde_json::json!({ "ts": now_millis(), "report": report });
    let key = cache_key(provider_id);
    // Both places, for as long as an old reader might be the one asking
    // (`migrate.local.md` §9.2.1). Cheap — one small row per provider per five
    // minutes — and the alternative is a client that sees no quota at all.
    let _ = store.set_gateway_setting(&key, &v.to_string());
    let _ = store.set_app_setting(&key, &v.to_string());
}

/// Query one provider's plan quota. Cached for 5 minutes unless `force`.
/// Outer `Err` = transient network failure; deterministic failures come back
/// as a `success: false` report.
pub async fn get_plan_quota_report(
    store: &Store,
    provider_id: &str,
    force: bool,
) -> Result<PlanQuotaReport, String> {
    if !force {
        if let Some(hit) = cache_read(store, provider_id) {
            return Ok(hit);
        }
    }
    let p = store
        .get_provider(provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Provider not found: {provider_id}"))?;
    let query: serde_json::Value = p
        .plan_query
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or_else(|| "This provider has no plan query configured".to_string())?;
    let template = query
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fields: HashMap<String, serde_json::Value> = query
        .get("fields")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let api_key = p.api_key.clone().unwrap_or_default();

    let mut report = match run_template(&template, &fields, &p.base_url, &api_key).await? {
        QuotaOutcome::Ok { tiers, note } => PlanQuotaReport {
            provider_id: provider_id.to_string(),
            template,
            success: true,
            error: None,
            note,
            tiers,
            queried_at: now_millis(),
            cached: false,
        },
        QuotaOutcome::Failed(error) => PlanQuotaReport {
            provider_id: provider_id.to_string(),
            template,
            success: false,
            error: Some(error),
            note: None,
            tiers: Vec::new(),
            queried_at: now_millis(),
            cached: false,
        },
    };
    if report.success {
        cache_write(store, provider_id, &report);
    }
    report.provider_id = provider_id.to_string();
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(provider_id: &str, note: &str) -> PlanQuotaReport {
        PlanQuotaReport {
            provider_id: provider_id.into(),
            template: "t".into(),
            success: true,
            error: None,
            note: Some(note.into()),
            tiers: Vec::new(),
            queried_at: 0,
            cached: false,
        }
    }

    /// The cache's move, in the three states that can exist while it happens
    /// (`migrate.local.md` §9.2.1).
    ///
    /// A daemon from before wrote only the old row; one from after writes both;
    /// a reader must find it either way, and must prefer the new one when they
    /// disagree — the new one is what this build writes and what a later build
    /// will keep.
    #[test]
    fn the_quota_cache_reads_the_new_place_first_and_the_old_one_after() {
        let store = Store::open_in_memory().unwrap();
        let key = cache_key("p-1");

        // Nothing anywhere: no report, and no panic.
        assert!(cache_read(&store, "p-1").is_none());

        // Only the old row — what a daemon from before this change left.
        let old = serde_json::json!({ "ts": now_millis(), "report": report("p-1", "old") });
        store.set_app_setting(&key, &old.to_string()).unwrap();
        assert_eq!(
            cache_read(&store, "p-1").unwrap().note.as_deref(),
            Some("old"),
            "an old daemon's row is still read"
        );

        // Both, disagreeing: the new one wins.
        let new = serde_json::json!({ "ts": now_millis(), "report": report("p-1", "new") });
        store.set_gateway_setting(&key, &new.to_string()).unwrap();
        assert_eq!(
            cache_read(&store, "p-1").unwrap().note.as_deref(),
            Some("new")
        );

        // And a write fills both, so a client from either side of the move sees
        // what this daemon just fetched.
        cache_write(&store, "p-2", &report("p-2", "both"));
        assert!(store.gateway_setting(&cache_key("p-2")).is_some());
        assert!(
            store.app_setting(&cache_key("p-2")).is_some(),
            "an older reader would otherwise see no quota at all"
        );
    }

    /// A row older than the TTL is not a cache hit, wherever it lives.
    #[test]
    fn a_stale_row_is_not_a_hit_in_either_place() {
        let store = Store::open_in_memory().unwrap();
        let stale = serde_json::json!({ "ts": 0, "report": report("p-1", "stale") });
        store
            .set_gateway_setting(&cache_key("p-1"), &stale.to_string())
            .unwrap();
        assert!(cache_read(&store, "p-1").is_none());
    }
}
