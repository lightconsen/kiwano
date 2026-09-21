//! The conversion path: an Anthropic `/v1/messages` inbound on an
//! OpenAI-compatible provider goes through the adapters sublayer in both
//! directions, and is metered from the upstream OpenAI usage fields.

use std::sync::Arc;
use std::time::Instant;

use axum::body::{Body, Bytes};
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;
use tokio::sync::mpsc;

use kiwano_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_adapters::proxy::providers::streaming::create_anthropic_sse_stream;
use kiwano_adapters::proxy::providers::transform::{
    anthropic_to_openai, inject_openai_stream_include_usage, openai_to_anthropic,
};

use crate::error::GatewayError;
use crate::forward::headers::{
    build_upstream_headers, copy_response_headers, response_headers_text, select_upstream_key,
};
use crate::forward::metering::{record_pending_usage, record_sample};
use crate::forward::sample::{attribution_str, CompletedLog, UsageSample};
use crate::forward::stream::{SseUsageStream, StreamPolicy};
use crate::forward::upstream::{read_upstream_body_capped, send_upstream, MAX_UPSTREAM_BODY_BYTES};
use crate::forward::BoxError;
use crate::log_capture::cap_body;
use crate::meter::{parse_response_usage, request_model, Usage};
use crate::router::RoutedRequest;
use crate::server::data::upstream_url;
use crate::server::{error_into_response, error_response, GatewayState};
use crate::store::Protocol;

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
pub(crate) async fn forward_anthropic_via_openai(
    state: Arc<GatewayState>,
    method: Method,
    inbound_headers: HeaderMap,
    body: Bytes,
    routed: RoutedRequest,
    inbound: Option<Protocol>,
    started: Instant,
    started_unix: i64,
    log: Option<CompletedLog>,
) -> Response {
    let provider = &routed.provider;

    // The requested Anthropic model is authoritative for metering.
    let model = request_model(&body);

    // Conversion failure is a client-shape problem (422) or an internal one.
    let converted_body = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            let message = format!("kiwanod: inbound body is not valid JSON: {e}");
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
            // The reason, not a summary of it: a conversion refusal exists to say
            // what could not be carried across, and "conversion failed" leaves the
            // reader with no way to find out.
            let reason = e.to_string();
            let resp = proxy_error_into_response(e, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                log.as_ref().map(|l| &l.capture),
                Some(routed.agent.clone()),
                Some(attribution_str(routed.attribution)),
                Some(provider.id.clone()),
                resp.status(),
                "conversion_failed",
                format!("kiwanod: adapters conversion failed: {reason}"),
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

    let mut upstream = match send_upstream(
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
                catalog_id: provider.catalog_id.clone(),
                started_unix,
                model,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                // Outbound is always OpenAI here (Anthropic → OpenAI conversion).
                // Outbound is always OpenAI here (Anthropic → OpenAI conversion).
                cache_inclusive: true,
                // Filled at `finish`, from what the scanner saw.
                usage_missing: false,
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
                    request_notes: l.request_notes,
                }),
            },
            started,
            StreamPolicy {
                capture_cap: max_body_bytes,
                redactor: state.redactor(),
                strip_usage_chunk: false,
            },
            inbound,
            state.stream_timeouts(),
        );
        let mut response = Response::new(Body::from_stream(stream));
        *response.status_mut() = status;
        *response.headers_mut() = response_headers;
        response
    } else {
        let bytes = match read_upstream_body_capped(&mut upstream, MAX_UPSTREAM_BODY_BYTES).await {
            Ok(b) => b,
            Err(e) => {
                let resp =
                    error_into_response(GatewayError::Upstream(e.message().to_string()), inbound);
                crate::log_capture::persist_failure(
                    &state.store,
                    log.as_ref().map(|l| &l.capture),
                    Some(routed.agent.clone()),
                    Some(attribution_str(routed.attribution)),
                    Some(provider.id.clone()),
                    resp.status(),
                    e.kind(),
                    e.message().to_string(),
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
                let (response_body, truncated) =
                    cap_body(&bytes, max_body_bytes, &state.redactor());
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
                catalog_id: provider.catalog_id.clone(),
                started_unix,
                model: model.or(upstream_model),
                usage_missing: usage.is_none(),
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
            let (response_body, truncated) = cap_body(&out, max_body_bytes, &state.redactor());
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
            catalog_id: provider.catalog_id.clone(),
            started_unix,
            model: model.or(upstream_model),
            usage_missing: usage.is_none(),
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
        &format!("kiwanod: adapters conversion failed: {e}"),
    )
}
