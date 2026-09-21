//! The SSE passthrough stream: the bytes go out unchanged, a scanner reads
//! the usage events alongside, and a silence limit ends a stalled stream.

use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};
use std::time::Instant;

use axum::body::Bytes;
use futures_core::Stream;
use tokio::sync::mpsc;

use crate::forward::sample::UsageSample;
use crate::forward::BoxError;
use crate::meter::UsageScanner;
use crate::store::Protocol;

/// The error event a client can act on, shaped for the protocol it is speaking.
///
/// A stream that just stops looks like a truncated response — an agent client
/// shows a parse error or nothing at all. Naming the failure in the client's own
/// dialect is what lets it surface a reason and retry. `None` when the inbound
/// protocol is unknown: there is no shape to imitate, so the stream ends.
fn stream_error_event(inbound: Option<Protocol>, message: &str) -> Option<Vec<u8>> {
    let quoted = serde_json::Value::String(message.to_string());
    match inbound {
        Some(Protocol::Anthropic) => Some(
            format!(
                "event: error\ndata: {{\"type\":\"error\",\"error\":{{\"type\":\"stream_timeout\",\"message\":{quoted}}}}}\n\n"
            )
            .into_bytes(),
        ),
        Some(Protocol::OpenAI) => Some(
            format!(
                "data: {{\"error\":{{\"message\":{quoted},\"type\":\"stream_timeout\"}}}}\n\ndata: [DONE]\n\n"
            )
            .into_bytes(),
        ),
        // Gemini's SSE error event carries the same code/message/status
        // envelope as its non-streaming errors; a stream event is not a
        // response body, so the status is the family name rather than a number.
        Some(Protocol::Gemini) => Some(
            format!(
                "data: {{\"error\":{{\"code\":504,\"message\":{quoted},\"status\":\"DEADLINE_EXCEEDED\"}}}}\n\n"
            )
            .into_bytes(),
        ),
        None => None,
    }
}

/// A byte-preserving SSE passthrough stream that scans complete lines for
/// usage events and submits the metered sample when the upstream stream ends.
///
/// Bytes are forwarded unchanged: complete `\n`-terminated lines are emitted
/// as they arrive, the trailing partial line is flushed at stream end. While
/// request logging is on, the same chunks tee into a capture buffer (capped)
/// that lands in `request_bodies` at stream end.
/// What a stream does to the bytes it forwards, and to the copy it keeps.
///
/// One value rather than a parameter each: they are decisions made together at
/// the request, and a constructor that took them separately grew past the point
/// where the argument list says what it is for.
pub(crate) struct StreamPolicy {
    /// Per-body capture cap; `None` stores every byte.
    pub(crate) capture_cap: Option<usize>,
    /// The credential scrubber, so a streamed response body goes through the
    /// same pass as every other body. It did not, for as long as this feature
    /// has existed: the one-shot paths called `cap_body` and this one copied its
    /// buffer straight into the row, so every streamed response was stored
    /// unredacted — on the side where an upstream may echo back the key it
    /// rejected.
    pub(crate) redactor: std::sync::Arc<crate::log_capture::Redactor>,
    /// Drop the usage-only chunk from what the client receives. True only when
    /// the gateway asked for that chunk itself — a client that never requested
    /// usage must not have one appear in its stream.
    pub(crate) strip_usage_chunk: bool,
}

pub(crate) struct SseUsageStream {
    inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
    buffer: Vec<u8>,
    scanner: UsageScanner,
    sample: UsageSample,
    tx: mpsc::Sender<UsageSample>,
    started: Instant,
    inner_ended: bool,
    /// `finish` has run, so neither the normal end nor `Drop` may run it again:
    /// it sends the sample, and a request must not be metered twice.
    finished: bool,
    // Response capture (request logging):
    capture_buf: Vec<u8>,
    capture_truncated: bool,
    streamed_bytes: u64,
    /// How the forwarded bytes and the kept copy are treated.
    policy: StreamPolicy,
    first_chunk: Option<Instant>,
    /// Who the client is, so a timeout can be reported in a shape it parses.
    inbound: Option<Protocol>,
    /// Silence limit: the first byte, then the gap between bytes.
    timeouts: crate::store::StreamTimeouts,
    /// Armed while a limit is outstanding; None when both limits are disabled.
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    timed_out: bool,
}

