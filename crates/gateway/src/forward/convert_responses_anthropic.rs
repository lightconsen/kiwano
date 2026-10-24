//! The fourth conversion path: an OpenAI **Responses** request inbound on a
//! provider that speaks only **Anthropic Messages** — Codex against an upstream
//! that exposes nothing but `/v1/messages`.
//!
//! Both halves are compositions rather than a fourth converter: the request is
//! `responses_to_chat` then `chat_to_anthropic`, the response
//! `anthropic_to_chat` then `chat_to_responses`, and the stream
//! `create_responses_sse_stream_from_anthropic` (Anthropic → Chat → Responses).
//! What that inherits is written down rather than hidden: `previous_response_id`
//! and `store: true` are refused by `responses_to_chat`, `reasoning` input items
//! are dropped by it, and the streaming Responses adapter does not model
//! reasoning output — the same behaviour Codex already gets against a Chat-only
//! provider.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;

use kiwano_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_adapters::proxy::providers::chat_streaming::create_responses_sse_stream_from_anthropic;
use kiwano_adapters::proxy::providers::transform::{
    anthropic_to_responses, chat_to_anthropic, responses_to_chat,
};

use crate::error::GatewayError;
use crate::forward::convert::proxy_error_into_response;
use crate::forward::finish::{buffered_response, finish_log, log_failure, sse_response};
use crate::forward::headers::{
    copy_response_headers, response_headers_text, upstream_key_and_headers,
};
use crate::forward::sample::{attribution_str, CompletedLog, UsageSample};
use crate::forward::stream::StreamPolicy;
use crate::forward::upstream::{read_body_or_response, send_upstream};
use crate::forward::BoxError;
use crate::meter::{parse_response_usage, request_model, Usage};
use crate::router::RoutedRequest;
use crate::server::data::upstream_url;
use crate::server::{error_into_response, error_response, GatewayState};
use crate::store::Protocol;

