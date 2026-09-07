//! Transparent forwarding with SSE passthrough + usage capture (tech.md §4.3).
//!
//! Non-SSE upstream responses are buffered and metered inline. SSE responses
//! are piped through byte-for-byte while a scanning stream watches the events
//! for usage fields; the metered sample is persisted after the stream ends.
//!
//! Protocol conversion (tech.md §4.3 / cc-adapters phase): an Anthropic
//! inbound request (`POST /v1/messages`) bound to an OpenAI-compatible
//! provider is converted with the `kiwano-cc-adapters` sublayer — request via
//! `anthropic_to_openai` (model via `model_mapper`), response (JSON + SSE)
//! back via `openai_to_anthropic` / the streaming converter — and metered
//! from the upstream OpenAI usage fields.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_core::Stream;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use kiwano_cc_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_cc_adapters::proxy::providers::streaming::create_anthropic_sse_stream;
use kiwano_cc_adapters::proxy::providers::transform::{
    anthropic_to_openai, inject_openai_stream_include_usage, openai_to_anthropic,
};

use crate::error::GatewayError;
use crate::meter::{parse_response_usage, request_model, Usage, UsageScanner};
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::data::upstream_url;
use crate::server::{error_into_response, error_response, GatewayState};
use crate::store::now_rfc3339;
use crate::store::{Protocol, UsageRecord};

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

/// One metered request, ready for the `usage` table.
#[derive(Clone)]
struct UsageSample {
    agent: String,
    provider_id: String,
    model: Option<String>,
    usage: Usage,
    latency_ms: i64,
    status: &'static str,
}

impl UsageSample {
    fn into_record(self) -> UsageRecord {
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
        }
    }
}

/// Pick the credential for one upstream request: the provider's key pool is
/// `[primary, extras…]`; pools with more than one entry rotate per request
/// (spec §4.1 P1 多 Key 轮询，避免单 Key 触发上游限流).
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
                HeaderValue::from_str(&key).map_err(|e| GatewayError::Upstream(e.to_string()))?;
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
    }
    Ok(out)
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

/// Forward one resolved request to its provider and return the client-facing
/// response, metering usage on the way.
pub async fn forward(
    state: Arc<GatewayState>,
    method: Method,
    path: String,
    query: Option<String>,
    inbound_headers: HeaderMap,
    body: Bytes,
    routed: RoutedRequest,
    inbound: Option<Protocol>,
) -> Response {
    let started = Instant::now();
    let provider = &routed.provider;

    // Anthropic inbound on an OpenAI-compatible provider → convert via
    // cc-adapters. Legacy Anthropic paths other than `/v1/messages` have no
    // OpenAI equivalent and still fail cleanly.
    if let Some(inbound_proto) = inbound {
        if inbound_proto != provider.protocol {
            if inbound_proto == Protocol::Anthropic
                && provider.protocol == Protocol::OpenAI
                && path == "/v1/messages"
            {
                return forward_anthropic_via_openai(
                    state,
                    method,
                    inbound_headers,
                    body,
                    routed,
                    inbound,
                    started,
                )
                .await;
            }
            return error_response(
                inbound,
                StatusCode::BAD_GATEWAY,
                "protocol_mismatch",
                &format!(
                    "provider `{}` speaks `{}` but path `{}` is `{}`; only Anthropic `/v1/messages` -> OpenAI conversion is supported",
                    provider.id,
                    provider.protocol.as_str(),
                    path,
                    inbound_proto.as_str()
                ),
            );
        }
    }

    let mut url = upstream_url(provider, &path);
    if let Some(q) = &query {
        url.push('?');
        url.push_str(q);
    }

    let api_key = match select_upstream_key(&state, provider) {
        Ok(k) => k,
        Err(e) => return error_into_response(e, inbound),
    };
    let headers = match build_upstream_headers(&inbound_headers, provider, &api_key) {
        Ok(h) => h,
        Err(e) => return error_into_response(e, inbound),
    };

    let upstream = match state
        .http
        .request(method, &url)
        .headers(headers)
        .body(body.clone())
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            state.engine.record(&routed.agent, &provider.id, false).await;
            return error_into_response(
                GatewayError::Upstream(format!("request to `{url}` failed: {e}")),
                inbound,
            );
        }
    };

    let status = upstream.status();
    // 熔断回写（tech.md §4.7）：按响应头时刻的成功/失败计——流中断不追溯。
    state
        .engine
        .record(&routed.agent, &provider.id, status.is_success())
        .await;
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

    if is_sse {
        // Streaming passthrough with usage scanning; the sample is persisted
        // by a side task when the stream finishes.
        let (tx, rx) = mpsc::channel::<UsageSample>(1);
        tokio::spawn(record_pending_usage(state.clone(), rx));
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
            },
            started,
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    } else {
        let bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return error_into_response(
                    GatewayError::Upstream(format!("reading upstream body failed: {e}")),
                    inbound,
                );
            }
        };
        let latency_ms = started.elapsed().as_millis() as i64;
        let (usage, upstream_model) = parse_response_usage(provider.protocol, &bytes);
        let sample = UsageSample {
            agent: routed.agent.clone(),
            provider_id: provider.id.clone(),
            model: model.or(upstream_model),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: if status.is_success() { "ok" } else { "error" },
        };
        record_sample(&state, sample);

        let mut response = Response::new(Body::from(bytes));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    }
}

