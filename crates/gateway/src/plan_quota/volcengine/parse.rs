//! The two usage payload parsers: `GetAFPUsage`'s absolute quotas and
//! `GetCodingPlanUsage`'s percentages.

use crate::plan_quota::json::{extract_reset_time, parse_f64};
use crate::plan_quota::types::PlanTierVm;

pub(crate) fn parse_afp_tiers(result: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    for (key, name) in [
        ("AFPFiveHour", "five_hour"),
        ("AFPWeekly", "weekly_limit"),
        ("AFPMonthly", "monthly"),
    ] {
        let Some(win) = result.get(key) else { continue };
        let quota = win.get("Quota").and_then(parse_f64).unwrap_or(0.0);
        if quota <= 0.0 {
            continue;
        }
        let used = win.get("Used").and_then(parse_f64).unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization: used / quota * 100.0,
            resets_at: win.get("ResetTime").and_then(extract_reset_time),
            used: Some(used),
            limit: Some(quota),
            unit: None,
        });
    }
    tiers
}

/// Normalize a `GetCodingPlanUsage` window label to a tier name.
fn volcengine_coding_window(label: &str) -> Option<&'static str> {
    match label.to_lowercase().as_str() {
        "session" | "5h" | "fivehour" | "five_hour" | "rolling_5h" => Some("five_hour"),
        "weekly" | "week" | "7d" => Some("weekly_limit"),
        "monthly" | "month" => Some("monthly"),
        _ => None,
    }
}

/// Parse `GetCodingPlanUsage`'s `Result` defensively: the API is not fully
/// documented; fields are matched loosely (`Level` is the real label as of
/// 2026-06, with fallbacks), percent-only values, second-level resets.
pub(crate) fn parse_coding_plan_tiers(result: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let arr = result
        .get("QuotaUsage")
        .and_then(|v| v.as_array())
        .or_else(|| result.get("Usages").and_then(|v| v.as_array()))
        .or_else(|| result.get("Details").and_then(|v| v.as_array()));
    let Some(arr) = arr else { return tiers };

    for item in arr {
        let label = item
            .get("Level")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("Type").and_then(|v| v.as_str()))
            .or_else(|| item.get("Period").and_then(|v| v.as_str()))
            .or_else(|| item.get("Label").and_then(|v| v.as_str()))
            .or_else(|| item.get("Window").and_then(|v| v.as_str()))
            .unwrap_or("");
        let Some(name) = volcengine_coding_window(label) else {
            continue;
        };
        let utilization = item
            .get("Percent")
            .and_then(parse_f64)
            .or_else(|| item.get("UsedPercent").and_then(parse_f64))
            .or_else(|| item.get("UsagePercent").and_then(parse_f64))
            .unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization,
            resets_at: item
                .get("ResetTime")
                .or_else(|| item.get("ResetTimestamp"))
                .and_then(extract_reset_time),
            used: None,
            limit: None,
            unit: None,
        });
    }
    tiers
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn volcengine_afp_three_windows_skip_daily_and_zero_quota() {
        let result = json!({
            "PlanType": "Large",
            "AFPFiveHour": { "Quota": 50.0,  "Used": 12.5,  "ResetTime": 1778806800000_i64 },
            "AFPDaily":    { "Quota": 100.0, "Used": 22.5,  "ResetTime": 1778803200000_i64 },
            "AFPWeekly":   { "Quota": 500.0, "Used": 150.0, "ResetTime": 1779062400000_i64 },
            "AFPMonthly":  { "Quota": 0.0,   "Used": 0.0 }
        });
        let tiers = parse_afp_tiers(&result);
        assert_eq!(tiers.len(), 2, "daily hidden, zero-quota monthly skipped");
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 25.0).abs() < 1e-9);
        assert_eq!(tiers[0].used, Some(12.5));
        assert_eq!(tiers[0].limit, Some(50.0));
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 30.0).abs() < 1e-9);
    }

    #[test]
    fn volcengine_coding_plan_real_shape() {
        let result = json!({
            "Status": "Running",
            "QuotaUsage": [
                { "Level": "session", "Percent": 0.0,      "ResetTimestamp": -1_i64 },
                { "Level": "weekly",  "Percent": 1.672568, "ResetTimestamp": 1782057600_i64 },
                { "Level": "monthly", "Percent": 0.836284, "ResetTimestamp": 1784303999_i64 }
            ]
        });
        let tiers = parse_coding_plan_tiers(&result);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "five_hour");
        assert!(tiers[0].resets_at.is_none(), "ResetTimestamp=-1 → no reset");
        assert_eq!(tiers[1].name, "weekly_limit");
        assert_eq!(tiers[2].name, "monthly");
    }
}
