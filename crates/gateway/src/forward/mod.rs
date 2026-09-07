//! Transparent forwarding with SSE passthrough + usage capture (tech.md §4.3).
//!
//! Non-SSE upstream responses are buffered and metered inline. SSE responses
//! are piped through byte-for-byte while a scanning stream watches the events
//! for usage fields; the metered sample is persisted after the stream ends.
//! Protocol conversion (Anthropic↔OpenAI) belongs to `cc-adapters` and is not
//! part of this MVP leg — a mismatching binding fails with a clean error.

use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_core::Stream;
use tokio::sync::mpsc;

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

/// Build upstream request headers: copy the inbound set (minus hop-by-hop and
/// local auth headers), then inject the provider's real credential in its
/// native auth style.
fn build_upstream_headers(
    inbound: &HeaderMap,
    provider: &UpstreamProvider,
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

    let key = provider.api_key.clone().ok_or_else(|| {
        GatewayError::Upstream(format!("provider `{}` has no API key configured", provider.id))
    })?;
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

    // MVP is transparent-forward only: provider protocol must match inbound.
    if let Some(inbound_proto) = inbound {
        if inbound_proto != provider.protocol {
            return error_response(
                inbound,
                StatusCode::BAD_GATEWAY,
                "protocol_mismatch",
                &format!(
                    "provider `{}` speaks `{}` but path `{}` is `{}`; protocol conversion (cc-adapters) is not enabled in this build",
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

    let headers = match build_upstream_headers(&inbound_headers, provider) {
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
            return error_into_response(
                GatewayError::Upstream(format!("request to `{url}` failed: {e}")),
                inbound,
            );
        }
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

    if is_sse {
        // Streaming passthrough with usage scanning; the sample is persisted
        // by a side task when the stream finishes.
        let (tx, rx) = mpsc::channel::<UsageSample>(1);
        tokio::spawn(record_pending_usage(state.clone(), rx));
        let stream = SseUsageStream::new(
            Box::pin(upstream.bytes_stream()),
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
    inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
    buffer: Vec<u8>,
    scanner: UsageScanner,
    sample: UsageSample,
    tx: mpsc::Sender<UsageSample>,
    started: Instant,
    inner_ended: bool,
}

impl SseUsageStream {
    fn new(
        inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
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
                    return Poll::Ready(Some(Err(Box::new(e))));
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
        let headers =
            build_upstream_headers(&inbound_headers(), &provider(Protocol::Anthropic, None))
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
        let headers =
            build_upstream_headers(&inbound_headers(), &provider(Protocol::OpenAI, None)).unwrap();
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
        let err = build_upstream_headers(&inbound_headers(), &p).unwrap_err();
        assert!(matches!(err, GatewayError::Upstream(_)));
    }

    #[test]
    fn response_headers_drop_framing() {
        let mut src = HeaderMap::new();
        src.insert(
            axum::http::header::CONTENT_TYPE,
            HeaderValue::from_static("text/event-stream"),
        );
        src.insert(axum::http::header::CONTENT_LENGTH, HeaderValue::from_static("123"));
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
        chunks: vec::IntoIter<reqwest::Result<Bytes>>,
    }

    impl Stream for ChunksStream {
        type Item = reqwest::Result<Bytes>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            Poll::Ready(self.chunks.next())
        }
    }

    #[tokio::test]
    async fn sse_stream_forwards_bytes_and_scans_usage() {
        let chunks: Vec<reqwest::Result<Bytes>> = vec![
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
        let inner: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>> =
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
        while let Some(item) =
            std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await
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
