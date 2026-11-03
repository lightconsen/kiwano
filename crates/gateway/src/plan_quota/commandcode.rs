//! Command Code quota: two rolling windows, plus the month's credit balance.
//!
//! `GET https://api.commandcode.ai/alpha/billing/credits`, Bearer-authenticated
//! with the provider key (`user_…`):
//!
//! ```json
//! {"credits":{"monthlyCredits":48.66,…},
//!  "windowLimits":{"fiveHour":{"used":1.85,"cap":14,"resetAt":1793696247011},
//!                  "weekly":  {"used":20.70,"cap":35,"resetAt":1793934869373}}}
//! ```
//!
//! The windows are **rolling** — they open on the first request and reset five
//! hours / seven days later — and their caps are credit value, so the tiers
//! carry `used`/`limit`/`unit` rather than a percentage alone.
//!
//! The month is a **balance, not a ratio**: the endpoint reports what is left
//! (`credits.monthlyCredits`) and there is no monthly-usage route (`GET
//! /alpha/billing/usage` and friends are 404). Turning that into a percentage
//! would need the plan's grant, which lives in the pricing page rather than in
//! the API — so it rides in the note instead of being invented as a ring.

use crate::plan_quota::http::fold_response;
use crate::plan_quota::json::{extract_reset_time, parse_f64};
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

/// The rolling windows the endpoint reports, and the tier names they map to.
/// `monthly` is deliberately absent: see the module note.
const WINDOWS: [(&str, &str); 2] = [("fiveHour", "five_hour"), ("weekly", "weekly_limit")];

fn parse_commandcode_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    let Some(limits) = body.get("windowLimits") else {
        return Vec::new();
    };
    let mut tiers = Vec::new();
    for (key, tier_name) in WINDOWS {
        let Some(window) = limits.get(key) else {
            continue;
        };
        let (Some(used), Some(cap)) = (
            window.get("used").and_then(parse_f64),
            window.get("cap").and_then(parse_f64),
        ) else {
            continue;
        };
        // A zero cap is a window that is not being enforced (the endpoint has
        // one for accounts with no window limits at all). Reporting "0 of 0" as
        // a ring would read as "no budget left".
        if cap <= 0.0 {
            continue;
        }
        tiers.push(PlanTierVm {
            name: tier_name.to_string(),
            utilization: used / cap * 100.0,
            resets_at: window.get("resetAt").and_then(extract_reset_time),
            used: Some(used),
            limit: Some(cap),
            unit: Some("USD".to_string()),
        });
    }
    tiers
}

/// The month's remaining credit, as the note. `None` when the field is absent
/// (an account the balance endpoint does not describe).
fn monthly_note(body: &serde_json::Value) -> Option<String> {
    let remaining = body
        .get("credits")
        .and_then(|c| c.get("monthlyCredits"))
        .and_then(parse_f64)?;
    Some(format!("{remaining:.2} credits left this month"))
}

pub(crate) async fn query_commandcode(
    client: &reqwest::Client,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let req = client
        .get("https://api.commandcode.ai/alpha/billing/credits")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let body = match fold_response(
        req.send()
            .await
            .map_err(|e| format!("Network error: {e}"))?,
    )
    .await?
    {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };

    let tiers = parse_commandcode_tiers(&body);
    let note = monthly_note(&body);
    if tiers.is_empty() {
        // No window parsed *and* no balance: the shape changed (this is an
        // alpha route) or the account has no plan. Either way, say so rather
        // than render an empty card.
        return Ok(QuotaOutcome::Failed(match body.get("success") {
            Some(v) if v == &serde_json::Value::Bool(false) => body
                .get("message")
                .and_then(|m| m.as_str())
                .unwrap_or("Command Code refused the billing read")
                .to_string(),
            _ => "No usage windows in the response (alpha route may have changed)".to_string(),
        }));
    }
    Ok(QuotaOutcome::Ok { tiers, note })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> serde_json::Value {
        serde_json::json!({
            "credits": {"monthlyCredits": 48.6638129998, "purchasedCredits": 0},
            "windowLimits": {
                "limited": true,
                "fiveHour": {"used": 1.8528, "cap": 14, "exceeded": false,
                             "resetAt": 1_793_696_247_011_i64},
                "weekly": {"used": 20.6965, "cap": 35, "exceeded": false,
                           "resetAt": 1_793_934_869_373_i64}
            }
        })
    }

    #[test]
    fn both_rolling_windows_become_tiers_with_their_amounts() {
        let tiers = parse_commandcode_tiers(&body());
        assert_eq!(tiers.len(), 2);
        let five = &tiers[0];
        assert_eq!(five.name, "five_hour");
        assert!(
            (five.utilization - 13.234).abs() < 0.01,
            "{}",
            five.utilization
        );
        assert_eq!(five.used, Some(1.8528));
        assert_eq!(five.limit, Some(14.0));
        assert_eq!(five.unit.as_deref(), Some("USD"));
        assert!(five.resets_at.is_some(), "the resetAt millis are read");
        assert_eq!(tiers[1].name, "weekly_limit");
    }

    #[test]
    fn a_zero_cap_is_not_a_ring() {
        // The endpoint reports caps of 0 for accounts with no window limits;
        // "0 of 0" would render as a full red ring.
        let v = serde_json::json!({"windowLimits": {"fiveHour": {"used": 0, "cap": 0}}});
        assert!(parse_commandcode_tiers(&v).is_empty());
    }

    #[test]
    fn the_month_is_a_balance_not_a_ratio() {
        assert_eq!(
            monthly_note(&body()).as_deref(),
            Some("48.66 credits left this month")
        );
        // No monthly field: no note, and no month tier invented from nothing.
        assert_eq!(monthly_note(&serde_json::json!({})), None);
        assert!(parse_commandcode_tiers(&body())
            .iter()
            .all(|t| t.name != "monthly"));
    }
}
