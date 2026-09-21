//! Sending one request upstream: which failures are worth retrying, the
//! per-provider timeout, and the ceiling on a buffered body.

use std::time::{Duration, Instant};

use axum::body::Bytes;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::response::Response;

use crate::error::GatewayError;
use crate::forward::finish::log_failure;
use crate::forward::sample::CompletedLog;
use crate::log_capture::RequestCapture;
use crate::router::{RoutedRequest, UpstreamProvider};
use crate::server::{error_into_response, GatewayState};
use crate::store::Protocol;

/// Whether an upstream status means "this provider did not serve it": request
/// timeout, rate limiting, and server-side errors. Client errors (4xx besides
/// 408/429) are deterministic — they are the request's own problem, and asking
/// a second provider to re-reject the same body only spends money to get the
/// same answer.
///
/// Read twice: by the same-provider retry loop (migration v8) and by the data
/// plane's decision to replay the request against the next candidate.
pub(crate) fn is_retryable_status(status: StatusCode) -> bool {
    status == StatusCode::REQUEST_TIMEOUT
        || status == StatusCode::TOO_MANY_REQUESTS
        || status.is_server_error()
}

/// Ceiling on a non-streaming upstream response the gateway will hold whole.
///
/// An SSE body is streamed through and costs no memory however long it runs;
/// this is about the other kind. A non-streaming body is buffered because three
/// things need it at once — it is metered, it is captured for the request log,
/// and on the conversion path it is rewritten — so without a ceiling a single
/// upstream (or a proxy in front of one) could ask this process for as much
/// memory as it liked. Past the cap the request fails with a message naming the
/// limit, rather than a truncated answer that would be billed as a whole one.
///
/// 16 MiB is far past any real completion (the largest published output caps are
/// ~64k tokens, well under a megabyte) and far enough under the 32 MiB inbound
/// cap that a request and its answer are bounded by the same order of magnitude.
pub const MAX_UPSTREAM_BODY_BYTES: usize = 16 * 1024 * 1024;

/// Why a non-streaming upstream body could not be taken whole.
pub(crate) enum UpstreamBodyError {
    /// Past the cap. A distinct kind in the log, because the fix is not the same
    /// as for a broken transport: nothing is wrong with the upstream, the answer
    /// is simply bigger than this gateway will hold.
    TooLarge(String),
    /// The transport broke mid-body.
    Read(String),
}

impl UpstreamBodyError {
    pub(crate) fn message(&self) -> &str {
        match self {
            UpstreamBodyError::TooLarge(m) | UpstreamBodyError::Read(m) => m,
        }
    }

    pub(crate) fn kind(&self) -> &'static str {
        match self {
            UpstreamBodyError::TooLarge(_) => "response_too_large",
            UpstreamBodyError::Read(_) => "upstream_error",
        }
    }
}

/// Read an upstream body whole, refusing to grow past `limit`.
///
/// Checks `content-length` first so a declared oversize fails before any of it is
/// read, then enforces the cap while reading for upstreams that stream chunked and
/// declare nothing.
pub(crate) async fn read_upstream_body_capped(
    upstream: &mut reqwest::Response,
    limit: usize,
) -> Result<Bytes, UpstreamBodyError> {
    if let Some(len) = upstream.content_length() {
        if len > limit as u64 {
            return Err(UpstreamBodyError::TooLarge(format!(
                "upstream response declares {len} bytes, over the {limit}-byte cap"
            )));
        }
    }
    let mut out: Vec<u8> = Vec::new();
    loop {
        let chunk = match upstream.chunk().await {
            Ok(Some(c)) => c,
            Ok(None) => break,
            Err(e) => {
                return Err(UpstreamBodyError::Read(format!(
                    "reading upstream body failed: {e}"
                )))
            }
        };
        if out.len() + chunk.len() > limit {
            return Err(UpstreamBodyError::TooLarge(format!(
                "upstream response exceeded the {limit}-byte cap (this request is not streaming; ask the upstream to stream)"
            )));
        }
        out.extend_from_slice(&chunk);
    }
    Ok(Bytes::from(out))
}

