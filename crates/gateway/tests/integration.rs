//! Hermetic end-to-end tests: gateway data plane against in-process mock
//! upstreams (loopback, no external network), SQLite via tempfile.

use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};

use axum::body::{Body, Bytes};
use axum::extract::State as AxumState;
use axum::http::{header, HeaderMap, Request, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::post;
use axum::{Json, Router};
use futures_core::Stream;
use kiwanod::server::{
    admin_plane_router, data_plane_router, GatewayState, ADMIN_TOKEN_HEADER, ADMIN_TOKEN_KEY,
};
use kiwanod::store::{
    now_rfc3339, Billing, Binding, LogConfig, Protocol, Provider, RequestLogEntry,
    RequestLogFilter, Store, StrategyType,
};
use serde_json::{json, Value};
use tower::ServiceExt;

const REAL_KEY: &str = "sk-real-provider-key";

/// Captured auth headers from mock upstreams (placeholder key must never leak).
type Captured = Arc<Mutex<Vec<(String, String)>>>;

/// Captured request bodies from mock upstreams (used by conversion tests).
type CapturedBodies = Arc<Mutex<Vec<Value>>>;

fn capture(state: &Captured, headers: &HeaderMap, name: &str) {
    if let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) {
        state
            .lock()
            .unwrap()
            .push((name.to_string(), v.to_string()));
    }
}

/// Spawn a mock Anthropic upstream on 127.0.0.1:0.
async fn mock_anthropic(response: MockReply) -> (String, Captured) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1/messages",
            post(
                move |AxumState(c): AxumState<Captured>, headers: HeaderMap| {
                    let reply = response.clone();
                    async move {
                        capture(&c, &headers, "x-api-key");
                        mock_response(&reply)
                    }
                },
            ),
        )
        .with_state(captured.clone());
    (spawn(app).await, captured)
}

/// Spawn a mock OpenAI upstream on 127.0.0.1:0 (bodies not captured).
async fn mock_openai(response: MockReply) -> (String, Captured) {
    let (url, captured, _) = mock_openai_capturing_body(response).await;
    (url, captured)
}

/// Spawn a mock OpenAI upstream that also records the JSON request bodies it
/// receives (lets conversion tests assert the converted request shape).
async fn mock_openai_capturing_body(response: MockReply) -> (String, Captured, CapturedBodies) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let bodies: CapturedBodies = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1/chat/completions",
            post(
                move |AxumState((c, b)): AxumState<(Captured, CapturedBodies)>,
                      headers: HeaderMap,
                      body: axum::body::Bytes| {
                    let reply = response.clone();
                    async move {
                        capture(&c, &headers, "authorization");
                        if let Ok(v) = serde_json::from_slice::<Value>(&body) {
                            b.lock().unwrap().push(v);
                        }
                        mock_response(&reply)
                    }
                },
            ),
        )
        .with_state((captured.clone(), bodies.clone()));
    let url = spawn(app).await;
    (url, captured, bodies)
}

#[derive(Clone)]
enum MockReply {
    Json(Value),
    Sse(Vec<&'static str>),
    /// A bare failure status — what a failover is for.
    Status(StatusCode),
    /// Events, then silence forever: a stream that stalled mid-flight.
    SseThenStall(Vec<&'static str>),
}

fn mock_response(reply: &MockReply) -> Response {
    match reply {
        MockReply::Json(v) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(v.clone()),
        )
            .into_response(),
        MockReply::Status(status) => (
            *status,
            [(header::CONTENT_TYPE, "application/json")],
            Json(json!({"error": {"message": "the upstream refused it"}})),
        )
            .into_response(),
        MockReply::Sse(chunks) => {
            let chunks: Vec<Result<Bytes, std::convert::Infallible>> = chunks
                .iter()
                .map(|c| Ok(Bytes::from_static(c.as_bytes())))
                .collect();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ChunkStream {
                    chunks,
                    idx: 0,
                    stall_after: false,
                }))
                .unwrap()
        }
        MockReply::SseThenStall(chunks) => {
            let chunks: Vec<Result<Bytes, std::convert::Infallible>> = chunks
                .iter()
                .map(|c| Ok(Bytes::from_static(c.as_bytes())))
                .collect();
            Response::builder()
                .status(StatusCode::OK)
                .header(header::CONTENT_TYPE, "text/event-stream")
                .body(Body::from_stream(ChunkStream {
                    chunks,
                    idx: 0,
                    stall_after: true,
                }))
                .unwrap()
        }
    }
}

/// Emit pre-baked chunks (awkwardly split, to exercise line buffering).
struct ChunkStream {
    chunks: Vec<Result<Bytes, std::convert::Infallible>>,
    idx: usize,
    /// Keep the connection open after the chunks instead of ending the stream —
    /// an upstream that returned its headers and then went quiet.
    stall_after: bool,
}

