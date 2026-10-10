//! The second conversion path: an OpenAI **Responses** request inbound on a
//! provider that serves only **Chat Completions**.
//!
//! The OpenAI family has two wires, and until now the gateway treated them as
//! one: `/v1/chat/completions` and `/v1/responses` both classified as
//! `Protocol::OpenAI`, both passed through verbatim, and a provider that serves
//! only the first answered the second with a 404 of its own — a failure the
//! gateway had no way to explain, because it had no way to notice. A provider
//! can now say which wire it serves (`openai_wire`, migration v30), and when the
//! answer is "Chat only", a Responses request is converted here instead.
//!
//! The shape mirrors `forward::convert` (the Anthropic leg) exactly, because the
//! work is the same: convert the request in the adapters sublayer, send it to the
//! endpoint the provider actually serves, and convert the answer back — with the
//! metering scanner reading the *client-visible* bytes, so usage is counted from
//! what the client was told rather than from an upstream shape it never sees.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;

use kiwano_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_adapters::proxy::providers::responses_streaming::create_responses_sse_stream;
use kiwano_adapters::proxy::providers::transform::{
    chat_to_responses, inject_openai_stream_include_usage, responses_to_chat,
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

/// Forward a `/v1/responses` request to a Chat-Completions-only provider.
///
/// Request: `responses_to_chat` + `stream_options.include_usage` injection (a
/// Chat upstream reports usage only when asked) + the 1M-context marker strip.
/// Response: non-SSE bodies go through `chat_to_responses`; SSE streams are
/// rebuilt as the Responses event sequence by `create_responses_sse_stream`,
/// whose `response.completed` event is what the metering scanner reads. Upstream
/// error bodies pass through unconverted, for the reason the Anthropic leg gives.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_responses_via_chat(
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

    // The model the client asked for is authoritative for metering; the upstream
    // names the same one back, and `model.or(upstream_model)` prefers the ask.
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
    let mut chat_body = match responses_to_chat(converted_body) {
        Ok(v) => v,
        Err(e) => {
            // The reason, not a summary of it: the refusals in this direction
            // exist to say what the Chat wire cannot carry (`previous_response_id`,
            // a built-in tool), and that is the only thing the reader can act on.
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
    inject_openai_stream_include_usage(&mut chat_body);
    let chat_body = strip_one_m_suffix_for_upstream_from_body(chat_body);
    tracing::info!(
        provider = %provider.id,
        agent = %routed.agent,
        model = model.as_deref().unwrap_or("<none>"),
        "converting Responses request to OpenAI chat completions"
    );
    let chat_bytes = match serde_json::to_vec(&chat_body) {
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

    // The converted request is a Chat request whatever the client sent, so this
    // is the path it goes to — not the one that arrived.
    let url = upstream_url(provider, "/v1/chat/completions");
    // Query strings are meaningless across the conversion; drop them.
    let headers = match upstream_key_and_headers(
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
        Bytes::from(chat_bytes),
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
        "upstream responded to converted Responses request"
    );

    let response_headers = copy_response_headers(upstream.headers());

    if is_sse {
        // Rebuild the upstream chunks as the Responses event sequence. The
        // scanner then reads `response.completed` — the shape it already
        // understands, because this is the wire Codex sends when it is not
        // converted at all.
        let converted = create_responses_sse_stream(Box::pin(upstream.bytes_stream()));
        let max_body_bytes = state.log_config().max_body_bytes;
        sse_response(
            &state,
            Box::pin(converted.map(|r| r.map_err(|e| Box::new(e) as BoxError))),
            UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                client_key_id: routed.client_key_id.clone(),
                catalog_id: provider.catalog_id.clone(),
                started_unix,
                model,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                // The upstream is OpenAI Chat, whose `prompt_tokens` is inclusive
                // of the cache buckets — the same arithmetic the Anthropic leg
                // applies, and the reason `chat_to_responses` keeps `input_tokens`
                // inclusive.
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
        let (usage, upstream_model) = parse_response_usage(Protocol::OpenAI, &bytes);

        if !status.is_success() {
            // An upstream error is the vendor's own shape (`{"error": {...}}`),
            // not a chat.completion object; converting it would rewrite a message
            // the client is meant to read verbatim.
            let log = finish_log(log, &bytes, status, Some(&response_headers), &state);
            let sample = UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                client_key_id: routed.client_key_id.clone(),
                catalog_id: provider.catalog_id.clone(),
                started_unix,
                model: model.or(upstream_model),
                usage_missing: usage.is_none(),
                usage: usage.unwrap_or_default(),
                latency_ms,
                status: "error",
                cache_inclusive: true,
                log,
            };
            return buffered_response(&state, bytes, sample, status, response_headers);
        }

        let responses = match serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| GatewayError::Upstream(format!("upstream body is not JSON: {e}")))
            .and_then(|v| {
                chat_to_responses(v)
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
            started_unix,
            model: model.or(upstream_model),
            usage_missing: usage.is_none(),
            usage: usage.unwrap_or_default(),
            latency_ms,
            status: "ok",
            cache_inclusive: true,
            log,
        };
        // The upstream headers described a Chat payload; the converted body is
        // always JSON.
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