/// Read the upstream body whole — or hand back the response the client gets
/// instead, its request-log row already written.
#[allow(clippy::result_large_err)] // Err is the client-facing response, not a diagnostic
pub(crate) async fn read_body_or_response(
    upstream: &mut reqwest::Response,
    state: &GatewayState,
    log: Option<&CompletedLog>,
    provider: &UpstreamProvider,
    routed: &RoutedRequest,
    inbound: Option<Protocol>,
) -> Result<Bytes, Response> {
    match read_upstream_body_capped(upstream, MAX_UPSTREAM_BODY_BYTES).await {
        Ok(b) => Ok(b),
        Err(e) => {
            let resp =
                error_into_response(GatewayError::Upstream(e.message().to_string()), inbound);
            log_failure(
                state,
                log,
                provider,
                routed,
                resp.status(),
                e.kind(),
                e.message().to_string(),
            );
            Err(resp)
        }
    }
}

/// A failed send attempt: either the per-provider response-header timeout
/// elapsed or the transport itself broke (connect/DNS/reset).
enum SendFailure {
    Timeout(u64),
    Transport(reqwest::Error),
}

impl SendFailure {
    /// A transport failure carrying no URL of its own.
    ///
    /// `reqwest::Error`'s `Display` ends in `for url (<the full url>)`, query
    /// string and all, so the error would hand back exactly the credential
    /// [`Self::message`] is about to redact — and this message is persisted as
    /// `error_message`. The message names the URL already; the error does not
    /// need to repeat it. Stripping it here, at the one place the failure is
    /// built, keeps the trace, the client-facing error and the stored column
    /// from disagreeing.
    fn transport(e: reqwest::Error) -> Self {
        SendFailure::Transport(e.without_url())
    }

