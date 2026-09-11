//! Gateway servers: data plane (:8317) + admin plane (:8310), tech.md §4.1.
//!
//! Both servers share one [`GatewayState`]: the SQLite store (SSOT) and the
//! in-memory route table that `/reload` swaps atomically.

pub mod admin;
pub mod data;

pub use admin::admin_plane_router;
pub use data::data_plane_router;

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::error::{GatewayError, Result};
use crate::router::RouteTable;
use crate::store::{LogConfig, Protocol, Store};

/// Max inbound body size forwarded to upstreams (32 MiB is generous for
/// long-context agent payloads).
pub const MAX_BODY_BYTES: usize = 32 * 1024 * 1024;

/// Shared gateway state for both planes.
pub struct GatewayState {
    pub store: Arc<Store>,
    pub http: reqwest::Client,
    pub engine: crate::strategy::StrategyEngine,
    route_table: RwLock<Arc<RouteTable>>,
    /// Per-provider rotating cursor for multi-key round-robin
    /// (spec §4.1 P1 multi-key rotation). In-memory only; a gateway restart
    /// simply restarts each pool at its primary.
    key_cursors: std::sync::Mutex<std::collections::HashMap<String, usize>>,
    /// Request-log capture config, refreshed alongside the route table.
    log_cfg: RwLock<LogConfig>,
    /// Bundled model price table (crates/adapters resources/models.json);
    /// consulted at usage-record time to cost every metered request.
    pub(crate) pricing: RwLock<kiwano_adapters::model_pricing::PricingTable>,
    /// Providers currently over a billing limit. Derived state, recomputed by
    /// `crate::limits::run`; selection reads it, so the gateway enforces limits
    /// whether or not the desktop app is open to.
    limits: RwLock<Arc<crate::limits::LimitState>>,
    pub started_at: Instant,
    pub version: &'static str,
    /// Flipped by the signal handler and by `POST /shutdown`. Owned here so the
    /// admin plane can request a graceful stop — the app needs that to replace
    /// a gateway left over from another version, and a daemon it adopted is not
    /// its child, so there is no handle to kill.
    shutdown: tokio::sync::watch::Sender<bool>,
}

/// The price table to serve from: the GUI-seeded `model_pricing` mirror when it
/// has rows (so a Hub price refresh reaches cost recording), else the snapshot
/// compiled into the binary. An empty mirror must never blank out pricing —
/// it only means the seeder has not run yet.
fn resolve_pricing(store: &Store) -> kiwano_adapters::model_pricing::PricingTable {
    match store.load_model_pricing() {
        Ok(rows) if !rows.is_empty() => {
            kiwano_adapters::model_pricing::PricingTable::from_entries(rows)
        }
        _ => kiwano_adapters::model_pricing::PricingTable::bundled(),
    }
}

