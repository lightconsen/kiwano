//! Data plane (:8317): agent clients → upstream providers (tech.md §4.1).
//!
//! Per request: classify path → protocol, attribute the placeholder key to an
//! agent (401 if it is missing or unknown — that key is the whole of this
//! plane's inbound auth, see `crate::router::route_agent`), select the bound
//! provider (strategy engine), then forward transparently. The forward leg
//! lives in [`crate::forward`].

use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;

use crate::log_capture::{persist_failure, RequestCapture};
use crate::protocol::{classify_path, PathProtocol};
use crate::router::resolve_via_engine;
use crate::server::{error_into_response, error_response, GatewayState, MAX_BODY_BYTES};

/// Data-plane router: the paths agents actually call, plus the two read-only
/// endpoints a probe or a scraper needs (see [`crate::metrics`] for why those
/// two are the exception to this plane's key gate).
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
        // Gemini API (Gemini CLI): the model list plus the `{model}:{method}`
        // actions (generateContent / streamGenerateContent / countTokens), all
        // one path segment so a single param route covers them.
        .route("/v1beta/models", get(proxy))
        .route("/v1beta/models/{model_method}", post(proxy))
        .route("/health", get(health_probe))
        .route("/metrics", get(metrics))
        .fallback(not_found)
        .with_state(state)
}

async fn proxy(State(state): State<Arc<GatewayState>>, req: Request) -> Response {
    handle(state, req).await
}

/// Liveness for a supervisor: no key, no log row, no usage row.
async fn health_probe(State(state): State<Arc<GatewayState>>) -> Response {
    crate::metrics::health(&state)
}

/// Prometheus scrape: likewise outside the key gate and outside the meter.
///
/// When `KIWANO_METRICS_TOKEN` is configured, the `Authorization: Bearer`
/// bearer decides between the full exposition and a 401. When none is
/// configured the endpoint stays open — a scraper cannot be expected to hold
/// a token before one exists — but the per-agent labels come back redacted, so
/// the labels still aggregate without naming ids (see `crate::metrics`).
async fn metrics(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    let authorized = match state.metrics_token.as_deref() {
        None => true,
        Some(expected) => token_bearer(&headers).is_some_and(|t| constant_time_eq(t, expected)),
    };
    if !authorized {
        return error_response(
            None,
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "kiwanod: /metrics requires the `Authorization: Bearer` metrics token",
        );
    }
    (
        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
        crate::metrics::prometheus(&state, state.metrics_token.is_none()).await,
    )
        .into_response()
}

/// The `Authorization: Bearer <token>` value, when present and well-formed.
fn token_bearer(headers: &HeaderMap) -> Option<&str> {
    let value = headers
        .get(axum::http::header::AUTHORIZATION)?
        .to_str()
        .ok()?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
}

/// Token compare that does not exit on the first differing byte: token
/// comparison is the one place a timing signal is worth the few cycles it costs.
fn constant_time_eq(a: &str, b: &str) -> bool {
    let mut diff = a.len() ^ b.len();
    for (x, y) in a.bytes().zip(b.bytes()) {
        diff |= usize::from(x ^ y);
    }
    diff == 0
}

/// Axum-level fallback: paths no route matches never reach `handle`, so this
/// records the request-log row itself (every data-plane request is audited).
async fn not_found(State(state): State<Arc<GatewayState>>, req: Request) -> Response {
    let (parts, _body) = req.into_parts();
    let path = parts.uri.path().to_string();
    let query = parts.uri.query().map(|q| q.to_string());
    let capture = RequestCapture::start(
        state.log_config().enabled,
        parts.method.as_str(),
        &path,
        query.as_deref(),
        &parts.headers,
    );
    persist_failure(
        &state.store,
        capture.as_ref(),
        None,
        None,
        None,
        StatusCode::NOT_FOUND,
        "unsupported_path",
        "kiwanod: unknown data-plane path".to_string(),
    );
    error_response(
        None,
        StatusCode::NOT_FOUND,
        "unsupported_path",
        "kiwanod: unknown data-plane path",
    )
}

