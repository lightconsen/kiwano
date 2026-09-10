//! Transparent forwarding with SSE passthrough + usage capture (tech.md §4.3).
//!
//! Non-SSE upstream responses are buffered and metered inline. SSE responses
//! are piped through byte-for-byte while a scanning stream watches the events
//! for usage fields; the metered sample is persisted after the stream ends.
//!
//! Protocol conversion (tech.md §4.3 / adapters phase): an Anthropic
//! inbound request (`POST /v1/messages`) bound to an OpenAI-compatible
//! provider is converted with the `kiwano-adapters` sublayer — request via
//! `anthropic_to_openai` (model via `model_mapper`), response (JSON + SSE)
//! back via `openai_to_anthropic` / the streaming converter — and metered
//! from the upstream OpenAI usage fields.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use kiwano_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_adapters::proxy::providers::streaming::create_anthropic_sse_stream;
use kiwano_adapters::proxy::providers::transform::{
    anthropic_to_openai, inject_openai_stream_include_usage, openai_to_anthropic,
};

use crate::error::GatewayError;
use crate::log_capture::{cap_body, RequestCapture};
use crate::meter::{parse_response_usage, request_model, Usage, UsageScanner};
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::data::upstream_url;
use crate::server::{error_into_response, error_response, GatewayState};
use crate::store::now_rfc3339;
use crate::store::{Protocol, RequestLogNew, UsageRecord};

type BoxError = Box<dyn std::error::Error + Send + Sync>;

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

/// Response/request facts for the full request log, gathered by the forward
/// leg and persisted together with the metered usage sample.
#[derive(Clone)]
struct CompletedLog {
    capture: RequestCapture,
    attribution: String,
    status_code: u16,
    error_kind: Option<String>,
    error_message: Option<String>,
    is_streaming: bool,
    first_token_ms: Option<i64>,
    /// Client-visible response body (converted stream for the Anthropic path).
    response_body: Option<String>,
    response_size: i64,
    truncated: bool,
    response_headers: Option<String>,
}

/// One metered request, ready for the `usage` table (+ full request log).
#[derive(Clone)]
struct UsageSample {
    agent: String,
    provider_id: String,
    model: Option<String>,
    usage: Usage,
    latency_ms: i64,
    status: &'static str,
    /// Input-token semantics of the outbound protocol: openai/gemini
    /// `input_tokens` already contain the cache buckets (deducted before
    /// billing); anthropic reports fresh input only.
    cache_inclusive: bool,
    /// Full-log payload; None while request logging is disabled.
    log: Option<CompletedLog>,
}

impl UsageSample {
    fn into_record(self, cost: Option<f64>, cost_currency: Option<String>) -> UsageRecord {
        UsageRecord {
            ts: now_rfc3339(),
            agent: self.agent,
            provider_id: self.provider_id,
            model: self.model,
            input_tokens: self.usage.input_tokens,
            output_tokens: self.usage.output_tokens,
            cache_read_tokens: self.usage.cache_read_tokens,
            cache_creation_tokens: self.usage.cache_creation_tokens,
            latency_ms: Some(self.latency_ms),
            status: self.status.to_string(),
            cost,
            cost_currency,
        }
    }
}

/// Resolve the sample's price and compute its cost in the price entry's
/// currency. Unpriced / unknown models yield `(None, None)` (row keeps
/// NULL cost).
fn compute_sample_cost(
    state: &GatewayState,
    sample: &UsageSample,
) -> (Option<f64>, Option<String>) {
    let Some(model) = sample.model.as_deref().filter(|m| !m.is_empty()) else {
        return (None, None);
    };
    let pricing = state.pricing.read().expect("pricing lock poisoned");
    let Some(entry) = pricing.find(model) else {
        return (None, None);
    };
    let cost = kiwano_adapters::model_pricing::compute_cost(
        entry,
        sample.usage.input_tokens.max(0) as u64,
        sample.usage.output_tokens.max(0) as u64,
        sample.usage.cache_read_tokens.max(0) as u64,
        sample.usage.cache_creation_tokens.max(0) as u64,
        sample.cache_inclusive,
    );
    (cost, Some(entry.currency.clone()))
}

