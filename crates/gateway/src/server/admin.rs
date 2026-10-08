//! Admin plane: status chip + route hot reload + shutdown (tech.md §4.6
//! control channel; §4.3 flow 2), and the event stream the app's live numbers
//! ride on.
//!
//! The Tauri app calls `POST /reload` after writing new bindings to SQLite;
//! the next request routed by the gateway hits the new provider. `GET
//! /status` powers the first-screen gateway state chip and sidecar re-connect,
//! and `GET /events` streams a tick per recorded request, which is what lets a
//! screen re-read its numbers instead of polling for them.
//!
//! # Transport
//!
//! These are the same HTTP routes as ever, served over the local IPC
//! endpoint described in [`super::admin_ipc`] — a unix domain socket, or a
//! per-user named pipe on Windows — instead of a loopback TCP port. No handler
//! below knows or cares which: the router is transport-independent, which is
//! why the tests in this module drive it with `oneshot` while
//! [`super::admin_ipc`]'s drive it through a real socket, and why the app's
//! hand-written requests did not have to change shape when the plane moved.
//! (`/events` is the one route whose response does not end, which is why its
//! test runs over a real listener rather than `oneshot`.)
//!
//! # Auth
//!
//! `/reload` and `/shutdown` route the operator's traffic and stop the process,
//! and `/events` says when there is traffic at all, so all three require
//! [`ADMIN_TOKEN_HEADER`], carrying the token the gateway minted for itself on
//! first run.
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

use axum::extract::{Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{delete, get, patch, post, put};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::server::{GatewayEvent, GatewayState};
use crate::store::Store;

/// Header carrying the admin-plane token.
pub const ADMIN_TOKEN_HEADER: &str = "x-kiwano-admin-token";

/// `app_settings` key holding the admin-plane token. Both processes open the
/// same SQLite file, so this row is the whole of the handshake.
pub const ADMIN_TOKEN_KEY: &str = "gateway.admin_token";

pub fn admin_plane_router(state: Arc<GatewayState>) -> Router {
    // The resource API is one nested router with **one** auth layer, rather
    // than a check at the top of each handler. The difference is not style:
    // axum runs a handler's extractors before its body, so a per-handler check
    // is skipped for every request whose body does not parse — an
    // unauthenticated caller would get 415 from an endpoint whose contract says
    // 401, and the guard would be one refactor away from not running at all.
    // A layer cannot be overtaken that way: it runs before any extractor does.
    let api = Router::new()
        .route(
            "/providers/{provider_id}/keys",
            get(list_provider_keys).post(add_provider_key),
        )
        .route("/keys/{id}", delete(delete_provider_key))
        .route("/agent-routes", get(list_agent_routes))
        .route("/agents/{agent}/strategy", post(set_agent_strategy_route))
        .route("/agents/{agent}/limits", put(set_agent_limits_route))
        .route(
            "/agents/{agent}/bindings",
            post(add_agent_binding_route).put(reorder_agent_bindings_route),
        )
        .route(
            "/agents/{agent}/bindings/{provider_id}",
            patch(update_agent_binding_route).delete(remove_agent_binding_route),
        )
        .route(
            "/agents/{target}/route/apply",
            post(apply_agent_route_route),
        )
        .route("/providers/{id}/enabled", put(set_provider_enabled_route))
        .route("/providers/{id}", delete(delete_provider_route))
        .route("/custom-agents", post(create_custom_agent_route))
        .route(
            "/custom-agents/{id}",
            put(update_custom_agent_route).delete(remove_custom_agent_route),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            require_admin_token,
        ));

    Router::new()
        .route("/status", get(status))
        .route("/events", get(events))
        .route("/reload", post(reload))
        .route("/shutdown", post(shutdown))
        .nest("/api", api)
        .with_state(state)
}

/// The gate every resource route sits behind: the same token `/reload` wants,
/// applied once for the whole subtree.
async fn require_admin_token(
    State(state): State<Arc<GatewayState>>,
    headers: HeaderMap,
    request: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    if !authorized(&state, &headers) {
        tracing::warn!(
            path = %request.uri().path(),
            "resource request refused: admin token missing or invalid"
        );
        return unauthorized();
    }
    next.run(request).await
}

