//! Gateway servers: data plane (loopback :8317) + admin plane (a unix socket or
//! a named pipe, see [`admin_ipc`]), tech.md §4.1.
//!
//! Both servers share one [`GatewayState`]: the SQLite store (SSOT) and the
//! in-memory route table that `/reload` swaps atomically.
//!
//! Only the data plane is a TCP port, and deliberately so: agents are
//! configured with a URL. The admin plane is a local IPC endpoint, which is
//! what its two clients (the app and the CLI) can use and no one else can.

pub mod admin;
pub mod admin_ipc;
pub mod data;

pub use admin::{admin_plane_router, ensure_admin_token, ADMIN_TOKEN_HEADER, ADMIN_TOKEN_KEY};
pub use admin_ipc::{AdminEndpoint, AdminListener, AdminStream, ADMIN_SOCKET_ENV};
pub use data::data_plane_router;

use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use axum::http::header::AUTHORIZATION;
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;

use crate::error::{GatewayError, Result};
use crate::router::RouteTable;
use crate::store::{
    CompatShimConfig, DlpConfig, DlpMode, LogConfig, Protocol, Store, StreamTimeouts,
};

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
    /// Compat shim on/off, same lifecycle as `log_cfg`: read at startup,
    /// re-read on `/reload`.
    shim_cfg: RwLock<CompatShimConfig>,
    /// The credentials this install holds, for scrubbing captured bodies by
    /// value. Refreshed with the rest of the cached configuration, so a key
    /// added in the UI applies to the next request rather than the next
    /// restart — and so no request pays for a store read to be scrubbed.
    redactor: RwLock<Arc<crate::log_capture::Redactor>>,
    /// Streaming timeouts, same contract as `log_cfg`: read at startup, re-read
    /// on `/reload` so a change lands without restarting the daemon.
    stream_cfg: RwLock<StreamTimeouts>,
    /// Outbound credential detection, same contract as `log_cfg`: read at
    /// startup, re-read on `/reload`. Governance only — `Alert` adds a note to
    /// the request log and never touches the request itself.
    dlp_cfg: RwLock<DlpConfig>,
    /// Model price table, built from the GUI-seeded `model_pricing` mirror and
    /// consulted at usage-record time to cost every metered request.
    pub(crate) pricing: RwLock<kiwano_adapters::model_pricing::PricingTable>,
    /// The prices users declared for their own providers, keyed by the local
    /// provider row id rather than by catalog entry. Asked **first** at
    /// usage-record time: these are the user's own statement about what a
    /// provider charges, and for a provider the Hub publishes nothing for they
    /// are the only statement there is.
    ///
    /// A table of its own rather than more rows in `pricing`, so the two key
    /// namespaces cannot shadow each other by string equality.
    pub(crate) declared: RwLock<kiwano_adapters::model_pricing::PricingTable>,
    /// Providers currently over a billing limit. Derived state, recomputed by
    /// `crate::limits::run`; selection reads it, so the gateway enforces limits
    /// whether or not the desktop app is open to.
    limits: RwLock<Arc<crate::limits::LimitState>>,
    pub started_at: Instant,
    pub version: &'static str,
    /// Optional bearer token guarding `/metrics` (KIW-PRIV-001). Set by the
    /// daemon from `KIWANO_METRICS_TOKEN`. Absent, `/metrics` stays open to any
    /// loopback caller but serves per-agent labels redacted; configured, only a
    /// caller holding the token sees the full exposition (see `crate::metrics`).
    metrics_token: Option<Arc<str>>,
    /// Flipped by the signal handler and by `POST /shutdown`. Owned here so the
    /// admin plane can request a graceful stop — the app needs that to replace
    /// a gateway left over from another version, and a daemon it adopted is not
    /// its child, so there is no handle to kill.
    shutdown: tokio::sync::watch::Sender<bool>,
    /// Ticks saying a request has just been metered, which is what the admin
    /// plane's `/events` streams to the app. The numbers themselves are not in
    /// here on purpose: they have exactly one source (the store read the app
    /// makes after a tick), and a copy in the event would be a second truth.
    ///
    /// A broadcast rather than a watch: one of these is a moment, not a state,
    /// and every subscriber has to see every one of them.
    usage_ticks: tokio::sync::broadcast::Sender<()>,
}

/// How many ticks a subscriber may fall behind before it is told it lagged
/// rather than given them one by one. The consumer re-reads a screen per tick,
/// so what it does with a backlog is the same thing either way; the buffer only
/// has to be big enough that the ordinary case never sees a gap.
const USAGE_TICK_BACKLOG: usize = 16;

/// The price table to serve from: the GUI-seeded `model_pricing` mirror, which
/// is how a Hub price refresh reaches cost recording.
///
/// There is no compiled snapshot behind it any more, so an empty mirror is
/// simply an empty table: the seeder has not run yet and requests are recorded
/// unpriced until it does. A read failure is logged rather than swallowed — it
/// now serves the same empty table as "not seeded yet", and a silent dash on
/// every cost is the hardest version of this to diagnose.
fn resolve_pricing(store: &Store) -> kiwano_adapters::model_pricing::PricingTable {
    match store.load_model_pricing() {
        Ok(rows) => kiwano_adapters::model_pricing::PricingTable::from_entries(rows),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the price mirror; serving no prices");
            Default::default()
        }
    }
}