/// Pick the credential for one upstream request: the provider's key pool is
/// `[primary, extras…]`; pools with more than one entry rotate per request
/// (spec §4.1 P1 multi-key rotation, so a single key never trips upstream rate limits).
fn select_upstream_key(
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
fn build_upstream_headers(
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

/// Whether an upstream status warrants a same-provider retry (migration v8):
/// request timeout, rate limiting, and server-side errors. Client errors
/// (4xx besides 408/429) are deterministic — retrying cannot help.
fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// A failed send attempt: either the per-provider response-header timeout
/// elapsed or the transport itself broke (connect/DNS/reset).
enum SendFailure {
    Timeout(u64),
    Transport(reqwest::Error),
}

impl SendFailure {
    fn message(&self, url: &str) -> String {
        match self {
            SendFailure::Timeout(secs) => format!("request to `{url}` timed out after {secs}s"),
            SendFailure::Transport(e) => format!("request to `{url}` failed: {e}"),
        }
    }
}

/// Send one request upstream with the provider's advanced forwarding settings
/// (migration v8): a per-provider timeout bounding the wait for response
/// headers, and same-provider retries for retryable failures.
///
/// The timeout wraps the `send()` future rather than using
/// `RequestBuilder::timeout()` on purpose: reqwest's per-request timeout is a
/// total deadline that includes the body read and would abort long SSE
/// streams. Wrapping `send()` bounds only time-to-response-headers; body
/// reads stay under the client-wide 300s read timeout.
///
/// Each attempt records breaker feedback exactly once (success is judged at
/// response-header time, tech.md §4.7). A retryable outcome (transport error,
/// 408/429/5xx) with attempts left backs off briefly and re-sends the same
/// request to the same provider; the strategy layer only takes over after
/// this loop gives up. The loop returns before any byte reaches the client,
/// so in-flight streams are never retried; body-read failures after headers
/// are also not retried (the breaker was already fed at header time).
#[allow(clippy::too_many_arguments)] // every argument is a distinct routing input
async fn send_upstream(
    state: &GatewayState,
    provider: &UpstreamProvider,
    agent: &str,
    attribution: &str,
    method: Method,
    url: &str,
    headers: HeaderMap,
    body: Bytes,
    inbound: Option<Protocol>,
    capture: Option<&RequestCapture>,
) -> Result<reqwest::Response, Response> {
    let attempts = provider.retries.map_or(1, |r| r as usize + 1);
    for attempt in 1..=attempts {
        let last = attempt == attempts;
        let send = state
            .http
            .request(method.clone(), url)
            .headers(headers.clone())
            .body(body.clone())
            .send();
        let outcome: Result<reqwest::Response, SendFailure> = match provider.timeout_secs {
            Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), send).await {
                Ok(r) => r.map_err(SendFailure::Transport),
                Err(_) => Err(SendFailure::Timeout(secs)),
            },
            None => send.await.map_err(SendFailure::Transport),
        };

        match outcome {
            Ok(upstream) => {
                let status = upstream.status();
                state
                    .engine
                    .record(agent, &provider.id, status.is_success())
                    .await;
                if !last && is_retryable_status(status) {
                    tracing::warn!(
                        attempt,
                        status = status.as_u16(),
                        provider = %provider.id,
                        "retryable upstream status; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis((300 * attempt as u64).min(2000)))
                        .await;
                    continue;
                }
                return Ok(upstream);
            }
            Err(failure) => {
                let message = failure.message(url);
                state.engine.record(agent, &provider.id, false).await;
                if !last {
                    tracing::warn!(
                        attempt,
                        provider = %provider.id,
                        error = %message,
                        "upstream attempt failed; retrying"
                    );
                    tokio::time::sleep(Duration::from_millis((300 * attempt as u64).min(2000)))
                        .await;
                    continue;
                }
                let resp = error_into_response(GatewayError::Upstream(message.clone()), inbound);
                crate::log_capture::persist_failure(
                    &state.store,
                    capture,
                    Some(agent.to_string()),
                    Some(attribution.to_string()),
                    Some(provider.id.clone()),
                    resp.status(),
                    "upstream_error",
                    message,
                );
                return Err(resp);
            }
        }
    }
    unreachable!("retry loop always returns or continues")
}

/// Copy upstream response headers onto a gateway response, dropping framing
/// headers that hyper/axum will regenerate for the client side.
fn copy_response_headers(src: &HeaderMap) -> HeaderMap {
    let mut out = HeaderMap::new();
    for (k, v) in src {
        if k == axum::http::header::CONTENT_LENGTH || is_hop_by_hop(k) {
            continue;
        }
        out.insert(k, v.clone());
    }
    out
}

/// How the agent was attributed, as stored in `request_logs.attribution`.
fn attribution_str(a: crate::router::Attribution) -> String {
    match a {
        crate::router::Attribution::PlaceholderKey => "key".to_string(),
        crate::router::Attribution::PathFallback => "path_fallback".to_string(),
    }
}

/// Endpoint/protocol resolution for one inbound request (multi-protocol
/// providers, migration v7).
enum InboundResolution {
    /// Provider's own protocol (or ambiguous inbound): native forward.
    Native,
    /// A registered per-protocol endpoint matches: forward natively as that
    /// protocol via the endpoint's URL (headers + metering follow it).
    Alternate {
        protocol: Protocol,
        base_url: String,
        api_path: Option<String>,
    },
    /// Anthropic `/v1/messages` inbound on an OpenAI provider without an
    /// Anthropic endpoint: convert via the adapters sublayer.
    ConvertAnthropicToOpenAI,
    /// No endpoint and no conversion: fail cleanly with `protocol_mismatch`.
    Mismatch { message: String },
}

fn resolve_inbound(
    provider: &UpstreamProvider,
    inbound: Option<Protocol>,
    path: &str,
) -> InboundResolution {
    let Some(inbound_proto) = inbound else {
        return InboundResolution::Native;
    };
    if inbound_proto == provider.protocol {
        return InboundResolution::Native;
    }
    if let Some(e) = provider.endpoint_for(inbound_proto) {
        return InboundResolution::Alternate {
            protocol: inbound_proto,
            base_url: e.base_url.clone(),
            api_path: e.api_path.clone(),
        };
    }
    if inbound_proto == Protocol::Anthropic
        && provider.protocol == Protocol::OpenAI
        && path == "/v1/messages"
    {
        return InboundResolution::ConvertAnthropicToOpenAI;
    }
    InboundResolution::Mismatch {
        message: format!(
            "provider `{}` speaks `{}` but path `{}` is `{}`; no `{}` endpoint is configured on the provider and only Anthropic `/v1/messages` -> OpenAI conversion is supported",
            provider.id,
            provider.protocol.as_str(),
            path,
            inbound_proto.as_str(),
            inbound_proto.as_str()
        ),
    }
}

/// Forward one resolved request to its provider and return the client-facing
/// response, metering usage on the way. `capture` carries the request-side
/// full-log context (None while request logging is disabled).
#[allow(clippy::too_many_arguments)] // entry point: the request's own fields
pub async fn forward(
    state: Arc<GatewayState>,
    method: Method,
    path: String,
    query: Option<String>,
    inbound_headers: HeaderMap,
    body: Bytes,
    routed: RoutedRequest,
    inbound: Option<Protocol>,
    capture: Option<RequestCapture>,
) -> Response {
    let started = Instant::now();
    let provider = &routed.provider;
    let mut log = capture.map(|c| CompletedLog {
        capture: c,
        attribution: attribution_str(routed.attribution),
        status_code: 0,
        error_kind: None,
        error_message: None,
        is_streaming: false,
        first_token_ms: None,
        response_body: None,
        response_size: 0,
        truncated: false,
        response_headers: None,
    });

    // Resolve the endpoint + protocol for the inbound flavor: native when it
    // matches the provider (or the path is ambiguous), natively via a
    // registered per-protocol endpoint when one exists, otherwise the legacy
    // Anthropic → OpenAI conversion, or a clean mismatch failure.
    let provider_for_alt;
    let provider: &UpstreamProvider = match resolve_inbound(provider, inbound, &path) {
        InboundResolution::Native => provider,
        InboundResolution::Alternate {
            protocol,
            base_url,
            api_path,
        } => {
            provider_for_alt = UpstreamProvider {
                protocol,
                base_url,
                api_path,
                ..provider.clone()
            };
            &provider_for_alt
        }
        InboundResolution::ConvertAnthropicToOpenAI => {
            return forward_anthropic_via_openai(
                state,
                method,
                inbound_headers,
                body,
                routed,
                inbound,
                started,
                log,
            )
            .await;
        }
        InboundResolution::Mismatch { message } => {
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                StatusCode::BAD_GATEWAY,
                "protocol_mismatch",
                message.clone(),
            );
            return error_response(
                inbound,
                StatusCode::BAD_GATEWAY,
                "protocol_mismatch",
                &message,
            );
        }
    };

    let mut url = upstream_url(provider, &path);
    if let Some(q) = &query {
        url.push('?');
        url.push_str(q);
    }

    let api_key = match select_upstream_key(&state, provider) {
        Ok(k) => k,
        Err(e) => {
            let resp = error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "upstream_error",
                "selecting upstream key failed".to_string(),
            );
            return resp;
        }
    };
    let headers = match build_upstream_headers(&inbound_headers, provider, &api_key) {
        Ok(h) => h,
        Err(e) => {
            let resp = error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "upstream_error",
                "building upstream headers failed".to_string(),
            );
            return resp;
        }
    };

    let upstream = match send_upstream(
        &state,
        provider,
        &routed.agent,
        &attribution_str(routed.attribution),
        method,
        &url,
        headers,
        body.clone(),
        inbound,
        log.as_ref().map(|l| &l.capture),
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let status = upstream.status();
    let is_sse = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    tracing::info!(
        url = %url,
        provider = %provider.id,
        agent = %routed.agent,
        upstream_status = status.as_u16(),
        sse = is_sse,
        "upstream responded"
    );

    let model = request_model(&body);

    // reqwest consumes the Response on bytes()/bytes_stream(), so snapshot
    // the client-facing headers first.
    let response_headers = copy_response_headers(upstream.headers());
    if let Some(l) = log.as_mut() {
        l.response_headers = Some(response_headers_text(&response_headers));
    }

    if is_sse {
        // Streaming passthrough with usage scanning + response capture; the
        // sample is persisted by a side task when the stream finishes.
        let (tx, rx) = mpsc::channel::<UsageSample>(1);
        tokio::spawn(record_pending_usage(state.clone(), rx));
        let max_body_bytes = log
            .as_ref()
            .map(|_| state.log_config().max_body_bytes)
            .unwrap_or(0);
        let stream = SseUsageStream::new(
            Box::pin(upstream.bytes_stream().map(|r| r.map_err(BoxError::from))),
            tx,
            UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                model,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                cache_inclusive: matches!(provider.protocol, Protocol::OpenAI | Protocol::Gemini),
                log: log.map(|l| CompletedLog {
                    is_streaming: true,
                    status_code: status.as_u16(),
                    response_headers: l.response_headers,
                    capture: l.capture,
                    attribution: l.attribution,
                    error_kind: None,
                    error_message: None,
                    first_token_ms: None,
                    response_body: None,
                    response_size: 0,
                    truncated: false,
                }),
            },
            started,
            max_body_bytes,
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    } else {
        let bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let resp = error_into_response(
                    GatewayError::Upstream(format!("reading upstream body failed: {e}")),
                    inbound,
                );
                crate::log_capture::persist_failure(
                    &state.store,
                    log.as_ref().map(|l| &l.capture),
                    Some(routed.agent.clone()),
                    Some(attribution_str(routed.attribution)),
                    Some(provider.id.clone()),
                    resp.status(),
                    "upstream_error",
                    format!("reading upstream body failed: {e}"),
                );
                return resp;
            }
        };
        let latency_ms = started.elapsed().as_millis() as i64;
        let (usage, upstream_model) = parse_response_usage(provider.protocol, &bytes);
        let log = log.map(|mut l| {
            let max_body_bytes = state.log_config().max_body_bytes;
            let (response_body, truncated) = cap_body(&bytes, max_body_bytes);
            l.response_body = Some(response_body);
            l.response_size = bytes.len() as i64;
            l.truncated = truncated;
            l.status_code = status.as_u16();
            l
        });
        let sample = UsageSample {
            agent: routed.agent.clone(),
            provider_id: provider.id.clone(),
            model: model.or(upstream_model),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: if status.is_success() { "ok" } else { "error" },
            cache_inclusive: matches!(provider.protocol, Protocol::OpenAI | Protocol::Gemini),
            log,
        };
        record_sample(&state, sample);

        let mut response = Response::new(Body::from(bytes));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    }
}

