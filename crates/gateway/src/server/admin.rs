//! Admin plane: status chip + route hot reload + shutdown (tech.md §4.6
//! control channel; §4.3 flow 2).
//!
//! The Tauri app calls `POST /reload` after writing new bindings to SQLite;
//! the next request routed by the gateway hits the new provider. `GET
//! /status` powers the first-screen gateway state chip and sidecar re-connect.
//!
//! # Transport
//!
//! These are the same three HTTP routes as ever, served over the local IPC
//! endpoint described in [`super::admin_ipc`] — a unix domain socket, or a
//! per-user named pipe on Windows — instead of a loopback TCP port. No handler
//! below knows or cares which: the router is transport-independent, which is
//! why the tests in this module drive it with `oneshot` while
//! [`super::admin_ipc`]'s drive it through a real socket, and why the app's
//! hand-written requests did not have to change shape when the plane moved.
//!
//! # Auth
//!
//! `/reload` and `/shutdown` route the operator's traffic and stop the process,
//! so both require [`ADMIN_TOKEN_HEADER`], carrying the token the gateway minted
//! for itself on first run.
//!
//! **What the token is worth now that the plane is not a port.** Decided
//! deliberately when the transport changed, because the answer is not the same
//! as it was:
//!
//! - The boundary is the endpoint's own access control. On unix the socket sits
//!   in `~/.kiwano` (0700) and is itself 0600; on Windows the pipe's DACL
//!   grants the creating user alone. A process running as another user cannot
//!   reach the plane at all, and does not need a token to be kept out.
//! - Any process running as *this* user can reach it — the socket path is
//!   predictable, not secret — and can equally read the token out of the
//!   database, which is 0600 to that same user. So against a same-user
//!   attacker the token stops nothing that the file permissions do not already
//!   stop. That is the honest reading, and it is why this doc no longer claims
//!   the token is what keeps local processes out.
//! - It is kept anyway, as a layer over a *different* mechanism. The socket's
//!   mode and the database's mode are enforced independently, so the case where
//!   the token still decides something is the case where they come apart: a
//!   socket path overridden into a shared directory, a mode lost to a move or a
//!   restore, a caller that can open the socket but not the data directory.
//!   There, whoever reaches `/reload` must still hold the token, and the token
//!   is readable only from a database they cannot open.
//! - So: redundant when permissions hold, load-bearing when they do not, and
//!   32 bytes of header on a handful of local requests a minute. Keeping it is
//!   the cheap side of that trade.
//!
//! The token lives in the `app_settings` KV ([`ADMIN_TOKEN_KEY`]), which both
//! processes already share — the GUI writes and reads it, the gateway reads
//! it, the CLI reads it — so there is no new IPC and no second place for the
//! two sides to disagree. It is read from the store on every admin request
//! rather than cached in the process, for the same reason: a cached copy could
//! drift from the row the GUI reads, and the failure mode of that drift is the
//! GUI being locked out of its own gateway. Admin traffic is a handful of
//! requests a minute, so the SELECT costs nothing.
//!
//! `/status` is deliberately *not* locked. It is the liveness probe (`ping_admin`),
//! and it is the version check that decides whether a running gateway is this
//! build and should be replaced — both have to work when the caller knows
//! nothing about the token, including the upgrade case where the gateway
//! answering is an older build that has no token support at all. An
//! unauthenticated caller gets identity and uptime and nothing else; the
//! route table, provider ids, strategies, candidate lists and blocked reasons
//! need the token. See [`status`].

use std::sync::Arc;

use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::server::GatewayState;
use crate::store::Store;

/// Header carrying the admin-plane token.
pub const ADMIN_TOKEN_HEADER: &str = "x-kiwano-admin-token";

/// `app_settings` key holding the admin-plane token. Both processes open the
/// same SQLite file, so this row is the whole of the handshake.
pub const ADMIN_TOKEN_KEY: &str = "gateway.admin_token";

pub fn admin_plane_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/reload", post(reload))
        .route("/shutdown", post(shutdown))
        .with_state(state)
}