impl GatewayState {
    pub fn new(store: Store) -> Result<GatewayState> {
        let route_table = Arc::new(RouteTable::load(&store)?);
        let log_config = store.load_log_config().unwrap_or_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(300))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;
        // Resolve before `store` moves into the Arc below. The limits are
        // seeded here so the very first request already respects them, rather
        // than being served in the window before the task's first tick.
        let pricing = resolve_pricing(&store);
        let limits = crate::limits::evaluate(&store);
        let (shutdown, _) = tokio::sync::watch::channel(false);
        Ok(GatewayState {
            store: Arc::new(store),
            http,
            engine: crate::strategy::StrategyEngine::new(),
            route_table: RwLock::new(route_table),
            key_cursors: std::sync::Mutex::new(std::collections::HashMap::new()),
            log_cfg: RwLock::new(log_config),
            pricing: RwLock::new(pricing),
            limits: RwLock::new(Arc::new(limits)),
            started_at: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
            shutdown,
        })
    }

    /// A receiver the serve loops await; resolves once a stop is requested.
    /// `subscribe` rather than a stored receiver so `main` needs no plumbing.
    pub fn shutdown_rx(&self) -> tokio::sync::watch::Receiver<bool> {
        self.shutdown.subscribe()
    }

    /// Ask both planes to stop. Idempotent — a second request is still fine,
    /// since the serve loops may not have dropped their receivers yet.
    pub fn request_shutdown(&self) {
        let _ = self.shutdown.send(true);
    }

    /// Current price-table snapshot (cheap clone; bundled data is static but
    /// a future backend-fed table swaps here on `/reload`).
    pub fn pricing(&self) -> kiwano_adapters::model_pricing::PricingTable {
        self.pricing.read().expect("pricing lock poisoned").clone()
    }

    /// The providers over a limit right now (cheap `Arc` clone).
    pub fn limits(&self) -> Arc<crate::limits::LimitState> {
        self.limits.read().expect("limits lock poisoned").clone()
    }

    /// Publish a fresh evaluation, returning it. Only `crate::limits::run`
    /// writes here.
    pub fn set_limits(&self, next: crate::limits::LimitState) -> Arc<crate::limits::LimitState> {
        let next = Arc::new(next);
        *self.limits.write().expect("limits lock poisoned") = next.clone();
        next
    }

    /// Current route table snapshot (cheap `Arc` clone).
    pub fn route_table(&self) -> Arc<RouteTable> {
        self.route_table
            .read()
            .expect("route table lock poisoned")
            .clone()
    }

    /// Current request-log capture config (cheap clone).
    pub fn log_config(&self) -> LogConfig {
        self.log_cfg
            .read()
            .expect("log config lock poisoned")
            .clone()
    }

    /// Next index into a provider's key pool (round-robin, spec §4.1 P1).
    /// Pools of size <= 1 always answer 0 and never touch the map.
    pub fn next_key_index(&self, provider_id: &str, pool_len: usize) -> usize {
        if pool_len <= 1 {
            return 0;
        }
        let mut cursors = self.key_cursors.lock().expect("key cursors poisoned");
        let cursor = cursors.entry(provider_id.to_string()).or_default();
        let idx = *cursor % pool_len;
        *cursor = cursor.wrapping_add(1);
        idx
    }

    /// Rebuild the route table from SQLite; returns the number of agents
    /// routed (used by admin `/reload`, tech.md §4.3 flow 2). The request-log
    /// config is refreshed too (best-effort — never blocks a reload).
    pub fn reload_routes(&self) -> Result<usize> {
        let table = Arc::new(RouteTable::load(&self.store)?);
        let agents = table.routes.len();
        *self.route_table.write().expect("route table lock poisoned") = table;
        if let Ok(cfg) = self.store.load_log_config() {
            *self.log_cfg.write().expect("log config lock poisoned") = cfg;
        }
        // Rebuild prices too: the GUI re-seeds the mirror after a Hub refresh
        // and then reloads, which is how a price update takes effect.
        *self.pricing.write().expect("pricing lock poisoned") = resolve_pricing(&self.store);
        Ok(agents)
    }
}

/// Extract the local placeholder key from inbound auth headers
/// (tech.md §4.6: `x-api-key` / `Authorization: Bearer` / `x-goog-api-key`).
pub fn extract_placeholder_key(headers: &HeaderMap) -> Option<String> {
    if let Some(v) = headers.get("x-api-key").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    if let Some(v) = headers.get(AUTHORIZATION).and_then(|v| v.to_str().ok()) {
        if let Some(key) = v.strip_prefix("Bearer ") {
            let key = key.trim();
            if !key.is_empty() {
                return Some(key.to_string());
            }
        }
    }
    if let Some(v) = headers.get("x-goog-api-key").and_then(|v| v.to_str().ok()) {
        let v = v.trim();
        if !v.is_empty() {
            return Some(v.to_string());
        }
    }
    None
}

/// Build a protocol-flavored JSON error response: native shape per inbound
/// protocol family (Anthropic / OpenAI / Gemini).
pub fn error_response(
    inbound: Option<Protocol>,
    status: StatusCode,
    kind: &str,
    message: &str,
) -> Response {
    let body = match inbound {
        Some(Protocol::OpenAI) => json!({
            "error": {
                "message": message,
                "type": kind,
                "code": status.as_u16(),
            }
        }),
        Some(Protocol::Gemini) => json!({
            "error": {
                "code": status.as_u16(),
                "message": message,
                "status": kind,
            }
        }),
        _ => json!({
            "type": "error",
            "error": { "type": kind, "message": message }
        }),
    };
    (status, axum::Json(body)).into_response()
}