/// Serialize a response header map for the request log (same redaction as
/// the request side; `copy_response_headers` output has no credentials).
fn response_headers_text(headers: &HeaderMap) -> String {
    let mut map = serde_json::Map::new();
    for (name, value) in headers.iter() {
        let v = value
            .to_str()
            .map(str::to_string)
            .unwrap_or_else(|_| "<binary>".to_string());
        map.insert(name.as_str().to_string(), serde_json::Value::String(v));
    }
    serde_json::to_string(&map).unwrap_or_else(|_| "{}".to_string())
}

/// Forward an Anthropic `/v1/messages` request to an OpenAI-compatible
/// provider with protocol conversion (adapters sublayer).
///
/// Request: `anthropic_to_openai` + `stream_options.include_usage` injection +
/// `model_mapper` 1M-context marker stripping. Response: non-SSE bodies go
/// through `openai_to_anthropic`; SSE streams are converted to the Anthropic
/// event stream by `create_anthropic_sse_stream` (whose emitted
/// `message_start`/`message_delta` usage is what the metering scanner sees).
/// Upstream error bodies are passed through unconverted.
#[allow(clippy::too_many_arguments)]
async fn forward_anthropic_via_openai(
    state: Arc<GatewayState>,
    method: Method,
    inbound_headers: HeaderMap,
    body: Bytes,
    routed: RoutedRequest,
    inbound: Option<Protocol>,
    started: Instant,
    log: Option<CompletedLog>,
) -> Response {
    let provider = &routed.provider;

    // The requested Anthropic model is authoritative for metering.
    let model = request_model(&body);

    // Conversion failure is a client-shape problem (422) or an internal one.
    let converted_body = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            let message = format!("kiwano-gateway: inbound body is not valid JSON: {e}");
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                StatusCode::BAD_REQUEST,
                "invalid_request",
                message.clone(),
            );
            return error_response(
                inbound,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &message,
            );
        }
    };
    let mut openai_body = match anthropic_to_openai(converted_body) {
        Ok(v) => v,
        Err(e) => {
            let resp = proxy_error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "conversion_failed",
                "adapters conversion failed".to_string(),
            );
            return resp;
        }
    };
    inject_openai_stream_include_usage(&mut openai_body);
    let openai_body = strip_one_m_suffix_for_upstream_from_body(openai_body);
    tracing::info!(
        provider = %provider.id,
        agent = %routed.agent,
        model = model.as_deref().unwrap_or("<none>"),
        "converting Anthropic request to OpenAI chat completions"
    );
    let openai_bytes = match serde_json::to_vec(&openai_body) {
        Ok(b) => b,
        Err(e) => {
            let resp = error_into_response(
                GatewayError::Upstream(format!("serializing converted body failed: {e}")),
                inbound,
            );
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "internal_error",
                format!("serializing converted body failed: {e}"),
            );
            return resp;
        }
    };

    let url = upstream_url(provider, "/v1/chat/completions");
    // Query strings are meaningless across protocol conversion; drop them.
    let api_key = match select_upstream_key(&state, provider) {
        Ok(k) => k,
        Err(e) => {
            let resp = error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "upstream_error",
                "selecting upstream key failed".to_string(),
            );
            return resp;
        }
    };
    let headers = match build_upstream_headers(&inbound_headers, provider, &api_key) {
        Ok(h) => h,
        Err(e) => {
            let resp = error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "upstream_error",
                "building upstream headers failed".to_string(),
            );
            return resp;
        }
    };

    let upstream = match send_upstream(
        &state,
        provider,
        &routed.agent,
        &attribution_str(routed.attribution),
        method,
        &url,
        headers,
        Bytes::from(openai_bytes),
        inbound,
        log.as_ref().map(|l| &l.capture),
    )
    .await
    {
        Ok(r) => r,
        Err(resp) => return resp,
    };

    let status = upstream.status();
    let is_sse = upstream
        .headers()
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|ct| ct.contains("text/event-stream"));

    tracing::info!(
        url = %url,
        provider = %provider.id,
        agent = %routed.agent,
        upstream_status = status.as_u16(),
        sse = is_sse,
        "upstream responded to converted request"
    );

    let response_headers = copy_response_headers(upstream.headers());

    if is_sse {
        // Convert the OpenAI chunk stream into an Anthropic event stream; the
        // metering scanner then reads the converted Anthropic usage events
        // and the capture tee records the client-visible stream.
        let (tx, rx) = mpsc::channel::<UsageSample>(1);
        tokio::spawn(record_pending_usage(state.clone(), rx));
        let converted = create_anthropic_sse_stream(Box::pin(upstream.bytes_stream()));
        let max_body_bytes = state.log_config().max_body_bytes;
        let stream = SseUsageStream::new(
            Box::pin(converted.map(|r| r.map_err(|e| Box::new(e) as BoxError))),
            tx,
            UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                model,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                // Outbound is always OpenAI here (Anthropic → OpenAI conversion).
                cache_inclusive: true,
                log: log.map(|l| CompletedLog {
                    is_streaming: true,
                    status_code: status.as_u16(),
                    response_headers: Some(response_headers_text(&response_headers)),
                    capture: l.capture,
                    attribution: l.attribution,
                    error_kind: None,
                    error_message: None,
                    first_token_ms: None,
                    response_body: None,
                    response_size: 0,
                    truncated: false,
                }),
            },
            started,
            max_body_bytes,
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    } else {
        let bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(e) => {
                let resp = error_into_response(
                    GatewayError::Upstream(format!("reading upstream body failed: {e}")),
                    inbound,
                );
                crate::log_capture::persist_failure(
                    &state.store,
                    log.as_ref().map(|l| &l.capture),
                    Some(routed.agent.clone()),
                    Some(attribution_str(routed.attribution)),
                    Some(provider.id.clone()),
                    resp.status(),
                    "upstream_error",
                    format!("reading upstream body failed: {e}"),
                );
                return resp;
            }
        };
        let latency_ms = started.elapsed().as_millis() as i64;
        let (usage, upstream_model) = parse_response_usage(Protocol::OpenAI, &bytes);

        if !status.is_success() {
            // Pass upstream error bodies through unconverted (error shapes
            // are not chat.completion objects; converting would corrupt them).
            let log = log.map(|mut l| {
                let max_body_bytes = state.log_config().max_body_bytes;
                let (response_body, truncated) = cap_body(&bytes, max_body_bytes);
                l.response_body = Some(response_body);
                l.response_size = bytes.len() as i64;
                l.truncated = truncated;
                l.status_code = status.as_u16();
                l.response_headers = Some(response_headers_text(&response_headers));
                l
            });
            let sample = UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                model: model.or(upstream_model),
                usage: usage.unwrap_or_default(),
                latency_ms,
                status: "error",
                // Outbound is always OpenAI here (Anthropic → OpenAI conversion).
                cache_inclusive: true,
                log,
            };
            record_sample(&state, sample);
            let mut response = Response::new(Body::from(bytes));
            *response.status_mut() = status;
            *response.headers_mut() = response_headers;
            return response;
        }

        let anthropic = match serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| GatewayError::Upstream(format!("upstream body is not JSON: {e}")))
            .and_then(|v| {
                openai_to_anthropic(v)
                    .map_err(|e| GatewayError::Upstream(format!("response conversion failed: {e}")))
            }) {
            Ok(v) => v,
            Err(e) => {
                let message = e.to_string();
                let resp = error_into_response(e, inbound);
                crate::log_capture::persist_failure(
                    &state.store,
                    log.as_ref().map(|l| &l.capture),
                    Some(routed.agent.clone()),
                    Some(attribution_str(routed.attribution)),
                    Some(provider.id.clone()),
                    resp.status(),
                    "conversion_failed",
                    message,
                );
                return resp;
            }
        };
        let out = match serde_json::to_vec(&anthropic) {
            Ok(b) => b,
            Err(e) => {
                let resp = error_into_response(
                    GatewayError::Upstream(format!("serializing converted response failed: {e}")),
                    inbound,
                );
                crate::log_capture::persist_failure(
                    &state.store,
                    log.as_ref().map(|l| &l.capture),
                    Some(routed.agent.clone()),
                    Some(attribution_str(routed.attribution)),
                    Some(provider.id.clone()),
                    resp.status(),
                    "internal_error",
                    format!("serializing converted response failed: {e}"),
                );
                return resp;
            }
        };

        let log = log.map(|mut l| {
            let max_body_bytes = state.log_config().max_body_bytes;
            let (response_body, truncated) = cap_body(&out, max_body_bytes);
            l.response_body = Some(response_body);
            l.response_size = out.len() as i64;
            l.truncated = truncated;
            l.status_code = status.as_u16();
            l.response_headers = Some(response_headers_text(&response_headers));
            l
        });
        let sample = UsageSample {
            agent: routed.agent.clone(),
            provider_id: provider.id.clone(),
            model: model.or(upstream_model),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: "ok",
            // Outbound is always OpenAI here (Anthropic → OpenAI conversion).
            cache_inclusive: true,
            log,
        };
        record_sample(&state, sample);

        let mut response = Response::new(Body::from(out));
        *response.status_mut() = status;
        // The upstream headers were snapshotted for an OpenAI payload; the
        // converted body is always JSON.
        *response.headers_mut() = response_headers;
        if let Ok(ct) = HeaderValue::from_str("application/json") {
            response
                .headers_mut()
                .insert(axum::http::header::CONTENT_TYPE, ct);
        }
        response
    }
}

