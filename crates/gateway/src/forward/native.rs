//! The native forward: the provider speaks the inbound protocol, so the bytes
//! pass through unmapped and are metered on the way.

use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;
use futures_util::StreamExt;

use crate::forward::convert::forward_anthropic_via_openai;
use crate::forward::convert_chat::forward_chat_via_anthropic;
use crate::forward::convert_responses_anthropic::forward_responses_via_anthropic;
use crate::forward::finish::{buffered_response, finish_log, log_failure, sse_response};
use crate::forward::headers::{
    copy_response_headers, response_headers_text, upstream_key_and_headers,
};
use crate::forward::inbound::{resolve_inbound, InboundResolution};
use crate::forward::sample::{attribution_str, CompletedLog, UsageSample};
use crate::forward::shim::{apply_compat_shim, ensure_openai_stream_usage};
use crate::forward::stream::StreamPolicy;
use crate::forward::upstream::{read_body_or_response, send_upstream};
use crate::forward::BoxError;
use crate::log_capture::RequestCapture;
use crate::meter::{model_from_path, parse_response_usage, request_model, Usage};
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::data::upstream_url;
use crate::server::{error_response, GatewayState};
use crate::store::Protocol;

/// A conversion's upstream attempt: the provider row with the endpoint and
/// protocol the conversion actually speaks.
///
/// The protocol has to be overridden together with the URL. A conversion is
/// aimed at a protocol the *endpoint* speaks, which is not always the
/// provider's primary one — leaving the row's protocol in place would build
/// the wrong credentials (an `Authorization` bearer against an Anthropic
/// endpoint, or `x-api-key` against an OpenAI one).
fn conversion_target(
    provider: &UpstreamProvider,
    protocol: Protocol,
    base_url: String,
    api_path: Option<String>,
) -> UpstreamProvider {
    UpstreamProvider {
        protocol,
        base_url,
        api_path,
        ..provider.clone()
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
    // The wall clock the start corresponds to: `Instant` measures, it cannot say
    // *when*, and the price tier is decided by when.
    let started_unix = Utc::now().timestamp();
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
        request_notes: None,
    });

    // The model the client asked for, read once: the endpoint decision below
    // needs it too (a catalogue that splits its models across endpoints), and
    // the metering path reads the same field for the same reason.
    let model = request_model(&body).or_else(|| model_from_path(&path));

    // Resolve the endpoint + protocol for the inbound flavor: native when it
    // matches the provider (or the path is ambiguous), natively via a
    // registered per-protocol endpoint when one exists, otherwise the legacy
    // Anthropic → OpenAI conversion, or a clean mismatch failure.
    let provider_for_alt;
    let provider: &UpstreamProvider =
        match resolve_inbound(provider, inbound, &path, model.as_deref()) {
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
            InboundResolution::ConvertResponsesToChat => {
                return crate::forward::convert_responses::forward_responses_via_chat(
                    state,
                    method,
                    inbound_headers,
                    body,
                    routed,
                    inbound,
                    started,
                    started_unix,
                    log,
                )
                .await;
            }
            InboundResolution::ConvertAnthropicToOpenAI { base_url, api_path } => {
                // The conversion may be aimed at an endpoint other than the
                // provider's primary one — the model decided that above — so the
                // attempt carries the chosen URL rather than the row's own.
                let target = RoutedRequest {
                    provider: conversion_target(provider, Protocol::OpenAI, base_url, api_path),
                    ..routed
                };
                return forward_anthropic_via_openai(
                    state,
                    method,
                    inbound_headers,
                    body,
                    target,
                    inbound,
                    started,
                    started_unix,
                    log,
                )
                .await;
            }
            InboundResolution::ConvertChatToAnthropic { base_url, api_path } => {
                let target = RoutedRequest {
                    provider: conversion_target(provider, Protocol::Anthropic, base_url, api_path),
                    ..routed
                };
                return forward_chat_via_anthropic(
                    state,
                    method,
                    inbound_headers,
                    body,
                    target,
                    inbound,
                    started,
                    started_unix,
                    log,
                )
                .await;
            }
            InboundResolution::ConvertResponsesToAnthropic { base_url, api_path } => {
                let target = RoutedRequest {
                    provider: conversion_target(provider, Protocol::Anthropic, base_url, api_path),
                    ..routed
                };
                return forward_responses_via_anthropic(
                    state,
                    method,
                    inbound_headers,
                    body,
                    target,
                    inbound,
                    started,
                    started_unix,
                    log,
                )
                .await;
            }
            InboundResolution::Mismatch { message } => {
                // count_tokens is the one path where a mismatch is a gap the
                // gateway can close itself: the client is asking how much
                // context it has left, not for an upstream answer. Estimate and
                // answer in the Anthropic shape rather than failing a check
                // that runs before every request. See `forward/count_tokens.rs`.
                if path == "/v1/messages/count_tokens" {
                    return answer_count_tokens(state, body, log, &routed, inbound, started);
                }
                log_failure(
                    &state,
                    log.as_ref(),
                    provider,
                    &routed,
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

    // `model` was read above, where the endpoint decision needed it.

    // The compat shim runs first, on the client's own bytes: strip parameters
    // the effective upstream cannot parse, and remember what changed so the
    // request log can explain the delta. Keyed on the resolved provider —
    // an Alternate endpoint's protocol counts — and skipped entirely when the
    // global switch is off.
    let shimmed = if state.compat_shim_enabled() {
        apply_compat_shim(&body, provider.protocol, &provider.base_url)
    } else {
        None
    };
    // One note field, two sources. The credential detector read the request on
    // the way in (`data.rs`), and the shim describes what it rewrote here.
    // Either, both or neither can be present, so this composes rather than
    // assigns — an assignment would let the shim's silence erase a finding.
    // Detector first: what left the machine is the more important line.
    if let Some(l) = log.as_mut() {
        let dlp = l.capture.dlp_note.clone();
        let shim = shimmed.as_ref().map(|(_, notes)| notes.clone());
        l.request_notes = match (dlp, shim) {
            (Some(d), Some(s)) => Some(format!("{d}\n{s}")),
            (Some(d), None) => Some(d),
            (None, Some(s)) => Some(s),
            (None, None) => None,
        };
    }
    let body = shimmed.map_or_else(|| body.clone(), |(b, _)| b);

    // Ask the upstream to report usage when the client did not, and remember
    // that we did: the extra chunk it sends back is ours to take out again.
    //
    // Native OpenAI chat is the one path where this is needed. A converted
    // request (Anthropic → OpenAI) already gets the field injected by the
    // converter, and the Responses API reports usage without being asked — so
    // what is left is a chat-completions client that never set the option, and
    // whose spend the meter could not see at all.
    let (body, strip_usage_chunk) = if provider.protocol == crate::store::Protocol::OpenAI {
        match ensure_openai_stream_usage(&body) {
            Some(patched) => (patched, true),
            None => (body.clone(), false),
        }
    } else {
        (body.clone(), false)
    };

    let mut upstream = match send_upstream(
        &state,
        provider,
        &routed.agent,
        &attribution_str(routed.attribution),
        method,
        &url,
        headers,
        body,
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

    // A 404 on a `/v1/responses` request to a provider declared as serving both
    // wires is the one failure the user can fix and cannot diagnose: the request
    // went out as it arrived, and the vendor's "no such endpoint" says nothing
    // about a declaration the gateway offers. The note names the fix rather than
    // the gateway guessing at it — a `base_url` we cannot see behind is not
    // something to infer a wire from.
    if status == StatusCode::NOT_FOUND
        && provider.protocol == crate::store::Protocol::OpenAI
        && provider.openai_wire == crate::store::OpenAiWire::Both
        && crate::protocol::openai_wire_of_path(&path)
            == Some(crate::protocol::OpenAiWirePath::Responses)
    {
        if let Some(l) = log.as_mut() {
            let note = format!(
                "provider `{}` answered 404 for `/v1/responses`; if it serves only Chat \
                 Completions, declare that with `kiwano providers edit {} --openai-wire chat` and \
                 the request will be converted instead",
                provider.id, provider.id
            );
            l.request_notes = Some(match l.request_notes.take() {
                Some(existing) => format!("{existing}\n{note}"),
                None => note,
            });
        }
        tracing::warn!(
            provider = %provider.id,
            agent = %routed.agent,
            "404 on /v1/responses from a provider declared as serving both OpenAI wires"
        );
    }

    // reqwest consumes the Response on bytes()/bytes_stream(), so snapshot
    // the client-facing headers first.
    let response_headers = copy_response_headers(upstream.headers());
    if let Some(l) = log.as_mut() {
        l.response_headers = Some(response_headers_text(&response_headers));
    }

    if is_sse {
        // Streaming passthrough with usage scanning + response capture; the
        // sample is persisted by a side task when the stream finishes.
        //
        // `None` here is both "logging is off" and "no cap configured", and
        // the two are the same answer for the buffer this bounds: with logging
        // off nothing is persisted, and with no cap configured the whole
        // response is kept. Neither is a cap of zero.
        let max_body_bytes = log.as_ref().and_then(|_| state.log_config().max_body_bytes);
        sse_response(
            &state,
            Box::pin(upstream.bytes_stream().map(|r| r.map_err(BoxError::from))),
            UsageSample {
                agent: routed.agent.clone(),
                provider_id: provider.id.clone(),
                client_key_id: routed.client_key_id.clone(),
                catalog_id: provider.catalog_id.clone(),
                upstream_key: Some(upstream_key.clone()),
                started_unix,
                model,
                // Filled at `finish` from the stream's own `message_start`.
                served_model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                cache_inclusive: matches!(provider.protocol, Protocol::OpenAI | Protocol::Gemini),
                // The stream fills this in at `finish`, from what the scanner saw.
                usage_missing: false,
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
                    request_notes: l.request_notes,
                }),
            },
            started,
            StreamPolicy {
                capture_cap: max_body_bytes,
                redactor: state.redactor(),
                strip_usage_chunk,
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
        let (usage, upstream_model) = parse_response_usage(provider.protocol, &bytes);
        // The headers were written to the log before the branch, for both
        // shapes; the body is the half this leg still owes it.
        let log = finish_log(log, &bytes, status, None, &state);
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
            status: if status.is_success() { "ok" } else { "error" },
            cache_inclusive: matches!(provider.protocol, Protocol::OpenAI | Protocol::Gemini),
            log,
        };
        buffered_response(&state, bytes, sample, status, response_headers)
    }
}

/// Answer `POST /v1/messages/count_tokens` locally, for an upstream that has
/// no such endpoint.
///
/// The client is asking how much of its context window is spent — a question
/// the gateway can answer approximately from the bytes it already holds, and
/// the wrong direction of error is known: over-counting compacts early, which
/// costs a turn; under-counting runs out of window mid-turn, which costs the
/// request. See `forward/count_tokens.rs` for how the number is made.
///
/// The row in the request log says what happened: status 200, no error, and a
/// note that the count is an estimate — so a reader of the audit trail is not
/// misled into thinking the upstream answered. **No usage row is written**:
/// the number is an approximation for context management, not a billing fact,
/// and nothing was sent upstream to be metered.
fn answer_count_tokens(
    state: Arc<GatewayState>,
    body: Bytes,
    log: Option<CompletedLog>,
    routed: &RoutedRequest,
    inbound: Option<Protocol>,
    started: Instant,
) -> Response {
    let Some(tokens) = crate::forward::count_tokens::estimate(&body) else {
        // The client asked a question about a body we cannot parse. A 400 with
        // the reason is more useful than an estimate of nothing.
        return error_response(
            inbound,
            StatusCode::BAD_REQUEST,
            "invalid_request_error",
            "kiwanod: the request body is not JSON",
        );
    };
    let bytes = Bytes::from(serde_json::json!({ "input_tokens": tokens }).to_string());
    let latency_ms = started.elapsed().as_millis() as i64;

    if let Some(mut l) = log {
        l.status_code = 200;
        l.response_size = bytes.len() as i64;
        l.response_body = Some(String::from_utf8_lossy(&bytes).into_owned());
        l.request_notes = Some(
            "count_tokens answered locally: the upstream has no such endpoint, so this is an \
             estimate (≈4 chars/token; images count 1500 each), not a metered count"
                .to_string(),
        );
        // A success row without usage: the request log's persistence path for
        // a request that was answered but not metered.
        let store = state.store.clone();
        let entry = crate::store::RequestLogNew {
            ts: l.capture.ts.clone(),
            method: l.capture.method.clone(),
            path: l.capture.path.clone(),
            query: l.capture.query.clone(),
            agent: Some(routed.agent.clone()),
            attribution: Some(attribution_str(routed.attribution)),
            provider_id: Some(routed.provider.id.clone()),
            model: crate::meter::request_model(&body),
            status_code: 200,
            error_kind: None,
            error_message: None,
            session_id: l.capture.session_id.clone(),
            is_streaming: false,
            input_tokens: tokens as i64,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(latency_ms),
            first_token_ms: None,
            request_headers: l.capture.request_headers.clone(),
            response_headers: None,
            request_body: l.capture.request_body.clone(),
            response_body: l.response_body.clone(),
            request_size: l.capture.request_size,
            response_size: l.response_size,
            truncated: l.capture.truncated,
            // No cost: nothing was sent, nothing is owed.
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
            request_notes: l.request_notes,
            upstream_key: None,
        };
        if let Err(e) = store.insert_request_log(&entry) {
            tracing::warn!(error = %e, "failed to persist count_tokens estimate");
        }
    }

    let mut response = Response::new(axum::body::Body::from(bytes));
    *response.status_mut() = StatusCode::OK;
    response
        .headers_mut()
        .insert("content-type", "application/json".parse().unwrap());
    response
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::provider;

    /// The conversion carries the endpoint's protocol, not the row's: otherwise
    /// the headers would be built for the provider's primary protocol and send
    /// the wrong credential to the endpoint the model chose.
    #[test]
    fn a_conversion_target_takes_the_endpoints_protocol() {
        let primary_openai = provider(Protocol::OpenAI, None);
        let target = conversion_target(
            &primary_openai,
            Protocol::Anthropic,
            "https://up.example.com".into(),
            Some("/anthropic".into()),
        );
        assert_eq!(target.protocol, Protocol::Anthropic);
        assert_eq!(target.base_url, "https://up.example.com");
        assert_eq!(target.api_path.as_deref(), Some("/anthropic"));
        // The rest of the row rides along.
        assert_eq!(target.id, primary_openai.id);
    }
}