impl SseUsageStream {
    pub(crate) fn new(
        inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>>,
        tx: mpsc::Sender<UsageSample>,
        sample: UsageSample,
        started: Instant,
        policy: StreamPolicy,
        inbound: Option<Protocol>,
        timeouts: crate::store::StreamTimeouts,
    ) -> Self {
        let mut stream = SseUsageStream {
            inner,
            buffer: Vec::new(),
            scanner: UsageScanner::new(),
            sample,
            tx,
            started,
            inner_ended: false,
            finished: false,
            capture_buf: Vec::new(),
            capture_truncated: false,
            streamed_bytes: 0,
            policy,
            first_chunk: None,
            inbound,
            timeouts,
            deadline: None,
            timed_out: false,
        };
        // The first wait is for the stream's first byte, not for a gap.
        stream.arm();
        stream
    }

    /// Re-arm the silence limit for the wait that is now outstanding: the first
    /// byte until one arrives, the next byte after that.
    fn arm(&mut self) {
        let Some(after) = self.timeouts.deadline(self.first_chunk.is_some()) else {
            self.deadline = None;
            return;
        };
        let at = tokio::time::Instant::now() + after;
        match self.deadline.as_mut() {
            Some(d) => d.as_mut().reset(at),
            None => self.deadline = Some(Box::pin(tokio::time::sleep_until(at))),
        }
    }

    /// Give up on a stream that has stopped producing. Returns the error event
    /// the client can act on (or ends the stream when the inbound protocol is
    /// unknown and there is no shape to imitate).
    fn abort_on_timeout(&mut self) -> Poll<Option<Result<Bytes, BoxError>>> {
        let waited = self
            .timeouts
            .deadline(self.first_chunk.is_some())
            .unwrap_or_default();
        let message = format!(
            "upstream sent nothing for {}s ({}); the gateway abandoned the stream",
            waited.as_secs(),
            if self.first_chunk.is_some() {
                "no byte since the last one"
            } else {
                "no first byte"
            }
        );
        tracing::warn!(
            agent = %self.sample.agent,
            provider = %self.sample.provider_id,
            secs = waited.as_secs(),
            first_byte_seen = self.first_chunk.is_some(),
            "stream went silent; abandoned"
        );
        self.sample.status = "stream_timeout";
        if let Some(log) = self.sample.log.as_mut() {
            log.error_kind = Some("stream_timeout".to_string());
            log.error_message = Some(message.clone());
        }
        // The stream is over either way; `finish()` runs on the next poll.
        self.inner_ended = true;
        match stream_error_event(self.inbound, &message) {
            Some(bytes) => Poll::Ready(Some(Ok(Bytes::from(bytes)))),
            None => {
                self.finish();
                Poll::Ready(None)
            }
        }
    }

    fn scan_lines(&mut self, bytes: &[u8]) {
        for line in String::from_utf8_lossy(bytes).lines() {
            self.scanner.feed_line(line);
        }
    }

    /// Tee a passthrough chunk into the capture buffer (capped).
    fn capture_chunk(&mut self, chunk: &[u8]) {
        self.streamed_bytes += chunk.len() as u64;
        if self.sample.log.is_none() {
            return;
        }
        let Some(cap) = self.policy.capture_cap else {
            // No cap configured: the whole response is kept. This buffer is
            // the reason a cap exists at all — it holds a response in memory
            // until the stream ends — so an install that sets none is trading
            // disk growth for one in-flight response per stream.
            self.capture_buf.extend_from_slice(chunk);
            return;
        };
        let remaining = cap.saturating_sub(self.capture_buf.len());
        if remaining == 0 {
            self.capture_truncated = true;
            return;
        }
        if chunk.len() > remaining {
            self.capture_truncated = true;
        }
        self.capture_buf
            .extend_from_slice(&chunk[..chunk.len().min(remaining)]);
    }

    /// Submit the metered sample (best-effort) once the stream is exhausted.
    fn finish(&mut self) {
        self.sample.usage = self.scanner.usage();
        self.sample.usage_missing = !self.scanner.saw_usage();
        if self.sample.model.is_none() {
            self.sample.model = self.scanner.model().map(String::from);
        }
        self.sample.latency_ms = self.started.elapsed().as_millis() as i64;
        if let Some(log) = self.sample.log.as_mut() {
            log.first_token_ms = self
                .first_chunk
                .map(|t| (t - self.started).as_millis() as i64);
            // Through the same cap-and-scrub every other body gets. The buffer
            // is already capped at capture time, so the cap here is a no-op and
            // the credentials are not.
            let (body, _) = crate::log_capture::cap_body(
                &self.capture_buf,
                self.policy.capture_cap,
                &self.policy.redactor,
            );
            log.response_body = Some(body);
            log.response_size = self.streamed_bytes as i64;
            log.truncated = log.truncated || self.capture_truncated;
        }
        self.finished = true;
        if let Err(e) = self.tx.try_send(self.sample.clone()) {
            tracing::warn!(error = %e, "usage channel unavailable; stream usage not persisted");
        }
    }
}

