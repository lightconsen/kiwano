//! ZenMux quota. The endpoint is user-supplied, so there is no base-url
//! derivation and no parser split — the whole parse is inline here.

use crate::plan_quota::http::fetch_json;
use crate::plan_quota::json::parse_f64;
use crate::plan_quota::types::{PlanTierVm, QuotaOutcome};

pub(crate) async fn query_zenmux(
    client: &reqwest::Client,
    quota_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let req = client
        .get(quota_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let body = match fetch_json(req).await? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(QuotaOutcome::Failed(format!("Endpoint error: {msg}")));
    }
    let Some(data) = body.get("data") else {
        return Ok(QuotaOutcome::Failed(
            "Response is missing the data field".to_string(),
        ));
    };

    let mut tiers = Vec::new();
    for (key, name) in [
        ("quota_5_hour", "five_hour"),
        ("quota_7_day", "weekly_limit"),
    ] {
        let Some(q) = data.get(key) else { continue };
        let usage_pct = q.get("usage_percentage").and_then(parse_f64).unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization: usage_pct * 100.0,
            resets_at: q
                .get("resets_at")
                .and_then(|v| v.as_str())
                .map(String::from),
            used: q.get("used_value_usd").and_then(parse_f64),
            limit: q.get("max_value_usd").and_then(parse_f64),
            unit: Some("USD".to_string()),
        });
    }

    // Plan tier + account status as the note.
    let plan_tier = data
        .get("plan")
        .and_then(|p| p.get("tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let account_status = data
        .get("account_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let note = (!plan_tier.is_empty()).then(|| format!("{plan_tier} ({account_status})"));

    Ok(QuotaOutcome::Ok { tiers, note })
}
