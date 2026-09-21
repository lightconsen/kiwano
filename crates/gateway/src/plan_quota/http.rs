//! The HTTP plumbing every adapter builds on: one 15-second client, the
//! GET-and-fold helper, and the fold that tells transient transport failures
//! (outer `Err`) from deterministic ones (inner `Err`).

/// Fold a received HTTP response: outer `Err` = transient (network), inner
/// `Err` = deterministic failure with a user-facing message.
pub(crate) async fn fold_response(
    resp: reqwest::Response,
) -> Result<Result<serde_json::Value, String>, String> {
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(Err(format!(
            "Auth failed (HTTP {status}): the API key is invalid or expired"
        )));
    }
    if !status.is_success() {
        let body = resp.text().await.unwrap_or_default();
        return Ok(Err(format!("Endpoint error (HTTP {status}): {body}")));
    }
    // Read the whole body before parsing: a read failure is transient (outer
    // Err), a parse failure of a complete body is deterministic.
    let raw = resp
        .text()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    match serde_json::from_str(&raw) {
        Ok(v) => Ok(Ok(v)),
        Err(e) => Ok(Err(format!("Failed to parse the response: {e}"))),
    }
}

/// GET a JSON endpoint and fold transport errors.
pub(crate) async fn fetch_json(
    req: reqwest::RequestBuilder,
) -> Result<Result<serde_json::Value, String>, String> {
    let resp = req
        .send()
        .await
        .map_err(|e| format!("Network error: {e}"))?;
    fold_response(resp).await
}

pub(crate) fn client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}