impl Drop for SseUsageStream {
    /// A stream the client walked away from still happened.
    ///
    /// Dropping this future — the client hung up, or the response was
    /// discarded — used to take the sample with it: no usage row, no request
    /// row, nothing anywhere to say the request was ever made. The upstream had
    /// already been asked, and may already have billed for what it generated.
    ///
    /// So the record is closed out with what is known: the tokens the scanner
    /// read, and the part of the body that had arrived. That body is not the
    /// whole response, which is what `truncated` says — the same flag a capture
    /// cap sets, read by the same column.
    ///
    /// The status is left as it was: it describes what the *upstream* did, and
    /// the upstream was fine. A client hanging up is not an upstream error, and
    /// writing one would put a failure in the logs that the provider never had.
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // No scan of the leftover buffer here: it holds at most a partial line
        // (the last drain took everything up to the last newline), and a partial
        // line is not a usage event. What did arrive in full was scanned on the
        // way out, and that is what this row reports.
        self.capture_truncated = true;
        self.finish();
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
            // Silence is a failure mode, not a state to wait in: without this the
            // client would hold the open socket until its own read timeout.
            if let Some(deadline) = self.deadline.as_mut() {
                if deadline.as_mut().poll(cx).is_ready() {
                    self.timed_out = true;
                }
            }
            if self.timed_out {
                return self.abort_on_timeout();
            }
            match self.inner.as_mut().poll_next(cx) {
                Poll::Ready(Some(Ok(chunk))) => {
                    if self.first_chunk.is_none() {
                        self.first_chunk = Some(Instant::now());
                    }
                    self.arm();
                    self.capture_chunk(&chunk);
                    self.buffer.extend_from_slice(&chunk);
                    // Emit only up to the last complete line; keep the tail.
                    if let Some(split) =
                        self.buffer.iter().rposition(|&b| b == b'\n').map(|i| i + 1)
                    {
                        let complete: Vec<u8> = self.buffer.drain(..split).collect();
                        self.scan_lines(&complete);
                        if self.policy.strip_usage_chunk {
                            // Scanned first, dropped after: the numbers are
                            // ours to keep, the chunk is not the client's to
                            // receive. A block that was nothing but that chunk
                            // emits nothing at all, so this loops for more.
                            let filtered = drop_usage_only_events(&complete);
                            if filtered.is_empty() {
                                continue;
                            }
                            return Poll::Ready(Some(Ok(Bytes::from(filtered))));
                        }
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
                    let rest = if self.policy.strip_usage_chunk {
                        drop_usage_only_events(&rest)
                    } else {
                        rest
                    };
                    if rest.is_empty() {
                        self.finish();
                        return Poll::Ready(None);
                    }
                    return Poll::Ready(Some(Ok(Bytes::from(rest))));
                    // finish() runs on the next poll (inner_ended branch).
                }
                Poll::Pending => return Poll::Pending,
            }
        }
    }
}

/// Whether an SSE line is the usage-only chunk: an empty `choices` array beside
/// a `usage` object. That is the shape `include_usage` produces, and the shape
/// a client that did not ask for the option has no reason to expect.
fn is_usage_only_event(line: &[u8]) -> bool {
    let Ok(text) = std::str::from_utf8(line) else {
        return false;
    };
    let Some(payload) = text.trim_end().strip_prefix("data:") else {
        return false;
    };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload.trim()) else {
        return false;
    };
    v.get("choices")
        .and_then(serde_json::Value::as_array)
        .is_some_and(Vec::is_empty)
        && v.get("usage").is_some_and(|u| u.is_object())
}

