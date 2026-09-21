//! Zhipu GLM quota, personal and team: the two `TOKENS_LIMIT` windows, whose
//! classification is the most ordering-sensitive code in the module.

use crate::plan_quota::http::fetch_json;
use crate::plan_quota::json::millis_to_iso8601;
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

/// Zhipu `TOKENS_LIMIT` entries are classified by the explicit `unit` field.
enum ZhipuWindow {
    FiveHour,
    Weekly,
}

/// `unit: 3, number: 5` → 5-hour rolling window; `unit: 6` (number 7 or 1,
/// both observed) → weekly window. Unknown/missing units return None and the
/// caller falls back to the reset-time heuristic.
fn classify_zhipu_window(item: &serde_json::Value) -> Option<ZhipuWindow> {
    match item.get("unit").and_then(|v| v.as_i64()) {
        Some(3) => Some(ZhipuWindow::FiveHour),
        Some(6) => Some(ZhipuWindow::Weekly),
        _ => None,
    }
}

/// Parse `data.limits[]` into the two known windows. Explicit `unit`
/// classification wins; entries without a recognized unit fall back to
/// reset-time order (earliest first fills five_hour, then weekly). The
/// reset-time fallback must not override explicit classification: at the end
/// of a weekly cycle the weekly window can reset sooner than the 5-hour one.
fn parse_zhipu_token_tiers(data: &serde_json::Value) -> Vec<PlanTierVm> {
    type Entry = (Option<i64>, f64, Option<String>);
    let mut five_hour: Option<Entry> = None;
    let mut weekly: Option<Entry> = None;
    let mut unclassified: Vec<Entry> = Vec::new();

    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            let limit_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            // Case-insensitive: upstream casing drift must not break detection.
            if !(limit_type.eq_ignore_ascii_case("TOKENS_LIMIT")
                || limit_type.eq_ignore_ascii_case("CREDIT_LIMIT"))
            {
                continue;
            }
            let percentage = item
                .get("percentage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reset_ms = item.get("nextResetTime").and_then(|v| v.as_i64());
            let reset_iso = reset_ms.and_then(millis_to_iso8601);
            let entry = (reset_ms, percentage, reset_iso);
            match classify_zhipu_window(item) {
                Some(ZhipuWindow::FiveHour) if five_hour.is_none() => five_hour = Some(entry),
                Some(ZhipuWindow::Weekly) if weekly.is_none() => weekly = Some(entry),
                _ => unclassified.push(entry),
            }
        }
    }

    unclassified.sort_by_key(|(reset, _, _)| (reset.is_some(), reset.unwrap_or(i64::MIN)));
    for entry in unclassified {
        if five_hour.is_none() {
            five_hour = Some(entry);
        } else if weekly.is_none() {
            weekly = Some(entry);
        }
        // Zhipu currently sends at most two TOKENS_LIMIT rows; extras are dropped.
    }

    let mut tiers = Vec::new();
    for (name, slot) in [("five_hour", five_hour), ("weekly_limit", weekly)] {
        if let Some((_, percentage, resets_at)) = slot {
            tiers.push(PlanTierVm {
                name: name.to_string(),
                utilization: percentage,
                resets_at,
                used: None,
                limit: None,
                unit: None,
            });
        }
    }
    tiers
}

/// Resolve the Zhipu quota host from the configured base_url: bigmodel.cn
/// (cn) and api.z.ai (international) share the same quota path and shape.
fn zhipu_quota_base(base_url: &str) -> &'static str {
    if base_url.to_lowercase().contains("bigmodel.cn") {
        "https://open.bigmodel.cn"
    } else {
        "https://api.z.ai"
    }
}

/// Parse the Zhipu quota body (personal and team share the same shape).
/// No network IO here, so every failure is deterministic.
fn zhipu_outcome(body: &serde_json::Value) -> QuotaOutcome {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return QuotaOutcome::Failed(format!("Endpoint error: {msg}"));
    }
    let Some(data) = body.get("data") else {
        return QuotaOutcome::Failed("Response is missing the data field".to_string());
    };
    let note = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    QuotaOutcome::Ok {
        tiers: parse_zhipu_token_tiers(data),
        note,
    }
}