/// `GET /api/providers/{provider_id}/keys` — the provider's key pool, masked.
///
/// Masked before it gets here rather than by the caller: the daemon holds the
/// plaintext, and a response that carried it would put it on a channel that has
/// no need for it. The masking is `crate::api::keys`'s — the same function
/// `vm::list_api_keys` re-exports — so the app's direct path and this one cannot
/// come to disagree about what a reader sees.
///
/// What is left here is transport: the status, and the envelope.
async fn list_provider_keys(
    State(state): State<Arc<GatewayState>>,
    Path(provider_id): Path<String>,
) -> Response {
    match crate::api::keys::list_api_keys(&state.store, &provider_id) {
        Ok(keys) => Json(keys).into_response(),
        Err(e) => resource_error(e),
    }
}

/// `GET /events` — a stream of "a request was just metered" ticks.
///
/// This is what keeps the app's numbers live without polling for them: every
/// recorded request sends one, and the app re-reads what its screen is showing
/// when it arrives. It carries no numbers itself — see `GatewayState`'s
/// `usage_ticks`.
///
/// Token-guarded like `/reload`: how busy this machine is and when is the
/// operator's business, and "every local process may watch" is not a property
/// the loopback bind ever gave this plane anyway (see `admin_ipc`).
async fn events(State(state): State<Arc<GatewayState>>, headers: HeaderMap) -> Response {
    if !authorized(&state, &headers) {
        return unauthorized();
    }
    // `KeepAlive` writes a comment line through an idle stream, which is also
    // how a dead peer is noticed at all: without it, a stream that says nothing
    // for an hour looks exactly like one whose other end is gone.
    Sse::new(usage_events(state.usage_ticks()))
        .keep_alive(KeepAlive::default())
        .into_response()
}

