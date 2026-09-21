//! Volcengine Agent Plan / Coding Plan.
//!
//! Unlike the Bearer data-plane endpoints in the sibling modules, Volcengine
//! usage lives on the control-plane OpenAPI gateway (`open.volcengineapi.com`)
//! behind Signature V4 (AK/SK) — the inference API key is rejected with 400
//! InvalidAuthorization. Users configure the account AccessKey ID + Secret in
//! the plan-query fields. Detection is automatic: GetAFPUsage (Agent Plan,
//! absolute quotas) first, then GetCodingPlanUsage (Coding Plan, percentages).
//!
//! The submodules are the pieces a query is made of: `sign` builds the
//! credential (a leaf — it reads nothing but the constants below), `transport`
//! makes the one OpenAPI POST and folds the error taxonomy into [`VolcCall`],
//! `parse` reads the two usage payloads, and `query` runs the two-plan probe.
//! The constants below and the region / error helpers after them are what the
//! pieces share.

pub mod parse;
pub mod query;
pub mod sign;
pub mod transport;

pub(crate) use query::query_volcengine;

pub(crate) const VOLCENGINE_OPENAPI_HOST: &str = "open.volcengineapi.com";
pub(crate) const VOLCENGINE_API_VERSION: &str = "2024-01-01";
pub(crate) const VOLCENGINE_DEFAULT_REGION: &str = "cn-beijing";
pub(crate) const VOLCENGINE_SERVICE: &str = "ark";
pub(crate) const VOLCENGINE_CONTENT_TYPE: &str = "application/json; charset=utf-8";
pub(crate) const VOLCENGINE_SIGNED_HEADERS: &str = "host;x-date;x-content-sha256;content-type";
pub(crate) const VOLCENGINE_AKSK_HINT: &str =
    "Check that the AccessKey ID / Secret are correct and the account has Ark usage query (OpenAPI) permission";

pub(crate) enum VolcCall {
    Body(serde_json::Value),
    Auth(String),
    Soft(String),
    Transient(String),
}

/// Extract the control-plane region from the data-plane base_url
/// (`ark.cn-beijing.volces.com` → `cn-beijing`), falling back to cn-beijing.
pub(crate) fn volcengine_region(base_url: &str) -> String {
    let host = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or("");
    host.split('.')
        .find(|p| p.starts_with("cn-") || p.starts_with("ap-"))
        .map(|p| p.to_string())
        .unwrap_or_else(|| VOLCENGINE_DEFAULT_REGION.to_string())
}

/// Auth-class OpenAPI error codes stop the two-plan probe immediately (both
/// plans share the same AK/SK credential).
pub(crate) fn volcengine_is_auth_error_code(code: &str) -> bool {
    let c = code.to_lowercase();
    c.contains("auth")
        || c.contains("signature")
        || c.contains("accessdenied")
        || c.contains("denied")
        || c.contains("unauthorized")
        || c.contains("forbidden")
        || c.contains("credential")
        || c.contains("token")
}

/// Pull `ResponseMetadata.Error` (or top-level `Error`) out of an OpenAPI body.
pub(crate) fn volcengine_response_error(body: &serde_json::Value) -> Option<(String, String)> {
    let err = body
        .get("ResponseMetadata")
        .and_then(|m| m.get("Error"))
        .or_else(|| body.get("Error"))?;
    let code = err.get("Code").and_then(|v| v.as_str()).unwrap_or("");
    let msg = err.get("Message").and_then(|v| v.as_str()).unwrap_or("");
    if code.is_empty() && msg.is_empty() {
        None
    } else {
        Some((code.to_string(), msg.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn volcengine_auth_error_classification() {
        assert!(volcengine_is_auth_error_code("AccessDenied"));
        assert!(volcengine_is_auth_error_code("SignatureDoesNotMatch"));
        assert!(volcengine_is_auth_error_code("InvalidAuthorization"));
        assert!(!volcengine_is_auth_error_code("InvalidParameter.Action"));
        assert!(!volcengine_is_auth_error_code("InternalError"));

        let body = json!({
            "ResponseMetadata": { "Error": { "Code": "AccessDenied", "Message": "no permission" } }
        });
        let (code, msg) = volcengine_response_error(&body).expect("extracts error");
        assert_eq!(code, "AccessDenied");
        assert_eq!(msg, "no permission");
        assert!(volcengine_response_error(&json!({ "ResponseMetadata": {} })).is_none());
    }
}