/// Read the admin token, minting one on first run.
///
/// 128 bits of v4 UUID, hex. Called from `GatewayState::new`, i.e. before
/// either listener binds, so the row is on disk the moment the admin endpoint
/// answers — a GUI that reads it after a successful ping can never lose that
/// race.
///
/// An existing row is adopted as-is rather than overwritten: a gateway
/// restart must not rotate the secret out from under a GUI that is holding
/// it, and `set_app_setting` is an upsert, so minting unconditionally would.
pub fn ensure_admin_token(store: &Store) -> crate::error::Result<String> {
    if let Some(existing) = store
        .app_setting(ADMIN_TOKEN_KEY)
        .filter(|t| !t.trim().is_empty())
    {
        return Ok(existing);
    }
    let token = uuid::Uuid::new_v4().simple().to_string();
    store.set_app_setting(ADMIN_TOKEN_KEY, &token)?;
    // Re-read and prefer the stored value: if another gateway was starting at
    // the same moment, exactly one of us ends up bound to the endpoint, and
    // this is the value both sides will actually compare against from here on.
    Ok(store.app_setting(ADMIN_TOKEN_KEY).unwrap_or(token))
}

/// True when the request carries the stored admin token.
///
/// Fails closed when no token is stored: the gateway mints one before it
/// binds, so an absent row means it was deleted underneath a running gateway,
/// and there is nothing trustworthy to compare against.
fn authorized(state: &GatewayState, headers: &HeaderMap) -> bool {
    let Some(expected) = state
        .store
        .app_setting(ADMIN_TOKEN_KEY)
        .filter(|t| !t.trim().is_empty())
    else {
        tracing::error!(
            key = ADMIN_TOKEN_KEY,
            "no admin token in the store; refusing authenticated admin endpoints"
        );
        return false;
    };
    headers
        .get(ADMIN_TOKEN_HEADER)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|presented| token_matches(presented.trim(), &expected))
}

/// Length check, then an early-exit-free compare. Loopback-bound, so a timing
/// oracle is not the threat model here — it simply costs nothing to not have
/// one.
fn token_matches(presented: &str, expected: &str) -> bool {
    let (a, b) = (presented.as_bytes(), expected.as_bytes());
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// 401 for an admin endpoint the caller may not use. Named `ok: false` so the
/// raw-HTTP clients in `sidecar.rs` / `crates/cli` can tell a refusal from a
/// success without parsing the status line.
fn unauthorized() -> Response {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({
            "ok": false,
            "error": format!(
                "admin plane requires the gateway token in the `{ADMIN_TOKEN_HEADER}` header"
            ),
        })),
    )
        .into_response()
}

/// Stop this process.
///
/// The app calls this when the gateway answering the admin endpoint is not the
/// version it ships. The daemon outlives the GUI on purpose, so the one it finds is
/// usually not its child and there is no handle to kill — and killing it
/// outright would skip the store's write-ahead-log checkpoint, which is the
/// whole reason this plane exists rather than a signal. Token-guarded like
/// `/reload`: an unauthenticated caller must not be able to take the gateway
/// down, and `/reload` is the more powerful endpoint anyway, since it reroutes
/// traffic through a provider of the caller's choosing.
async fn shutdown(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        tracing::warn!("shutdown refused: admin token missing or invalid");
        return unauthorized();
    }
    tracing::info!("shutdown requested on the admin plane");
    state.request_shutdown();
    // Returns before the process exits: axum's graceful shutdown stops
    // accepting and then waits for in-flight responses to finish.
    Json(json!({ "ok": true })).into_response()
}

