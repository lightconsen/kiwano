//! MiniMax coding-plan quota: the remaining-percent buckets, inverted.

use crate::plan_quota::http::fetch_json;
use crate::plan_quota::json::millis_to_iso8601;
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

pub(crate) async fn query_minimax(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let domain = if base_url.to_lowercase().contains("minimaxi.com") {
        "api.minimaxi.com"
    } else {
        "api.minimax.io"
    };
    let url = format!("https://{domain}/v1/api/openplatform/coding_plan/remains");
    let req = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json");
    let body = match fetch_json(req).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    // Business-level error envelope.
    if let Some(base_resp) = body.get("base_resp") {
        let code = base_resp
            .get("status_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if code != 0 {
            let msg = base_resp
                .get("status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Ok(QuotaOutcome::Failed(format!(
                "Endpoint error (code {code}): {msg}"
            )));
        }
    }
    Ok(QuotaOutcome::Ok {
        tiers: parse_minimax_tiers(&body),
        note: None,
    })
}

/// The `coding_plan/remains` endpoint reports remaining percent for the
/// `general` (coding plan) model bucket; video and friends are skipped. The
/// 5h bucket is always present; the weekly bucket only exists when
/// `current_weekly_status == 1` (status 3 = plan has no weekly cap and the
/// percent is pinned at 100 — not a real bucket).
fn parse_minimax_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let Some(model_remains) = body.get("model_remains").and_then(|v| v.as_array()) else {
        return tiers;
    };
    let Some(item) = model_remains.iter().find(|item| {
        item.get("model_name")
            .and_then(|v| v.as_str())
            .map(|s| s == "general")
            .unwrap_or(false)
    }) else {
        return tiers;
    };

    if let Some(remain_pct) = item
        .get("current_interval_remaining_percent")
        .and_then(|v| v.as_f64())
    {
        tiers.push(PlanTierVm {
            name: "five_hour".to_string(),
            utilization: 100.0 - remain_pct,
            resets_at: item
                .get("end_time")
                .and_then(|v| v.as_i64())
                .and_then(millis_to_iso8601),
            used: None,
            limit: None,
            unit: None,
        });
    }

    if item.get("current_weekly_status").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(remain_pct) = item
            .get("current_weekly_remaining_percent")
            .and_then(|v| v.as_f64())
        {
            tiers.push(PlanTierVm {
                name: "weekly_limit".to_string(),
                utilization: 100.0 - remain_pct,
                resets_at: item
                    .get("weekly_end_time")
                    .and_then(|v| v.as_i64())
                    .and_then(millis_to_iso8601),
                used: None,
                limit: None,
                unit: None,
            });
        }
    }
    tiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn minimax_general_tiers_invert_remaining_percent() {
        let body = json!({
            "model_remains": [
                { "model_name": "general",
                  "current_interval_remaining_percent": 98.0,
                  "current_weekly_remaining_percent": 95.0,
                  "current_weekly_status": 1,
                  "end_time": 1_780_329_600_000_i64,
                  "weekly_end_time": 1_780_848_000_000_i64 },
                { "model_name": "video", "current_interval_remaining_percent": 50.0 }
            ]
        });
        let tiers = parse_minimax_tiers(&body);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 2.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 5.0).abs() < 1e-9);
    }

    #[test]
    fn minimax_weekly_status_3_skips_weekly_tier() {
        // status=3 = plan has no weekly cap; the pinned 100% must not render
        // as a fake "0% used" bucket.
        let body = json!({
            "model_remains": [{
                "model_name": "general",
                "current_interval_remaining_percent": 99.0,
                "current_weekly_status": 3,
                "current_weekly_remaining_percent": 100.0
            }]
        });
        let tiers = parse_minimax_tiers(&body);
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 1.0).abs() < 1e-9);
    }
}
