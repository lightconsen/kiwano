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
use kiwano_gateway::server::{admin_plane_router, data_plane_router, GatewayState};
use kiwano_gateway::store::{now_rfc3339, Billing, Binding, Protocol, Provider, Store};
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
}

fn mock_response(reply: &MockReply) -> Response {
    match reply {
        MockReply::Json(v) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/json")],
            Json(v.clone()),
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
                .body(Body::from_stream(ChunkStream { chunks, idx: 0 }))
                .unwrap()
        }
    }
}

/// Emit pre-baked chunks (awkwardly split, to exercise line buffering).
struct ChunkStream {
    chunks: Vec<Result<Bytes, std::convert::Infallible>>,
    idx: usize,
}

impl Stream for ChunkStream {
    type Item = Result<Bytes, std::convert::Infallible>;

    fn poll_next(mut self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        if self.idx < self.chunks.len() {
            let item = self.chunks[self.idx].clone();
            self.idx += 1;
            Poll::Ready(Some(item))
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
        protocol,
        base_url,
        api_path: None,
        api_key: Some(REAL_KEY.to_string()),
        billing: Billing::Metered,
        period_limit: None,
        limit_unit: None,
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
) -> kiwano_gateway::store::UsageTotals {
    for _ in 0..60 {
        let totals = state.store.usage_totals(Some(agent), None).unwrap();
        if totals.requests >= expected {
            return totals;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("usage row for agent `{agent}` not persisted in time");
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
    let totals = state.store.usage_totals(Some("claude"), None).unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(totals.input_tokens, 2095);
    assert_eq!(totals.output_tokens, 503);
    assert_eq!(totals.cache_creation_tokens, 2095);

    let by_provider = state.store.usage_by_provider(Some("claude"), None).unwrap();
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
    let (upstream_url, captured) = mock_anthropic(MockReply::Sse(chunks.clone())).await;

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

    let _guard = captured.lock().unwrap();

    let totals = wait_for_usage(&state, "claude", 1).await;
    assert_eq!(totals.input_tokens, 25);
    assert_eq!(totals.output_tokens, 171); // cumulative from message_delta
    assert_eq!(totals.cache_read_tokens, 11);
    assert_eq!(totals.cache_creation_tokens, 3);
}

#[tokio::test]
async fn openai_path_without_key_falls_back_to_codex_agent() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let upstream_body = json!({
        "id": "chatcmpl-1",
        "object": "chat.completion",
        "model": "gpt-4o",
        "choices": [{"index": 0, "message": {"role": "assistant", "content": "hi"}, "finish_reason": "stop"}],
        "usage": {"prompt_tokens": 42, "completion_tokens": 7,
                  "prompt_tokens_details": {"cached_tokens": 32}, "total_tokens": 49}
    });
    let (upstream_url, captured) = mock_openai(MockReply::Json(upstream_body)).await;

    store
        .insert_provider(&provider("p-oai", Protocol::OpenAI, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("codex", "p-oai", 0)).unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    // No auth header at all: tech.md §4.6 fallback attributes by path protocol.
    let response = post_json(
        &app,
        "/v1/chat/completions",
        None,
        r#"{"model":"gpt-4o","messages":[]}"#,
    )
    .await;
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["object"], "chat.completion");

    // OpenAI auth style: real key injected as Bearer.
    let expected_auth = format!("Bearer {REAL_KEY}");
    let captured = captured.lock().unwrap();
    assert_eq!(
        captured.last().map(|(k, v)| (k.as_str(), v.as_str())),
        Some(("authorization", expected_auth.as_str()))
    );
    drop(captured);

    let totals = state.store.usage_totals(Some("codex"), None).unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(totals.input_tokens, 42);
    assert_eq!(totals.output_tokens, 7);
    assert_eq!(totals.cache_read_tokens, 32);
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
    let response = admin
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/reload")
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
    let totals = state.store.usage_totals(Some("claude"), None).unwrap();
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
    let sent = bodies.lock().unwrap();
    assert_eq!(sent.len(), 1);
    assert_eq!(sent[0]["stream"], true);
    assert_eq!(
        sent[0]["stream_options"]["include_usage"], true,
        "usage capture depends on include_usage injection"
    );

    // Usage metered from the converted Anthropic usage events
    // (cache-conserved values, matching what the client observed).
    let totals = wait_for_usage(&state, "claude", 1).await;
    assert_eq!(totals.input_tokens, 6);
    assert_eq!(totals.output_tokens, 4);
    assert_eq!(totals.cache_read_tokens, 5);
}

/// Spawn a mock Gemini upstream on 127.0.0.1:0 (`{model}:{method}` segment).
async fn mock_gemini(response: MockReply) -> (String, Captured) {
    let captured: Captured = Arc::new(Mutex::new(Vec::new()));
    let app = Router::new()
        .route(
            "/v1beta/models/{model_method}",
            post(
                move |AxumState(c): AxumState<Captured>, headers: HeaderMap| {
                    let reply = response.clone();
                    async move {
                        capture(&c, &headers, "x-goog-api-key");
                        mock_response(&reply)
                    }
                },
            ),
        )
        .with_state(captured.clone());
    (spawn(app).await, captured)
}

/// Gemini CLI speaks the native Gemini API (`/v1beta/models/{model}:…`,
/// `x-goog-api-key` auth) to a Gemini-protocol provider: passthrough +
/// usageMetadata metering, placeholder key stripped and replaced.
#[tokio::test]
async fn gemini_inbound_passthrough_meters_usage_metadata() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open(dir.path().join("t.db")).unwrap();

    let upstream_body = json!({
        "candidates": [{"content": {"parts": [{"text": "hello"}], "role": "model"}}],
        "modelVersion": "gemini-2.5-pro",
        "usageMetadata": {"promptTokenCount": 88, "candidatesTokenCount": 31,
                          "cachedContentTokenCount": 12, "totalTokenCount": 119}
    });
    let (upstream_url, captured) = mock_gemini(MockReply::Json(upstream_body)).await;

    store
        .insert_provider(&provider("p-gem", Protocol::Gemini, upstream_url))
        .unwrap();
    store.upsert_binding(&bind("gemini", "p-gem", 0)).unwrap();
    store
        .upsert_placeholder_key("kw-ag-gemini-test", "gemini")
        .unwrap();

    let state = Arc::new(GatewayState::new(store).unwrap());
    let app = data_plane_router(state.clone());

    let response = app
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/v1beta/models/gemini-2.5-pro:generateContent")
                .header("content-type", "application/json")
                .header("x-goog-api-key", "kw-ag-gemini-test")
                .body(Body::from(r#"{"contents":[{"parts":[{"text":"hi"}]}]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let body: Value = serde_json::from_slice(&response_body(response).await).unwrap();
    assert_eq!(body["modelVersion"], "gemini-2.5-pro");
    assert_eq!(body["candidates"][0]["content"]["parts"][0]["text"], "hello");

    // The real key went upstream as x-goog-api-key; the placeholder never left.
    let captured = captured.lock().unwrap();
    assert_eq!(
        captured.last().map(|(k, v)| (k.as_str(), v.as_str())),
        Some(("x-goog-api-key", REAL_KEY))
    );
    drop(captured);

    // usageMetadata metered into SQLite, attributed to the gemini agent.
    let totals = state.store.usage_totals(Some("gemini"), None).unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(totals.input_tokens, 88);
    assert_eq!(totals.output_tokens, 31);
    assert_eq!(totals.cache_read_tokens, 12);

    let by_provider = state.store.usage_by_provider(Some("gemini"), None).unwrap();
    assert_eq!(by_provider.len(), 1);
    assert_eq!(by_provider[0].provider_id, "p-gem");
}

#[tokio::test]
async fn unknown_key_on_anthropic_path_falls_back_to_claude() {
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
    assert_eq!(response.status(), StatusCode::OK);

    // Fallback attribution still meters to the claude agent.
    let totals = state.store.usage_totals(Some("claude"), None).unwrap();
    assert_eq!(totals.requests, 1);
    assert_eq!(captured.lock().unwrap().len(), 1);
}