/// Forward a `/v1/responses` request to an Anthropic-only provider.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_responses_via_anthropic(
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

    let model = request_model(&body);

    let converted_body = match serde_json::from_slice::<serde_json::Value>(&body) {
        Ok(v) => v,
        Err(e) => {
            let message = format!("kiwanod: inbound body is not valid JSON: {e}");
            log_failure(
                &state,
                log.as_ref(),
                provider,
                &routed,
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
    let anthropic_body = match responses_to_chat(converted_body).and_then(chat_to_anthropic) {
        Ok(v) => v,
        Err(e) => {
            // Either refusal is the caller's to act on: a Responses shape the
            // Chat wire cannot carry, or a Chat shape the Anthropic wire cannot.
            let reason = e.to_string();
            let resp = proxy_error_into_response(e, inbound);
            log_failure(
                &state,
                log.as_ref(),
                provider,
                &routed,
                resp.status(),
                "conversion_failed",
                format!("kiwanod: adapters conversion failed: {reason}"),
            );
            return resp;
        }
    };
    let anthropic_body = strip_one_m_suffix_for_upstream_from_body(anthropic_body);
    tracing::info!(
        provider = %provider.id,
        agent = %routed.agent,
        model = model.as_deref().unwrap_or("<none>"),
        "converting Responses request to Anthropic messages"
    );
    let anthropic_bytes = match serde_json::to_vec(&anthropic_body) {
        Ok(b) => b,
        Err(e) => {
            let resp = error_into_response(
                GatewayError::Upstream(format!("serializing converted body failed: {e}")),
                inbound,
            );
            log_failure(
                &state,
                log.as_ref(),
                provider,
                &routed,
                resp.status(),
                "internal_error",
                format!("serializing converted body failed: {e}"),
            );
            return resp;
        }
    };

    let url = upstream_url(provider, "/v1/messages");
    // Query strings are meaningless across the conversion; drop them.
    let (headers, upstream_key) = match upstream_key_and_headers(
        &state,
        provider,
        &inbound_headers,
        inbound,
        log.as_ref(),
        &routed,
    ) {
        Ok(h) => h,
        Err(resp) => return resp,
    };

    let mut upstream = match send_upstream(
        &state,
        provider,
        &routed.agent,
        &attribution_str(routed.attribution),
        method,
        &url,
        headers,
        Bytes::from(anthropic_bytes),
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
        // The scanner reads the final Responses bytes; the intermediate Chat
        // usage chunk is consumed by the composition and never forwarded, and
        // `response.completed` always carries usage on the Responses wire.
        let converted =
            create_responses_sse_stream_from_anthropic(Box::pin(upstream.bytes_stream()));
        let max_body_bytes = state.log_config().max_body_bytes;
        sse_response(
            &state,
            Box::pin(converted.map(|r| r.map_err(|e| Box::new(e) as BoxError))),
            UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                client_key_id: routed.client_key_id.clone(),
                catalog_id: provider.catalog_id.clone(),
                upstream_key: Some(upstream_key.clone()),
                started_unix,
                model,
                served_model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                // The Responses usage the adapter emits keeps `input_tokens`
                // inclusive, the same arithmetic the Chat intermediate used.
                cache_inclusive: true,
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
            status,
            response_headers,
        )
    } else {
        let bytes = match read_body_or_response(
            &mut upstream,
            &state,
            log.as_ref(),
            provider,
            &routed,
            inbound,
        )
        .await
        {
            Ok(b) => b,
            Err(resp) => return resp,
        };
        let latency_ms = started.elapsed().as_millis() as i64;
        // Raw Anthropic bytes: `input_tokens` excludes the cache buckets.
        let (usage, upstream_model) = parse_response_usage(Protocol::Anthropic, &bytes);

        if !status.is_success() {
            let log = finish_log(log, &bytes, status, Some(&response_headers), &state);
            let sample = UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                client_key_id: routed.client_key_id.clone(),
                catalog_id: provider.catalog_id.clone(),
                upstream_key: Some(upstream_key.clone()),
                started_unix,
                model: model.or_else(|| upstream_model.clone()),
                served_model: upstream_model,
                usage_missing: usage.is_none(),
                usage: usage.unwrap_or_default(),
                latency_ms,
                status: "error",
                cache_inclusive: false,
                log,
            };
            return buffered_response(&state, bytes, sample, status, response_headers);
        }

        let responses = match serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| GatewayError::Upstream(format!("upstream body is not JSON: {e}")))
            .and_then(|v| {
                anthropic_to_responses(v)
                    .map_err(|e| GatewayError::Upstream(format!("response conversion failed: {e}")))
            }) {
            Ok(v) => v,
            Err(e) => {
                let message = e.to_string();
                let resp = error_into_response(e, inbound);
                log_failure(
                    &state,
                    log.as_ref(),
                    provider,
                    &routed,
                    resp.status(),
                    "conversion_failed",
                    message,
                );
                return resp;
            }
        };
        let out = match serde_json::to_vec(&responses) {
            Ok(b) => b,
            Err(e) => {
                let resp = error_into_response(
                    GatewayError::Upstream(format!("serializing converted response failed: {e}")),
                    inbound,
                );
                log_failure(
                    &state,
                    log.as_ref(),
                    provider,
                    &routed,
                    resp.status(),
                    "internal_error",
                    format!("serializing converted response failed: {e}"),
                );
                return resp;
            }
        };

        let log = finish_log(log, &out, status, Some(&response_headers), &state);
        let sample = UsageSample {
            agent: routed.agent.clone(),
            provider_id: provider.id.clone(),
            client_key_id: routed.client_key_id.clone(),
            catalog_id: provider.catalog_id.clone(),
            upstream_key: Some(upstream_key.clone()),
            started_unix,
            model: model.or_else(|| upstream_model.clone()),
            served_model: upstream_model,
            usage_missing: usage.is_none(),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: "ok",
            cache_inclusive: false,
            log,
        };
        let mut response =
            buffered_response(&state, Bytes::from(out), sample, status, response_headers);
        if let Ok(ct) = HeaderValue::from_str("application/json") {
            response
                .headers_mut()
                .insert(axum::http::header::CONTENT_TYPE, ct);
        }
        response
    }
}