/// Shared request pipeline for POST and GET endpoints.
async fn handle(state: Arc<GatewayState>, req: Request) -> Response {
    let (parts, body) = req.into_parts();
    let method = parts.method.clone();
    let path = parts.uri.path().to_string();
    let inbound_headers = parts.headers.clone();
    let query = parts.uri.query().map(|q| q.to_string());

    // Request-log capture starts here so even rejected requests are recorded.
    let log_cfg = state.log_config();
    let mut capture = RequestCapture::start(
        log_cfg.enabled,
        method.as_str(),
        &path,
        query.as_deref(),
        &inbound_headers,
    );

    let key = crate::server::extract_placeholder_key(&parts.headers);
    let inbound = match classify_path(&path) {
        PathProtocol::Fixed(p) => Some(p),
        PathProtocol::Ambiguous => None, // resolved via agent attribution
        PathProtocol::Unknown => {
            persist_failure(
                &state.store,
                capture.as_ref(),
                None,
                None,
                None,
                StatusCode::NOT_FOUND,
                "unsupported_path",
                format!("kiwanod: no protocol mapping for path `{path}`"),
            );
            return error_response(
                None,
                StatusCode::NOT_FOUND,
                "unsupported_path",
                &format!("kiwanod: no protocol mapping for path `{path}`"),
            );
        }
    };

    // Read the full body (needed for usage capture and replay upstream).
    let body_bytes = match axum::body::to_bytes(body, MAX_BODY_BYTES).await {
        Ok(b) => b,
        Err(e) => {
            persist_failure(
                &state.store,
                capture.as_ref(),
                None,
                None,
                None,
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                format!("kiwanod: failed to read request body: {e}"),
            );
            return error_response(
                inbound,
                StatusCode::PAYLOAD_TOO_LARGE,
                "body_too_large",
                &format!("kiwanod: failed to read request body: {e}"),
            );
        }
    };
    if let Some(c) = capture.as_mut() {
        c.set_body(&body_bytes, log_cfg.max_body_bytes);
    }

    // Attribution + strategy-engine provider selection (tech.md §4.7).
    let table = state.route_table();
    let session = session_hint(&parts.headers, &body_bytes);
    if let Some(c) = capture.as_mut() {
        c.session_id = session.clone();
    }
    // The inbound protocol is passed to `forward` below, not to attribution:
    // it decides which upstream endpoint is spoken to, never which agent pays.
    let plan = match resolve_via_engine(
        &table,
        &state.engine,
        &state.store,
        &state.limits(),
        key.as_deref(),
        session.as_deref(),
    )
    .await
    {
        Ok(r) => r,
        Err(e) => {
            let message = e.to_string();
            let (agent, kind) = match &e {
                crate::error::GatewayError::Unauthorized(_) => (None, "unauthorized"),
                crate::error::GatewayError::NoBinding(a) => (Some(a.clone()), "no_provider_bound"),
                crate::error::GatewayError::AllOverLimit { agent, .. } => {
                    (Some(agent.clone()), "provider_over_limit")
                }
                // Attributed to the agent, because that is who this refusal is
                // about: the store holds no provider for it.
                crate::error::GatewayError::AgentOverLimit { agent, .. } => {
                    (Some(agent.clone()), "agent_over_limit")
                }
                _ => (None, "routing_error"),
            };
            let resp = error_into_response(e, inbound);
            persist_failure(
                &state.store,
                capture.as_ref(),
                agent,
                None,
                None,
                resp.status(),
                kind,
                message,
            );
            return resp;
        }
    };

    // Replay the request down the plan until somebody serves it or the plan runs
    // out. This is the request's own failover: the plan is ordered best-first,
    // and a candidate that answers 408/429/5xx (or whose breaker refuses it) has
    // simply not served this request — the client asked once and is owed one
    // answer, not a list of the gateway's misfortunes.
    //
    // Attempts are logged as they happen, so a failed-over request leaves the
    // failed candidate's row beside the one that served it: that pair is how
    // someone learns their primary is flaking. Usage is untouched by the failed
    // attempt — nothing was metered, so nothing was billed.
    for tried in 0..plan.candidates.len() {
        let last = tried + 1 == plan.candidates.len();
        let attempt = plan.attempt(tried).expect("tried is within the plan");
        let provider_id = attempt.provider.id.clone();

        tracing::info!(
            method = %method,
            path = %path,
            agent = %plan.agent,
            attribution = ?plan.attribution,
            provider = %provider_id,
            attempt = tried + 1,
            body_bytes = body_bytes.len(),
            "routing request"
        );

        // MVP transparent forward requires the provider to speak the inbound
        // protocol (conversion is adapters territory, wired in later phases).
        // Ambiguous paths (GET /v1/models) forward natively to the provider.
        // Query strings are forwarded untouched by the forward leg.
        let response = crate::forward::forward(
            state.clone(),
            method.clone(),
            path.clone(),
            query.clone(),
            inbound_headers.clone(),
            body_bytes.clone(),
            attempt,
            inbound,
            capture.clone(),
        )
        .await;

        if last || !crate::forward::is_retryable_status(response.status()) {
            return response;
        }
        tracing::warn!(
            provider = %provider_id,
            status = %response.status(),
            attempt = tried + 1,
            "provider did not serve the request; failing over to the next candidate"
        );
    }
    unreachable!("the loop returns on its last iteration")
}