/// Forward an Anthropic `/v1/messages` request to an OpenAI-compatible
/// provider with protocol conversion (cc-adapters sublayer).
///
/// Request: `anthropic_to_openai` + `stream_options.include_usage` injection +
/// `model_mapper` 1M-context marker stripping. Response: non-SSE bodies go
/// through `openai_to_anthropic`; SSE streams are converted to the Anthropic
/// event stream by `create_anthropic_sse_stream` (whose emitted
/// `message_start`/`message_delta` usage is what the metering scanner sees).
/// Upstream error bodies are passed through unconverted.
async fn forward_anthropic_via_openai(
    state: Arc<GatewayState>,
    method: Method,
    inbound_headers: HeaderMap,
    body: Bytes,
    routed: RoutedRequest,
    inbound: Option<Protocol>,
    started: Instant,
) -> Response {
    let provider = &routed.provider;

    // The requested Anthropic model is authoritative for metering.
    let model = request_model(&body);

    // Conversion failure is a client-shape problem (422) or an internal one.
    let converted_body = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            return error_response(
                inbound,
                StatusCode::BAD_REQUEST,
                "invalid_request",
                &format!("kiwano-gateway: inbound body is not valid JSON: {e}"),
            );
        }
    };
    let mut openai_body = match anthropic_to_openai(converted_body) {
        Ok(v) => v,
        Err(e) => return proxy_error_into_response(e, inbound),
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
            return error_into_response(
                GatewayError::Upstream(format!("serializing converted body failed: {e}")),
                inbound,
            );
        }
    };

    let url = upstream_url(provider, "/v1/chat/completions");
    // Query strings are meaningless across protocol conversion; drop them.
    let api_key = match select_upstream_key(&state, provider) {
        Ok(k) => k,
        Err(e) => return error_into_response(e, inbound),
    };
    let headers = match build_upstream_headers(&inbound_headers, provider, &api_key) {
        Ok(h) => h,
        Err(e) => return error_into_response(e, inbound),
    };

    let upstream = match state
        .http
        .request(method, &url)
        .headers(headers)
        .body(openai_bytes)
        .send()
        .await
    {
        Ok(r) => r,
        Err(e) => {
            state.engine.record(&routed.agent, &provider.id, false).await;
            return error_into_response(
                GatewayError::Upstream(format!("request to `{url}` failed: {e}")),
                inbound,
            );
        }
    };

    let status = upstream.status();
    state
        .engine
        .record(&routed.agent, &provider.id, status.is_success())
        .await;
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
        // metering scanner then reads the converted Anthropic usage events.
        let (tx, rx) = mpsc::channel::<UsageSample>(1);
        tokio::spawn(record_pending_usage(state.clone(), rx));
        let converted = create_anthropic_sse_stream(Box::pin(upstream.bytes_stream()));
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
            },
            started,
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    } else {
        let bytes = match upstream.bytes().await {
            Ok(b) => b,
            Err(e) => {
                return error_into_response(
                    GatewayError::Upstream(format!("reading upstream body failed: {e}")),
                    inbound,
                );
            }
        };
        let latency_ms = started.elapsed().as_millis() as i64;
        let (usage, upstream_model) = parse_response_usage(Protocol::OpenAI, &bytes);
        let sample = UsageSample {
            agent: routed.agent.clone(),
            provider_id: provider.id.clone(),
            model: model.or(upstream_model),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: if status.is_success() { "ok" } else { "error" },
        };
        record_sample(&state, sample);

        if !status.is_success() {
            // Pass upstream error bodies through unconverted (error shapes
            // are not chat.completion objects; converting would corrupt them).
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
            Err(e) => return error_into_response(e, inbound),
        };
        let out = match serde_json::to_vec(&anthropic) {
            Ok(b) => b,
            Err(e) => {
                return error_into_response(
                    GatewayError::Upstream(format!("serializing converted response failed: {e}")),
                    inbound,
                );
            }
        };

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

/// Map a cc-adapters `ProxyError` onto a gateway error response.
fn proxy_error_into_response(
    e: kiwano_cc_adapters::proxy::ProxyError,
    inbound: Option<Protocol>,
) -> Response {
    use kiwano_cc_adapters::proxy::error_mapper::map_proxy_error_to_status;
    let status =
        StatusCode::from_u16(map_proxy_error_to_status(&e)).unwrap_or(StatusCode::BAD_GATEWAY);
    error_response(
        inbound,
        status,
        "conversion_failed",
        &format!("kiwano-gateway: cc-adapters conversion failed: {e}"),
    )
}

async fn record_pending_usage(state: Arc<GatewayState>, mut rx: mpsc::Receiver<UsageSample>) {
    if let Some(sample) = rx.recv().await {
        record_sample(&state, sample);
    }
}

fn record_sample(state: &GatewayState, sample: UsageSample) {
    let record = sample.into_record();
    tracing::info!(
        agent = %record.agent,
        provider_id = %record.provider_id,
        input = record.input_tokens,
        output = record.output_tokens,
        cache_read = record.cache_read_tokens,
        cache_creation = record.cache_creation_tokens,
        latency_ms = record.latency_ms,
        status = %record.status,
        "usage captured"
    );
    if let Err(e) = state.store.record_usage(&record) {
        tracing::warn!(error = %e, "failed to persist usage record");
    }
}

/// A byte-preserving SSE passthrough stream that scans complete lines for
/// usage events and submits the metered sample when the upstream stream ends.
///
/// Bytes are forwarded unchanged: complete `\n`-terminated lines are emitted
/// as they arrive, the trailing partial line is flushed at stream end.
struct SseUsageStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    buffer: Vec<u8>,
    scanner: UsageScanner,
    sample: UsageSample,
    tx: mpsc::Sender<UsageSample>,
    started: Instant,
    inner_ended: bool,
}

