//! Full request logging (bodies + metadata) for the data plane.
//!
//! This is a local tool: every data-plane request is captured to
//! `request_logs` + `request_bodies` unless the user turns capture off
//! (`LogConfig`, reloaded from `gateway_settings` alongside the route
//! table). Credential headers are redacted; bodies are capped per body
//! and stored lossy-UTF8 (they are JSON/SSE text in practice).

use axum::http::{HeaderMap, StatusCode};

use crate::store::{now_rfc3339, RequestLogNew, Store};

/// Header names never written to the log (credentials / cookies).
const REDACTED_HEADERS: &[&str] = &[
    "authorization",
    "x-api-key",
    "x-goog-api-key",
    "cookie",
    "proxy-authorization",
];

/// Serialize headers as JSON with credential headers dropped.
fn redact_headers(headers: &HeaderMap) -> String {
    let mut map = serde_json::Map::new();
    for (name, value) in headers.iter() {
        if REDACTED_HEADERS.contains(&name.as_str()) {
            continue;
        }
        let v = value
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|_| "<binary>".to_string());
        map.insert(name.as_str().to_string(), serde_json::Value::String(v));
    }
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

/// Request-side capture, created once per request in `handle()` and carried
/// through the forward leg; `None` while logging is disabled.
#[derive(Debug, Clone)]
pub struct RequestCapture {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub session_id: Option<String>,
    pub request_headers: Option<String>,
    /// lossy-UTF8 body, truncated to `max_body_bytes` when capture_bodies is on.
    pub request_body: Option<String>,
    pub request_size: i64,
    pub truncated: bool,
}

impl RequestCapture {
    /// Start a capture for an inbound request; `None` when disabled.
    pub fn start(
        enabled: bool,
        method: &str,
        path: &str,
        query: Option<&str>,
        headers: &HeaderMap,
    ) -> Option<Self> {
        if !enabled {
            return None;
        }
        Some(RequestCapture {
            ts: now_rfc3339(),
            method: method.to_string(),
            path: path.to_string(),
            query: query.map(str::to_string),
            session_id: None,
            request_headers: Some(redact_headers(headers)),
            request_body: None,
            request_size: 0,
            truncated: false,
        })
    }

    /// Attach the (possibly truncated) request body when body capture is on.
    /// The original byte size is always recorded.
    pub fn set_body(&mut self, body: &[u8], capture_bodies: bool, max_body_bytes: usize) {
        self.request_size = body.len() as i64;
        if !capture_bodies {
            return;
        }
        self.truncated = body.len() > max_body_bytes;
        let capped = &body[..body.len().min(max_body_bytes)];
        self.request_body = Some(String::from_utf8_lossy(capped).into_owned());
    }
}

/// Truncate response bytes to the capture cap and lossy-decode them.
pub fn cap_body(bytes: &[u8], max_body_bytes: usize) -> (String, bool) {
    let truncated = bytes.len() > max_body_bytes;
    let capped = &bytes[..bytes.len().min(max_body_bytes)];
    (String::from_utf8_lossy(capped).into_owned(), truncated)
}

/// Persist a request that failed before/inside the forward leg (no usage).
/// Best-effort: logging problems never change the client response.
#[allow(clippy::too_many_arguments)]
pub fn persist_failure(
    store: &Store,
    capture: Option<&RequestCapture>,
    agent: Option<String>,
    attribution: Option<String>,
    provider_id: Option<String>,
    status: StatusCode,
    error_kind: &str,
    error_message: String,
) {
    let Some(c) = capture else {
        return;
    };
    let record = RequestLogNew {
        ts: c.ts.clone(),
        method: c.method.clone(),
        path: c.path.clone(),
        query: c.query.clone(),
        agent,
        attribution,
        provider_id,
        model: None,
        status_code: status.as_u16() as i64,
        error_kind: Some(error_kind.to_string()),
        error_message: Some(error_message),
        session_id: c.session_id.clone(),
        is_streaming: false,
        input_tokens: 0,
        output_tokens: 0,
        cache_read_tokens: 0,
        cache_creation_tokens: 0,
        latency_ms: None,
        first_token_ms: None,
        request_headers: c.request_headers.clone(),
        response_headers: None,
        request_body: c.request_body.clone(),
        response_body: None,
        request_size: c.request_size,
        response_size: 0,
        truncated: c.truncated,
        // Pre-forward failures carry no token usage and hence no cost.
        cost: None,
        cost_currency: None,
    };
    if let Err(e) = store.insert_request_log(&record) {
        tracing::warn!(error = %e, "failed to persist request log");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redact_headers_drops_credentials() {
        let mut headers = HeaderMap::new();
        headers.insert("x-api-key", "kw-ag-claude-secret".parse().unwrap());
        headers.insert(
            axum::http::header::AUTHORIZATION,
            "Bearer sk-upstream".parse().unwrap(),
        );
        headers.insert("content-type", "application/json".parse().unwrap());
        headers.insert("user-agent", "claude-cli/1.0".parse().unwrap());

        let json = redact_headers(&headers);
        assert!(json.contains("content-type"));
        assert!(json.contains("user-agent"));
        assert!(!json.contains("kw-ag-claude-secret"));
        assert!(!json.contains("sk-upstream"));
        assert!(!json.to_lowercase().contains("authorization"));
    }

    #[test]
    fn set_body_respects_cap_and_records_size() {
        let mut c = RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new())
            .expect("capture");
        c.set_body(b"hello", true, 3);
        assert_eq!(c.request_body.as_deref(), Some("hel"));
        assert!(c.truncated);
        assert_eq!(c.request_size, 5);

        // Body capture off: size recorded, text omitted.
        let mut c = RequestCapture::start(true, "POST", "/v1/messages", None, &HeaderMap::new())
            .unwrap();
        c.set_body(b"hello", false, 1024);
        assert_eq!(c.request_body, None);
        assert!(!c.truncated);
        assert_eq!(c.request_size, 5);

        // Logging disabled → no capture at all.
        assert!(RequestCapture::start(false, "POST", "/v1/messages", None, &HeaderMap::new()).is_none());
    }
}