impl Stream for ChunkStream {
    type Item = Result<Bytes, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.idx < self.chunks.len() {
            let item = self.chunks[self.idx].clone();
            self.idx += 1;
            Poll::Ready(Some(item))
        } else if self.stall_after {
            Poll::Pending
        } else {
            Poll::Ready(None)
        }
    }
}

async fn spawn(app: Router) -> String {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move { axum::serve(listener, app).await });
    format!("http://{addr}")
}

fn provider(id: &str, protocol: Protocol, base_url: String) -> Provider {
    Provider {
        id: id.to_string(),
        name: format!("prov-{id}"),
        catalog_id: None,
        protocol,
        base_url,
        api_path: None,
        endpoints: Vec::new(),
        api_key: Some(REAL_KEY.to_string()),
        model_default: None,
        billing: Billing::Metered,
        period_limit: None,
        limit_unit: None,
        plan_query: None,
        plan_limits: None,
        timeout_secs: None,
        retries: None,
        headers: None,
        reset_period: None,
        enabled: true,
        created_at: now_rfc3339(),
        updated_at: now_rfc3339(),
    }
}

fn bind(agent: &str, provider_id: &str, priority: i64) -> Binding {
    Binding {
        agent: agent.to_string(),
        provider_id: provider_id.to_string(),
        priority,
        weight: 1,
        win_start: None,
        win_end: None,
        enabled: true,
    }
}

async fn post_json(app: &Router, path: &str, key: Option<&str>, body: &str) -> Response {
    let mut builder = Request::builder().method("POST").uri(path);
    if let Some(k) = key {
        builder = builder.header("x-api-key", k);
    }
    app.clone()
        .oneshot(
            builder
                .header("content-type", "application/json")
                .body(Body::from(body.to_string()))
                .unwrap(),
        )
        .await
        .unwrap()
}

async fn response_body(response: Response) -> Bytes {
    axum::body::to_bytes(response.into_body(), 8 * 1024 * 1024)
        .await
        .unwrap()
}

/// Wait for the async stream-usage recorder to land the row.
async fn wait_for_usage(
    state: &GatewayState,
    agent: &str,
    expected: i64,
) -> kiwanod::store::UsageTotals {
    for _ in 0..60 {
        let totals = state.store.usage_totals(Some(agent), None, None).unwrap();
        if totals.requests >= expected {
            return totals;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("usage row for agent `{agent}` not persisted in time");
}

/// Wait for the async stream recorder to land the request_logs row(s).
async fn wait_for_log(state: &GatewayState, expected: i64) -> Vec<RequestLogEntry> {
    for _ in 0..60 {
        let (rows, total) = state
            .store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        if total >= expected {
            return rows;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("request log row not persisted in time");
}

#[tokio::test]
async fn anthropic_request_forwards_captures_usage_and_hides_placeholder_key() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let upstream_body = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "hello"}],
        "usage": {
            "input_tokens": 2095,
            "output_tokens": 503,
            "cache_creation_input_tokens": 2095,
            "cache_read_input_tokens": 0
        }
    });
    let (upstream_url, captured) = mock_anthropic(MockReply::Json(upstream_body.clone())).await;

    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"claude-sonnet-4-5","stream":false,"messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["id"], "msg_1");
    assert_eq!(body["content"][0]["text"], "hello");

    // The real key went upstream; the placeholder key never left the gateway.
    let captured = captured.lock().unwrap();
    assert_eq!(captured.len(), 1);
    assert_eq!(captured[0], ("x-api-key".to_string(), REAL_KEY.to_string()));
    drop(captured);

    // Usage metered into SQLite, attributed to agent + provider.
    let totals = state
        .store
        .usage_totals(Some("claude"), None, None)
        .unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(totals.input_tokens, 2095);
    assert_eq!(totals.output_tokens, 503);
    assert_eq!(totals.cache_creation_tokens, 2095);

    let by_provider = state
        .store
        .usage_by_provider(Some("claude"), None, None)
        .unwrap();
    assert_eq!(by_provider.len(), 1);
    assert_eq!(by_provider[0].provider_id, "p-ant");
}

#[tokio::test]
async fn sse_stream_passthrough_is_byte_exact_and_metered() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    // Events deliberately split across chunks at non-line boundaries.
    let chunks = vec![
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1,\"cache_read_input_tokens\":11,\"cache_creation_input_tokens\":3}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"he",
        "llo\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":171}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    ];
    let (upstream_url, _captured) = mock_anthropic(MockReply::Sse(chunks.clone())).await;

    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"claude-sonnet-4-5","stream":true,"messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/event-stream"));

    let body = response_body(response).await;
    let expected: String = chunks.concat();
    assert_eq!(String::from_utf8(body.to_vec()).unwrap(), expected);

    let totals = wait_for_usage(&state, "claude", 1).await;
    assert_eq!(totals.input_tokens, 25);
    assert_eq!(totals.output_tokens, 171); // cumulative from message_delta
    assert_eq!(totals.cache_read_tokens, 11);
    assert_eq!(totals.cache_creation_tokens, 3);
}

