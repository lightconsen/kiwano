//! OpenCode Go quota: three usage windows, parsed defensively.

use crate::plan_quota::http::fold_response;
use crate::plan_quota::json::{extract_reset_time, parse_f64};
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

/// Response shape (undocumented first-party route):
/// `{"usage":{"rolling"|"weekly"|"monthly":{"status","percent","resetsAt"}}}`.
/// Each window is parsed defensively — the endpoint changed shape once on
/// launch day, so malformed windows are skipped rather than failing all.
/// A percent of 0 makes the upstream `resetsAt` a placeholder (now+window),
/// which is dropped.
fn parse_opencode_go_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    const WINDOWS: [(&str, &str); 3] = [
        ("rolling", "five_hour"),
        ("weekly", "weekly_limit"),
        ("monthly", "monthly"),
    ];
    let Some(usage) = body.get("usage") else {
        return Vec::new();
    };
    let mut tiers = Vec::new();
    for (key, tier_name) in WINDOWS {
        let Some(window) = usage.get(key) else {
            continue;
        };
        let Some(percent) = window.get("percent").and_then(parse_f64) else {
            continue;
        };
        let resets_at = if percent > 0.0 {
            window.get("resetsAt").and_then(extract_reset_time)
        } else {
            None
        };
        tiers.push(PlanTierVm {
            name: tier_name.to_string(),
            utilization: percent,
            resets_at,
            used: None,
            limit: None,
            unit: None,
        });
    }
    tiers
}

pub(crate) async fn query_opencode_go(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    // The usage endpoint only accepts `Authorization: Bearer` — the inverse
    // of the inference side, which wants x-api-key.
    let req = client
        .get("https://opencode.ai/zen/go/v1/usage")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let resp = match req.send().await {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };
    let status = resp.status();
    // 403 EntitlementError: the key is valid (Zen and Go share the workspace
    // key) but the workspace has no Go subscription — a distinct message.
    if status == reqwest::StatusCode::FORBIDDEN {
        return Ok(QuotaOutcome::Failed(
            "API key is valid but not subscribed to OpenCode Go (HTTP 403)".to_string(),
        ));
    }
    let body = match fold_response(resp).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    let tiers = parse_opencode_go_tiers(&body);
    // No window parsed = shape unrecognized (the endpoint changed shape once
    // on launch day) — fail loudly instead of rendering an empty card.
    if tiers.is_empty() {
        return Ok(QuotaOutcome::Failed(
            "Unrecognized response shape".to_string(),
        ));
    }
    Ok(QuotaOutcome::Ok { tiers, note: None })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn opencode_go_three_windows_parse_defensively() {
        let body = json!({
            "usage": {
                "rolling": { "status": "ok", "percent": 37, "resetsAt": "2026-08-26T14:12:03.000Z" },
                "weekly":  { "status": "ok", "percent": "62", "resetsAt": "2026-08-31T00:00:00.000Z" },
                "monthly": { "status": "rate-limited", "percent": 100, "resetsAt": "2026-09-11T00:00:00.000Z" }
            }
        });
        let tiers = parse_opencode_go_tiers(&body);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 37.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 62.0).abs() < 1e-9);
        assert_eq!(tiers[2].name, "monthly");
        assert!((tiers[2].utilization - 100.0).abs() < 1e-9);
    }

    #[test]
    fn opencode_go_zero_percent_drops_placeholder_reset() {
        let body = json!({
            "usage": { "rolling": { "status": "ok", "percent": 0, "resetsAt": "2026-08-26T15:00:00.000Z" } }
        });
        let tiers = parse_opencode_go_tiers(&body);
        assert_eq!(tiers.len(), 1);
        assert!(tiers[0].resets_at.is_none());
    }

    #[test]
    fn opencode_go_legacy_flat_shape_returns_empty() {
        let body = json!({
            "rollingUsage": { "status": "ok", "usagePercent": 37, "resetInSec": 3600 }
        });
        assert!(parse_opencode_go_tiers(&body).is_empty());
    }
}
