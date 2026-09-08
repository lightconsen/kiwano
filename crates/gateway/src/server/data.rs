//! Data plane (:8317): agent clients → upstream providers (tech.md §4.1).
//!
//! Per request: classify path → protocol, attribute the placeholder key to an
//! agent (key map, fallback by path), select the bound provider (single
//! strategy), then forward transparently. The forward leg lives in
//! [`crate::forward`].

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

use crate::protocol::{classify_path, PathProtocol};
use crate::router::resolve_via_engine;
use crate::server::{error_into_response, error_response, GatewayState, MAX_BODY_BYTES};

/// Data-plane router: the paths agents actually call.
pub fn data_plane_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/v1/messages", post(proxy))
        .route("/v1/messages/count_tokens", post(proxy))
        .route("/v1/complete", post(proxy))
        .route("/v1/chat/completions", post(proxy))
        .route("/v1/responses", post(proxy))
        .route("/v1/completions", post(proxy))
        .route("/v1/embeddings", post(proxy))
        .route("/v1/models", get(proxy))
        // Gemini API (Gemini CLI): list models + `{model}:{method}` actions
        // (generateContent / streamGenerateContent / countTokens), all one
        // path segment so a single param route covers them.
        .route("/v1beta/models", get(proxy))
        .route("/v1beta/models/{model_method}", post(proxy))
        .fallback(not_found)
        .with_state(state)
}

async fn proxy(State(state): State<Arc<GatewayState>>, req: Request) -> Response {
    handle(state, req).await
}

async fn not_found() -> Response {
    error_response(
        None,
        StatusCode::NOT_FOUND,
        "unsupported_path",
        "kiwano-gateway: unknown data-plane path",
    )
}

/// Shared request pipeline for POST and GET endpoints.
async fn handle(state: Arc<GatewayState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let inbound_headers = parts.headers.clone();
    let query = parts.uri.query().map(|q| q.to_string());

    let key = crate::server::extract_placeholder_key(&parts.headers);
    let inbound = match classify_path(&path) {
        PathProtocol::Fixed(p) => Some(p),
        PathProtocol::Ambiguous => None, // resolved via agent attribution
        PathProtocol::Unknown => {
            return error_response(
                None,
                StatusCode::NOT_FOUND,
                "unsupported_path",
                &format!("kiwano-gateway: no protocol mapping for path `{path}`"),
            );
        }
    };

    // Read the full body (needed for usage capture and replay upstream).
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            return error_response(
                inbound,
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("kiwano-gateway: failed to read request body: {e}"),
            );
        }
    };

    // Attribution + strategy-engine provider selection (tech.md §4.7).
    let table = state.route_table();
    let session = session_hint(&parts.headers, &body_bytes);
    let routed = match resolve_via_engine(
        &table,
        &state.engine,
        &state.store,
        inbound,
        key.as_deref(),
        session.as_deref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => return error_into_response(e, inbound),
    };

    tracing::info!(
        method = %method,
        path = %path,
        agent = %routed.agent,
        attribution = ?routed.attribution,
        provider = %routed.provider.id,
        body_bytes = body_bytes.len(),
        "routing request"
    );

    // MVP transparent forward requires the provider to speak the inbound
    // protocol (conversion is adapters territory, wired in later phases).
    // Ambiguous paths (GET /v1/models) forward natively to the provider.
    // Query strings are forwarded untouched by the forward leg.
    crate::forward::forward(
        state,
        method,
        path,
        query,
        inbound_headers,
        body_bytes,
        routed,
        inbound,
    )
    .await
}

/// Roundrobin session identity (tech.md §4.7: session-granularity rotation to
/// preserve the upstream prompt cache): the explicit `x-kw-session` header
/// wins, then the Anthropic body's `metadata.user_id`, then body top-level `session_id`.
pub fn session_hint(headers: &axum::http::HeaderMap, body: &[u8]) -> Option<String> {
    if let Some(v) = headers.get("x-kw-session").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.pointer("/metadata/user_id")
        .or_else(|| v.get("session_id"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Build the upstream URL for a provider and inbound path.
pub fn upstream_url(provider: &crate::router::UpstreamProvider, path: &str) -> String {
    let base = provider.base_url.trim_end_matches('/');
    let prefix = provider
        .api_path
        .as_deref()
        .map(|p| p.trim_end_matches('/'))
        .filter(|p| !p.is_empty());
    match prefix {
        Some(p) if !p.starts_with('/') => format!("{base}/{p}{}", normalized(path)),
        Some(p) => format!("{base}{p}{}", normalized(path)),
        None => format!("{base}{}", normalized(path)),
    }
}

fn normalized(path: &str) -> String {
    if path.starts_with('/') {
        path.to_string()
    } else {
        format!("/{path}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::Protocol;

    fn prov(base: &str, api_path: Option<&str>) -> crate::router::UpstreamProvider {
        crate::router::UpstreamProvider {
            id: "p".into(),
            name: "p".into(),
            protocol: Protocol::Anthropic,
            base_url: base.into(),
            api_path: api_path.map(Into::into),
            api_key: None,
            extra_keys: Vec::new(),
            weight: 1,
            win_start: None,
            win_end: None,
        }
    }

    #[test]
    fn session_hint_prefers_header_then_body() {
        let mut h = axum::http::HeaderMap::new();
        h.insert("x-kw-session", axum::http::HeaderValue::from_static("s-1"));
        assert_eq!(
            session_hint(&h, br#"{"metadata":{"user_id":"u-9"}}"#).as_deref(),
            Some("s-1")
        );

        let empty = axum::http::HeaderMap::new();
        assert_eq!(
            session_hint(&empty, br#"{"metadata":{"user_id":"user_acct__session_xyz"}}"#).as_deref(),
            Some("user_acct__session_xyz")
        );
        assert_eq!(
            session_hint(&empty, br#"{"session_id":"abc"}"#).as_deref(),
            Some("abc")
        );
        assert_eq!(session_hint(&empty, b"not json"), None);
        assert_eq!(
            session_hint(&empty, br#"{"metadata":{"user_id":42}}"#),
            None
        );
    }

    #[test]
    fn upstream_url_composition() {
        assert_eq!(
            upstream_url(&prov("https://api.anthropic.com", None), "/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            upstream_url(
                &prov("https://api.deepseek.com/", Some("/anthropic")),
                "/v1/messages"
            ),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        assert_eq!(
            upstream_url(
                &prov("https://api.x.com", Some("anthropic")),
                "/v1/messages"
            ),
            "https://api.x.com/anthropic/v1/messages"
        );
    }
}
