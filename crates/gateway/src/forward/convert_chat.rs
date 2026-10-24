//! The third conversion path: an OpenAI **Chat Completions** request inbound on
//! a provider that speaks only **Anthropic Messages**.
//!
//! The mirror of `forward::convert`, and the leg that lets an OpenAI-only client
//! (Cline, Aider, Continue, OpenCode, Droid, …) use an upstream which exposes
//! nothing but `/v1/messages`.
//!
//! Two metering details differ from the other legs, and both follow from the
//! upstream being Anthropic:
//!
//! - The buffered path makes one call before converting anything, so it is
//!   metered from the **raw Anthropic** body — where `input_tokens` excludes the
//!   cache buckets — with `cache_inclusive: false`.
//! - The streaming path is metered from the **converted Chat** bytes, whose
//!   usage the adapter builds inclusive (`prompt_tokens` contains the cache
//!   buckets), with `cache_inclusive: true`. The usage chunk is always emitted
//!   and `StreamPolicy` drops it from the client's view when the request did not
//!   ask for it, the same way the native Chat path handles a client that did not
//!   request usage.

use std::sync::Arc;
use std::time::Instant;

use axum::body::Bytes;
use axum::http::{HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;

use kiwano_adapters::proxy::model_mapper::strip_one_m_suffix_for_upstream_from_body;
use kiwano_adapters::proxy::providers::chat_streaming::create_chat_sse_stream_from_anthropic;
use kiwano_adapters::proxy::providers::transform::{anthropic_to_chat, chat_to_anthropic};

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

/// Forward a `/v1/chat/completions` request to an Anthropic-only provider.
///
/// Request: `chat_to_anthropic` + the 1M-context marker strip. Response:
/// non-SSE bodies go through `anthropic_to_chat`; SSE streams are rebuilt as
/// Chat chunks by `create_chat_sse_stream_from_anthropic`. Upstream error
/// bodies pass through unconverted, for the reason the other legs give: an
/// error object is not a `chat.completion`, and converting it would corrupt
/// the one thing the caller can act on.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn forward_chat_via_anthropic(
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

    // The requested model is authoritative for metering.
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
    // Read before conversion: `stream_options` has no Anthropic home, and this
    // is the only thing the gateway needs it for — whether the client is owed
    // the trailing usage chunk.
    let client_wants_usage = converted_body.get("stream").and_then(|v| v.as_bool()) == Some(true)
        && converted_body
            .pointer("/stream_options/include_usage")
            .and_then(|v| v.as_bool())
            == Some(true);

    let anthropic_body = match chat_to_anthropic(converted_body) {
        Ok(v) => v,
        Err(e) => {
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
        "converting OpenAI chat request to Anthropic messages"
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
    // Query strings are meaningless across protocol conversion; drop them.
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
        // The scanner reads the converted Chat bytes, whose usage is inclusive.
        let converted = create_chat_sse_stream_from_anthropic(Box::pin(upstream.bytes_stream()));
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
                // Filled at `finish` from the stream's own chunks.
                served_model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                // The adapter builds Chat usage inclusive: prompt_tokens holds
                // the cache buckets.
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
                // Scanned first, dropped after: metering keeps the numbers, a
                // client that never asked for usage never sees the chunk.
                strip_usage_chunk: !client_wants_usage,
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
        // The upstream is Anthropic: `input_tokens` is fresh input only, so the
        // cache buckets are separate and must not be subtracted again.
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

        let chat = match serde_json::from_slice::<serde_json::Value>(&bytes)
            .map_err(|e| GatewayError::Upstream(format!("upstream body is not JSON: {e}")))
            .and_then(|v| {
                anthropic_to_chat(v)
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
        let out = match serde_json::to_vec(&chat) {
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
        // The converted body is always JSON, whatever the upstream sent.
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