/// Map a adapters `ProxyError` onto a gateway error response.
fn proxy_error_into_response(
    e: kiwano_adapters::proxy::ProxyError,
    inbound: Option<Protocol>,
) -> Response {
    use kiwano_adapters::proxy::error_mapper::map_proxy_error_to_status;
    let status =
        StatusCode::from_u16(map_proxy_error_to_status(&e)).unwrap_or(StatusCode::BAD_GATEWAY);
    error_response(
        inbound,
        status,
        "conversion_failed",
        &format!("kiwano-gateway: adapters conversion failed: {e}"),
    )
}

async fn record_pending_usage(state: Arc<GatewayState>, mut rx: mpsc::Receiver<UsageSample>) {
    if let Some(sample) = rx.recv().await {
        record_sample(&state, sample);
    }
}

fn record_sample(state: &GatewayState, sample: UsageSample) {
    let mut sample = sample;
    let log = sample.log.take();
    let (cost, cost_currency) = compute_sample_cost(state, &sample);
    let record = sample.into_record(cost, cost_currency);
    tracing::info!(
        agent = %record.agent,
        provider_id = %record.provider_id,
        input = record.input_tokens,
        output = record.output_tokens,
        cache_read = record.cache_read_tokens,
        cache_creation = record.cache_creation_tokens,
        latency_ms = record.latency_ms,
        status = %record.status,
        cost = record.cost,
        "usage captured"
    );
    if let Err(e) = state.store.record_usage(&record) {
        tracing::warn!(error = %e, "failed to persist usage record");
    }
    if let Some(log) = log {
        let entry = RequestLogNew {
            ts: log.capture.ts,
            method: log.capture.method,
            path: log.capture.path,
            query: log.capture.query,
            agent: Some(record.agent),
            attribution: Some(log.attribution),
            provider_id: Some(record.provider_id),
            model: record.model,
            status_code: log.status_code as i64,
            error_kind: log.error_kind,
            error_message: log.error_message,
            session_id: log.capture.session_id,
            is_streaming: log.is_streaming,
            input_tokens: record.input_tokens,
            output_tokens: record.output_tokens,
            cache_read_tokens: record.cache_read_tokens,
            cache_creation_tokens: record.cache_creation_tokens,
            latency_ms: record.latency_ms,
            first_token_ms: log.first_token_ms,
            request_headers: log.capture.request_headers,
            response_headers: log.response_headers,
            request_body: log.capture.request_body,
            response_body: log.response_body,
            request_size: log.capture.request_size,
            response_size: log.response_size,
            truncated: log.capture.truncated || log.truncated,
            cost: record.cost,
            cost_currency: record.cost_currency,
        };
        if let Err(e) = state.store.insert_request_log(&entry) {
            tracing::warn!(error = %e, "failed to persist request log");
        }
    }
}

