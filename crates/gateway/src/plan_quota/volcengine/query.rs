//! The two-plan probe, in the order the console shows them.

use crate::plan_quota::types::QuotaOutcome;
use crate::plan_quota::volcengine::parse::{parse_afp_tiers, parse_coding_plan_tiers};
use crate::plan_quota::volcengine::transport::volcengine_openapi_call;
use crate::plan_quota::volcengine::{volcengine_region, VolcCall};

pub(crate) async fn query_volcengine(
    client: &reqwest::Client,
    base_url: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> Result<QuotaOutcome, String> {
    let region = volcengine_region(base_url);
    let mut soft_errors: Vec<String> = Vec::new();
    let mut empty_responses: Vec<String> = Vec::new();
    let summarize = |action: &str, body: &serde_json::Value| -> String {
        let raw: String = body.to_string().chars().take(700).collect();
        format!("{action}={raw}")
    };

    // 1) Agent Plan: GetAFPUsage
    match volcengine_openapi_call(
        client,
        &region,
        access_key_id,
        secret_access_key,
        "GetAFPUsage",
    )
    .await
    {
        VolcCall::Auth(detail) => return Ok(QuotaOutcome::Failed(detail)),
        VolcCall::Transient(detail) => return Err(format!("GetAFPUsage: {detail}")),
        VolcCall::Soft(detail) => soft_errors.push(format!("GetAFPUsage: {detail}")),
        VolcCall::Body(body) => {
            let result = body.get("Result").unwrap_or(&body);
            let tiers = parse_afp_tiers(result);
            if !tiers.is_empty() {
                let plan = result
                    .get("PlanType")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("Agent Plan {s}"));
                return Ok(QuotaOutcome::Ok { tiers, note: plan });
            }
            empty_responses.push(summarize("GetAFPUsage", &body));
        }
    }

    // 2) Coding Plan: GetCodingPlanUsage
    match volcengine_openapi_call(
        client,
        &region,
        access_key_id,
        secret_access_key,
        "GetCodingPlanUsage",
    )
    .await
    {
        VolcCall::Auth(detail) => return Ok(QuotaOutcome::Failed(detail)),
        VolcCall::Transient(detail) => return Err(format!("GetCodingPlanUsage: {detail}")),
        VolcCall::Soft(detail) => soft_errors.push(format!("GetCodingPlanUsage: {detail}")),
        VolcCall::Body(body) => {
            let result = body.get("Result").unwrap_or(&body);
            let tiers = parse_coding_plan_tiers(result);
            if !tiers.is_empty() {
                return Ok(QuotaOutcome::Ok {
                    tiers,
                    note: Some("Coding Plan".to_string()),
                });
            }
            empty_responses.push(summarize("GetCodingPlanUsage", &body));
        }
    }

    if !soft_errors.is_empty() {
        Ok(QuotaOutcome::Failed(soft_errors.join("; ")))
    } else if !empty_responses.is_empty() {
        // Signature passed and the request reached the business layer, but no
        // quota could be parsed. Include the raw payloads for diagnosis.
        Ok(QuotaOutcome::Failed(format!(
            "No active plan subscription found (signature passed). Raw response: {}",
            empty_responses.join(" || ")
        )))
    } else {
        Ok(QuotaOutcome::Failed(
            "No active Agent Plan or Coding Plan subscription under these credentials".to_string(),
        ))
    }
}
