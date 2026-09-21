//! How a forwarded request ends: the request-log row a failure leaves, the
//! full-log row a buffered response completes, and the two response shapes —
//! streamed and buffered — the client is handed back.

use std::pin::Pin;
use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use futures_util::Stream;
use tokio::sync::mpsc;

use crate::forward::headers::response_headers_text;
use crate::forward::metering::{record_pending_usage, record_sample};
use crate::forward::sample::{attribution_str, CompletedLog, UsageSample};
use crate::forward::stream::{SseUsageStream, StreamPolicy};
use crate::forward::BoxError;
use crate::log_capture::cap_body;
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::GatewayState;
use crate::store::Protocol;

/// The request-log row for a failure the calling leg is about to answer with.
///
/// Every leg reports its own failures through this one call — the same columns
/// keyed to the routed request, with the status, kind and message the site
/// knows. `log` is `None` whenever request logging is off, and then nothing is
/// written at all.
pub(crate) fn log_failure(
    state: &GatewayState,
    log: Option<&CompletedLog>,
    provider: &UpstreamProvider,
    routed: &RoutedRequest,
    status: StatusCode,
    kind: &str,
    message: String,
) {
    crate::log_capture::persist_failure(
        &state.store,
        log.map(|l| &l.capture),
        Some(routed.agent.clone()),
        Some(attribution_str(routed.attribution)),
        Some(provider.id.clone()),
        status,
        kind,
        message,
    );
}

/// Close out the full-log row of a buffered response: the body as the client
/// saw it (capped, redacted), its size, and the status. `response_headers` is
/// `None` for a leg that already wrote them — an upstream response is not the
/// safer half of the exchange, so they are written once, not twice.
pub(crate) fn finish_log(
    log: Option<CompletedLog>,
    body: &[u8],
    status: StatusCode,
    response_headers: Option<&HeaderMap>,
    state: &GatewayState,
) -> Option<CompletedLog> {
    log.map(|mut l| {
        let max_body_bytes = state.log_config().max_body_bytes;
        let (response_body, truncated) = cap_body(body, max_body_bytes, &state.redactor());
        l.response_body = Some(response_body);
        l.response_size = body.len() as i64;
        l.truncated = truncated;
        l.status_code = status.as_u16();
        if let Some(headers) = response_headers {
            l.response_headers = Some(response_headers_text(headers));
        }
        l
    })
}

/// A response carrying `body` under the upstream's own status and headers.
fn response_with(body: Body, status: StatusCode, headers: HeaderMap) -> Response {
    let mut response = Response::new(body);
    *response.status_mut() = status;
    *response.headers_mut() = headers;
    response
}

/// Hand the client the streamed response while the sample is metered behind
/// it: the scanner rides the same stream, and the sample is persisted by a
/// side task when the stream finishes.
#[allow(clippy::too_many_arguments)] // the stream's own construction inputs
pub(crate) fn sse_response(
    state: &Arc<GatewayState>,
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    sample: UsageSample,
    started: Instant,
    policy: StreamPolicy,
    inbound: Option<Protocol>,
    status: StatusCode,
    response_headers: HeaderMap,
) -> Response {
    let (tx, rx) = mpsc::channel::<UsageSample>(1);
    tokio::spawn(record_pending_usage(state.clone(), rx));
    let stream = SseUsageStream::new(
        inner,
        tx,
        sample,
        started,
        policy,
        inbound,
        state.stream_timeouts(),
    );
    response_with(Body::from_stream(stream), status, response_headers)
}

/// Hand the client the buffered body that was just metered.
pub(crate) fn buffered_response(
    state: &GatewayState,
    body: Bytes,
    sample: UsageSample,
    status: StatusCode,
    response_headers: HeaderMap,
) -> Response {
    record_sample(state, sample);
    response_with(Body::from(body), status, response_headers)
}