/// A byte-preserving SSE passthrough stream that scans complete lines for
/// usage events and submits the metered sample when the upstream stream ends.
///
/// Bytes are forwarded unchanged: complete `\n`-terminated lines are emitted
/// as they arrive, the trailing partial line is flushed at stream end. While
/// request logging is on, the same chunks tee into a capture buffer (capped)
/// that lands in `request_bodies` at stream end.
struct SseUsageStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    buffer: Vec<u8>,
    scanner: UsageScanner,
    sample: UsageSample,
    tx: mpsc::Sender<UsageSample>,
    started: Instant,
    inner_ended: bool,
    // Response capture (request logging):
    capture_buf: Vec<u8>,
    capture_truncated: bool,
    streamed_bytes: u64,
    capture_cap: usize,
    first_chunk: Option<Instant>,
}

impl SseUsageStream {
    fn new(
        inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
        tx: mpsc::Sender<UsageSample>,
        sample: UsageSample,
        started: Instant,
        capture_cap: usize,
    ) -> Self {
        SseUsageStream {
            inner,
            buffer: Vec::new(),
            scanner: UsageScanner::new(),
            sample,
            tx,
            started,
            inner_ended: false,
            capture_buf: Vec::new(),
            capture_truncated: false,
            streamed_bytes: 0,
            capture_cap,
            first_chunk: None,
        }
    }

