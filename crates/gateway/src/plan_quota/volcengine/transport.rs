//! The control-plane call itself: one signed POST, with the OpenAPI error
//! envelope folded into [`VolcCall`] so the two-plan probe can tell an auth
//! failure (stop) from a soft one (try the next plan).

use crate::plan_quota::volcengine::sign::{volcengine_canonical_query, volcengine_sign};
use crate::plan_quota::volcengine::{
    volcengine_is_auth_error_code, volcengine_response_error, VolcCall, VOLCENGINE_AKSK_HINT,
    VOLCENGINE_CONTENT_TYPE, VOLCENGINE_OPENAPI_HOST,
};

pub(crate) async fn volcengine_openapi_call(
    client: &reqwest::Client,
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    action: &str,
) -> VolcCall {
    let canonical_query = volcengine_canonical_query(action, region);
    let url = format!("https://{VOLCENGINE_OPENAPI_HOST}/?{canonical_query}");
    let body: &[u8] = b"";
    let (authorization, x_date, x_content_sha256) = volcengine_sign(
        access_key_id,
        secret_access_key,
        region,
        &canonical_query,
        body,
        chrono::Utc::now(),
    );

    let resp = client
        .post(&url)
        .header("X-Date", x_date)
        .header("X-Content-Sha256", x_content_sha256)
        .header("Content-Type", VOLCENGINE_CONTENT_TYPE)
        .header("Authorization", authorization)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await;
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return VolcCall::Transient(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return VolcCall::Auth(format!(
            "Auth failed (HTTP {status}). {VOLCENGINE_AKSK_HINT}"
        ));
    }
    if !status.is_success() {
        // The gateway returns 4xx (often 400) with the same
        // ResponseMetadata.Error envelope as the 200 path for signature /
        // credential errors — parse it so a rejected key surfaces as an
        // auth error with the AK/SK hint, not a generic API error.
        let raw = resp.text().await.unwrap_or_default();
        if let Ok(body) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some((code, msg)) = volcengine_response_error(&body) {
                if volcengine_is_auth_error_code(&code) {
                    return VolcCall::Auth(format!(
                        "Auth failed (HTTP {status}, {code}): {msg}. {VOLCENGINE_AKSK_HINT}"
                    ));
                }
                return VolcCall::Soft(format!("Endpoint error (HTTP {status}, {code}): {msg}"));
            }
        }
        return VolcCall::Soft(format!("Endpoint error (HTTP {status}): {raw}"));
    }

    let raw = match resp.text().await {
        Ok(b) => b,
        Err(e) => return VolcCall::Transient(format!("Network error: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return VolcCall::Soft(format!("Failed to parse the response: {e}")),
    };

    // Business errors arrive as 200 + ResponseMetadata.Error.
    if let Some((code, msg)) = volcengine_response_error(&body) {
        if volcengine_is_auth_error_code(&code) {
            return VolcCall::Auth(format!(
                "Auth failed ({code}): {msg}. {VOLCENGINE_AKSK_HINT}"
            ));
        }
        return VolcCall::Soft(format!("Endpoint error ({code}): {msg}"));
    }
    VolcCall::Body(body)
}
