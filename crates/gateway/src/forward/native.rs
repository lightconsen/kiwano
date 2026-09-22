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
                started_unix,
                log,
            )
            .await;
        }
        InboundResolution::Mismatch { message } => {
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

    // Read the model from the client's own body, before the send consumes it —
    // the body states the model for the JSON-API protocols; the native Gemini
    // API names it in the path instead, so that shape is read from there.
    let model = request_model(&body).or_else(|| model_from_path(&path));

    // The compat shim runs first, on the client's own bytes: strip parameters
    // the effective upstream cannot parse, and remember what changed so the
    // request log can explain the delta. Keyed on the resolved provider —
    // an Alternate endpoint's protocol counts — and skipped entirely when the
    // global switch is off.
    let shimmed = if state.compat_shim_enabled() {
        apply_compat_shim(&body, provider.protocol)
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
                catalog_id: provider.catalog_id.clone(),
                started_unix,
                model,
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
            catalog_id: provider.catalog_id.clone(),
            started_unix,
            model: model.or(upstream_model),
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
