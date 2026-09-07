//! Admin plane (:8310, loopback only): status chip + route hot reload
//! (tech.md §4.6 control channel; §4.3 flow 2).
//!
//! The Tauri app calls `POST /reload` after writing new bindings to SQLite;
//! the next request routed by the gateway hits the new provider. `GET
//! /status` powers the first-screen gateway state chip and sidecar re-connect.

use std::sync::Arc;

use axum::extract::State;
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::{json, Value};

use crate::server::GatewayState;

pub fn admin_plane_router(state: Arc<GatewayState>) -> Router {
    Router::new()
        .route("/status", get(status))
        .route("/reload", post(reload))
        .with_state(state)
}

async fn status(State(state): State<Arc<GatewayState>>) -> Response {
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
    }))
    .into_response()
}

async fn reload(State(state): State<Arc<GatewayState>>) -> Response {
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
    use tower::ServiceExt; // oneshot

    fn seed(store: &Store, with_binding: bool) {
        let p = Provider {
            id: "p-ant".into(),
            name: "Anthropic".into(),
            protocol: Protocol::Anthropic,
            base_url: "https://api.anthropic.com".into(),
            api_path: None,
            api_key: Some("sk-ant".into()),
            billing: Billing::Metered,
            period_limit: None,
            reset_period: None,
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        };
        store.insert_provider(&p).unwrap();
        if with_binding {
            store
                .upsert_binding(&Binding {
                    agent: "claude".into(),
                    provider_id: "p-ant".into(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .unwrap();
        }
    }

    async fn body_json(response: Response) -> Value {
        assert_eq!(response.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .unwrap();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[tokio::test]
    async fn status_reports_counts_and_routes() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, true);
        let state = Arc::new(GatewayState::new(store).unwrap());

        let response = admin_plane_router(state.clone())
            .oneshot(
                Request::builder()
                    .uri("/status")
                    .body(Body::empty())
                    .unwrap(),
            )
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

    #[tokio::test]
    async fn reload_picks_up_new_bindings() {
        let dir = tempfile::tempdir().unwrap();
        let store = Store::open(dir.path().join("t.db")).unwrap();
        seed(&store, false);
        let state = Arc::new(GatewayState::new(store).unwrap());

        // Before reload: no agents routed, requests would fail.
        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(
                    Request::builder()
                        .uri("/status")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap(),
        )
        .await;
        assert_eq!(v["agents_routed"], 0);

        // App writes the binding to SQLite then hot-reloads the gateway.
        state
            .store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p-ant".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        let v = body_json(
            admin_plane_router(state.clone())
                .oneshot(
                    Request::builder()
                        .method("POST")
                        .uri("/reload")
                        .body(Body::empty())
                        .unwrap(),
                )
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

    #[tokio::test]
    async fn data_plane_serves_after_reload() {
        // End-to-end wiring sanity inside one process: unknown path 404s with
        // the gateway error shape; /v1/messages with no binding 503s.
        let dir = tempfile::tempdir().unwrap();
        let state =
            Arc::new(GatewayState::new(Store::open(dir.path().join("t.db")).unwrap()).unwrap());

        let response = data_plane_router(state.clone())
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/v1/messages")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);

        let response = data_plane_router(state.clone())
            .oneshot(Request::builder().uri("/nope").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }
}
