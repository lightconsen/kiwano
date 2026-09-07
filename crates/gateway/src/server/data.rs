//! Data plane (:8317): agent clients → upstream providers (tech.md §4.1).
//!
//! Per request: classify path → protocol, attribute the placeholder key to an
//! agent (key map, fallback by path), select the bound provider (single
//! strategy), then forward transparently. The forward leg lives in
//! [`crate::forward`].

use axum::body::Bytes;
use axum::extract::{Request, State};
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

use crate::protocol::{classify_path, PathProtocol};
use crate::router::resolve;
use crate::server::{error_into_response, error_response, GatewayState, MAX_BODY_BYTES};
use crate::store::Protocol;

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

    // Attribution + single-strategy provider selection.
    let table = state.route_table();
    let routed = match resolve(&table, inbound, key.as_deref()) {
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
    // protocol (conversion is cc-adapters territory, wired in later phases).
    // Ambiguous paths (GET /v1/models) forward natively to the provider.
    // Query strings are forwarded untouched by the forward leg.

    forward_or_stub(state, method, path, query, inbound_headers, body_bytes, routed, inbound).await
}

/// Forward leg. Committed as a skeleton in the routing commit and replaced by
/// the reqwest forward + SSE passthrough module.
async fn forward_or_stub(
    _state: Arc<GatewayState>,
    _method: Method,
    _path: String,
    _query: Option<String>,
    _headers: HeaderMap,
    _body: Bytes,
    routed: crate::router::RoutedRequest,
    inbound: Option<Protocol>,
) -> Response {
    let _ = routed;
    error_response(
        inbound,
        StatusCode::NOT_IMPLEMENTED,
        "not_implemented",
        "kiwano-gateway: upstream forwarding lands with the proxy module",
    )
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
        }
    }

    #[test]
    fn upstream_url_composition() {
        assert_eq!(
            upstream_url(&prov("https://api.anthropic.com", None), "/v1/messages"),
            "https://api.anthropic.com/v1/messages"
        );
        assert_eq!(
            upstream_url(&prov("https://api.deepseek.com/", Some("/anthropic")), "/v1/messages"),
            "https://api.deepseek.com/anthropic/v1/messages"
        );
        assert_eq!(
            upstream_url(&prov("https://api.x.com", Some("anthropic")), "/v1/messages"),
            "https://api.x.com/anthropic/v1/messages"
        );
    }
}