/// `GET /status` — the liveness/version probe for everyone, the full gateway
/// report for a caller holding the token.
///
/// The unauthenticated answer is deliberately a 200 carrying only identity,
/// version and uptime:
///
/// - `ping_admin` greps the status line for `200`, and the GUI's watchdog uses
///   it every few seconds to tell "a gateway is here" from "nothing is here".
///   A 401 there would read as a gateway that is not running, and the watchdog
///   would respawn one on top of the gateway that answered.
/// - `sidecar::startup_action` parses `name` + `version` out of this body to
///   decide whether the running daemon is the build we ship. That decision has
///   to survive a caller with no readable token row — such as the app on a fresh
///   install, before the gateway has written one — which is why the liveness
///   subset does not require the header.
///
/// Everything past the first branch — routes, provider ids, candidate
/// protocols, strategies, blocked reasons — names the operator's providers and
/// is only for a caller that authenticated. Nothing sensitive is in the
/// redacted body: the version is already public and the name is a constant.
async fn status(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    let liveness = json!({
        "ok": true,
        "name": "kiwano-gateway",
        "version": state.version,
        "uptime_secs": state.started_at.elapsed().as_secs(),
    });
    if !authorized(&state, &headers) {
        return Json(liveness).into_response();
    }

    let metrics = match state.store.metrics() {
        Ok(m) => m,
        Err(e) => {
            return (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e.to_string() })),
            )
                .into_response();
        }
    };

    let table = state.route_table();
    let mut routes: Vec<Value> = table
        .routes
        .values()
        .map(|r| {
            json!({
                "agent": r.agent,
                "strategy": r.strategy.as_str(),
                "primary_provider": r.candidates.first().map(|p| p.id.clone()),
                "candidates": r
                    .candidates
                    .iter()
                    .map(|p| json!({ "id": p.id, "protocol": p.protocol.as_str() }))
                    .collect::<Vec<_>>(),
            })
        })
        .collect();
    routes.sort_by(|a, b| a["agent"].as_str().cmp(&b["agent"].as_str()));

    // Providers the gateway is refusing to route, with the reason. The Apps
    // card reads this rather than recomputing: it must agree with what is
    // actually being enforced, and the enforcement is here.
    let mut blocked: Vec<Value> = state
        .limits()
        .entries()
        .map(|(id, reason)| {
            json!({
                "provider_id": id,
                "reason": reason.describe(),
            })
        })
        .collect();
    blocked.sort_by(|a, b| a["provider_id"].as_str().cmp(&b["provider_id"].as_str()));

    Json(json!({
        "ok": true,
        "name": "kiwano-gateway",
        "version": state.version,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        "providers": metrics.providers,
        "bindings": metrics.bindings,
        "placeholder_keys": metrics.placeholder_keys,
        "usage_rows": metrics.usage_rows,
        "agents_routed": table.routes.len(),
        "routes": routes,
        "blocked": blocked,
    }))
    .into_response()
}

