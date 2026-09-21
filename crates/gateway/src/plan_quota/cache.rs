//! The `app_settings` KV cache for plan quota reports (5-minute TTL) and the
//! refresh step that fills it.

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
    let raw = store.app_setting(&cache_key(provider_id))?;
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
    let _ = store.set_app_setting(&cache_key(provider_id), &v.to_string());
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
