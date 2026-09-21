//! Credentials and headers, both directions: which key one upstream request
//! carries, what the upstream is told, and what the client is told back.

use axum::http::{HeaderMap, HeaderValue};

use crate::error::GatewayError;
use crate::router::UpstreamProvider;
use crate::store::Protocol;

/// Hop-by-hop headers never forwarded in either direction.
fn is_hop_by_hop(name: &axum::http::HeaderName) -> bool {
    matches!(
        name.as_str(),
        "connection"
            | "transfer-encoding"
            | "keep-alive"
            | "upgrade"
            | "proxy-authenticate"
            | "proxy-authorization"
            | "te"
            | "trailer"
    )
}

/// Pick the credential for one upstream request: the provider's key pool is
/// `[primary, extras…]`; pools with more than one entry rotate per request
/// (spec §4.1 P1 multi-key rotation, so a single key never trips upstream rate limits).
pub(crate) fn select_upstream_key(
    state: &crate::server::GatewayState,
    provider: &UpstreamProvider,
) -> Result<String, GatewayError> {
    let pool = provider.key_pool();
    if pool.is_empty() {
        return Err(GatewayError::Upstream(format!(
            "provider `{}` has no API key configured",
            provider.id
        )));
    }
    let idx = state.next_key_index(&provider.id, pool.len());
    Ok(pool[idx].to_string())
}

/// Build upstream request headers: copy the inbound set (minus hop-by-hop and
/// local auth headers), then inject the provider's real credential in its
/// native auth style.
pub(crate) fn build_upstream_headers(
    inbound: &HeaderMap,
    provider: &UpstreamProvider,
    key: &str,
) -> Result<HeaderMap, GatewayError> {
    let mut out = HeaderMap::new();
    for (k, v) in inbound {
        if k == axum::http::header::HOST
            || k == axum::http::header::CONTENT_LENGTH
            || k == axum::http::header::AUTHORIZATION
            || k == "x-api-key"
            || k == "x-goog-api-key"
            || is_hop_by_hop(k)
        {
            continue;
        }
        out.insert(k, v.clone());
    }

    match provider.protocol {
        Protocol::Anthropic => {
            let value =
                HeaderValue::from_str(key).map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert("x-api-key", value);
            if !out.contains_key("anthropic-version") {
                out.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
            }
        }
        Protocol::OpenAI => {
            let value = HeaderValue::from_str(&format!("Bearer {key}"))
                .map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert(axum::http::header::AUTHORIZATION, value);
        }
        // The native Gemini API takes its credential in `x-goog-api-key`.
        Protocol::Gemini => {
            let value =
                HeaderValue::from_str(key).map_err(|e| GatewayError::Upstream(e.to_string()))?;
            out.insert("x-goog-api-key", value);
        }
    }

    // Custom per-provider headers (migration v8): merged after credential
    // injection with insert-replace semantics, so they can override the
    // injected credentials and defaults (e.g. an Azure `api-key` or an
    // `OpenAI-Organization`). Framing/hop-by-hop names are refused; invalid
    // names or values are skipped with a warning rather than failing the
    // whole forward.
    if let Some(custom) = &provider.headers {
        for (name, value) in custom {
            let Ok(hn) = axum::http::HeaderName::from_bytes(name.as_bytes()) else {
                tracing::warn!(provider = %provider.id, header = %name, "invalid custom header name; skipping");
                continue;
            };
            if is_hop_by_hop(&hn)
                || hn == axum::http::header::HOST
                || hn == axum::http::header::CONTENT_LENGTH
            {
                continue;
            }
            match HeaderValue::from_str(value) {
                Ok(hv) => {
                    out.insert(hn, hv);
                }
                Err(e) => {
                    tracing::warn!(provider = %provider.id, header = %name, error = %e, "invalid custom header value; skipping");
                }
            }
        }
    }
    Ok(out)
}

/// Copy upstream response headers onto a gateway response, dropping framing
/// headers that hyper/axum will regenerate for the client side.
pub(crate) fn copy_response_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src {
        if k == axum::http::header::CONTENT_LENGTH || is_hop_by_hop(k) {
            continue;
        }
        out.insert(k, v.clone());
    }
    out
}