/// The declared prices to serve from: every provider row that carries them,
/// keyed by the row's own id (`Store::load_declared_prices`).
///
/// Same posture as `resolve_pricing` beside it — a read failure is logged and
/// serves an empty table rather than refusing to start. Losing declared prices
/// costs those providers their own rates (their requests fall back to the Hub's
/// table); refusing to start would cost every agent its route.
fn resolve_declared_pricing(store: &Store) -> kiwano_adapters::model_pricing::PricingTable {
    match store.load_declared_prices() {
        Ok(rows) => kiwano_adapters::model_pricing::PricingTable::from_entries(rows),
        Err(e) => {
            tracing::warn!(error = %e, "could not read the declared prices; serving none");
            Default::default()
        }
    }
}

impl GatewayState {
    pub fn new(store: Store) -> Result<GatewayState> {
        let route_table = Arc::new(RouteTable::load(&store)?);
        let log_config = store.load_log_config().unwrap_or_default();
        let shim_cfg = store.load_compat_shim_config().unwrap_or_default();
        let stream_cfg = store.load_stream_timeouts().unwrap_or_default();
        let dlp_cfg = store.load_dlp_config().unwrap_or_default();
        let http = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(300))
            .pool_idle_timeout(Duration::from_secs(90))
            .build()?;
        // Resolve before `store` moves into the Arc below. The limits are
        // seeded here so the very first request already respects them, rather
        // than being served in the window before the task's first tick.
        let redactor = crate::log_capture::Redactor::from_store(&store);
        let pricing = resolve_pricing(&store);
        let declared = resolve_declared_pricing(&store);
        let limits = crate::limits::evaluate(&store);
        // The admin plane's token, minted before either listener can bind so
        // the row the GUI reads to authenticate is on disk before the port
        // answers. A store that cannot hold it is a gateway that cannot
        // enforce anything — the same class of failure as the route table.
        admin::ensure_admin_token(&store)?;
        let (shutdown, _) = tokio::sync::watch::channel(false);
        let (usage_ticks, _) = tokio::sync::broadcast::channel(USAGE_TICK_BACKLOG);
        Ok(GatewayState {
            store: Arc::new(store),
            http,
            engine: crate::strategy::StrategyEngine::new(),
            route_table: RwLock::new(route_table),
            key_cursors: std::sync::Mutex::new(std::collections::HashMap::new()),
            log_cfg: RwLock::new(log_config),
            shim_cfg: RwLock::new(shim_cfg),
            redactor: RwLock::new(Arc::new(redactor)),
            stream_cfg: RwLock::new(stream_cfg),
            dlp_cfg: RwLock::new(dlp_cfg),
            pricing: RwLock::new(pricing),
            declared: RwLock::new(declared),
            limits: RwLock::new(Arc::new(limits)),
            started_at: Instant::now(),
            version: env!("CARGO_PKG_VERSION"),
            metrics_token: None,
            shutdown,
            usage_ticks,
        })
    }

    /// Opt `/metrics` into bearer-token auth (KIW-PRIV-001). Called from the
    /// daemon's startup env `KIWANO_METRICS_TOKEN`; empty or unset keeps the
    /// open-and-redacted default, so an existing Prometheus scrape keeps
    /// working without reconfiguration.
    pub fn with_metrics_token(mut self, token: Option<String>) -> Self {
        self.metrics_token = token.filter(|t| !t.is_empty()).map(Arc::from);
        self
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

    /// Tell `/events` subscribers that a request has just been metered.
    ///
    /// Best-effort by construction: with nobody listening the send is a no-op,
    /// and a subscriber that fell behind is told by its own `Lagged`, which the
    /// stream turns into the one tick it would have carried anyway.
    pub fn notify_usage(&self) {
        let _ = self.usage_ticks.send(());
    }

    /// A receiver for those ticks — one per `/events` connection.
    pub fn usage_ticks(&self) -> tokio::sync::broadcast::Receiver<()> {
        self.usage_ticks.subscribe()
    }

    /// Current price-table snapshot (cheap clone; rebuilt from the mirror on
    /// `/reload`).
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

    /// Whether the compat shim sanitizes passthrough bodies (Copy, so free).
    pub fn compat_shim_enabled(&self) -> bool {
        self.shim_cfg
            .read()
            .expect("shim config lock poisoned")
            .enabled
    }

    /// The credential scrubber for captured bodies (cheap clone of an Arc).
    pub fn redactor(&self) -> Arc<crate::log_capture::Redactor> {
        self.redactor
            .read()
            .expect("redactor lock poisoned")
            .clone()
    }

    /// Current streaming timeouts (Copy, so free).
    pub fn stream_timeouts(&self) -> StreamTimeouts {
        *self.stream_cfg.read().expect("stream config lock poisoned")
    }

    /// What the credential detector does with a finding (Copy, so free).
    pub fn dlp_mode(&self) -> DlpMode {
        self.dlp_cfg.read().expect("dlp config lock poisoned").mode
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
        if let Ok(cfg) = self.store.load_compat_shim_config() {
            *self.shim_cfg.write().expect("shim config lock poisoned") = cfg;
        }
        // Rebuilt rather than patched, and on every reload: a provider whose key
        // the user just replaced must not be scrubbed by the old value only.
        // Bodies captured earlier keep their [REDACTED] — that is what scrubbing
        // them at capture time is for.
        *self.redactor.write().expect("redactor lock poisoned") =
            Arc::new(crate::log_capture::Redactor::from_store(&self.store));
        if let Ok(cfg) = self.store.load_stream_timeouts() {
            *self
                .stream_cfg
                .write()
                .expect("stream config lock poisoned") = cfg;
        }
        if let Ok(cfg) = self.store.load_dlp_config() {
            *self.dlp_cfg.write().expect("dlp config lock poisoned") = cfg;
        }
        // Rebuild prices too: the GUI re-seeds the mirror after a Hub refresh
        // and then reloads, which is how a price update takes effect — and this
        // is the same call the app makes after saving a provider, which is how
        // prices the user just declared start costing requests.
        *self.pricing.write().expect("pricing lock poisoned") = resolve_pricing(&self.store);
        *self
            .declared
            .write()
            .expect("declared pricing lock poisoned") = resolve_declared_pricing(&self.store);
        // And re-evaluate the ceilings: this is the call the app makes after an
        // edit, and a limit the user just typed should bind the next request
        // rather than wait up to a tick (30s) for the patrol.
        crate::limits::publish(self, crate::limits::evaluate(&self.store));
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
    // Gemini CLI's auth header for the native Gemini API.
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
        // Gemini's error envelope: code/message/status, where `status` is the
        // family name the API uses (e.g. `INVALID_ARGUMENT`) — here the
        // gateway's own kind, since it is the one refusing the request.
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
        // The data plane's inbound auth: no placeholder key, or not one of
        // ours. 401 so an agent client reports an auth problem instead of
        // retrying a request the gateway will never forward.
        GatewayError::Unauthorized(_) => (StatusCode::UNAUTHORIZED, "unauthorized"),
        GatewayError::NoBinding(_) => (StatusCode::SERVICE_UNAVAILABLE, "no_provider_bound"),
        // 429 rather than 503: the provider exists and works, the caller has
        // simply spent what it was allowed to for this period.
        GatewayError::AllOverLimit { .. } => (StatusCode::TOO_MANY_REQUESTS, "provider_over_limit"),
        // Same status as a provider ceiling and its own kind: "this agent has
        // spent what it was allowed" is a different story to tell a reader.
        GatewayError::AgentOverLimit { .. } => (StatusCode::TOO_MANY_REQUESTS, "agent_over_limit"),
        // 503, not 502: nothing upstream went wrong, the provider is simply not
        // being asked right now (open, or a recovery probe is already in
        // flight). It clears itself once the breaker's timeout elapses.
        GatewayError::CircuitOpen { .. } => (StatusCode::SERVICE_UNAVAILABLE, "circuit_open"),
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
    // A rejected caller is not a gateway fault, and a misconfigured agent can
    // retry this endpoint every few seconds — logging each refusal as an error
    // would bury the failures that are ours.
    if status.is_client_error() {
        tracing::warn!(error = %err, status = %status, "request refused");
    } else {
        tracing::error!(error = %err, status = %status, "request failed");
    }
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

        // Gemini CLI carries the placeholder key here, speaking the native
        // Gemini API.
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
            long_context: None,
            provider_id: String::new(),
            off_peak: None,
            peak_hours: None,
            model_id: id.into(),
            display_name: id.into(),
            input: "1".into(),
            output: "2".into(),
            cache_read: "0".into(),
            cache_creation: "0".into(),
            currency: "USD".into(),
        }
    }

    /// An unseeded mirror serves no prices. There is no compiled snapshot
    /// behind it any more, so this is also what a fresh install looks like:
    /// every request is recorded unpriced until the first sync.
    #[test]
    fn resolve_pricing_of_an_unseeded_mirror_is_empty() {
        let store = Store::open_in_memory().unwrap();
        assert!(resolve_pricing(&store)
            .find("", "claude-opus-4-8")
            .is_none());
    }

    /// A seeded mirror is served, and it is the *whole* table: the seeder
    /// always writes a complete document, so nothing is merged in behind it.
    /// This is the guard against the mirror going unread again.
    #[test]
    fn resolve_pricing_serves_the_mirror() {
        let store = Store::open_in_memory().unwrap();
        store.upsert_model_pricing(&entry("kw-test-model")).unwrap();
        let table = resolve_pricing(&store);
        assert!(
            table.find("", "kw-test-model").is_some(),
            "mirror row is served"
        );
        assert!(
            table.find("", "claude-opus-4-8").is_none(),
            "and nothing else is"
        );
    }
}