impl SseUsageStream {
    fn new(
        inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
        tx: mpsc::Sender<UsageSample>,
        sample: UsageSample,
        started: Instant,
    ) -> Self {
        SseUsageStream {
            inner,
            buffer: Vec::new(),
            scanner: UsageScanner::new(),
            sample,
            tx,
            started,
            inner_ended: false,
        }
    }

    fn scan_lines(&mut self, bytes: &[u8]) {
        for line in String::from_utf8_lossy(bytes).lines() {
            self.scanner.feed_line(line);
        }
    }

    /// Submit the metered sample (best-effort) once the stream is exhausted.
    fn finish(&mut self) {
        self.sample.usage = self.scanner.usage();
        if self.sample.model.is_none() {
            self.sample.model = self.scanner.model().map(String::from);
        }
        self.sample.latency_ms = self.started.elapsed().as_millis() as i64;
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
            api_key: Some("sk-real-key".into()),
            extra_keys: Vec::new(),
            weight: 1,
            win_start: None,
            win_end: None,
        }
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
    fn missing_provider_key_fails_cleanly() {
        let mut p = provider(Protocol::Anthropic, None);
        p.api_key = None;
        let state = crate::server::GatewayState::new(
            crate::store::Store::open_in_memory().expect("store"),
        )
        .expect("state");
        let err = select_upstream_key(&state, &p).unwrap_err();
        assert!(matches!(err, GatewayError::Upstream(_)));
    }

    #[test]
    fn multi_key_pool_rotates_per_request() {
        let state = crate::server::GatewayState::new(
            crate::store::Store::open_in_memory().expect("store"),
        )
        .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.extra_keys = vec!["sk-two".into(), "sk-three".into()];

        // 单 Key 池恒取主 Key
        let mut single = provider(Protocol::OpenAI, None);
        single.extra_keys.clear();
        for _ in 0..3 {
            assert_eq!(select_upstream_key(&state, &single).unwrap(), "sk-real-key");
        }

        // 多 Key 池：主 Key → 附加 Key 依次轮转，再回到主 Key
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
            },
            Instant::now(),
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