/// Serialize a response header map for the request log.
///
/// The request side's redaction, not a copy of it: an upstream response is not
/// the safer half of the exchange. `set-cookie` is a live session credential
/// and it reaches this map — `copy_response_headers` only drops framing and
/// hop-by-hop names — so the same rule has to run here. Sharing the one
/// function is what keeps the two sides from drifting apart.
pub(crate) fn response_headers_text(headers: &HeaderMap) -> String {
    crate::log_capture::redact_headers(headers)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::provider;

    fn inbound_headers() -> HeaderMap {
        let mut h = HeaderMap::new();
        h.insert("x-api-key", HeaderValue::from_static("kw-ag-claude-abc"));
        h.insert(
            axum::http::header::AUTHORIZATION,
            HeaderValue::from_static("Bearer kw-ag-claude-abc"),
        );
        h.insert("anthropic-version", HeaderValue::from_static("2023-06-01"));
        h.insert("user-agent", HeaderValue::from_static("claude-code/1.0"));
        h.insert("content-type", HeaderValue::from_static("application/json"));
        h
    }

    #[test]
    fn anthropic_upstream_headers_replace_local_auth() {
        let headers = build_upstream_headers(
            &inbound_headers(),
            &provider(Protocol::Anthropic, None),
            "sk-real-key",
        )
        .unwrap();
        assert_eq!(headers.get("x-api-key").unwrap(), "sk-real-key");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
        assert_eq!(headers.get("user-agent").unwrap(), "claude-code/1.0");
        assert_eq!(headers.get("content-type").unwrap(), "application/json");
        // The placeholder key must never reach the upstream.
        assert!(headers.get(axum::http::header::AUTHORIZATION).is_none());
    }

    #[test]
    fn openai_upstream_headers_use_bearer() {
        let headers = build_upstream_headers(
            &inbound_headers(),
            &provider(Protocol::OpenAI, None),
            "sk-real-key",
        )
        .unwrap();
        assert_eq!(
            headers.get(axum::http::header::AUTHORIZATION).unwrap(),
            "Bearer sk-real-key"
        );
        // x-api-key from the inbound request is stripped, not forwarded.
        assert!(headers.get("x-api-key").is_none());
    }

    #[test]
    fn custom_provider_headers_override_injected_credentials() {
        // A vendor (Azure-style) that wants `api-key` plus an overriding
        // x-api-key: custom headers merge after credential injection.
        let mut p = provider(Protocol::Anthropic, None);
        let mut custom = std::collections::BTreeMap::new();
        custom.insert("api-key".to_string(), "azure-key".to_string());
        custom.insert("x-api-key".to_string(), "override".to_string());
        custom.insert("OpenAI-Organization".to_string(), "org-1".to_string());
        p.headers = Some(custom);

        let headers = build_upstream_headers(&inbound_headers(), &p, "sk-real-key").unwrap();
        assert_eq!(headers.get("x-api-key").unwrap(), "override");
        assert_eq!(headers.get("api-key").unwrap(), "azure-key");
        assert_eq!(headers.get("openai-organization").unwrap(), "org-1");
        assert_eq!(headers.get("anthropic-version").unwrap(), "2023-06-01");
    }

    #[test]
    fn custom_provider_headers_skip_framing_and_invalid() {
        let mut p = provider(Protocol::OpenAI, None);
        let mut custom = std::collections::BTreeMap::new();
        custom.insert("connection".to_string(), "close".to_string());
        custom.insert("host".to_string(), "evil.example.com".to_string());
        custom.insert("content-length".to_string(), "0".to_string());
        custom.insert("x-good".to_string(), "kept".to_string());
        // Header names are case-insensitive per HTTP; an invalid value is skipped.
        custom.insert("x-bad-value".to_string(), "\n\r inject".to_string());
        p.headers = Some(custom);

        let headers = build_upstream_headers(&inbound_headers(), &p, "sk-real-key").unwrap();
        assert_eq!(headers.get("x-good").unwrap(), "kept");
        assert!(headers.get("connection").is_none());
        // host/content-length are never settable via custom headers
        assert_ne!(
            headers.get("host").map(|v| v.to_str().unwrap()),
            Some("evil.example.com")
        );
        assert!(headers.get("x-bad-value").is_none());
    }

    #[test]
    fn missing_provider_key_fails_cleanly() {
        let mut p = provider(Protocol::Anthropic, None);
        p.api_key = None;
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let err = select_upstream_key(&state, &p).unwrap_err();
        assert!(matches!(err, GatewayError::Upstream(_)));
    }

    #[test]
    fn multi_key_pool_rotates_per_request() {
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.extra_keys = vec!["sk-two".into(), "sk-three".into()];

        // Single-key pool always returns the primary key
        let mut single = provider(Protocol::OpenAI, None);
        single.extra_keys.clear();
        for _ in 0..3 {
            assert_eq!(select_upstream_key(&state, &single).unwrap(), "sk-real-key");
        }

        // Multi-key pool: primary key → extra keys rotate in turn, then back to the primary key
        let mut seen = Vec::new();
        for _ in 0..3 {
            seen.push(select_upstream_key(&state, &p).unwrap());
        }
        assert_eq!(seen, vec!["sk-real-key", "sk-two", "sk-three"]);
        assert_eq!(select_upstream_key(&state, &p).unwrap(), "sk-real-key");
    }

    #[test]
    fn response_headers_drop_framing() {
        let mut src = HeaderMap::new();
        src.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        src.insert(
            axum::http::header::CONTENT_LENGTH,
            HeaderValue::from_static("123"),
        );
        src.insert(
            axum::http::header::TRANSFER_ENCODING,
            HeaderValue::from_static("chunked"),
        );
        let out = copy_response_headers(&src);
        assert!(out.get(axum::http::header::CONTENT_TYPE).is_some());
        assert!(out.get(axum::http::header::CONTENT_LENGTH).is_none());
        assert!(out.get(axum::http::header::TRANSFER_ENCODING).is_none());
    }

    /// An upstream response carries credentials of its own — a session cookie
    /// is the common one — and this map is what lands in `response_headers`.
    #[test]
    fn response_headers_are_redacted_like_the_request_side() {
        let mut src = HeaderMap::new();
        src.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("application/json"),
        );
        src.insert(
            axum::http::header::SET_COOKIE,
            HeaderValue::from_static("session=upstream-secret; HttpOnly"),
        );
        src.insert("x-request-id", HeaderValue::from_static("req-7"));

        let json = response_headers_text(&src);
        assert!(!json.contains("upstream-secret"), "{json}");
        assert!(json.contains(r#""set-cookie":"[REDACTED]""#), "{json}");
        assert!(json.contains("application/json"), "{json}");
        assert!(json.contains("req-7"), "{json}");
    }
}