/// Roundrobin session identity (tech.md §4.7: session-granularity rotation to
/// preserve the upstream prompt cache): the explicit `x-kw-session` header
/// wins, then the Anthropic body's `metadata.user_id`, then the Responses
/// body's `client_metadata.session_id` (which is what Codex sends), then
/// `prompt_cache_key`, then body top-level `session_id`.
///
/// The last two are not called "session" by the client that sends them, and
/// they are read anyway: a `prompt_cache_key` is the client stating which
/// requests share a cache prefix, which is the whole reason to keep a session
/// on one candidate. Codex sends both and they agree; a client whose
/// `prompt_cache_key` changes per request gets a fresh session each time,
/// which is the behaviour it would have had without any hint at all — so
/// reading it cannot make a client worse off than it is today, only better.
pub fn session_hint(headers: &axum::http::HeaderMap, body: &[u8]) -> Option<String> {
    if let Some(v) = headers.get("x-kw-session").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    let v: serde_json::Value = serde_json::from_slice(body).ok()?;
    v.pointer("/metadata/user_id")
        .or_else(|| v.pointer("/client_metadata/session_id"))
        .or_else(|| v.get("prompt_cache_key"))
        .or_else(|| v.get("session_id"))
        .and_then(|x| x.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(String::from)
}

/// Build the upstream URL for a provider and inbound path.
pub fn upstream_url(provider: &crate::router::UpstreamProvider, path: &str) -> String {
    compose_upstream(&provider.base_url, provider.api_path.as_deref(), path)
}

/// The same composition from its parts, for a caller that has a stored row
/// rather than a route-table candidate — the Apps screen's latency test, which
/// has to reach a provider that is bound to nobody.
///
/// One function so the two cannot compose the URL differently: a test that
/// measured a URL the gateway would never send to would report a latency for a
/// request nobody makes.
pub fn compose_upstream(base_url: &str, api_path: Option<&str>, path: &str) -> String {
    let base = base_url.trim_end_matches('/');
    let prefix = api_path
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
            catalog_id: None,
            protocol: Protocol::Anthropic,
            base_url: base.into(),
            api_path: api_path.map(Into::into),
            endpoints: Vec::new(),
            api_key: None,
            extra_keys: Vec::new(),
            weight: 1,
            win_start: None,
            win_end: None,
            timeout_secs: None,
            retries: None,
            headers: None,
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
            session_hint(
                &empty,
                br#"{"metadata":{"user_id":"user_acct__session_xyz"}}"#
            )
            .as_deref(),
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

        // Codex nests its conversation id in `client_metadata`, and sends the
        // same value as `prompt_cache_key` at the top level. Neither spelling
        // was read, so every Codex request was a session of one — the log
        // columns were null and roundrobin could not keep a conversation on
        // the candidate whose prompt cache it had warmed.
        assert_eq!(
            session_hint(
                &empty,
                br#"{"client_metadata":{"session_id":"01a0838a-6ce7","turn_id":"t1"},"prompt_cache_key":"01a0838a-6ce7"}"#
            )
            .as_deref(),
            Some("01a0838a-6ce7")
        );
        // …and the cache key alone is enough when that is all a client sends:
        // it is the client saying which requests share a prefix.
        assert_eq!(
            session_hint(&empty, br#"{"prompt_cache_key":"ck-7"}"#).as_deref(),
            Some("ck-7")
        );
        // A client that sends neither is exactly where it was before.
        assert_eq!(session_hint(&empty, br#"{"model":"gpt-5.1"}"#), None);
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