/// The event stream, as SSE events.
///
/// `unfold` over `recv` rather than a `Stream` impl: `poll_recv` is not part of
/// the broadcast receiver in the tokio this builds against, and this is the same
/// three cases without the `Pin` ceremony. Each event carries its payload as
/// one line of JSON — the reader treats every `data:` line as one event, so a
/// payload must never span lines.
fn usage_events(
    rx: tokio::sync::broadcast::Receiver<GatewayEvent>,
) -> impl futures_core::Stream<Item = Result<Event, std::convert::Infallible>> {
    futures_util::stream::unfold(rx, |mut rx| async move {
        match rx.recv().await {
            // A subscriber that fell behind is told what the events would have
            // told it: there is newer data than it has seen. An error would
            // close a stream that has nothing wrong with it, and the reader's
            // answer to either is the same re-read.
            Ok(event) => {
                let json = serde_json::to_string(&event)
                    .unwrap_or_else(|_| r#"{"kind":"usage"}"#.to_string());
                Some((Ok(Event::default().data(json)), rx))
            }
            // Behind the buffer: there is newer data than it has seen. The
            // usage re-read covers the lost ticks; a lost finding is re-read
            // by the same request-log the event would have linked to.
            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                Some((Ok(Event::default().data(r#"{"kind":"usage"}"#)), rx))
            }
            // Closed: the gateway is going away.
            Err(tokio::sync::broadcast::error::RecvError::Closed) => None,
        }
    })
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
        "name": "kiwanod",
        "version": state.version,
        "uptime_secs": state.started_at.elapsed().as_secs(),
    });
    if !authorized(&state, &headers) {
        return Json(liveness).into_response();
    }

    // Past the token check, so only a caller holding the admin token sees it.
    // The identity is what an app compares against its own database to notice
    // that the gateway is writing somewhere else.
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
        "name": "kiwanod",
        "version": state.version,
        "uptime_secs": state.started_at.elapsed().as_secs(),
        // Which database this process opened. A client that has its own open
        // compares the two — the only way to notice the split described in
        // `migrate.local.md` §13.1 from the outside.
        "install_id": state.install_id,
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

/// A resource failure as a response: the **kind** picks the status, the
/// **message** is what a person reads.
///
/// One function rather than a match per handler, for the reason the token check
/// is a layer rather than a line: thirty copies of a mapping is thirty chances
/// for one of them to answer 500 where the contract says 404.
///
/// The body keeps the envelope the client already parses (`{"ok":false,
/// "error":…}` — `sidecar::admin_send` reads `error`) and adds `kind` beside
/// it. Nothing has to read the new field yet; it is there so a caller *can*
/// tell "retry this" from "this will never work" without matching on prose.
///
/// [`ApiError`]: kiwano_api::error::ApiError
fn resource_error(err: kiwano_api::error::ApiError) -> Response {
    use kiwano_api::error::ApiErrorKind;
    let status = match err.kind() {
        ApiErrorKind::NotFound => StatusCode::NOT_FOUND,
        ApiErrorKind::Invalid => StatusCode::BAD_REQUEST,
        ApiErrorKind::Failed => StatusCode::INTERNAL_SERVER_ERROR,
    };
    if status.is_server_error() {
        // The caller gets the sentence; the operator gets it with the path.
        tracing::error!(error = %err, "resource request failed");
    }
    (
        status,
        Json(json!({ "ok": false, "error": err.message(), "kind": err.kind() })),
    )
        .into_response()
}

/// `POST /api/providers/{provider_id}/keys` — add a rotation key.
///
/// Idempotent on the key's value, and that is the store's decision rather than
/// this handler's, so the `vm::` path agrees with it. A retry therefore answers
/// with the row that is already there rather than a second one, which is what
/// lets a client retry a write at all (`migrate.local.md` §6.1).
#[derive(serde::Deserialize)]
struct AddKeyBody {
    api_key: String,
    #[serde(default)]
    label: Option<String>,
}

async fn add_provider_key(
    State(state): State<Arc<GatewayState>>,
    Path(provider_id): Path<String>,
    Json(body): Json<AddKeyBody>,
) -> Response {
    match crate::api::keys::add_api_key(
        &state.store,
        &provider_id,
        &body.api_key,
        body.label.as_deref(),
    ) {
        Ok(key) => Json(key).into_response(),
        Err(e) => resource_error(e),
    }
}

/// `DELETE /api/keys/{id}` — remove a rotation key. `false` when there was no
/// such row, which is the state a retry finds and is not an error.
async fn delete_provider_key(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<i64>,
) -> Response {
    match crate::api::keys::delete_api_key(&state.store, id) {
        Ok(deleted) => Json(json!({ "deleted": deleted })).into_response(),
        Err(e) => resource_error(e),
    }
}

// ── Agent routes and bindings (`migrate.local.md` §7 batch 1) ──

/// Re-read the route table after a write, then answer.
///
/// This is what the app did for the daemon before the write moved here: every
/// mutation ended with a `POST /reload`, because the process doing the writing
/// was not the one holding the table. Now it is the same process, so it
/// invalidates its own copy — one fewer round trip, and one fewer thing a
/// caller can forget to do.
///
/// A failed reload does **not** fail the request: the row is committed, and
/// answering 500 would report a problem the caller has no way to act on. The
/// operator gets the log line instead.
fn after_write(state: &GatewayState, result: Result<(), kiwano_api::error::ApiError>) -> Response {
    match result {
        Ok(()) => {
            reload_after_write(state);
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => resource_error(e),
    }
}

/// [`after_write`] for a write whose answer is the resource it produced.
fn after_write_with<T: serde::Serialize>(
    state: &GatewayState,
    result: Result<T, kiwano_api::error::ApiError>,
) -> Response {
    match result {
        Ok(value) => {
            reload_after_write(state);
            Json(value).into_response()
        }
        Err(e) => resource_error(e),
    }
}

fn reload_after_write(state: &GatewayState) {
    if let Err(e) = state.reload_routes() {
        tracing::error!(error = %e, "route table reload failed after a write");
    }
}

/// `GET /api/agent-routes` — one row per agent that has bindings.
async fn list_agent_routes(State(state): State<Arc<GatewayState>>) -> Response {
    match crate::api::routes::build_agent_routes(&state.store) {
        Ok(routes) => Json(routes).into_response(),
        Err(e) => resource_error(e),
    }
}

#[derive(serde::Deserialize)]
struct StrategyBody {
    strategy: String,
    /// Strategy JSON payload (quota: `{"limit","unit"}`); absent otherwise.
    #[serde(default)]
    config: Option<String>,
}

/// `POST /api/agents/{agent}/strategy` — set an agent's strategy, and its
/// optional payload.
async fn set_agent_strategy_route(
    State(state): State<Arc<GatewayState>>,
    Path(agent): Path<String>,
    Json(body): Json<StrategyBody>,
) -> Response {
    let result = crate::api::routes::set_agent_strategy(
        &state.store,
        &agent,
        &body.strategy,
        body.config.as_deref(),
    );
    after_write(&state, result)
}

/// `PUT /api/agents/{agent}/limits` — the agent's whole ceiling set. An empty
/// array clears it, which is how the screen says "no limit".
async fn set_agent_limits_route(
    State(state): State<Arc<GatewayState>>,
    Path(agent): Path<String>,
    Json(limits): Json<Vec<kiwano_api::routes::AgentLimitVm>>,
) -> Response {
    let result = crate::api::routes::set_agent_limits(&state.store, &agent, limits);
    after_write(&state, result)
}

#[derive(serde::Deserialize)]
struct BindBody {
    provider_id: String,
}

/// `POST /api/agents/{agent}/bindings` — add a candidate at the queue tail.
/// Binding a provider that is already bound is a no-op, so a retry is safe.
async fn add_agent_binding_route(
    State(state): State<Arc<GatewayState>>,
    Path(agent): Path<String>,
    Json(body): Json<BindBody>,
) -> Response {
    let result = crate::api::routes::add_agent_binding(&state.store, &agent, &body.provider_id);
    after_write(&state, result)
}

#[derive(serde::Deserialize)]
struct ReorderBody {
    provider_ids: Vec<String>,
}

/// `PUT /api/agents/{agent}/bindings` — rewrite the priority order. A `PUT`
/// rather than a `POST` because the body *is* the resulting order: sending it
/// twice leaves the same route, which is the property the verb claims.
async fn reorder_agent_bindings_route(
    State(state): State<Arc<GatewayState>>,
    Path(agent): Path<String>,
    Json(body): Json<ReorderBody>,
) -> Response {
    let result =
        crate::api::routes::reorder_agent_bindings(&state.store, &agent, &body.provider_ids);
    after_write(&state, result)
}

/// A patch to one binding. Every field is optional and its absence means "leave
/// it alone" — which is why this cannot be a `PUT` of the whole binding.
#[derive(serde::Deserialize)]
struct BindingPatchBody {
    #[serde(default)]
    weight: Option<i64>,
    #[serde(default)]
    win_start: Option<String>,
    #[serde(default)]
    win_end: Option<String>,
}

/// `PATCH /api/agents/{agent}/bindings/{provider_id}` — weight and window.
async fn update_agent_binding_route(
    State(state): State<Arc<GatewayState>>,
    Path((agent, provider_id)): Path<(String, String)>,
    Json(body): Json<BindingPatchBody>,
) -> Response {
    let result = crate::api::routes::update_agent_binding(
        &state.store,
        &agent,
        &provider_id,
        body.weight,
        body.win_start,
        body.win_end,
    );
    after_write(&state, result)
}

/// `DELETE /api/agents/{agent}/bindings/{provider_id}` — unbind one candidate.
/// Unlike the key pool's delete, a second call is an error: the binding is not
/// a row you can re-point at, and "it was already gone" is worth telling apart
/// from "it never was" (`vm::remove_agent_binding` has always decided that).
async fn remove_agent_binding_route(
    State(state): State<Arc<GatewayState>>,
    Path((agent, provider_id)): Path<(String, String)>,
) -> Response {
    let result = crate::api::routes::remove_agent_binding(&state.store, &agent, &provider_id);
    after_write(&state, result)
}

#[derive(serde::Deserialize)]
struct ApplyRouteBody {
    source: String,
}

/// `POST /api/agents/{target}/route/apply` — copy `source`'s route onto
/// `target`, replacing its own. The source keeps its bindings.
async fn apply_agent_route_route(
    State(state): State<Arc<GatewayState>>,
    Path(target): Path<String>,
    Json(body): Json<ApplyRouteBody>,
) -> Response {
    let result = crate::api::routes::apply_agent_route(&state.store, &target, &body.source);
    after_write(&state, result)
}

// ── User-defined agents (`migrate.local.md` §7 batch 1) ──

/// The body of a create. **The id is the caller's**: it is minted client-side
/// (`kiwano_api::ids::mint_agent_id`) and is the identity of the operation, so a
/// replayed request lands on the agent it already made instead of making a
/// second one — the resolution §6.1 asked for on the one command whose id is
/// random.
#[derive(serde::Deserialize)]
struct CreateAgentBody {
    id: String,
    label: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
}

/// `POST /api/custom-agents` — define a user-defined agent: a row, a key and a
/// default route. Idempotent on `id`.
async fn create_custom_agent_route(
    State(state): State<Arc<GatewayState>>,
    Json(body): Json<CreateAgentBody>,
) -> Response {
    let result = crate::api::agents::create_custom_agent(
        &state.store,
        &body.id,
        &body.label,
        body.note.as_deref(),
        body.protocol.as_deref(),
    );
    after_write_with(&state, result)
}

/// The body of a rename: the resulting state, which is what makes a `PUT` the
/// right verb for it.
#[derive(serde::Deserialize)]
struct UpdateAgentBody {
    label: String,
    #[serde(default)]
    note: Option<String>,
    #[serde(default)]
    protocol: Option<String>,
}

/// `PUT /api/custom-agents/{id}` — rename, re-note, re-protocol. The id is in
/// the path because it is not what is changing.
async fn update_custom_agent_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<UpdateAgentBody>,
) -> Response {
    let result = crate::api::agents::update_custom_agent(
        &state.store,
        &id,
        &body.label,
        body.note.as_deref(),
        body.protocol.as_deref(),
    );
    after_write_with(&state, result)
}

/// `DELETE /api/custom-agents/{id}` — the agent, its route and its key. Its
/// usage history stays: it is not the route that was deleted.
async fn remove_custom_agent_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    let result = crate::api::agents::remove_custom_agent(&state.store, &id);
    after_write(&state, result)
}

// ── Provider writes (`migrate.local.md` §7 batch 1) ──

#[derive(serde::Deserialize)]
struct EnabledBody {
    enabled: bool,
}

/// `PUT /api/providers/{id}/enabled` — switch a provider on or off. A `PUT`
/// because the body is the state the caller wants: sending it twice leaves the
/// same row, and the function's own early return means it does not even touch
/// `updated_at` the second time.
async fn set_provider_enabled_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
    Json(body): Json<EnabledBody>,
) -> Response {
    let result = crate::api::providers::set_provider_enabled(&state.store, &id, body.enabled);
    after_write(&state, result)
}

/// `DELETE /api/providers/{id}` — delete a provider, promoting the next
/// candidate wherever it was some agent's primary.
async fn delete_provider_route(
    State(state): State<Arc<GatewayState>>,
    Path(id): Path<String>,
) -> Response {
    match crate::api::providers::delete_provider(&state.store, &id) {
        Ok(deleted) => {
            // Only a real deletion changes what routes; a retry of a delete that
            // already happened has nothing to re-read.
            if deleted {
                reload_after_write(&state);
            }
            Json(json!({ "deleted": deleted })).into_response()
        }
        Err(e) => resource_error(e),
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
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol,
            base_url: base_url.into(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some(format!("sk-{id}")),
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

    /// [`body_json`] for a response that is *supposed* to be a refusal: the
    /// status is the caller's assertion to make, not this helper's.
    async fn error_json(response: Response) -> Value {
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

    // ── the resource API (migrate.local.md §7 batch 1) ──────────────────

    /// The daemon serves the key pool the way the app used to build it, and
    /// **the plaintext never crosses the socket** — the assertion that matters,
    /// because the failure it catches is silent: a response carrying the real
    /// key looks exactly like a response carrying the masked one until someone
    /// reads it.
    #[tokio::test]
    async fn the_key_pool_is_served_masked() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        store
            .insert_api_key("p-ant", "sk-ant-rotated-abcdefghijkl", Some("backup"))
            .unwrap();
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request(
                "GET",
                "/api/providers/p-ant/keys",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        let v = body_json(response).await;
        let rows = v
            .as_array()
            .expect("a bare array, as the frontend receives");
        assert_eq!(rows.len(), 1);
        let masked = rows[0]["masked"].as_str().unwrap();
        assert!(
            !masked.contains("abcdefghijkl"),
            "the served key must not carry the plaintext: {masked}"
        );
        assert_eq!(rows[0]["label"], "backup");
    }

    /// Without the token it is not served at all — the resource routes are
    /// behind the same gate as `/reload`, and this is the test that says so.
    #[tokio::test]
    async fn the_key_pool_needs_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());

        let response = admin_plane_router(state)
            .oneshot(admin_request("GET", "/api/providers/p-ant/keys", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    /// The write endpoints are behind the token too — the resource routes are
    /// not a second, weaker door into the same state.
    #[tokio::test]
    async fn the_key_writes_need_the_token() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());

        for (method, uri) in [
            ("POST", "/api/providers/p-ant/keys"),
            ("DELETE", "/api/keys/1"),
        ] {
            let response = admin_plane_router(state.clone())
                .oneshot(admin_request(method, uri, None))
                .await
                .unwrap();
            assert_eq!(
                response.status(),
                StatusCode::UNAUTHORIZED,
                "{method} {uri}"
            );
        }
    }

    /// Adding the same key twice is one row, and the second answer is the same
    /// row — the daemon's half of `migrate.local.md` §6.1. Asserted through the
    /// endpoint rather than the store, because it is the *endpoint* a client
    /// retries.
    #[tokio::test]
    async fn adding_the_same_key_twice_is_one_row() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let mut ids = Vec::new();
        for _ in 0..2 {
            let body = serde_json::json!({ "api_key": "sk-ant-rotated-1", "label": "backup" });
            // Built here rather than by `admin_request`, which sends no body:
            // a JSON endpoint is asked with a JSON content type, and the client
            // (`sidecar::admin_post_json`) sends exactly this.
            let request = Request::builder()
                .method("POST")
                .uri("/api/providers/p-ant/keys")
                .header(ADMIN_TOKEN_HEADER, &token)
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            let response = admin_plane_router(state.clone())
                .oneshot(request)
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::OK);
            ids.push(body_json(response).await["id"].as_i64().unwrap());
        }
        assert_eq!(
            ids[0], ids[1],
            "a replay answers with the row already there"
        );

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request(
                "GET",
                "/api/providers/p-ant/keys",
                Some(&token),
            ))
            .await
            .unwrap();
        assert_eq!(body_json(response).await.as_array().unwrap().len(), 1);
    }

    /// A refused write answers with the status its **kind** implies, and with
    /// the sentence the `vm::` path produces — both halves matter.
    ///
    /// This is the test the first version of these endpoints could not have
    /// passed honestly: it had no kind, so it re-queried the store to decide
    /// between 400 and 404, which is a second place for the answer to live
    /// (`migrate.local.md` §10.8). The `error` strings are asserted at their
    /// endpoint because they are asserted at the `vm::` end too — the contract
    /// fixtures freeze them, and two assertions that have to agree is what a
    /// move of this kind is checked by.
    #[tokio::test]
    async fn a_refused_key_write_carries_a_status_and_a_kind() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let post = |path: &'static str, body: serde_json::Value| {
            let request = Request::builder()
                .method("POST")
                .uri(path)
                .header(ADMIN_TOKEN_HEADER, token.clone())
                .header(axum::http::header::CONTENT_TYPE, "application/json")
                .body(Body::from(body.to_string()))
                .unwrap();
            admin_plane_router(state.clone()).oneshot(request)
        };

        // Refused on the request's own terms: repeating it fails the same way,
        // so a client should not retry it.
        let response = post(
            "/api/providers/p-ant/keys",
            serde_json::json!({ "api_key": "   " }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST);
        let body = error_json(response).await;
        assert_eq!(body["kind"], "invalid");
        assert_eq!(body["error"], "API key must not be empty");

        // The request was fine; the world was not what it assumed.
        let response = post(
            "/api/providers/no-such-provider/keys",
            serde_json::json!({ "api_key": "sk-x" }),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
        let body = error_json(response).await;
        assert_eq!(body["kind"], "not_found");
        assert_eq!(body["error"], "provider `no-such-provider` not found");
    }

    /// A key that is not there to delete is `deleted: false`, not an error —
    /// the state a retry finds.
    #[tokio::test]
    async fn deleting_a_missing_key_is_not_an_error() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let response = admin_plane_router(state)
            .oneshot(admin_request("DELETE", "/api/keys/9999", Some(&token)))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["deleted"], false);
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

    /// `/events` over a real loopback listener: an SSE body does not end, so the
    /// in-process `oneshot` the rest of this module uses would wait on it
    /// forever. What has to hold is that the stream opens for a caller holding
    /// the token, is refused without one, and delivers a recorded request as an
    /// event.
    #[tokio::test]
    async fn the_event_stream_ticks_and_is_token_guarded() {
        use std::time::Duration;
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let served = state.clone();
        tokio::spawn(async move {
            let _ = axum::serve(listener, admin_plane_router(served)).await;
        });

        // Without the token: refused, and there is no stream behind the refusal.
        let mut refused = tokio::net::TcpStream::connect(addr).await.unwrap();
        refused
            .write_all(b"GET /events HTTP/1.1\r\nHost: localhost\r\n\r\n")
            .await
            .unwrap();
        let mut line = String::new();
        BufReader::new(refused).read_line(&mut line).await.unwrap();
        assert!(
            line.contains("401"),
            "an unauthenticated caller gets no stream: {line}"
        );

        // With it: the response opens as a stream, and one notify comes out of
        // it as one event.
        let mut stream = tokio::net::TcpStream::connect(addr).await.unwrap();
        stream
            .write_all(
                format!(
                    "GET /events HTTP/1.1\r\nHost: localhost\r\n{ADMIN_TOKEN_HEADER}: {token}\r\n\r\n"
                )
                .as_bytes(),
            )
            .await
            .unwrap();
        let mut reader = BufReader::new(stream);
        let mut status = String::new();
        reader.read_line(&mut status).await.unwrap();
        assert!(status.contains("200"), "expected a stream: {status}");
        loop {
            let mut header = String::new();
            reader.read_line(&mut header).await.unwrap();
            if header == "\r\n" {
                break;
            }
        }

        // Bounded, because the failure being guarded against is a stream that
        // never speaks: without the timeout this would hang instead of fail.
        // Reads skip whatever comes first — the keep-alive comments the handler
        // writes through an idle stream among them.
        let event = tokio::time::timeout(Duration::from_secs(5), async {
            state.notify_usage();
            loop {
                let mut line = String::new();
                assert!(
                    reader.read_line(&mut line).await.unwrap() > 0,
                    "stream ended"
                );
                if line.starts_with("data:") {
                    return line;
                }
            }
        })
        .await
        .expect("a recorded request shows up as an event");

        assert_eq!(
            event.trim_end(),
            r#"data: {"kind":"usage"}"#,
            "the usage tick carries its typed payload"
        );
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
        assert_eq!(v["name"], "kiwanod");
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

    /// A binding written through the API is routed **without** anyone calling
    /// `/reload`.
    ///
    /// Before the write moved here this took two steps and two callers: the app
    /// wrote SQLite and then had to remember to ping the gateway
    /// (`after_mutation` → `notify_reload`). Forgetting left a route the gateway
    /// would not use until it restarted — a failure with no symptom until
    /// traffic arrived. The daemon now performs the write and re-reads its own
    /// table, so there is nothing left to forget. This is the assertion that it
    /// does, and it is the reason the app's write commands lost their
    /// `after_mutation` call.
    #[tokio::test]
    async fn a_binding_written_through_the_api_is_routed_without_a_reload() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, false);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        assert!(
            state.route_table().select("claude").is_err(),
            "nothing is bound yet, so nothing routes"
        );

        let body = serde_json::json!({ "provider_id": "p-ant" });
        let request = Request::builder()
            .method("POST")
            .uri("/api/agents/claude/bindings")
            .header(ADMIN_TOKEN_HEADER, &token)
            .header(axum::http::header::CONTENT_TYPE, "application/json")
            .body(Body::from(body.to_string()))
            .unwrap();
        let response = admin_plane_router(state.clone())
            .oneshot(request)
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(body_json(response).await["ok"], true);

        // No `/reload` between the write and this read.
        let table = state.route_table();
        let routed = table
            .select("claude")
            .expect("the write itself re-read the table");
        assert_eq!(routed.id, "p-ant");
    }

    /// The route payload the app's Apps screen renders, served by the daemon —
    /// and the same shape `vm::build_agent_routes` produces, because it is the
    /// same function.
    #[tokio::test]
    async fn the_agent_routes_endpoint_serves_the_bound_routes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());
        let token = shared_token(&state);

        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("GET", "/api/agent-routes", Some(&token)))
            .await
            .unwrap();
        let routes = body_json(response).await;
        assert_eq!(routes.as_array().unwrap().len(), 1);
        assert_eq!(routes[0]["agent"], "claude");
        assert_eq!(routes[0]["strategy"], "single");
        assert_eq!(routes[0]["bindings"][0]["provider_id"], "p-ant");
        assert_eq!(
            routes[0]["bindings"][0]["logo_char"], "P",
            "the avatar letter"
        );

        // Without the token: refused, like every other resource route.
        let response = admin_plane_router(state.clone())
            .oneshot(admin_request("GET", "/api/agent-routes", None))
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
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
        assert!(liveness.contains("kiwanod"), "{liveness}");
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
