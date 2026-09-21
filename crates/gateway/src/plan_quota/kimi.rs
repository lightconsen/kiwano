//! Kimi For Coding quota: the rolling 5-hour window and the weekly limit.

use crate::plan_quota::http::fetch_json;
use crate::plan_quota::json::{extract_reset_time, parse_f64};
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

pub(crate) async fn query_kimi(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let req = client
        .get("https://api.kimi.com/coding/v1/usages")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let body = match fetch_json(req).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(QuotaOutcome::Ok {
        tiers: parse_kimi_tiers(&body),
        note: None,
    })
}

/// Kimi: `limits[].detail` is the rolling 5-hour window; `usage` is the
/// weekly limit. Both are limit/remaining pairs → invert to utilization.
fn parse_kimi_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let push = |tiers: &mut Vec<PlanTierVm>, name: &str, obj: &serde_json::Value| {
        let limit = obj.get("limit").and_then(parse_f64).unwrap_or(1.0);
        let remaining = obj.get("remaining").and_then(parse_f64).unwrap_or(0.0);
        let resets_at = obj.get("resetTime").and_then(extract_reset_time);
        let used = (limit - remaining).max(0.0);
        let utilization = if limit > 0.0 {
            used / limit * 100.0
        } else {
            0.0
        };
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization,
            resets_at,
            used: None,
            limit: None,
            unit: None,
        });
    };
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            if let Some(detail) = item.get("detail") {
                push(&mut tiers, "five_hour", detail);
            }
        }
    }
    if let Some(usage) = body.get("usage") {
        push(&mut tiers, "weekly_limit", usage);
    }
    tiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn kimi_tiers_from_limits_and_usage() {
        let body = json!({
            "limits": [
                { "detail": { "limit": 100.0, "remaining": 25.0, "resetTime": "2026-09-10T15:00:00Z" } }
            ],
            "usage": { "limit": 200.0, "remaining": 80.0, "resetTime": "2026-09-14T00:00:00Z" }
        });
        let tiers = parse_kimi_tiers(&body);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 75.0).abs() < 1e-9);
        assert!(tiers[0].resets_at.is_some());
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 60.0).abs() < 1e-9);
    }
}