/// Map a [`GatewayError`] onto a client-facing response.
pub fn error_into_response(err: GatewayError, inbound: Option<Protocol>) -> Response {
    let (status, kind) = match &err {
        GatewayError::NoBinding(_) => (StatusCode::SERVICE_UNAVAILABLE, "no_provider_bound"),
        // 429 rather than 503: the provider exists and works, the caller has
        // simply spent what it was allowed to for this period.
        GatewayError::AllOverLimit { .. } => (StatusCode::TOO_MANY_REQUESTS, "provider_over_limit"),
        GatewayError::ProviderNotFound(_) => (StatusCode::SERVICE_UNAVAILABLE, "provider_missing"),
        GatewayError::UnsupportedPath(_) => (StatusCode::NOT_FOUND, "unsupported_path"),
        GatewayError::Upstream(_) | GatewayError::Http(_) => {
            (StatusCode::BAD_GATEWAY, "upstream_error")
        }
        GatewayError::Store(_) | GatewayError::Internal(_) | GatewayError::Json(_) => {
            (StatusCode::INTERNAL_SERVER_ERROR, "internal_error")
        }
        GatewayError::Io(_) => (StatusCode::INTERNAL_SERVER_ERROR, "io_error"),
    };
    tracing::error!(error = %err, status = %status, "request failed");
    error_response(inbound, status, kind, &err.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::http::HeaderValue;

    #[test]
    fn extracts_key_from_supported_headers() {
        let mut headers = HeaderMap::new();
        assert_eq!(extract_placeholder_key(&headers), None);

        headers.insert("x-api-key", HeaderValue::from_static("kw-ag-claude-1"));
        assert_eq!(
            extract_placeholder_key(&headers).as_deref(),
            Some("kw-ag-claude-1")
        );

        let mut headers = HeaderMap::new();
        headers.insert(
            AUTHORIZATION,
            HeaderValue::from_static("Bearer kw-ag-codex-2"),
        );
        assert_eq!(
            extract_placeholder_key(&headers).as_deref(),
            Some("kw-ag-codex-2")
        );

        let mut headers = HeaderMap::new();
        headers.insert("x-goog-api-key", HeaderValue::from_static("kw-ag-gemini-3"));
        assert_eq!(
            extract_placeholder_key(&headers).as_deref(),
            Some("kw-ag-gemini-3")
        );

        // Bearer without a token or blank values are ignored.
        let mut headers = HeaderMap::new();
        headers.insert(AUTHORIZATION, HeaderValue::from_static("Bearer "));
        assert_eq!(extract_placeholder_key(&headers), None);
    }

    // ── price table resolution ───────────────────────────────────────────

    fn entry(id: &str) -> kiwano_adapters::model_pricing::ModelPriceEntry {
        kiwano_adapters::model_pricing::ModelPriceEntry {
            model_id: id.into(),
            display_name: id.into(),
            input: "1".into(),
            output: "2".into(),
            cache_read: "0".into(),
            cache_creation: "0".into(),
            currency: "USD".into(),
        }
    }

    /// A model the bundled table is known to price (see the adapters tests).
    const BUNDLED_MODEL: &str = "claude-opus-4-8";

    /// An unseeded mirror must not blank out pricing — it only means the GUI
    /// seeder has not run.
    #[test]
    fn resolve_pricing_falls_back_to_bundled_when_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(resolve_pricing(&store).find(BUNDLED_MODEL).is_some());
    }

    /// A seeded mirror is served, and it is the *whole* table: the seeder
    /// always writes a complete document, so nothing is merged in behind it.
    /// This is the guard against the mirror going unread again.
    #[test]
    fn resolve_pricing_prefers_the_mirror() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_model_pricing(&entry("kw-test-model")).unwrap();
        let table = resolve_pricing(&store);
        assert!(
            table.find("kw-test-model").is_some(),
            "mirror row is served"
        );
        assert!(
            table.find(BUNDLED_MODEL).is_none(),
            "mirror replaces bundled"
        );
    }
}