    fn scan_lines(&mut self, bytes: &[u8]) {
        for line in String::from_utf8_lossy(bytes).lines() {
            self.scanner.feed_line(line);
        }
    }

    /// Tee a passthrough chunk into the capture buffer (capped).
    fn capture_chunk(&mut self, chunk: &[u8]) {
        self.streamed_bytes += chunk.len() as u64;
        if self.sample.log.is_none() || self.capture_cap == 0 {
            return;
        }
        let remaining = self.capture_cap.saturating_sub(self.capture_buf.len());
        if remaining == 0 {
            self.capture_truncated = true;
            return;
        }
        if chunk.len() > remaining {
            self.capture_truncated = true;
        }
        self.capture_buf
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    /// Submit the metered sample (best-effort) once the stream is exhausted.
    fn finish(&mut self) {
        self.sample.usage = self.scanner.usage();
        if self.sample.model.is_none() {
            self.sample.model = self.scanner.model().map(String::from);
        }
        self.sample.latency_ms = self.started.elapsed().as_millis() as i64;
        if let Some(log) = self.sample.log.as_mut() {
            log.first_token_ms = self
                .first_chunk
                .map(|t| (t - self.started).as_millis() as i64);
            log.response_body = Some(String::from_utf8_lossy(&self.capture_buf).into_owned());
            log.response_size = self.streamed_bytes as i64;
            log.truncated = log.truncated || self.capture_truncated;
        }
        if let Err(e) = self.tx.try_send(self.sample.clone()) {
            tracing::warn!(error = %e, "usage channel unavailable; stream usage not persisted");
        }
    }
}

impl Stream for SseUsageStream {
    type Item = Result<Bytes, BoxError>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        loop {
            if self.inner_ended {
                self.finish();
                return Poll::Ready(None);
            }
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    if self.first_chunk.is_none() {
                        self.first_chunk = Some(Instant::now());
                    }
                    self.capture_chunk(&chunk);
                    self.buffer.extend_from_slice(&chunk);
                    // Emit only up to the last complete line; keep the tail.
                    if let Some(split) =
                        self.buffer.iter().rposition(|&b| b == b'\n').map(|i| i + 1)
                    {
                        let complete: Vec<u8> = self.buffer.drain(..split).collect();
                        self.scan_lines(&complete);
                        return Poll::Ready(Some(Ok(Bytes::from(complete))));
                    }
                    // No newline yet: keep buffering (inner will wake us).
                }
                Poll::Ready(Some(Err(e))) => {
                    self.inner_ended = true;
                    self.finish();
                    return Poll::Ready(Some(Err(e)));
                }
                Poll::Ready(None) => {
                    self.inner_ended = true;
                    if self.buffer.is_empty() {
                        self.finish();
                        return Poll::Ready(None);
                    }
                    let rest = std::mem::take(&mut self.buffer);
                    self.scan_lines(&rest);
                    return Poll::Ready(Some(Ok(Bytes::from(rest))));
                    // finish() runs on the next poll (inner_ended branch).
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::vec;

    fn provider(protocol: Protocol, api_path: Option<&str>) -> UpstreamProvider {
        UpstreamProvider {
            id: "p1".into(),
            name: "p1".into(),
            protocol,
            base_url: "https://up.example.com".into(),
            api_path: api_path.map(Into::into),
            endpoints: Vec::new(),
            api_key: Some("sk-real-key".into()),
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
    fn resolve_inbound_prefers_registered_protocol_endpoint() {
        let mut dual = provider(Protocol::OpenAI, None);
        dual.endpoints = vec![crate::store::ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://up.example.com/anthropic".into(),
            api_path: Some("/ant".into()),
        }];

        // Registered per-protocol endpoint wins over conversion.
        match resolve_inbound(&dual, Some(Protocol::Anthropic), "/v1/messages") {
            InboundResolution::Alternate {
                protocol,
                base_url,
                api_path,
            } => {
                assert_eq!(protocol, Protocol::Anthropic);
                assert_eq!(base_url, "https://up.example.com/anthropic");
                assert_eq!(api_path.as_deref(), Some("/ant"));
            }
            _ => panic!("expected Alternate"),
        }

        // Native when the inbound protocol matches the provider's own.
        assert!(matches!(
            resolve_inbound(&dual, Some(Protocol::OpenAI), "/v1/chat/completions"),
            InboundResolution::Native
        ));
        // Ambiguous inbound (/v1/models) stays native.
        assert!(matches!(
            resolve_inbound(&dual, None, "/v1/models"),
            InboundResolution::Native
        ));

        // No anthropic endpoint registered → conversion fallback.
        let plain = provider(Protocol::OpenAI, None);
        assert!(matches!(
            resolve_inbound(&plain, Some(Protocol::Anthropic), "/v1/messages"),
            InboundResolution::ConvertAnthropicToOpenAI
        ));
        // Neither endpoint nor conversion → clean mismatch.
        assert!(matches!(
            resolve_inbound(
                &plain,
                Some(Protocol::Gemini),
                "/v1beta/models/gemini-pro:generateContent"
            ),
            InboundResolution::Mismatch { .. }
        ));
        // Legacy anthropic paths have no OpenAI equivalent.
        assert!(matches!(
            resolve_inbound(&plain, Some(Protocol::Anthropic), "/v1/complete"),
            InboundResolution::Mismatch { .. }
        ));
    }

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
    fn retryable_status_classification() {
        assert!(is_retryable_status(StatusCode::REQUEST_TIMEOUT));
        assert!(is_retryable_status(StatusCode::TOO_MANY_REQUESTS));
        assert!(is_retryable_status(StatusCode::INTERNAL_SERVER_ERROR));
        assert!(is_retryable_status(StatusCode::BAD_GATEWAY));
        assert!(is_retryable_status(StatusCode::SERVICE_UNAVAILABLE));
        assert!(!is_retryable_status(StatusCode::OK));
        assert!(!is_retryable_status(StatusCode::BAD_REQUEST));
        assert!(!is_retryable_status(StatusCode::UNAUTHORIZED));
        assert!(!is_retryable_status(StatusCode::FORBIDDEN));
        assert!(!is_retryable_status(StatusCode::NOT_FOUND));
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
    fn record_sample_costs_priced_models_and_skips_unknown() {
        use crate::store::Protocol;

        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        state
            .store
            .insert_provider(&crate::store::Provider {
                id: "p1".into(),
                name: "p1".into(),
                protocol: Protocol::Anthropic,
                base_url: "https://a.example.com".into(),
                api_path: None,
                endpoints: Vec::new(),
                api_key: Some("sk".into()),
                billing: crate::store::Billing::Metered,
                period_limit: None,
                limit_unit: None,
                plan_query: None,
                plan_limits: None,
                timeout_secs: None,
                retries: None,
                headers: None,
                reset_period: None,
                enabled: true,
                created_at: crate::store::now_rfc3339(),
                updated_at: crate::store::now_rfc3339(),
            })
            .unwrap();

        // claude-opus-4-8 is priced in the bundled table (input 5 USD/M).
        let sample = |model: Option<&'static str>| UsageSample {
            agent: "claude".into(),
            provider_id: "p1".into(),
            model: model.map(str::to_string),
            usage: Usage {
                input_tokens: 1_000_000,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
            },
            latency_ms: 10,
            status: "ok",
            cache_inclusive: false,
            log: None,
        };
        record_sample(&state, sample(Some("claude-opus-4-8")));
        record_sample(&state, sample(Some("totally-unpriced-model")));
        record_sample(&state, sample(None));

        let totals = state.store.usage_totals(None, None, None).unwrap();
        assert_eq!(totals.requests, 3);
        // Only the priced model contributes; cost is stored in the price
        // entry's currency (1M fresh input @ 5 USD/M = 5.0).
        let costs = state
            .store
            .usage_cost_by_currency(None, None, None)
            .unwrap();
        assert_eq!(costs.len(), 1);
        assert_eq!(costs[0].0.as_deref(), Some("USD"));
        assert!((costs[0].1 - 5.0).abs() < 1e-9, "got {}", costs[0].1);
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

    /// Synthetic chunk source with awkward boundaries (a usage event split in
    /// half, no trailing newline).
    struct ChunksStream {
        chunks: vec::IntoIter<Result<Bytes, BoxError>>,
    }

    impl Stream for ChunksStream {
        type Item = Result<Bytes, BoxError>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.chunks.next())
        }
    }

    #[tokio::test]
    async fn sse_stream_forwards_bytes_and_scans_usage() {
        let chunks: Vec<Result<Bytes, BoxError>> = vec![
            Ok(Bytes::from_static(
                b"data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"us",
            )),
            Ok(Bytes::from_static(
                b"age\":{\"input_tokens\":7,\"output_tokens\":1}}}\n\ndata: {\"type\":\"mess",
            )),
            Ok(Bytes::from_static(
                b"age_delta\",\"usage\":{\"output_tokens\":9}}\n\ndata: [DONE]",
            )),
        ];
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>> =
            Box::pin(ChunksStream {
                chunks: chunks.into_iter(),
            });

        let (tx, mut rx) = mpsc::channel(1);
        let mut stream = SseUsageStream::new(
            inner,
            tx,
            UsageSample {
                agent: "claude".into(),
                provider_id: "p1".into(),
                model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                cache_inclusive: false,
                log: None,
            },
            Instant::now(),
            0,
        );

        let mut out: Vec<Bytes> = Vec::new();
        while let Some(item) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await
        {
            out.push(item.expect("stream error"));
        }

        // Byte-for-byte passthrough across the awkward chunk boundaries.
        let joined: Vec<u8> = out.concat();
        assert_eq!(
            String::from_utf8(joined).unwrap(),
            "data: {\"type\":\"message_start\",\"message\":{\"model\":\"m\",\"usage\":{\"input_tokens\":7,\"output_tokens\":1}}}\n\n\
             data: {\"type\":\"message_delta\",\"usage\":{\"output_tokens\":9}}\n\n\
             data: [DONE]"
        );

        let sample = rx.recv().await.expect("usage sample");
        assert_eq!(sample.usage.input_tokens, 7);
        assert_eq!(sample.usage.output_tokens, 9);
        assert_eq!(sample.model.as_deref(), Some("m"));
    }
}