    /// The failure text, with the URL's query credentials redacted.
    ///
    /// This message reaches three readers — the tracing line, the client's
    /// error response, and the `error_message` column — and it is built once
    /// here so none of them can be the one that forgot. The *request* is still
    /// sent to the raw URL; only the sentence about it is scrubbed. The client
    /// sees its own query replaced by `[REDACTED]` in the failure text, which
    /// costs it nothing (it sent the query) and stops the gateway from echoing
    /// a key back at whoever is reading over its shoulder.
    ///
    /// Note the provider's *response body* never arrives here:
    /// [`SendFailure::Transport`] wraps a reqwest transport error, not
    /// provider content, so an upstream that names the key it rejected in its
    /// error JSON cannot reach this column. The URL was the only way in.
    fn message(&self, url: &str) -> String {
        let url = crate::log_capture::redact_url(url);
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
/// Each attempt asks the breaker for admission and records its feedback exactly
/// once (success is judged at response-header time, tech.md §4.7). A retryable
/// outcome (transport error, 408/429/5xx) with attempts left backs off briefly
/// and re-sends the same request to the same provider; the strategy layer only
/// takes over after this loop gives up. The loop returns before any byte
/// reaches the client, so in-flight streams are never retried; body-read
/// failures after headers are also not retried (the breaker was already fed at
/// header time).
#[allow(clippy::too_many_arguments)] // every argument is a distinct routing input
#[allow(clippy::result_large_err)] // Err is the client-facing response, not a diagnostic
pub(crate) async fn send_upstream(
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
        // Admission is asked per attempt, not once per request: in HalfOpen the
        // breaker hands out a single probe permit, and a probe that has already
        // failed re-opened the circuit — sending the retry would be a second
        // probe the breaker never granted. Closed admits everything, so this is
        // free on the ordinary path.
        let admission = state.engine.allow(agent, &provider.id).await;
        if !admission.allowed {
            // Not recorded as a failure: nothing was attempted, and a denial
            // counted as a failure would drive the breaker further open on its
            // own refusals.
            let circuit = GatewayError::CircuitOpen {
                agent: agent.to_string(),
                provider: provider.id.clone(),
            };
            let message = circuit.to_string();
            let resp = error_into_response(circuit, inbound);
            crate::log_capture::persist_failure(
                &state.store,
                capture,
                Some(agent.to_string()),
                Some(attribution.to_string()),
                Some(provider.id.clone()),
                resp.status(),
                "circuit_open",
                message,
            );
            return Err(resp);
        }
        // Timed per attempt, not per request: what a retry cost in wall-clock
        // terms is the number that makes a flaky upstream visible.
        let attempt_started = Instant::now();
        let send = state
            .http
            .request(method.clone(), url)
            .headers(headers.clone())
            .body(body.clone())
            .send();
        let outcome: Result<reqwest::Response, SendFailure> = match provider.timeout_secs {
            Some(secs) => match tokio::time::timeout(Duration::from_secs(secs), send).await {
                Ok(r) => r.map_err(SendFailure::transport),
                Err(_) => Err(SendFailure::Timeout(secs)),
            },
            None => send.await.map_err(SendFailure::transport),
        };
        let attempt_ms = Some(attempt_started.elapsed().as_millis() as i64);

        match outcome {
            Ok(upstream) => {
                let status = upstream.status();
                state
                    .engine
                    .record(
                        agent,
                        &provider.id,
                        status.is_success(),
                        admission.used_half_open_permit,
                    )
                    .await;
                if !last && is_retryable_status(status) {
                    // Its own row: the client never sees this status (the retry
                    // hides it), and the request-level row will carry the last
                    // attempt's outcome instead. Without this the retry count and
                    // the time each one burned exist only in the daemon log.
                    crate::log_capture::persist_attempt_failure(
                        &state.store,
                        capture,
                        Some(agent.to_string()),
                        Some(attribution.to_string()),
                        Some(provider.id.clone()),
                        status,
                        "attempt_retryable_status",
                        format!("attempt {attempt}/{attempts}: upstream answered {status}"),
                        attempt_ms,
                    );
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
                state
                    .engine
                    .record(agent, &provider.id, false, admission.used_half_open_permit)
                    .await;
                if !last {
                    crate::log_capture::persist_attempt_failure(
                        &state.store,
                        capture,
                        Some(agent.to_string()),
                        Some(attribution.to_string()),
                        Some(provider.id.clone()),
                        StatusCode::BAD_GATEWAY,
                        "attempt_failed",
                        format!("attempt {attempt}/{attempts}: {message}"),
                        attempt_ms,
                    );
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::provider;

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

    /// The failure text is what lands in `error_message`, so the credential a
    /// client put in the query string must not survive into it.
    #[test]
    fn send_failure_message_redacts_query_credentials() {
        let url = "https://generativelanguage.googleapis.com/v1beta/models/g:generateContent?key=AIzaSyLiveKey&alt=json";

        let timeout = SendFailure::Timeout(30).message(url);
        assert!(!timeout.contains("AIzaSyLiveKey"), "{timeout}");
        assert!(timeout.contains("key=[REDACTED]"), "{timeout}");
        assert!(timeout.contains("&alt=json"), "{timeout}");
        assert!(timeout.contains("timed out after 30s"), "{timeout}");

        // An ordinary URL is quoted back unchanged, minus nothing.
        let plain = SendFailure::Timeout(30)
            .message("https://api.anthropic.com/v1/messages?model=x&stream=true");
        assert_eq!(
            plain,
            "request to `https://api.anthropic.com/v1/messages?model=x&stream=true` timed out after 30s"
        );
    }

    /// `reqwest::Error`'s `Display` ends in `for url (<full url>)`, so the
    /// transport error would put the key back after the redaction above. The
    /// assertion on the raw error proves the premise; the one on the message
    /// proves `SendFailure::transport` closes it.
    #[tokio::test]
    async fn transport_failure_carries_no_url_of_its_own() {
        // Port 1 on loopback refuses immediately — no listener, no network.
        let url = "http://127.0.0.1:1/v1/messages?key=AIzaSyLiveKey";
        let raw = reqwest::Client::new()
            .post(url)
            .send()
            .await
            .expect_err("connect to a closed port must fail");
        assert!(
            raw.to_string().contains("AIzaSyLiveKey"),
            "premise: reqwest's Display leaks the url, got `{raw}`"
        );

        let message = SendFailure::transport(raw).message(url);
        assert!(!message.contains("AIzaSyLiveKey"), "{message}");
        assert!(message.contains("key=[REDACTED]"), "{message}");
        assert!(message.contains("failed: "), "{message}");
    }

    // ── the retry loop and the per-provider timeout, end to end ──────────────
    //
    // `retryable_status_classification` pins which statuses *count* as
    // retryable; nothing until now drove the loop that acts on it, or the
    // timeout that ends a wait. Both need a socket, and neither needs a test
    // framework: a scripted stub is a few lines of raw HTTP.

    /// What one connection does. Each entry answers one connection, so a script
    /// is also a count of how many attempts actually arrived.
    enum Stub {
        Answer(u16, &'static str),
        /// Accept, then say nothing until the caller gives up — the only way to
        /// observe a timeout rather than a refusal.
        Silence,
    }

    /// A loopback server answering `script` in order, with the URL to call.
    async fn stub(script: Vec<Stub>) -> (String, tokio::task::JoinHandle<()>) {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind a loopback port");
        let addr = listener.local_addr().expect("a bound port has an address");
        let handle = tokio::spawn(async move {
            for step in script {
                let Ok((mut sock, _)) = listener.accept().await else {
                    break;
                };
                // Read the request head; the answer does not depend on it.
                let mut buf = [0u8; 8192];
                let _ = sock.read(&mut buf).await;
                match step {
                    Stub::Answer(status, body) => {
                        let head = format!(
                            "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            body.len()
                        );
                        let _ = sock.write_all(head.as_bytes()).await;
                        let _ = sock.write_all(body.as_bytes()).await;
                    }
                    Stub::Silence => tokio::time::sleep(Duration::from_secs(5)).await,
                }
                let _ = sock.shutdown().await;
            }
        });
        (format!("http://{addr}/v1/chat/completions"), handle)
    }

    /// The loop, not the predicate: a 429 is re-sent, and the second answer is
    /// the one the caller gets.
    #[tokio::test]
    async fn a_retryable_status_is_retried_and_the_second_answer_stands() {
        let (url, server) = stub(vec![
            Stub::Answer(429, ""),
            Stub::Answer(200, "{\"ok\":true}"),
        ])
        .await;
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.base_url = url.clone();
        p.retries = Some(1);

        let outcome = send_upstream(
            &state,
            &p,
            "claude",
            "claude:1",
            Method::POST,
            &url,
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
            None,
            None,
        )
        .await;

        let resp = outcome.expect("the retry's own answer, not a 502");
        assert_eq!(
            resp.status(),
            StatusCode::OK,
            "the second attempt's answer is what the caller sees"
        );
        server.abort();
    }

    /// A peer that accepts and never answers: the provider's own timeout ends the
    /// wait, and with no attempts left the caller gets a 502 instead of a hang.
    #[tokio::test]
    async fn a_silent_upstream_hits_the_timeout_and_answers_502() {
        let (url, server) = stub(vec![Stub::Silence]).await;
        let state =
            crate::server::GatewayState::new(crate::store::Store::open_in_memory().expect("store"))
                .expect("state");
        let mut p = provider(Protocol::OpenAI, None);
        p.base_url = url.clone();
        p.timeout_secs = Some(1);
        p.retries = None;

        let started = std::time::Instant::now();
        let outcome = send_upstream(
            &state,
            &p,
            "claude",
            "claude:1",
            Method::POST,
            &url,
            HeaderMap::new(),
            Bytes::from_static(b"{}"),
            None,
            None,
        )
        .await;

        let resp = outcome.expect_err("a timeout is not an answer");
        assert_eq!(resp.status(), StatusCode::BAD_GATEWAY);
        // The provider's second ended it, not the client's 300s read timeout.
        assert!(
            started.elapsed() < Duration::from_secs(4),
            "took {:?}",
            started.elapsed()
        );
        server.abort();
    }
}