/// The OpenAI-shaped path with no key at all: this is the request that used to
/// be attributed to Codex by path protocol and forwarded upstream on the
/// operator's real key. It is refused now, and the upstream is never called.
#[tokio::test]
async fn openai_path_without_key_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (upstream_url, captured) = mock_openai(MockReply::Json(json!({"id": "x"}))).await;
    store
        .insert_provider(&provider("p-oai", Protocol::OpenAI, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("codex", "p-oai", 0)).unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/chat/completions",
        None,
        r#"{"model":"gpt-4o","messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // The claim that matters: the provider was never called, so nothing was
    // spent on the operator's account.
    assert!(captured.lock().unwrap().is_empty());
    let totals = state.store.usage_totals(None, None, None).unwrap();
    assert_eq!(totals.requests, 0);
}

#[tokio::test]
async fn admin_reload_switches_provider_without_restart() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (url_a, captured_a) = mock_anthropic(MockReply::Json(json!({"id": "from-a"}))).await;
    let (url_b, captured_b) = mock_anthropic(MockReply::Json(json!({"id": "from-b"}))).await;

    store
        .insert_provider(&provider("p-a", Protocol::Anthropic, url_a))
        .unwrap();
    store
        .insert_provider(&provider("p-b", Protocol::Anthropic, url_b))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-a", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let data = data_plane_router(state.clone());
    let admin = admin_plane_router(state.clone());

    // First request hits provider A.
    let response = post_json(
        &data,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["id"], "from-a");

    // App demotes A, promotes B, then hot-reloads the route table.
    state
        .store
        .upsert_binding(&bind("claude", "p-a", 1))
        .unwrap();
    state
        .store
        .upsert_binding(&bind("claude", "p-b", 0))
        .unwrap();
    let token = state
        .store
        .app_setting(ADMIN_TOKEN_KEY)
        .expect("the gateway mints its admin token at startup");
    let response = admin
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
                .header(ADMIN_TOKEN_HEADER, &token)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let v: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(v["ok"], true);
    assert_eq!(v["agents_routed"], 1);

    // Next request reaches provider B; no restart happened.
    let response = post_json(
        &data,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["id"], "from-b");

    assert_eq!(captured_a.lock().unwrap().len(), 1);
    assert_eq!(captured_b.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn openai_inbound_to_anthropic_provider_still_fails_cleanly() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    // codex (OpenAI inbound) bound to an Anthropic provider: the only
    // conversion path implemented is Anthropic -> OpenAI, so this must not
    // forward.
    let (upstream_url, captured) = mock_anthropic(MockReply::Json(json!({"ok": true}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("codex", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-codex-test", "codex")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/chat/completions",
        Some("kw-ag-codex-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["error"]["type"], "protocol_mismatch");
    assert!(
        captured.lock().unwrap().is_empty(),
        "nothing may reach the upstream"
    );
}

/// Anthropic `/v1/messages` on an OpenAI-compatible provider: non-streaming
/// request converted by adapters, response converted back, usage metered.
#[tokio::test]
async fn anthropic_inbound_converts_non_streaming_to_openai_upstream() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let upstream_body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi from openai"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 42, "completion_tokens": 7,
                  "prompt_tokens_details": {"cached_tokens": 32}, "total_tokens": 49}
    });
    let (upstream_url, captured, bodies) =
        mock_openai_capturing_body(MockReply::Json(upstream_body)).await;

    store
        .insert_provider(&provider("p-oai", Protocol::OpenAI, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-oai", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"claude-sonnet-4-5","max_tokens":128,"stream":false,
            "messages":[{"role":"user","content":"hello"}]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // Response is a valid Anthropic message converted from the OpenAI body.
    // adapters conserves cache tokens: input_tokens excludes the cached
    // 32 (input + cache_read == prompt_tokens).
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["type"], "message");
    assert_eq!(body["id"], "chatcmpl-1");
    assert_eq!(body["content"][0]["text"], "hi from openai");
    assert_eq!(body["usage"]["input_tokens"], 10);
    assert_eq!(body["usage"]["cache_read_input_tokens"], 32);
    assert_eq!(body["usage"]["output_tokens"], 7);

    // The upstream saw a converted OpenAI chat request with Bearer auth.
    assert_eq!(
        captured
            .lock()
            .unwrap()
            .last()
            .map(|(k, v)| (k.as_str(), v.as_str())),
        Some(("authorization", format!("Bearer {REAL_KEY}").as_str()))
    );
    let sent = bodies.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["model"], "claude-sonnet-4-5");
    assert!(sent[0]["messages"].is_array());
    // Anthropic-only fields must not leak through the conversion.
    assert!(sent[0].get("max_tokens").is_none() || sent[0]["max_tokens"].is_number());
    assert!(sent[0].get("system").is_none());

    // Usage metered from the upstream OpenAI usage block.
    let totals = state
        .store
        .usage_totals(Some("claude"), None, None)
        .unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(totals.input_tokens, 42);
    assert_eq!(totals.output_tokens, 7);
    assert_eq!(totals.cache_read_tokens, 32);
}

/// Anthropic `/v1/messages` (stream) on an OpenAI-compatible provider: the
/// OpenAI chunk stream is converted into an Anthropic event stream and usage
/// is captured from the converted events.
#[tokio::test]
async fn anthropic_inbound_converts_openai_sse_stream_to_anthropic_events() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    // OpenAI chat chunks with usage on the final (choices-less) chunk.
    let chunks = vec![
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hel\"}}]}\n\n",
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"lo\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"id\":\"c1\",\"object\":\"chat.completion.chunk\",\"model\":\"gpt-4o\",\"choices\":[],\"usage\":{\"prompt_tokens\":11,\"completion_tokens\":4,\"prompt_tokens_details\":{\"cached_tokens\":5}}}\n\n",
        "data: [DONE]\n\n",
    ];
    let (upstream_url, _captured, bodies) =
        mock_openai_capturing_body(MockReply::Sse(chunks)).await;

    store
        .insert_provider(&provider("p-oai", Protocol::OpenAI, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-oai", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"claude-sonnet-4-5","stream":true,
            "messages":[{"role":"user","content":"hello"}]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    assert!(response
        .headers()
        .get(header::CONTENT_TYPE)
        .unwrap()
        .to_str()
        .unwrap()
        .contains("text/event-stream"));

    let body = response_body(response).await;
    let text = String::from_utf8(body.to_vec()).unwrap();

    // The client sees a genuine Anthropic event stream.
    assert!(text.contains("event: message_start"), "got:\n{text}");
    assert!(text.contains("event: content_block_start"));
    assert!(text.contains("\"text_delta\""), "got:\n{text}");
    assert!(text.contains("event: message_delta"));
    assert!(text.contains("\"stop_reason\":\"end_turn\""));
    assert!(text.contains("event: message_stop"));

    // message_start opens the stream; OpenAI upstreams only attach usage to
    // the final chunk, so the full metered usage lands on message_delta
    // (cache-conserved: input = prompt 11 - cached 5).
    let events: Vec<Value> = text
        .lines()
        .filter_map(|l| l.strip_prefix("data: "))
        .map(|payload| serde_json::from_str::<Value>(payload).unwrap())
        .collect();
    assert!(events
        .iter()
        .any(|e| e["type"] == "message_start" && e["message"]["usage"]["input_tokens"] == 0));
    let delta = events
        .iter()
        .find(|e| e["type"] == "message_delta")
        .expect("converted stream must end with message_delta");
    assert_eq!(delta["usage"]["input_tokens"], 6);
    assert_eq!(delta["usage"]["cache_read_input_tokens"], 5);
    assert_eq!(delta["usage"]["output_tokens"], 4);

    // The upstream saw a converted streaming OpenAI request that asks for
    // usage in the stream (adapters injects stream_options.include_usage).
    // Scoped: the guard must be released before the await below.
    {
        let sent = bodies.lock().unwrap();
        assert_eq!(sent.len(), 1);
        assert_eq!(sent[0]["stream"], true);
        assert_eq!(
            sent[0]["stream_options"]["include_usage"], true,
            "usage capture depends on include_usage injection"
        );
    }

    // Usage metered from the converted Anthropic usage events
    // (cache-conserved values, matching what the client observed).
    let totals = wait_for_usage(&state, "claude", 1).await;
    assert_eq!(totals.input_tokens, 6);
    assert_eq!(totals.output_tokens, 4);
    assert_eq!(totals.cache_read_tokens, 5);
}

/// An agent the user defined routes like any other, and meters under its own
/// name — the whole premise being that the gateway only ever needed a key and a
/// route, never a registry entry.
///
/// This crate cannot call `kiwano_core::vm::add_custom_agent` (core depends on
/// this one), so the three rows it writes are written here directly: the agent's
/// own row, its placeholder key, and its binding. If that ever stops being the
/// whole of it, this test is where it shows up.
#[tokio::test]
async fn a_user_defined_agents_key_routes_and_meters() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    let agent = "long-tasks-3f9a";

    let (upstream_url, captured) = mock_anthropic(MockReply::Json(json!({
        "id": "msg_1",
        "type": "message",
        "model": "claude-sonnet-4-5",
        "usage": {"input_tokens": 11, "output_tokens": 3}
    })))
    .await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind(agent, "p-ant", 0)).unwrap();
    store
        .insert_custom_agent(&kiwanod::store::CustomAgent {
            id: agent.to_string(),
            label: "Long tasks".to_string(),
            note: None,
            created_at: kiwanod::store::now_rfc3339(),
        })
        .unwrap();
    store
        .upsert_placeholder_key("kw-ag-long-tasks-3f9a-0001", agent)
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-long-tasks-3f9a-0001"),
        r#"{"model":"claude-sonnet-4-5","messages":[{"role":"user","content":"hi"}]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let _ = response_body(response).await;

    // The real key went upstream, the placeholder never did — same contract as
    // any built-in agent's traffic. Scoped so the guard is released before the
    // await below (a std guard across an await is a clippy error, not a style
    // preference: the task can be suspended holding it).
    {
        let seen = captured.lock().unwrap();
        assert_eq!(
            seen.last().map(|(k, v)| (k.as_str(), v.as_str())),
            Some(("x-api-key", REAL_KEY))
        );
    }

    // …and it is accounted under the agent the key names, which is the point of
    // having defined it.
    let totals = wait_for_usage(&state, agent, 1).await;
    assert_eq!(totals.input_tokens, 11);
    assert_eq!(totals.output_tokens, 3);
}