/// The same bytes with any usage-only event removed.
///
/// Line-aligned by construction: the caller drains complete lines. The blank
/// line after a dropped event is left where it is, because a blank line with no
/// event before it is not an event — an SSE reader dispatches nothing and
/// carries on.
fn drop_usage_only_events(block: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(block.len());
    // The blank line after a dropped event goes with it: an event is its data
    // line *and* its terminator, and leaving the terminator behind would make
    // this function report a block it had emptied as still worth sending.
    let mut dropping_terminator = false;
    for line in block.split_inclusive(|&b| b == b'\n') {
        if is_usage_only_event(line) {
            dropping_terminator = true;
            continue;
        }
        if dropping_terminator {
            dropping_terminator = false;
            if line.iter().all(|&b| b == b'\n' || b == b'\r') {
                continue;
            }
        }
        out.extend_from_slice(line);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forward::test_support::TEST_AT;
    use crate::meter::Usage;
    use std::vec;

    /// What the stripper recognizes, and what it must leave alone: a content
    /// chunk that happens to carry a `usage` key is not this chunk, and a
    /// `[DONE]` sentinel is not JSON at all.
    #[test]
    fn only_the_usage_only_chunk_is_recognized() {
        let usage_only = br#"data: {"id":"c1","choices":[],"usage":{"prompt_tokens":11}}"#;
        assert!(is_usage_only_event(usage_only));

        let content = br#"data: {"id":"c1","choices":[{"delta":{"content":"hi"}}],"usage":{"prompt_tokens":11}}"#;
        assert!(
            !is_usage_only_event(content),
            "a chunk with content is not it"
        );

        assert!(!is_usage_only_event(b"data: [DONE]"));
        assert!(!is_usage_only_event(b"event: message_stop"));
        assert!(!is_usage_only_event(b"data: not json"));
        assert!(!is_usage_only_event(b""));
    }

    /// The whole rewrite: the usage event goes, everything else arrives in the
    /// order it was sent, and an empty block stays empty so the caller can skip
    /// emitting it.
    #[test]
    fn dropping_the_usage_event_keeps_every_other_byte() {
        let block = concat!(
            "data: {\"choices\":[{\"delta\":{\"content\":\"hi\"}}]}\n",
            "\n",
            "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11}}\n",
            "\n",
            "data: [DONE]\n",
            "\n",
        );
        let out = String::from_utf8(drop_usage_only_events(block.as_bytes())).unwrap();
        assert!(!out.contains("prompt_tokens"), "{out}");
        assert!(out.contains("\"content\":\"hi\""), "{out}");
        assert!(out.contains("[DONE]"), "{out}");

        // A block that was only the usage event has nothing left to send.
        let only = "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":11}}\n\n";
        assert!(drop_usage_only_events(only.as_bytes()).is_empty());
    }

    /// A source that yields its chunks and then goes silent forever — the
    /// stalled stream an idle limit exists for. `Pending` with no wake is
    /// exactly what a stalled socket looks like to the poller.
    struct StallingStream {
        chunks: vec::IntoIter<Result<Bytes, BoxError>>,
    }

    impl Stream for StallingStream {
        type Item = Result<Bytes, BoxError>;

        fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
            match self.chunks.next() {
                Some(c) => Poll::Ready(Some(c)),
                None => Poll::Pending,
            }
        }
    }

    // A stream that stops producing is abandoned, and the client is told why in
    // its own dialect. There used to be no limit on that wait at all: the socket
    // stayed open until the client's own 300s read timeout, so a dead stream was
    // reported as the client timing out rather than as the failure it is.
    #[tokio::test]
    async fn a_silent_stream_is_abandoned_with_an_error_in_the_clients_dialect() {
        let inner: Pin<Box<dyn Stream<Item = Result<Bytes, BoxError>> + Send>> =
            Box::pin(StallingStream {
                chunks: vec![Ok(Bytes::from_static(
                    b"event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
                ))]
                .into_iter(),
            });
        let (tx, mut rx) = mpsc::channel(1);
        let mut stream = SseUsageStream::new(
            inner,
            tx,
            UsageSample {
                agent: "claude".into(),
                provider_id: "p1".into(),
                catalog_id: None,
                started_unix: TEST_AT,
                model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                cache_inclusive: false,
                usage_missing: false,
                log: None,
            },
            Instant::now(),
            StreamPolicy {
                capture_cap: Some(0),
                redactor: std::sync::Arc::new(crate::log_capture::Redactor::default()),
                strip_usage_chunk: false,
            },
            Some(Protocol::Anthropic),
            crate::store::StreamTimeouts {
                // The first byte arrived; only the gap is limited.
                first_byte_secs: 0,
                idle_secs: 1,
            },
        );

        let mut out = String::new();
        while let Some(item) = std::future::poll_fn(|cx| Pin::new(&mut stream).poll_next(cx)).await
        {
            out.push_str(&String::from_utf8_lossy(&item.expect("stream error")));
        }

        assert!(
            out.contains("content_block_delta"),
            "the bytes that did arrive still pass through: {out}"
        );
        assert!(out.contains("event: error"), "{out}");
        assert!(out.contains("\"type\":\"stream_timeout\""), "{out}");

        let sample = rx
            .try_recv()
            .expect("the abandoned stream still reports itself");
        assert_eq!(sample.status, "stream_timeout");
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
                catalog_id: None,
                started_unix: TEST_AT,
                model: None,
                usage: Usage::default(),
                latency_ms: 0,
                status: "ok",
                cache_inclusive: false,
                usage_missing: false,
                log: None,
            },
            Instant::now(),
            StreamPolicy {
                capture_cap: Some(0),
                redactor: std::sync::Arc::new(crate::log_capture::Redactor::default()),
                strip_usage_chunk: false,
            },
            Some(Protocol::Anthropic),
            crate::store::StreamTimeouts::default(),
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