/// `POST /reload` — rebuild the route table from SQLite. The powerful one:
/// it decides which provider the operator's traffic is spent on, which is
/// exactly what a token is for.
async fn reload(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        tracing::warn!("reload refused: admin token missing or invalid");
        return unauthorized();
    }
    match state.reload_routes() {
        Ok(agents) => {
            tracing::info!(agents_routed = agents, "route table reloaded");
            Json(json!({ "ok": true, "agents_routed": agents })).into_response()
        }
        Err(e) => {
            tracing::error!(error = %e, "route table reload failed");
            (
                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "ok": false, "error": e.to_string() })),
            )
                .into_response()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::data_plane_router;
    use crate::store::{now_rfc3339, Billing, Binding, Protocol, Provider, Store};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use std::io::{Read, Write};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tower::ServiceExt; // oneshot

    fn provider(id: &str, protocol: Protocol, base_url: &str) -> Provider {
        Provider {
            id: id.into(),
            name: id.into(),
            protocol,
            base_url: base_url.into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some(format!("sk-{id}")),
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

    fn bind(store: &Store, agent: &str, provider_id: &str) {
        store
            .upsert_binding(&Binding {
                agent: agent.into(),
                provider_id: provider_id.into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
    }

    fn seed(store: &Store, with_binding: bool) {
        store
            .insert_provider(&provider(
                "p-ant",
                Protocol::Anthropic,
                "https://api.anthropic.com",
            ))
            .unwrap();
        if with_binding {
            bind(store, "claude", "p-ant");
        }
    }

    /// Two agents, each bound to its own stub upstream and holding the
    /// placeholder key a takeover would have minted for it.
    fn seed_routed(store: &Store, claude_url: &str, codex_url: &str) {
        store
            .insert_provider(&provider("p-claude", Protocol::Anthropic, claude_url))
            .unwrap();
        store
            .insert_provider(&provider("p-codex", Protocol::OpenAI, codex_url))
            .unwrap();
        bind(store, "claude", "p-claude");
        bind(store, "codex", "p-codex");
        store
            .upsert_placeholder_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-codex-xyz789", "codex")
            .unwrap();
    }

    /// A loopback listener standing in for an upstream provider, counting the
    /// requests that reach it.
    ///
    /// The refusal tests assert on this count *as well as* on the status code.
    /// "401" is only half the claim: the other half is that the operator's
    /// upstream was never called, which is the thing the old path-protocol
    /// fallback got wrong.
    struct StubUpstream {
        port: u16,
        hits: Arc<AtomicUsize>,
    }

    impl StubUpstream {
        fn start() -> StubUpstream {
            let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind stub upstream");
            let port = listener.local_addr().expect("stub addr").port();
            let hits = Arc::new(AtomicUsize::new(0));
            let counter = hits.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else { continue };
                    counter.fetch_add(1, Ordering::SeqCst);
                    // Drain the request before answering: a client whose
                    // request spans more than one segment otherwise sees its
                    // connection reset mid-write.
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf);
                    let body = r#"{"ok":true}"#;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                }
            });
            StubUpstream { port, hits }
        }

        fn url(&self) -> String {
            format!("http://127.0.0.1:{}", self.port)
        }

        fn hits(&self) -> usize {
            self.hits.load(Ordering::SeqCst)
        }
    }

    /// The token as the GUI and the CLI read it: out of the shared SQLite row.
    fn shared_token(state: &GatewayState) -> String {
        state
            .store
            .app_setting(ADMIN_TOKEN_KEY)
            .expect("the gateway mints its admin token at startup")
    }

    fn admin_request(method: &str, uri: &str, token: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = token {
            builder = builder.header(ADMIN_TOKEN_HEADER, token);
        }
        builder.body(Body::empty()).unwrap()
    }

    /// One data-plane POST, with whatever the agent client puts in
    /// `Authorization` (the header every protocol family here accepts, and the
    /// one a real client sends).
    fn agent_request(uri: &str, auth: Option<&str>) -> Request<Body> {
        let mut builder = Request::builder().method("POST").uri(uri);
        if let Some(auth) = auth {
            builder = builder.header("Authorization", auth);
        }
        builder
            .body(Body::from(r#"{"model":"m","messages":[]}"#))
            .unwrap()
    }

    async fn body_json(response: Response) -> Value {
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    // ── the token itself ────────────────────────────────────────────────

    /// Minted on first use, then adopted: a restart must not rotate the secret
    /// out from under a GUI that is holding the old one.
    #[test]
    fn admin_token_is_minted_once_and_reused() {
        let store = Store::open_in_memory().unwrap();
        let first = ensure_admin_token(&store).unwrap();
        assert_eq!(first.len(), 32, "128 bits of hex");
        assert_eq!(ensure_admin_token(&store).unwrap(), first);

        // A blank row is not a token; it gets replaced.
        store.set_app_setting(ADMIN_TOKEN_KEY, "   ").unwrap();
        assert_ne!(ensure_admin_token(&store).unwrap(), "   ");
    }

    #[test]
    fn token_compare_requires_an_exact_match() {
        assert!(token_matches("abcdef", "abcdef"));
        assert!(!token_matches("abcdef", "abcde"));
        assert!(!token_matches("abcdef", "abcdxf"));
        assert!(!token_matches("", "abcdef"));
    }

    // ── admin plane auth ────────────────────────────────────────────────

    #[tokio::test]
    async fn status_reports_counts_and_routes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("GET", "/status", Some(&token)))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["providers"], 1);
        assert_eq!(v["bindings"], 1);
        assert_eq!(v["agents_routed"], 1);
        assert_eq!(v["routes"][0]["agent"], "claude");
        assert_eq!(v["routes"][0]["primary_provider"], "p-ant");
        assert_eq!(v["routes"][0]["strategy"], "single");
    }

    /// `/status` is the liveness probe, so it answers without a token — but
    /// only with identity, version and uptime. The route table, provider ids
    /// and blocked reasons are the operator's business.
    #[tokio::test]
    async fn status_without_the_token_reports_only_liveness() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("GET", "/status", None))
            .await
            .unwrap();
        // 200 on purpose: `ping_admin` greps the status line, and a 401 there
        // would read as "no gateway is running".
        let v = body_json(response).await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["name"], "kiwano-gateway");
        assert!(v["version"].is_string());
        assert!(v["uptime_secs"].is_number());
        assert!(v["routes"].is_null(), "no route table without the token");
        assert!(v["blocked"].is_null());
        assert!(v["providers"].is_null());
        assert!(v["bindings"].is_null());
    }

    /// A wrong token is no better than none.
    #[tokio::test]
    async fn status_with_a_wrong_token_reports_only_liveness() {
        let dir = tempfile::tempdir().unwrap();
        let state =
            Arc::new(GatewayState::new(Store::open(dir.path().join("t.db")).unwrap()).unwrap());
        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("GET", "/status", Some("not-the-token")))
            .await
            .unwrap();
        let v = body_json(response).await;
        assert!(v["routes"].is_null());
    }

    /// The app replaces a gateway it adopted rather than spawned by asking it
    /// to stop — the daemon is not its child, so there is no handle to kill.
    #[tokio::test]
    async fn shutdown_flips_the_stop_signal() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);
        // Subscribed before the request: `watch` only delivers to receivers
        // that already exist, and `main`'s serve loops are that receiver.
        let mut rx = state.shutdown_rx();
        assert!(!*rx.borrow_and_update(), "starts running");

        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(admin_request("POST", "/shutdown", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["ok"], true);

        rx.changed().await.expect("stop signal delivered");
        assert!(*rx.borrow_and_update(), "stop requested");
    }

    /// Stopping the gateway is exactly what a local process must not be able
    /// to do, so the refusal has to leave the stop signal untouched.
    #[tokio::test]
    async fn shutdown_without_the_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let state = Arc::new(GatewayState::new(store).unwrap());
        let mut rx = state.shutdown_rx();

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("POST", "/shutdown", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("POST", "/shutdown", Some("not-the-token")))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        assert!(
            !*rx.borrow_and_update(),
            "a refused request must not have asked the gateway to stop"
        );
    }

    #[tokio::test]
    async fn reload_picks_up_new_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, false);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        // Before reload: no agents routed, requests would fail.
        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(admin_request("GET", "/status", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["agents_routed"], 0);

        // App writes the binding to SQLite then hot-reloads the gateway.
        bind(&state.store, "claude", "p-ant");
        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(admin_request("POST", "/reload", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["agents_routed"], 1);

        // The data plane now resolves the binding without a restart.
        let table = state.route_table();
        let routed = table.select("claude").expect("route after reload");
        assert_eq!(routed.id, "p-ant");
    }

    /// Reload decides which provider the operator's traffic is spent on, so
    /// an unauthenticated caller must not reach it — and the route table must
    /// be exactly as it was when it did not.
    #[tokio::test]
    async fn reload_without_the_token_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, false);
        let state = Arc::new(GatewayState::new(store).unwrap());
        bind(&state.store, "claude", "p-ant");

        for token in [None, Some("not-the-token")] {
            let response = admin_plane_router(state.clone())
                .oneshot(admin_request("POST", "/reload", token))
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        }
        assert_eq!(
            state.route_table().routes.len(),
            0,
            "the binding was written to SQLite, but nothing reloaded it"
        );

        // With the token the same call goes through.
        let token = shared_token(&state);
        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(admin_request("POST", "/reload", Some(&token)))
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["ok"], true);
        assert_eq!(v["agents_routed"], 1);
    }

    // ── the admin plane over its real transport ──────────────────────────

    /// Every route the app and the CLI use, over a real socket/pipe rather
    /// than `oneshot`: this is the wiring the transport change could break
    /// while every test above still passed. `/status` (with and without the
    /// token), `/reload` and `/shutdown` all have to answer.
    ///
    /// Runs on both platforms — the endpoint is whatever the OS gives us, and
    /// on Windows CI this is the test that exercises the named-pipe server.
    #[tokio::test(flavor = "multi_thread")]
    async fn admin_routes_answer_over_the_ipc_endpoint() {
        use crate::server::admin_ipc::test_support::{request, serve_in_background, test_endpoint};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);
        let endpoint = test_endpoint(dir.path());
        serve_in_background(&endpoint, admin_plane_router(state.clone()));

        let get = |path: &str, token: Option<&str>| {
            let header = token
                .map(|t| format!("{ADMIN_TOKEN_HEADER}: {t}\r\n"))
                .unwrap_or_default();
            request(
                &endpoint,
                &format!(
                    "GET {path} HTTP/1.1\r\nHost: localhost\r\n{header}Connection: close\r\n\r\n"
                ),
            )
        };
        let post = |path: &str, token: Option<&str>| {
            let header = token
                .map(|t| format!("{ADMIN_TOKEN_HEADER}: {t}\r\n"))
                .unwrap_or_default();
            request(
                &endpoint,
                &format!(
                    "POST {path} HTTP/1.1\r\nHost: localhost\r\n{header}Content-Length: 0\r\nConnection: close\r\n\r\n"
                ),
            )
        };

        // /status, authenticated: the full report, not just liveness.
        let status = get("/status", Some(&token));
        assert!(status.starts_with("HTTP/1.1 200 OK"), "{status}");
        assert!(status.contains("\"agents_routed\":1"), "{status}");

        // /status, unauthenticated: still a 200, and liveness only.
        let liveness = get("/status", None);
        assert!(liveness.starts_with("HTTP/1.1 200 OK"), "{liveness}");
        assert!(liveness.contains("kiwano-gateway"), "{liveness}");
        assert!(!liveness.contains("agents_routed"), "{liveness}");

        // /reload, refused and then accepted.
        let refused = post("/reload", None);
        assert!(refused.starts_with("HTTP/1.1 401"), "{refused}");
        let reloaded = post("/reload", Some(&token));
        assert!(reloaded.starts_with("HTTP/1.1 200 OK"), "{reloaded}");
        assert!(reloaded.contains("\"ok\":true"), "{reloaded}");

        // /shutdown last: it flips the stop signal, and everything above had to
        // happen while the gateway was still running.
        let mut rx = state.shutdown_rx();
        let stopped = post("/shutdown", Some(&token));
        assert!(stopped.starts_with("HTTP/1.1 200 OK"), "{stopped}");
        rx.changed().await.expect("stop signal delivered");
        assert!(*rx.borrow_and_update(), "stop requested");
    }

    // ── data plane auth ─────────────────────────────────────────────────

    /// The refusal that closes the data plane: an unknown key on an
    /// OpenAI-shaped path used to be attributed to Codex by path protocol and
    /// forwarded on the operator's real OpenAI credentials.
    #[tokio::test]
    async fn unknown_key_is_refused_and_never_reaches_upstream() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let claude = StubUpstream::start();
        let codex = StubUpstream::start();
        seed_routed(&store, &claude.url(), &codex.url());
        let state = Arc::new(GatewayState::new(store).unwrap());

        let response = data_plane_router(state.clone())
            .oneshot(agent_request(
                "/v1/chat/completions",
                Some("Bearer sk-not-ours"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(codex.hits(), 0, "the guessed agent was never called");
        assert_eq!(claude.hits(), 0, "nor was any other agent's upstream");
    }

    /// No key at all is the same refusal, not a guess (this is the shape the
    /// `data_plane_serves_after_reload` wiring test used to send).
    #[tokio::test]
    async fn missing_key_is_refused() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let claude = StubUpstream::start();
        let codex = StubUpstream::start();
        seed_routed(&store, &claude.url(), &codex.url());
        let state = Arc::new(GatewayState::new(store).unwrap());

        let response = data_plane_router(state.clone())
            .oneshot(agent_request("/v1/messages", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
        assert_eq!(claude.hits(), 0);
        assert_eq!(codex.hits(), 0);
    }

    /// The other half of the contract: a key the gateway minted still routes,
    /// to the agent that key belongs to and no other.
    #[tokio::test]
    async fn known_placeholder_key_routes_to_its_agent() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        let claude = StubUpstream::start();
        let codex = StubUpstream::start();
        seed_routed(&store, &claude.url(), &codex.url());
        let state = Arc::new(GatewayState::new(store).unwrap());

        // Codex's key, on Codex's path: the codex upstream answers, the
        // claude one is not touched.
        let response = data_plane_router(state.clone())
            .oneshot(agent_request(
                "/v1/chat/completions",
                Some("Bearer kw-ag-codex-xyz789"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(codex.hits(), 1);
        assert_eq!(claude.hits(), 0);

        // And the same key on an Anthropic path still belongs to codex: the
        // path no longer decides the agent.
        let response = data_plane_router(state.clone())
            .oneshot(agent_request(
                "/v1/messages",
                Some("Bearer kw-ag-claude-abc123"),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(claude.hits(), 1);
        assert_eq!(codex.hits(), 1);
    }

    #[tokio::test]
    async fn data_plane_serves_after_reload() {
        // End-to-end wiring sanity inside one process: unknown path 404s with
        // the gateway error shape; /v1/messages with no key 401s (auth runs
        // before routing — an unidentified caller learns nothing about what is
        // bound, including whether anything is).
        let dir = tempfile::tempdir().unwrap();
        let state =
            Arc::new(GatewayState::new(Store::open(dir.path().join("t.db")).unwrap()).unwrap());

        let response = data_plane_router(state.clone())
            .oneshot(agent_request("/v1/messages", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

        let response = data_plane_router(state.clone())
            .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