/// The Gemini family is gone with its protocol and its agent: nothing speaks
/// `/v1beta` any more, so a stray request is refused as an unknown path rather
/// than routed to a provider in the wrong shape.
#[tokio::test]
async fn gemini_paths_are_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    let (upstream_url, captured) = mock_anthropic(MockReply::Json(json!({"id": "x"}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state);

    let response = app
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-pro:generateContent")
                .header("content-type", "application/json")
                .header("x-goog-api-key", "kw-ag-gemini-old")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hi"}]}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    assert!(
        captured.lock().unwrap().is_empty(),
        "nothing may reach an upstream"
    );
}

/// A foreign key on the Anthropic path is refused for the same reason as the
/// OpenAI one: the fallback attributed it to claude and forwarded it.
#[tokio::test]
async fn unknown_key_on_anthropic_path_is_refused() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (upstream_url, captured) = mock_anthropic(MockReply::Json(json!({"id": "x"}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("sk-some-foreign-key"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    assert!(captured.lock().unwrap().is_empty());
    let totals = state.store.usage_totals(None, None, None).unwrap();
    assert_eq!(totals.requests, 0);
}

/// A completed non-streaming request lands a full request_logs row: metadata,
/// redacted headers, usage tokens, and both bodies via the detail view.
#[tokio::test]
async fn request_log_captures_bodies_and_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let upstream_body = json!({
        "id": "msg_1",
        "type": "message",
        "role": "assistant",
        "model": "claude-sonnet-4-5",
        "content": [{"type": "text", "text": "hello"}],
        "usage": {"input_tokens": 2095, "output_tokens": 503,
                  "cache_creation_input_tokens": 2095, "cache_read_input_tokens": 0}
    });
    let (upstream_url, _captured) = mock_anthropic(MockReply::Json(upstream_body.clone())).await;

    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let request_body = r#"{"model":"claude-sonnet-4-5","stream":false,"messages":[]}"#;
    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        request_body,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let response_bytes = response_body(response).await;

    let (rows, total) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    assert_eq!(total, 1);
    let row = &rows[0];
    assert_eq!(row.method, "POST");
    assert_eq!(row.path, "/v1/messages");
    assert_eq!(row.agent.as_deref(), Some("claude"));
    assert_eq!(row.attribution.as_deref(), Some("key"));
    assert_eq!(row.provider_id.as_deref(), Some("p-ant"));
    assert_eq!(row.model.as_deref(), Some("claude-sonnet-4-5"));
    assert_eq!(row.status_code, 200);
    assert_eq!(row.error_kind, None);
    assert!(!row.is_streaming);
    assert_eq!(row.input_tokens, 2095);
    assert_eq!(row.output_tokens, 503);
    assert_eq!(row.cache_creation_tokens, 2095);
    assert!(row.latency_ms.is_some());

    // Headers are redacted: neither the placeholder nor the real key appears.
    let req_headers = row.request_headers.as_deref().unwrap();
    assert!(!req_headers.contains("kw-ag-claude-test"));
    assert!(!req_headers.contains(REAL_KEY));
    assert!(row
        .response_headers
        .as_deref()
        .unwrap()
        .contains("application/json"));

    // Bodies round-trip through the detail view (client saw the upstream body).
    let client_value: Value = serde_json::from_slice(&response_bytes).unwrap();
    assert_eq!(client_value, upstream_body);
    let detail = state.store.get_request_log(row.id).unwrap().unwrap();
    let stored: Value = serde_json::from_str(detail.response_body.as_deref().unwrap()).unwrap();
    assert_eq!(stored, upstream_body);
    assert_eq!(detail.request_body.as_deref(), Some(request_body));
    assert_eq!(detail.entry.request_size, request_body.len() as i64);
    assert_eq!(detail.entry.response_size, response_bytes.len() as i64);
    assert!(!row.truncated);
}

/// Requests that fail before/at forwarding still land request_logs rows with
/// error metadata and captured request bodies; the usage table is untouched.
#[tokio::test]
async fn request_log_records_failures_without_usage() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    // codex (OpenAI inbound) bound to an Anthropic provider: protocol mismatch.
    let (upstream_url, _captured) = mock_anthropic(MockReply::Json(json!({"ok": true}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("codex", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-codex-test", "codex")
        .unwrap();
    // Registered, but no binding: identified, with nothing to route to.
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/chat/completions",
        Some("kw-ag-codex-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);

    // A key the gateway did not mint: refused, without touching any provider
    // and without guessing an agent from the path.
    let response = post_json(&app, "/v1/messages", Some("sk-foreign"), r#"{"model":"m"}"#).await;
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    // A key it did mint, for an agent with no binding: 503, also untouched.
    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

    // Paths with no route at all hit the axum fallback and are audited too.
    let response = post_json(&app, "/v1/nope", None, r#"{}"#).await;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);

    let (rows, total) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    assert_eq!(total, 4);
    let unknown = rows.iter().find(|r| r.path == "/v1/nope").unwrap();
    assert_eq!(unknown.status_code, 404);
    assert_eq!(unknown.error_kind.as_deref(), Some("unsupported_path"));
    assert_eq!(unknown.agent, None);

    let mismatch = rows.iter().find(|r| r.status_code == 502).unwrap();
    assert_eq!(mismatch.error_kind.as_deref(), Some("protocol_mismatch"));
    assert_eq!(mismatch.agent.as_deref(), Some("codex"));
    assert_eq!(mismatch.provider_id.as_deref(), Some("p-ant"));

    let refused = rows.iter().find(|r| r.status_code == 401).unwrap();
    assert_eq!(refused.error_kind.as_deref(), Some("unauthorized"));
    assert_eq!(refused.agent, None);

    let unbound = rows.iter().find(|r| r.status_code == 503).unwrap();
    assert_eq!(unbound.error_kind.as_deref(), Some("no_provider_bound"));
    assert_eq!(unbound.agent.as_deref(), Some("claude"));

    // Usage metering semantics unchanged: failed requests are not usage.
    let totals = state.store.usage_totals(None, None, None).unwrap();
    assert_eq!(totals.requests, 0);

    // Request bodies are captured even for failed requests.
    let detail = state.store.get_request_log(mismatch.id).unwrap().unwrap();
    assert!(detail.request_body.is_some());
}

/// With log capture disabled, no request_logs rows are written while usage
/// metering continues unchanged.
#[tokio::test]
async fn request_log_disabled_records_nothing() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    store
        .save_log_config(&LogConfig {
            enabled: false,
            retain_days: 30,
            max_body_bytes: 4 * 1024 * 1024,
        })
        .unwrap();

    let (upstream_url, _captured) = mock_anthropic(MockReply::Json(json!({"id": "x"}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m","stream":false,"messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    let (_, total) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    assert_eq!(total, 0);

    let totals = state
        .store
        .usage_totals(Some("claude"), None, None)
        .unwrap();
    assert_eq!(totals.requests, 1);
}

/// The SSE capture records the client-visible stream: byte-exact body,
/// streaming flag, first-token latency, and the metered usage.
#[tokio::test]
async fn request_log_captures_client_visible_sse_stream() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let chunks = vec![
        "event: message_start\n",
        "data: {\"type\":\"message_start\",\"message\":{\"model\":\"claude-sonnet-4-5\",\"usage\":{\"input_tokens\":25,\"output_tokens\":1,\"cache_read_input_tokens\":11,\"cache_creation_input_tokens\":3}}}\n\n",
        "data: {\"type\":\"content_block_delta\",\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":171}}\n\n",
        "event: message_stop\n",
        "data: {\"type\":\"message_stop\"}\n\n",
    ];
    let (upstream_url, _captured) = mock_anthropic(MockReply::Sse(chunks.clone())).await;

    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"claude-sonnet-4-5","stream":true,"messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = response_body(response).await;
    let expected: String = chunks.concat();
    assert_eq!(String::from_utf8(body.to_vec()).unwrap(), expected);

    let rows = wait_for_log(&state, 1).await;
    let row = &rows[0];
    assert!(row.is_streaming);
    assert_eq!(row.status_code, 200);
    assert!(row.first_token_ms.is_some());
    assert_eq!(row.input_tokens, 25);
    assert_eq!(row.output_tokens, 171);
    assert!(row
        .response_headers
        .as_deref()
        .unwrap()
        .contains("text/event-stream"));

    // The stored body is exactly what the client observed.
    let detail = state.store.get_request_log(row.id).unwrap().unwrap();
    assert_eq!(detail.response_body.as_deref(), Some(expected.as_str()));
}

/// A credential in the query string is a shape real providers use (`?key=`),
/// and the failure path is the one that quotes the URL back: the
/// metadata column stores the redacted query, and `error_message` must not put
/// the raw one back. The upstream here is a closed port, so the row is written
/// by `persist_failure` through `SendFailure::message`.
#[tokio::test]
async fn query_credentials_are_redacted_in_the_log_and_the_error_message() {
    const SECRET: &str = "AIzaSyLiveKeyDoNotStore";

    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    // Port 1 on loopback: nothing listens, so the send fails immediately.
    store
        .insert_provider(&provider(
            "p-ant",
            Protocol::Anthropic,
            "http://127.0.0.1:1".into(),
        ))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        &format!("/v1/messages?key={SECRET}&model=x"),
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::BAD_GATEWAY);
    let client_saw = String::from_utf8_lossy(&response_body(response).await).to_string();
    assert!(
        !client_saw.contains(SECRET),
        "leaked to the client: {client_saw}"
    );

    let (rows, _) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    let row = rows.first().expect("the failure is audited");
    assert_eq!(
        row.query.as_deref(),
        Some("key=[REDACTED]&model=x"),
        "the stored query keeps its names and loses its value"
    );
    let message = row.error_message.as_deref().expect("a failure message");
    assert!(
        !message.contains(SECRET),
        "leaked into error_message: {message}"
    );
    assert!(message.contains("key=[REDACTED]"), "{message}");
}

/// A stream that goes silent is abandoned at the idle limit, and the client is
/// told why in the dialect it is speaking — instead of holding the socket until
/// its own read timeout fires. The row records the reason too, so a stream that
/// died mid-flight does not look like a short but healthy response.
#[tokio::test]
async fn a_stalled_stream_is_abandoned_and_recorded() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();
    // One second of silence is a stall. The gateway reads this at startup and on
    // /reload, which is what makes it a setting rather than a constant.
    store
        .save_stream_timeouts(&kiwanod::store::StreamTimeouts {
            first_byte_secs: 0,
            idle_secs: 1,
        })
        .unwrap();

    let (upstream_url, _) = mock_anthropic(MockReply::SseThenStall(vec![
        "event: content_block_delta\ndata: {\"type\":\"content_block_delta\"}\n\n",
    ]))
    .await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m","stream":true}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);

    // The wait is what the limit bounds, so the client sees the stream end —
    // with the event that explains it, not with a silent truncation.
    let body = String::from_utf8_lossy(&response_body(response).await).to_string();
    assert!(body.contains("content_block_delta"), "{body}");
    assert!(body.contains("event: error"), "the client is told: {body}");
    assert!(body.contains("stream_timeout"), "{body}");

    // The row is written by the async stream recorder, so it lands just after
    // the client's view of the stream ends.
    let rows = wait_for_log(&state, 1).await;
    let row = rows.first().expect("the stream is logged");
    assert_eq!(row.error_kind.as_deref(), Some("stream_timeout"));
    assert!(row.is_streaming);
}

/// A failing primary is replayed against the next candidate: the client asks
/// once and is owed one answer, not the failure the first provider had in store.
#[tokio::test]
async fn a_failing_primary_is_replayed_against_the_next_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (primary_url, primary_hits) =
        mock_anthropic(MockReply::Status(StatusCode::TOO_MANY_REQUESTS)).await;
    let (backup_url, backup_hits) = mock_anthropic(MockReply::Json(json!({
        "served_by": "backup",
        "usage": {"input_tokens": 7, "output_tokens": 3}
    })))
    .await;
    store
        .insert_provider(&provider("p-a", Protocol::Anthropic, primary_url))
        .unwrap();
    store
        .insert_provider(&provider("p-b", Protocol::Anthropic, backup_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-a", 0)).unwrap();
    store.upsert_binding(&bind("claude", "p-b", 1)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();
    // Failover is what makes the backup a fallback at all; the `single` case
    // below is the other half of that rule.
    store
        .upsert_strategy("claude", StrategyType::Failover, None)
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body = String::from_utf8_lossy(&response_body(response).await).to_string();
    assert!(
        body.contains("backup"),
        "the client sees the backup's answer: {body}"
    );

    assert!(
        !primary_hits.lock().unwrap().is_empty(),
        "the primary was asked first"
    );
    assert!(
        !backup_hits.lock().unwrap().is_empty(),
        "and the backup is who answered"
    );

    // Two rows for one client request — the attempt that failed and the one that
    // answered. That pair is how a flaking primary becomes visible at all.
    let (rows, total) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    assert_eq!(total, 2, "both attempts are in the log");
    let failed = rows
        .iter()
        .find(|r| r.provider_id.as_deref() == Some("p-a"))
        .expect("the failed attempt is logged");
    assert_eq!(failed.status_code, 429);
    let served = rows
        .iter()
        .find(|r| r.provider_id.as_deref() == Some("p-b"))
        .expect("the serving attempt is logged");
    assert_eq!(served.status_code, 200);

    // Tokens come only from the attempt that answered. `usage.requests` counts
    // upstream responses, and that is unchanged by failover — a 429 from a lone
    // provider already produced a row before any of this existed — so the pair
    // reads as two attempts, one of them empty and one of them the real work.
    let refused = state.store.usage_totals(None, Some("p-a"), None).unwrap();
    assert_eq!(refused.requests, 1, "the attempt is visible");
    assert_eq!(refused.input_tokens, 0, "but it generated nothing");
    assert_eq!(refused.output_tokens, 0);
    let served_totals = state.store.usage_totals(None, Some("p-b"), None).unwrap();
    assert_eq!(served_totals.requests, 1);
    assert_eq!(
        served_totals.input_tokens, 7,
        "the answer is what is billed"
    );
    assert_eq!(served_totals.output_tokens, 3);
}

/// `single` means exactly what it says: the backup is bound but is not a
/// fallback, so it is never asked — not by selection, not by replay. Choosing a
/// strategy that offers a fallback is what buys one.
#[tokio::test]
async fn a_single_strategy_never_replays_against_the_backup() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (primary_url, _) = mock_anthropic(MockReply::Status(StatusCode::TOO_MANY_REQUESTS)).await;
    let (backup_url, backup_hits) =
        mock_anthropic(MockReply::Json(json!({"served_by": "backup"}))).await;
    store
        .insert_provider(&provider("p-a", Protocol::Anthropic, primary_url))
        .unwrap();
    store
        .insert_provider(&provider("p-b", Protocol::Anthropic, backup_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-a", 0)).unwrap();
    store.upsert_binding(&bind("claude", "p-b", 1)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();
    store
        .upsert_strategy("claude", StrategyType::Single, None)
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(
        response.status(),
        StatusCode::TOO_MANY_REQUESTS,
        "the client sees the provider it was bound to"
    );
    assert!(
        backup_hits.lock().unwrap().is_empty(),
        "the bound-but-not-a-fallback provider is never asked"
    );
}

/// A provider whose breaker is open is refused before anything is sent: 503 with
/// an error kind of its own, not a 502 that would blame an upstream nobody asked.
///
/// This is the half of the breaker that had no effect. Selection treats an open
/// breaker as "unavailable", but only failover/roundrobin/quota consult it — the
/// default `single` strategy deliberately looks past it. So the send path is
/// where an open breaker has to bite, or it is decoration.
#[tokio::test]
async fn an_open_breaker_refuses_before_anything_is_sent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let (upstream_url, hits) = mock_anthropic(MockReply::Json(json!({"ok": true}))).await;
    store
        .insert_provider(&provider("p-ant", Protocol::Anthropic, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("claude", "p-ant", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-claude-test", "claude")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    // Open it the way real failures do; the default threshold is four.
    for _ in 0..4 {
        state.engine.record("claude", "p-ant", false, false).await;
    }

    let response = post_json(
        &app,
        "/v1/messages",
        Some("kw-ag-claude-test"),
        r#"{"model":"m"}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    assert!(
        hits.lock().unwrap().is_empty(),
        "the request never reached the upstream"
    );

    // Audited under its own kind, so "the breaker refused it" stays
    // distinguishable from "the upstream failed it".
    let (rows, _) = state
        .store
        .list_request_logs(1, 10, RequestLogFilter::default())
        .unwrap();
    let row = rows.first().expect("the refusal is audited");
    assert_eq!(row.error_kind.as_deref(), Some("circuit_open"));
    assert_eq!(row.agent.as_deref(), Some("claude"));
    assert_eq!(row.provider_id.as_deref(), Some("p-ant"));

    // Nothing ran, so nothing is billed.
    assert_eq!(
        state.store.usage_totals(None, None, None).unwrap().requests,
        0
    );
}