pub(crate) async fn query_zhipu(
    client: &reqwest::Client,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let url = format!(
        "{}/api/monitor/usage/quota/limit",
        zhipu_quota_base(base_url)
    );
    // Zhipu does NOT use a Bearer prefix.
    let req = client
        .get(&url)
        .header("Authorization", api_key)
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");
    let body = match fetch_json(req).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(zhipu_outcome(&body))
}

/// Zhipu team plan: personal-plan path + `?type=2` plus the
/// bigmodel-organization / bigmodel-project headers (all three credentials
/// required). Team plans only exist on the cn site.
pub(crate) async fn query_zhipu_team(
    client: &reqwest::Client,
    api_key: &str,
    organization_id: &str,
    project_id: &str,
) -> Result<QuotaOutcome, String> {
    let url = "https://open.bigmodel.cn/api/monitor/usage/quota/limit?type=2";
    let req = client
        .get(url)
        .header("Authorization", api_key)
        .header("bigmodel-organization", organization_id)
        .header("bigmodel-project", project_id)
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");
    let body = match fetch_json(req).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(zhipu_outcome(&body))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn zhipu_unit_field_overrides_reset_order() {
        // issue #3036 case: at the end of a weekly cycle the weekly window
        // resets sooner than the 5-hour one; the unit field must win.
        let data = json!({
            "limits": [
                { "type": "TOKENS_LIMIT", "unit": 6, "number": 7, "percentage": 42.0, "nextResetTime": 1_000_003_600_000_i64 },
                { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 1.0,  "nextResetTime": 1_000_018_000_000_i64 }
            ]
        });
        let tiers = parse_zhipu_token_tiers(&data);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 1.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 42.0).abs() < 1e-9);
    }

    #[test]
    fn zhipu_zero_percent_five_hour_has_no_reset_but_stays_five_hour() {
        // The 5-hour bucket at 0% may lack nextResetTime; it must not be
        // re-slotted as the weekly window.
        let data = json!({
            "limits": [
                { "type": "TOKENS_LIMIT", "percentage": 25.0, "nextResetTime": 2_000_000_000_000_i64 },
                { "type": "TOKENS_LIMIT", "percentage": 0.0 }
            ]
        });
        let tiers = parse_zhipu_token_tiers(&data);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!(tiers[0].resets_at.is_none());
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!(tiers[1].resets_at.is_some());
    }

    #[test]
    fn zhipu_quota_base_routes_by_host() {
        assert_eq!(
            zhipu_quota_base("https://open.bigmodel.cn/api/paas/v4"),
            "https://open.bigmodel.cn"
        );
        assert_eq!(
            zhipu_quota_base("https://api.z.ai/api/paas/v4"),
            "https://api.z.ai"
        );
        // Case-insensitive, matching the preset URL handling.
        assert_eq!(
            zhipu_quota_base("HTTPS://OPEN.BIGMODEL.CN/api/paas/v4"),
            "https://open.bigmodel.cn"
        );
        // Unknown hosts default to the international endpoint.
        assert_eq!(
            zhipu_quota_base("https://example.com/zhipu"),
            "https://api.z.ai"
        );
    }

    #[test]
    fn zhipu_business_error_and_missing_data() {
        let failed = zhipu_outcome(&json!({ "success": false, "msg": "bad key" }));
        assert!(matches!(failed, QuotaOutcome::Failed(m) if m.contains("bad key")));
        let missing = zhipu_outcome(&json!({ "success": true }));
        assert!(matches!(missing, QuotaOutcome::Failed(_)));
        let ok = zhipu_outcome(&json!({
            "success": true,
            "data": {
                "level": "max",
                "limits": [
                    { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 26.0 },
                    { "type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 5.0 }
                ]
            }
        }));
        match ok {
            QuotaOutcome::Ok { tiers, note } => {
                assert_eq!(tiers.len(), 2);
                assert_eq!(note.as_deref(), Some("max"));
            }
            _ => panic!("expected ok outcome"),
        }
    }
}
