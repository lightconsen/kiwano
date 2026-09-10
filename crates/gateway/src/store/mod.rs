//! SQLite persistence for the Kiwano gateway (tech.md §2.3 / §4.7).
//!
//! SQLite is the single source of truth (tech.md §4.2): the Tauri app writes
//! provider/binding configuration, the gateway reads it and writes usage.
//! The database file is opened in WAL mode so both processes can share it;
//! `busy_timeout` guards against transient cross-process lock contention.

use std::path::{Path, PathBuf};
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use kiwano_adapters::model_pricing::ModelPriceEntry;

/// Current schema version tracked via `PRAGMA user_version`.
pub const SCHEMA_VERSION: i32 = 10;

const MIGRATION_V1: &str = r#"
CREATE TABLE IF NOT EXISTS providers (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS agent_strategies (
    agent  TEXT PRIMARY KEY,
    type   TEXT NOT NULL DEFAULT 'single'
           CHECK (type IN ('single','failover','roundrobin','timewindow','quota')),
    config TEXT
);

CREATE TABLE IF NOT EXISTS agent_bindings (
    agent       TEXT NOT NULL,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    priority    INTEGER NOT NULL DEFAULT 0,
    weight      INTEGER NOT NULL DEFAULT 1,
    win_start   TEXT,
    win_end     TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    PRIMARY KEY (agent, provider_id)
);

CREATE TABLE IF NOT EXISTS placeholder_keys (
    key        TEXT PRIMARY KEY,
    agent      TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS usage (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    ts                    TEXT NOT NULL,
    agent                 TEXT NOT NULL,
    provider_id           TEXT NOT NULL,
    model                 TEXT,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms            INTEGER,
    status                TEXT NOT NULL DEFAULT 'ok' CHECK (status IN ('ok','error'))
);

CREATE INDEX IF NOT EXISTS idx_usage_ts            ON usage(ts);
CREATE INDEX IF NOT EXISTS idx_usage_provider_ts   ON usage(provider_id, ts);
CREATE INDEX IF NOT EXISTS idx_usage_agent_ts      ON usage(agent, ts);

CREATE TABLE IF NOT EXISTS provider_health (
    provider_id          TEXT PRIMARY KEY REFERENCES providers(id) ON DELETE CASCADE,
    status               TEXT NOT NULL DEFAULT 'unknown'
                         CHECK (status IN ('healthy','degraded','down','unknown')),
    last_latency_ms      INTEGER,
    last_check_at        TEXT,
    consecutive_failures INTEGER NOT NULL DEFAULT 0
);
"#;

/// v2: unit for the user-entered period cap (spending alerts / ring percentage, tech.md §2.4 A).
/// NULL rows predate the column and are read as 'requests'.
const MIGRATION_V2: &str = r#"
ALTER TABLE providers ADD COLUMN limit_unit TEXT
    CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens','cny'));
"#;

/// v3: extra API keys per provider (spec §4.1 P1 multi-key rotation).
/// providers.api_key stays the primary key of the pool; these rotate after it.
const MIGRATION_V3: &str = r#"
CREATE TABLE IF NOT EXISTS api_keys (
    id          INTEGER PRIMARY KEY AUTOINCREMENT,
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    api_key     TEXT NOT NULL,
    label       TEXT,
    enabled     INTEGER NOT NULL DEFAULT 1,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_api_keys_provider ON api_keys(provider_id);
"#;

/// v4: Gemini protocol flavor (P1 Gemini CLI takeover). CHECK constraints can't be
/// altered in place, so `providers` is rebuilt with the widened protocol set;
/// bindings/keys survive via named-column copy (FKs off during the swap).
const MIGRATION_V4: &str = r#"
PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    limit_unit   TEXT
                 CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens','cny')),
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL
);
INSERT INTO providers_new (id, name, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, reset_period,
                           enabled, created_at, updated_at)
    SELECT id, name, protocol, base_url, api_path, api_key,
           billing, period_limit, limit_unit, reset_period,
           enabled, created_at, updated_at
    FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
PRAGMA foreign_keys=ON;
"#;

/// v5: complete request logging (bodies + metadata) — this is a local tool, so
/// full captures are kept. Metadata lives in `request_logs`; the (possibly
/// large) bodies live in a sibling table so list queries never drag blobs.
/// `gateway_settings` is the gateway-visible KV store (the GUI's `app_settings`
/// lives in the same file but is GUI-owned; the gateway only reads its own).
const MIGRATION_V5: &str = r#"
CREATE TABLE IF NOT EXISTS request_logs (
    id                    INTEGER PRIMARY KEY AUTOINCREMENT,
    ts                    TEXT NOT NULL,
    method                TEXT NOT NULL,
    path                  TEXT NOT NULL,
    query                 TEXT,
    agent                 TEXT,
    attribution           TEXT,
    provider_id           TEXT,
    model                 TEXT,
    status_code           INTEGER NOT NULL,
    error_kind            TEXT,
    error_message         TEXT,
    session_id            TEXT,
    is_streaming          INTEGER NOT NULL DEFAULT 0,
    input_tokens          INTEGER NOT NULL DEFAULT 0,
    output_tokens         INTEGER NOT NULL DEFAULT 0,
    cache_read_tokens     INTEGER NOT NULL DEFAULT 0,
    cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
    latency_ms            INTEGER,
    first_token_ms        INTEGER,
    request_headers       TEXT,
    response_headers      TEXT,
    request_size          INTEGER NOT NULL DEFAULT 0,
    response_size         INTEGER NOT NULL DEFAULT 0,
    truncated             INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_request_logs_ts          ON request_logs(ts);
CREATE INDEX IF NOT EXISTS idx_request_logs_agent_ts    ON request_logs(agent, ts);
CREATE INDEX IF NOT EXISTS idx_request_logs_provider_ts ON request_logs(provider_id, ts);

CREATE TABLE IF NOT EXISTS request_bodies (
    log_id        INTEGER PRIMARY KEY REFERENCES request_logs(id) ON DELETE CASCADE,
    request_body  TEXT,
    response_body TEXT
);

CREATE TABLE IF NOT EXISTS gateway_settings (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);
"#;

/// v7: per-protocol endpoints on one provider — a single provider row can now
/// serve multiple inbound protocols, each with its own upstream URL (e.g. a
/// vendor exposing both an OpenAI-compatible and a native Anthropic endpoint).
/// The provider's primary protocol/endpoint stay in `providers`; this table
/// holds only the ADDITIONAL endpoints (one row per protocol).
const MIGRATION_V7: &str = r#"
CREATE TABLE IF NOT EXISTS provider_endpoints (
    provider_id TEXT NOT NULL REFERENCES providers(id) ON DELETE CASCADE,
    protocol    TEXT NOT NULL CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url    TEXT NOT NULL,
    api_path    TEXT,
    PRIMARY KEY (provider_id, protocol)
);
"#;

/// v8: per-provider advanced forwarding settings — NULL everywhere means the
/// gateway defaults apply. `timeout_secs` bounds the wait for upstream
/// response headers (never aborts an in-flight stream body); `retries` counts
/// same-provider re-attempts before the strategy layer moves on; `headers` is
/// a JSON object of custom request headers merged after credential injection.
const MIGRATION_V8: &str = r#"
ALTER TABLE providers ADD COLUMN timeout_secs INTEGER;
ALTER TABLE providers ADD COLUMN retries INTEGER;
ALTER TABLE providers ADD COLUMN headers TEXT;
"#;

/// v9: model pricing (port of cc-switch's price layer).
/// - `model_pricing` is the price table itself (TEXT decimals, currency per
///   row). The GUI seeds it from the bundled snapshot and refreshes it from the
///   Hub; the gateway builds its in-memory table from these rows at startup and
///   on `/reload` (`server::resolve_pricing`), falling back to the bundled
///   snapshot when the table is still empty. A price update therefore affects
///   costs recorded *after* the reload — history is not rewritten.
/// - `usage`/`request_logs` gain `cost` + `cost_currency` (cost is stored in
///   the price entry's currency; NULL for unpriced models).
/// - `providers.plan_query` holds the token-plan quota query template +
///   credentials as JSON (`{"template":"kimi","fields":{...}}`).
/// - `limit_unit` widens from ('requests','wan_tokens','cny') to those two
///   units plus any 3-letter uppercase currency code. CHECK constraints can't
///   be altered in place, so `providers` is rebuilt (v4 pattern); legacy
///   lowercase units are preserved verbatim, bare currency codes uppercased.
const MIGRATION_V9: &str = r#"
CREATE TABLE IF NOT EXISTS model_pricing (
    model_id        TEXT PRIMARY KEY,
    display_name    TEXT NOT NULL,
    input           TEXT NOT NULL,
    output          TEXT NOT NULL,
    cache_read      TEXT NOT NULL DEFAULT '0',
    cache_creation  TEXT NOT NULL DEFAULT '0',
    currency        TEXT NOT NULL DEFAULT 'USD',
    source          TEXT
);

ALTER TABLE usage ADD COLUMN cost REAL;
ALTER TABLE usage ADD COLUMN cost_currency TEXT;

ALTER TABLE request_logs ADD COLUMN cost REAL;
ALTER TABLE request_logs ADD COLUMN cost_currency TEXT;

PRAGMA foreign_keys=OFF;
CREATE TABLE providers_new (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    protocol     TEXT NOT NULL DEFAULT 'anthropic'
                 CHECK (protocol IN ('anthropic','openai','gemini')),
    base_url     TEXT NOT NULL,
    api_path     TEXT,
    api_key      TEXT,
    billing      TEXT NOT NULL DEFAULT 'metered'
                 CHECK (billing IN ('subscription','metered','unlimited')),
    period_limit REAL,
    limit_unit   TEXT
                 CHECK (limit_unit IS NULL OR limit_unit IN ('requests','wan_tokens')
                        OR (length(limit_unit) = 3 AND upper(limit_unit) = limit_unit)),
    plan_query   TEXT,
    reset_period TEXT
                 CHECK (reset_period IS NULL OR reset_period IN ('monthly','weekly','yearly')),
    enabled      INTEGER NOT NULL DEFAULT 1,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    timeout_secs INTEGER,
    retries      INTEGER,
    headers      TEXT
);
INSERT INTO providers_new (id, name, protocol, base_url, api_path, api_key,
                           billing, period_limit, limit_unit, plan_query,
                           reset_period, enabled, created_at, updated_at,
                           timeout_secs, retries, headers)
    SELECT id, name, protocol, base_url, api_path, api_key,
           billing, period_limit,
           CASE WHEN limit_unit IN ('requests','wan_tokens') THEN limit_unit
                WHEN limit_unit IS NULL THEN NULL
                ELSE upper(limit_unit) END,
           NULL,
           reset_period, enabled, created_at, updated_at,
           timeout_secs, retries, headers
    FROM providers;
DROP TABLE providers;
ALTER TABLE providers_new RENAME TO providers;
PRAGMA foreign_keys=ON;
"#;

/// v10: percent-of-window plan limits. `providers.plan_limits` holds the
/// plan-mode limit as JSON `{"five_hour":20,"weekly":60}` — per-window
/// utilization ceilings (percent of the vendor's rolling 5h / weekly window)
/// replacing the old number+unit+reset-cycle form for plan providers. Legacy
/// v9 columns (period_limit/limit_unit/reset_period) keep rendering for old
/// rows but are no longer written by the plan form; a plain nullable column
/// needs no rebuild.
const MIGRATION_V10: &str = r#"
ALTER TABLE providers ADD COLUMN plan_limits TEXT;
"#;

/// Inbound provider protocol flavor (drives data-plane dispatch, tech.md §4.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Protocol {
    Anthropic,
    OpenAI,
    Gemini,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Protocol::Anthropic => "anthropic",
            Protocol::OpenAI => "openai",
            Protocol::Gemini => "gemini",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "anthropic" => Some(Protocol::Anthropic),
            "openai" => Some(Protocol::OpenAI),
            "gemini" => Some(Protocol::Gemini),
            _ => None,
        }
    }
}

/// Billing model of a provider (tech.md §2.4 A).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Billing {
    Subscription,
    Metered,
    Unlimited,
}

impl Billing {
    pub fn as_str(self) -> &'static str {
        match self {
            Billing::Subscription => "subscription",
            Billing::Metered => "metered",
            Billing::Unlimited => "unlimited",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "subscription" => Some(Billing::Subscription),
            "metered" => Some(Billing::Metered),
            "unlimited" => Some(Billing::Unlimited),
            _ => None,
        }
    }
}

/// Agent strategy type (tech.md §4.7.1). MVP only activates `Single`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StrategyType {
    Single,
    Failover,
    Roundrobin,
    Timewindow,
    Quota,
}

impl StrategyType {
    pub fn as_str(self) -> &'static str {
        match self {
            StrategyType::Single => "single",
            StrategyType::Failover => "failover",
            StrategyType::Roundrobin => "roundrobin",
            StrategyType::Timewindow => "timewindow",
            StrategyType::Quota => "quota",
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag.
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "single" => Some(StrategyType::Single),
            "failover" => Some(StrategyType::Failover),
            "roundrobin" => Some(StrategyType::Roundrobin),
            "timewindow" => Some(StrategyType::Timewindow),
            "quota" => Some(StrategyType::Quota),
            _ => None,
        }
    }
}

/// An additional per-protocol upstream endpoint of a provider (migration v7).
/// One vendor can expose several protocol flavors (e.g. an OpenAI-compatible
/// and a native Anthropic URL); the primary one lives in `Provider.protocol`
/// + `Provider.base_url`, the rest here — keyed by protocol.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ProviderEndpoint {
    pub protocol: Protocol,
    pub base_url: String,
    /// Optional upstream path prefix, same semantics as `Provider.api_path`.
    pub api_path: Option<String>,
}

/// A configured upstream provider.
///
/// `api_key` holds the upstream credential for MVP (P1 plan: value lives in
/// the system keychain and this column keeps only a reference marker).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Provider {
    pub id: String,
    pub name: String,
    pub protocol: Protocol,
    pub base_url: String,
    /// Optional upstream path prefix, e.g. `/anthropic` for compatible endpoints.
    pub api_path: Option<String>,
    /// Additional per-protocol endpoints (migration v7): when an inbound
    /// request's protocol matches one of these, the gateway forwards natively
    /// to its URL instead of erroring or converting. Shared `api_key` pool.
    #[serde(default)]
    pub endpoints: Vec<ProviderEndpoint>,
    pub api_key: Option<String>,
    pub billing: Billing,
    /// User-entered spending/period cap used for the ring percentage estimate.
    pub period_limit: Option<f64>,
    /// Unit of `period_limit`: requests | wan_tokens | <3-letter currency> (NULL
    /// reads as requests). `default` keeps v1 export files deserializable
    /// (config share, share.rs).
    #[serde(default)]
    pub limit_unit: Option<String>,
    /// Token-plan quota query as JSON `{"template":"kimi","fields":{...}}`
    /// (migration v9). NULL = no plan quota configured.
    #[serde(default)]
    pub plan_query: Option<String>,
    /// Plan-mode percent limits as JSON `{"five_hour":20,"weekly":60}`
    /// (migration v10): utilization ceilings over the vendor's rolling 5h /
    /// weekly windows. NULL = no percent limit set.
    #[serde(default)]
    pub plan_limits: Option<String>,
    /// Per-provider cap on the wait for upstream response headers, seconds
    /// (migration v8). NULL = gateway defaults (10s connect / 300s read).
    #[serde(default)]
    pub timeout_secs: Option<i64>,
    /// Same-provider re-attempts before the strategy layer moves on (v8).
    #[serde(default)]
    pub retries: Option<i64>,
    /// Custom request headers as a JSON object {"Name":"value"} (v8); merged
    /// after credential injection, so they can override the defaults.
    #[serde(default)]
    pub headers: Option<String>,
    pub reset_period: Option<String>,
    pub enabled: bool,
    pub created_at: String,
    pub updated_at: String,
}

/// Per-agent strategy row (tech.md §4.7.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Strategy {
    pub agent: String,
    pub kind: StrategyType,
    /// JSON payload: roundrobin weights, timewindow timezone, quota thresholds, ...
    pub config: Option<String>,
}

/// Candidate binding of a provider for an agent (tech.md §4.7.2).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub agent: String,
    pub provider_id: String,
    /// 0 = primary, 1/2 = backup #1/#2 (failover order).
    pub priority: i64,
    pub weight: i64,
    pub win_start: Option<String>,
    pub win_end: Option<String>,
    pub enabled: bool,
}

/// Placeholder key `kw-ag-<agent>-<rand>` mapped to an agent (tech.md §4.6).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaceholderKey {
    pub key: String,
    pub agent: String,
    pub created_at: String,
}

/// One extra API key of a provider (spec §4.1 P1 multi-key rotation).
/// `providers.api_key` is the pool's primary; these rotate after it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ApiKeyRow {
    pub id: i64,
    pub provider_id: String,
    pub api_key: String,
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// One metered request (tech.md §4.3 request flow, usage capture).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UsageRecord {
    /// RFC3339 UTC timestamp of the request.
    pub ts: String,
    pub agent: String,
    pub provider_id: String,
    pub model: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub latency_ms: Option<i64>,
    pub status: String,
    /// Computed request cost in the price entry's currency (migration v9);
    /// NULL when the model is unpriced.
    pub cost: Option<f64>,
    /// Currency of `cost` (ISO code, e.g. "USD"); NULL when cost is NULL.
    pub cost_currency: Option<String>,
}

/// Aggregated token/request totals.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UsageTotals {
    pub requests: i64,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
}

impl UsageTotals {
    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(UsageTotals {
            requests: row.get(0)?,
            input_tokens: row.get(1)?,
            output_tokens: row.get(2)?,
            cache_read_tokens: row.get(3)?,
            cache_creation_tokens: row.get(4)?,
        })
    }
}

/// One health-probe verdict (prober → `provider_health` table).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderHealth {
    pub provider_id: String,
    /// healthy | degraded | down | unknown
    pub status: String,
    pub last_latency_ms: Option<i64>,
    pub last_check_at: Option<String>,
    pub consecutive_failures: i64,
}

/// Per-provider aggregation result.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderUsage {
    pub provider_id: String,
    pub totals: UsageTotals,
}

/// Per-day aggregation result (sparkline / dashboard, tech.md §2.4 A).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DailyUsage {
    pub day: String,
    pub totals: UsageTotals,
}

/// Health probe state of a provider (table prepared for the P1 failover engine).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HealthRecord {
    pub provider_id: String,
    /// healthy | degraded | down | unknown
    pub status: String,
    pub last_latency_ms: Option<i64>,
    pub last_check_at: Option<String>,
    pub consecutive_failures: i64,
}

/// One complete data-plane request awaiting persistence (tech.md: full request
/// logging). Bodies are optional — disabled capture or oversize truncation.
#[derive(Debug, Clone, PartialEq)]
pub struct RequestLogNew {
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    /// Attributed agent; None for failures before attribution (e.g. 404).
    pub agent: Option<String>,
    /// Attribution method: `key` / `path_fallback` (see router Attribution).
    pub attribution: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    /// Status code the client ultimately received.
    pub status_code: i64,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub is_streaming: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    /// Header maps serialized as JSON with credential headers redacted.
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    /// lossy-UTF8 request body (already truncated to the capture cap).
    pub request_body: Option<String>,
    pub response_body: Option<String>,
    /// Original byte sizes (before any truncation).
    pub request_size: i64,
    pub response_size: i64,
    pub truncated: bool,
    /// Computed request cost in the price entry's currency (migration v9).
    pub cost: Option<f64>,
    pub cost_currency: Option<String>,
}

/// Metadata row of `request_logs` (list view — never includes bodies).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestLogEntry {
    pub id: i64,
    pub ts: String,
    pub method: String,
    pub path: String,
    pub query: Option<String>,
    pub agent: Option<String>,
    pub attribution: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub status_code: i64,
    pub error_kind: Option<String>,
    pub error_message: Option<String>,
    pub session_id: Option<String>,
    pub is_streaming: bool,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub latency_ms: Option<i64>,
    pub first_token_ms: Option<i64>,
    pub request_headers: Option<String>,
    pub response_headers: Option<String>,
    pub request_size: i64,
    pub response_size: i64,
    pub truncated: bool,
}

/// Detail view: metadata + the captured bodies.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct RequestLogDetail {
    #[serde(flatten)]
    pub entry: RequestLogEntry,
    pub request_body: Option<String>,
    pub response_body: Option<String>,
}

/// Filter for `list_request_logs`. `status` is "ok" (<400) or "error" (>=400).
#[derive(Debug, Clone, Default)]
pub struct RequestLogFilter<'a> {
    pub agent: Option<&'a str>,
    pub provider_id: Option<&'a str>,
    pub status: Option<&'a str>,
}

/// Gateway-visible log capture configuration (`gateway_settings` JSON blob).
/// The GUI writes it; the gateway reads it at startup and on /reload.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogConfig {
    /// Master switch: capture data-plane requests at all.
    #[serde(default = "default_true")]
    pub enabled: bool,
    /// Store request/response bodies (when disabled, metadata only).
    #[serde(default = "default_true")]
    pub capture_bodies: bool,
    /// Rows older than this many days are pruned.
    #[serde(default = "default_retain_days")]
    pub retain_days: u32,
    /// Per-body capture cap in bytes; larger bodies are stored truncated.
    #[serde(default = "default_max_body_bytes")]
    pub max_body_bytes: usize,
}

fn default_true() -> bool {
    true
}
fn default_retain_days() -> u32 {
    30
}
fn default_max_body_bytes() -> usize {
    4 * 1024 * 1024
}

impl Default for LogConfig {
    fn default() -> Self {
        LogConfig {
            enabled: true,
            capture_bodies: true,
            retain_days: default_retain_days(),
            max_body_bytes: default_max_body_bytes(),
        }
    }
}

/// `gateway_settings` key holding the serialized `LogConfig`.
pub const LOG_CONFIG_KEY: &str = "request_logs";

/// Row counts surfaced by the admin `/status` endpoint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreMetrics {
    pub providers: i64,
    pub bindings: i64,
    pub placeholder_keys: i64,
    pub usage_rows: i64,
    pub request_log_rows: i64,
}

pub fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}

/// Thin handle around a SQLite connection (WAL, shared with the Tauri app).
pub struct Store {
    conn: Mutex<Connection>,
    #[allow(dead_code)]
    path: Option<PathBuf>,
}

impl Store {
    /// Open (creating if needed) and migrate a database file.
    pub fn open(path: impl AsRef<Path>) -> Result<Store> {
        let conn = Connection::open(path.as_ref())?;
        let store = Store {
            conn: Mutex::new(conn),
            path: Some(path.as_ref().to_path_buf()),
        };
        store.configure_and_migrate()?;
        Ok(store)
    }

    /// In-memory store, mainly for quick experiments (tests use tempfile files
    /// so WAL behaviour matches production).
    pub fn open_in_memory() -> Result<Store> {
        let conn = Connection::open_in_memory()?;
        let store = Store {
            conn: Mutex::new(conn),
            path: None,
        };
        store.configure_and_migrate()?;
        Ok(store)
    }

    fn configure_and_migrate(&self) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.pragma_update(None, "journal_mode", "WAL")?;
        conn.pragma_update(None, "synchronous", "NORMAL")?;
        conn.pragma_update(None, "foreign_keys", "ON")?;
        conn.busy_timeout(std::time::Duration::from_millis(5_000))?;
        self.migrate(&conn)?;
        Ok(())
    }

    /// Idempotent schema migration using `PRAGMA user_version`.
    fn migrate(&self, conn: &Connection) -> Result<()> {
        let version: i32 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if version < 1 {
            conn.execute_batch(MIGRATION_V1)?;
        }
        if version < 2 {
            conn.execute_batch(MIGRATION_V2)?;
        }
        if version < 3 {
            conn.execute_batch(MIGRATION_V3)?;
        }
        if version < 4 {
            conn.execute_batch(MIGRATION_V4)?;
        }
        if version < 5 {
            conn.execute_batch(MIGRATION_V5)?;
        }
        if version < 6 {
            // Repair: early dev builds stamped user_version ahead of the final
            // migration bodies, so a database can claim v5 while missing the v3
            // api_keys table, the v4 gemini-capable providers shape, or the v5
            // request-log tables. V3/V5 are IF NOT EXISTS and V4 is a lossless
            // named-column swap, so replaying all three on a complete schema
            // (or a partial one) converges to the target state.
            conn.execute_batch(MIGRATION_V3)?;
            conn.execute_batch(MIGRATION_V4)?;
            conn.execute_batch(MIGRATION_V5)?;
        }
        if version < 7 {
            conn.execute_batch(MIGRATION_V7)?;
        }
        if version < 8 {
            conn.execute_batch(MIGRATION_V8)?;
        }
        if version < 9 {
            conn.execute_batch(MIGRATION_V9)?;
        }
        if version < 10 {
            conn.execute_batch(MIGRATION_V10)?;
        }
        if version < SCHEMA_VERSION {
            conn.pragma_update(None, "user_version", SCHEMA_VERSION)?;
        }
        Ok(())
    }

    // ---- providers ------------------------------------------------------

    pub fn insert_provider(&self, p: &Provider) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO providers (id, name, protocol, base_url, api_path, api_key,
                                    billing, period_limit, limit_unit, plan_query,
                                    plan_limits, timeout_secs, retries, headers,
                                    reset_period, enabled, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18)",
            params![
                p.id,
                p.name,
                p.protocol.as_str(),
                p.base_url,
                p.api_path,
                p.api_key,
                p.billing.as_str(),
                p.period_limit,
                p.limit_unit,
                p.plan_query,
                p.plan_limits,
                p.timeout_secs,
                p.retries,
                p.headers,
                p.reset_period,
                p.enabled as i64,
                p.created_at,
                p.updated_at,
            ],
        )?;
        write_endpoints(&tx, &p.id, &p.endpoints)?;
        tx.commit()?;
        Ok(())
    }

    pub fn get_provider(&self, id: &str) -> Result<Option<Provider>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, base_url, api_path, api_key, billing,
                    period_limit, limit_unit, plan_query, plan_limits,
                    timeout_secs, retries, headers,
                    reset_period, enabled, created_at, updated_at
             FROM providers WHERE id = ?1",
        )?;
        let mut provider = stmt.query_row(params![id], provider_from_row).optional()?;
        if let Some(p) = &mut provider {
            p.endpoints = read_endpoints(&conn, &p.id)?;
        }
        Ok(provider)
    }

    pub fn list_providers(&self) -> Result<Vec<Provider>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, name, protocol, base_url, api_path, api_key, billing,
                    period_limit, limit_unit, plan_query, plan_limits,
                    timeout_secs, retries, headers,
                    reset_period, enabled, created_at, updated_at
             FROM providers ORDER BY created_at ASC, id ASC",
        )?;
        let rows = stmt.query_map([], provider_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            let mut p = row?;
            p.endpoints = read_endpoints(&conn, &p.id)?;
            out.push(p);
        }
        Ok(out)
    }

    /// Update an existing provider; refreshes `updated_at`. The additional
    /// endpoint set is replaced wholesale (add/update pass the full list).
    pub fn update_provider(&self, p: &Provider) -> Result<()> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let updated = if p.updated_at.is_empty() {
            now_rfc3339()
        } else {
            p.updated_at.clone()
        };
        let tx = conn.transaction()?;
        tx.execute(
            "UPDATE providers SET name = ?2, protocol = ?3, base_url = ?4, api_path = ?5,
                    api_key = ?6, billing = ?7, period_limit = ?8, limit_unit = ?9,
                    plan_query = ?10, plan_limits = ?11, timeout_secs = ?12,
                    retries = ?13, headers = ?14,
                    reset_period = ?15, enabled = ?16, updated_at = ?17
             WHERE id = ?1",
            params![
                p.id,
                p.name,
                p.protocol.as_str(),
                p.base_url,
                p.api_path,
                p.api_key,
                p.billing.as_str(),
                p.period_limit,
                p.limit_unit,
                p.plan_query,
                p.plan_limits,
                p.timeout_secs,
                p.retries,
                p.headers,
                p.reset_period,
                p.enabled as i64,
                updated,
            ],
        )?;
        write_endpoints(&tx, &p.id, &p.endpoints)?;
        tx.commit()?;
        Ok(())
    }

    pub fn delete_provider(&self, id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM providers WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    // ---- strategies & bindings (tech.md §4.7) ---------------------------

    pub fn upsert_strategy(
        &self,
        agent: &str,
        kind: StrategyType,
        config: Option<&str>,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO agent_strategies (agent, type, config) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent) DO UPDATE SET type = ?2, config = ?3",
            params![agent, kind.as_str(), config],
        )?;
        Ok(())
    }

    pub fn get_strategy(&self, agent: &str) -> Result<Option<Strategy>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt =
            conn.prepare("SELECT agent, type, config FROM agent_strategies WHERE agent = ?1")?;
        let s = stmt
            .query_row(params![agent], |row| {
                let type_str: String = row.get(1)?;
                Ok(Strategy {
                    agent: row.get(0)?,
                    kind: StrategyType::parse_str(&type_str).unwrap_or(StrategyType::Single),
                    config: row.get(2)?,
                })
            })
            .optional()?;
        Ok(s)
    }

    pub fn upsert_binding(&self, b: &Binding) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO agent_bindings (agent, provider_id, priority, weight,
                                         win_start, win_end, enabled)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(agent, provider_id) DO UPDATE SET
                priority = ?3, weight = ?4, win_start = ?5, win_end = ?6, enabled = ?7",
            params![
                b.agent,
                b.provider_id,
                b.priority,
                b.weight,
                b.win_start,
                b.win_end,
                b.enabled as i64,
            ],
        )?;
        Ok(())
    }

    /// Ordered candidate list for an agent (priority asc, enabled first).
    pub fn bindings_for_agent(&self, agent: &str) -> Result<Vec<Binding>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT agent, provider_id, priority, weight, win_start, win_end, enabled
             FROM agent_bindings WHERE agent = ?1
             ORDER BY enabled DESC, priority ASC, provider_id ASC",
        )?;
        let rows = stmt.query_map(params![agent], binding_from_row)?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// The provider id chosen under the `single` strategy (primary binding).
    pub fn primary_provider_id(&self, agent: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let id = conn
            .query_row(
                "SELECT provider_id FROM agent_bindings
                 WHERE agent = ?1 AND enabled = 1
                 ORDER BY priority ASC, provider_id ASC LIMIT 1",
                params![agent],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(id)
    }

    pub fn delete_binding(&self, agent: &str, provider_id: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute(
            "DELETE FROM agent_bindings WHERE agent = ?1 AND provider_id = ?2",
            params![agent, provider_id],
        )?;
        Ok(n > 0)
    }

    /// Distinct agents that have at least one binding row.
    pub fn bound_agents(&self) -> Result<Vec<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare("SELECT DISTINCT agent FROM agent_bindings ORDER BY agent")?;
        let rows = stmt.query_map([], |row| row.get::<_, String>(0))?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    // ---- placeholder keys (tech.md §4.6) --------------------------------

    pub fn upsert_placeholder_key(&self, key: &str, agent: &str) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO placeholder_keys (key, agent, created_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(key) DO UPDATE SET agent = ?2",
            params![key, agent, now_rfc3339()],
        )?;
        Ok(())
    }

    pub fn agent_for_key(&self, key: &str) -> Result<Option<String>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let agent = conn
            .query_row(
                "SELECT agent FROM placeholder_keys WHERE key = ?1",
                params![key],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(agent)
    }

    pub fn delete_placeholder_key(&self, key: &str) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM placeholder_keys WHERE key = ?1", params![key])?;
        Ok(n > 0)
    }

    pub fn list_placeholder_keys(&self) -> Result<Vec<PlaceholderKey>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT key, agent, created_at FROM placeholder_keys ORDER BY created_at ASC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(PlaceholderKey {
                key: row.get(0)?,
                agent: row.get(1)?,
                created_at: row.get(2)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    // ---- extra API keys (spec §4.1 P1 multi-key rotation) -----------------------

    pub fn list_api_keys(&self, provider_id: &str) -> Result<Vec<ApiKeyRow>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT id, provider_id, api_key, label, enabled, created_at
             FROM api_keys WHERE provider_id = ?1 ORDER BY id ASC",
        )?;
        let rows = stmt.query_map(params![provider_id], |row| {
            Ok(ApiKeyRow {
                id: row.get(0)?,
                provider_id: row.get(1)?,
                api_key: row.get(2)?,
                label: row.get(3)?,
                enabled: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Insert an extra key; returns its row id.
    pub fn insert_api_key(
        &self,
        provider_id: &str,
        api_key: &str,
        label: Option<&str>,
    ) -> Result<i64> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO api_keys (provider_id, api_key, label, enabled, created_at)
             VALUES (?1, ?2, ?3, 1, ?4)",
            params![provider_id, api_key, label, now_rfc3339()],
        )?;
        Ok(conn.last_insert_rowid())
    }

    pub fn delete_api_key(&self, id: i64) -> Result<bool> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM api_keys WHERE id = ?1", params![id])?;
        Ok(n > 0)
    }

    // ---- usage -----------------------------------------------------------

    pub fn record_usage(&self, u: &UsageRecord) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO usage (ts, agent, provider_id, model, input_tokens, output_tokens,
                                cache_read_tokens, cache_creation_tokens, latency_ms, status,
                                cost, cost_currency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12)",
            params![
                u.ts,
                u.agent,
                u.provider_id,
                u.model,
                u.input_tokens,
                u.output_tokens,
                u.cache_read_tokens,
                u.cache_creation_tokens,
                u.latency_ms,
                u.status,
                u.cost,
                u.cost_currency,
            ],
        )?;
        Ok(())
    }

    /// Cost sums grouped by currency (rows without a computed cost are
    /// skipped). Used to build currency-aware usage/dashboard figures.
    pub fn usage_cost_by_currency(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<(Option<String>, f64)>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT cost_currency, SUM(cost) FROM usage
             WHERE 1=1{cond} AND cost IS NOT NULL
             GROUP BY cost_currency",
        ))?;
        let mut out = Vec::new();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok((row.get::<_, Option<String>>(0)?, row.get::<_, f64>(1)?))
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Per-provider cost sums with their currency (dashboard by-provider
    /// breakdown needs the split before converting to the preferred currency).
    pub fn usage_cost_by_provider(
        &self,
        agent: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<(String, Option<String>, f64)>> {
        let (cond, params) = Self::usage_filters(agent, None, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT provider_id, cost_currency, SUM(cost) FROM usage
             WHERE 1=1{cond} AND cost IS NOT NULL
             GROUP BY provider_id, cost_currency",
        ))?;
        let mut out = Vec::new();
        let rows = stmt.query_map(rusqlite::params_from_iter(params), |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, Option<String>>(1)?,
                row.get::<_, f64>(2)?,
            ))
        })?;
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// WHERE fragment + positional params shared by usage-table aggregations
    /// (agent / provider / time window). Fragment indexes are 1-based and
    /// ordered, so callers splice it after `WHERE 1=1` and bind via
    /// `params_from_iter`. Params are owned: binding trait-object lifetimes
    /// to the borrowed inputs fights the borrows inside `if let` scopes.
    fn usage_filters(
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> (String, Vec<String>) {
        let mut cond = String::new();
        let mut params: Vec<String> = Vec::new();
        if let Some(a) = agent {
            params.push(a.to_string());
            cond.push_str(&format!(" AND agent = ?{}", params.len()));
        }
        if let Some(p) = provider_id {
            params.push(p.to_string());
            cond.push_str(&format!(" AND provider_id = ?{}", params.len()));
        }
        if let Some(s) = since {
            params.push(s.to_string());
            cond.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        (cond, params)
    }

    /// Aggregated totals, optionally filtered by agent, provider and/or a
    /// start timestamp.
    pub fn usage_totals(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<UsageTotals> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}",
        ))?;
        Ok(stmt.query_row(rusqlite::params_from_iter(params), UsageTotals::from_row)?)
    }

    /// Aggregated totals for one provider, optionally since a timestamp.
    /// Read by the quota strategy (tech.md §4.7 quota).
    pub fn usage_totals_for_provider(
        &self,
        provider_id: &str,
        since: Option<&str>,
    ) -> Result<UsageTotals> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT COUNT(*), COALESCE(SUM(input_tokens),0), COALESCE(SUM(output_tokens),0),
                    COALESCE(SUM(cache_read_tokens),0), COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE provider_id = ?1 AND (?2 IS NULL OR ts >= ?2)",
        )?;
        Ok(stmt.query_row(params![provider_id, since], UsageTotals::from_row)?)
    }

    /// Upsert one health-probe verdict (prober, tech.md §4.7 failover).
    pub fn upsert_provider_health(
        &self,
        provider_id: &str,
        status: &str,
        last_latency_ms: i64,
    ) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO provider_health (provider_id, status, last_latency_ms, last_check_at, consecutive_failures)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider_id) DO UPDATE SET
                status = ?2,
                last_latency_ms = ?3,
                last_check_at = ?4,
                consecutive_failures = ?5",
            params![
                provider_id,
                status,
                last_latency_ms,
                now_rfc3339(),
                if status == "down" {
                    // The streak counter increments in the same transaction: read the old value +1 (down) or reset to 0 (healthy)
                    conn.query_row(
                        "SELECT COALESCE((SELECT consecutive_failures FROM provider_health WHERE provider_id = ?1), 0) + 1",
                        params![provider_id],
                        |r| r.get::<_, i64>(0),
                    )
                    .unwrap_or(1)
                } else {
                    0
                },
            ],
        )?;
        Ok(())
    }

    /// Current health rows (for UI display and diagnostics).
    pub fn list_provider_health(&self) -> Result<Vec<ProviderHealth>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT provider_id, status, last_latency_ms, last_check_at, consecutive_failures
             FROM provider_health ORDER BY provider_id",
        )?;
        let rows = stmt
            .query_map([], |row| {
                Ok(ProviderHealth {
                    provider_id: row.get(0)?,
                    status: row.get(1)?,
                    last_latency_ms: row.get(2)?,
                    last_check_at: row.get(3)?,
                    consecutive_failures: row.get(4)?,
                })
            })?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Totals grouped by provider, optionally filtered by agent, provider
    /// and/or since.
    pub fn usage_by_provider(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<Vec<ProviderUsage>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT provider_id, COUNT(*), COALESCE(SUM(input_tokens),0),
                    COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}
             GROUP BY provider_id ORDER BY COUNT(*) DESC",
        ))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(ProviderUsage {
                provider_id: row.get(0)?,
                totals: UsageTotals {
                    requests: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                },
            })
        };
        let mut out = Vec::new();
        for row in stmt.query_map(rusqlite::params_from_iter(params), map_row)? {
            out.push(row?);
        }
        Ok(out)
    }

    /// Daily aggregation (UTC day = first 10 chars of the RFC3339 ts).
    /// Daily totals, bucketed by the caller's local day: `tz_offset_minutes`
    /// is minutes east of UTC, applied as an SQLite modifier before the date is
    /// taken (stored timestamps are UTC).
    pub fn usage_daily(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
        tz_offset_minutes: i64,
    ) -> Result<Vec<DailyUsage>> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(&format!(
            "SELECT strftime('%Y-%m-%d', ts, '{tz_offset_minutes:+} minutes') AS day, COUNT(*),
                    COALESCE(SUM(input_tokens),0),
                    COALESCE(SUM(output_tokens),0), COALESCE(SUM(cache_read_tokens),0),
                    COALESCE(SUM(cache_creation_tokens),0)
             FROM usage WHERE 1=1{cond}
             GROUP BY day ORDER BY day ASC",
        ))?;

        let map_row = |row: &rusqlite::Row<'_>| {
            Ok(DailyUsage {
                day: row.get(0)?,
                totals: UsageTotals {
                    requests: row.get(1)?,
                    input_tokens: row.get(2)?,
                    output_tokens: row.get(3)?,
                    cache_read_tokens: row.get(4)?,
                    cache_creation_tokens: row.get(5)?,
                },
            })
        };
        let mut out = Vec::new();
        for row in stmt.query_map(rusqlite::params_from_iter(params), map_row)? {
            out.push(row?);
        }
        Ok(out)
    }

    // ---- request logs (full captures) ------------------------------------

    /// Persist one completed request: metadata + bodies in a single
    /// transaction so a list row never appears without its bodies.
    pub fn insert_request_log(&self, r: &RequestLogNew) -> Result<i64> {
        let mut conn = self.conn.lock().expect("store mutex poisoned");
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO request_logs (ts, method, path, query, agent, attribution,
                                       provider_id, model, status_code, error_kind,
                                       error_message, session_id, is_streaming,
                                       input_tokens, output_tokens, cache_read_tokens,
                                       cache_creation_tokens, latency_ms, first_token_ms,
                                       request_headers, response_headers,
                                       request_size, response_size, truncated,
                                       cost, cost_currency)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13,
                     ?14, ?15, ?16, ?17, ?18, ?19, ?20, ?21, ?22, ?23, ?24, ?25, ?26)",
            params![
                r.ts,
                r.method,
                r.path,
                r.query,
                r.agent,
                r.attribution,
                r.provider_id,
                r.model,
                r.status_code,
                r.error_kind,
                r.error_message,
                r.session_id,
                r.is_streaming as i64,
                r.input_tokens,
                r.output_tokens,
                r.cache_read_tokens,
                r.cache_creation_tokens,
                r.latency_ms,
                r.first_token_ms,
                r.request_headers,
                r.response_headers,
                r.request_size,
                r.response_size,
                r.truncated as i64,
                r.cost,
                r.cost_currency,
            ],
        )?;
        let id = tx.last_insert_rowid();
        if r.request_body.is_some() || r.response_body.is_some() {
            tx.execute(
                "INSERT INTO request_bodies (log_id, request_body, response_body)
                 VALUES (?1, ?2, ?3)",
                params![id, r.request_body, r.response_body],
            )?;
        }
        tx.commit()?;
        Ok(id)
    }

    /// Paged list (newest first) with optional filters; returns rows + total.
    /// `filter.status` is "ok" (<400) or "error" (>=400); anything else means all.
    pub fn list_request_logs(
        &self,
        page: i64,
        page_size: i64,
        filter: RequestLogFilter<'_>,
    ) -> Result<(Vec<RequestLogEntry>, i64)> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        // Fixed positional params keep the SQL simple: absent filters bind NULL.
        const WHERE: &str = " WHERE (?1 IS NULL OR agent = ?1)
                             AND (?2 IS NULL OR provider_id = ?2)
                             AND (?3 IS NULL OR (?3 = 'ok' AND status_code < 400)
                                              OR (?3 = 'error' AND status_code >= 400))";
        let total: i64 = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs{WHERE}"),
            params![filter.agent, filter.provider_id, filter.status],
            |r| r.get(0),
        )?;

        let mut stmt = conn.prepare(&format!(
            "SELECT id, ts, method, path, query, agent, attribution, provider_id, model,
                    status_code, error_kind, error_message, session_id, is_streaming,
                    input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                    latency_ms, first_token_ms, request_headers, response_headers,
                    request_size, response_size, truncated
             FROM request_logs{WHERE}
             ORDER BY id DESC LIMIT ?4 OFFSET ?5",
        ))?;
        let offset = (page - 1).max(0) * page_size;
        let rows = stmt
            .query_map(
                params![
                    filter.agent,
                    filter.provider_id,
                    filter.status,
                    page_size,
                    offset
                ],
                request_log_from_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok((rows, total))
    }

    /// COUNT of data-plane requests (every row: forwarded + pre-forward
    /// failures), optionally filtered by agent, provider and/or a start
    /// timestamp. The dashboard headline reads this instead of `usage_totals`:
    /// usage rows only cover forwarded requests, so pre-forward failures would
    /// silently vanish from the top stat while the Logs card below still
    /// shows them.
    pub fn count_request_logs(
        &self,
        agent: Option<&str>,
        provider_id: Option<&str>,
        since: Option<&str>,
    ) -> Result<i64> {
        let (cond, params) = Self::usage_filters(agent, provider_id, since);
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.query_row(
            &format!("SELECT COUNT(*) FROM request_logs WHERE 1=1{cond}"),
            rusqlite::params_from_iter(params),
            |r| r.get(0),
        )?;
        Ok(n)
    }

    /// One request with bodies (detail view); None if the id is unknown.
    pub fn get_request_log(&self, id: i64) -> Result<Option<RequestLogDetail>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let entry = {
            let mut stmt = conn.prepare(
                "SELECT id, ts, method, path, query, agent, attribution, provider_id, model,
                        status_code, error_kind, error_message, session_id, is_streaming,
                        input_tokens, output_tokens, cache_read_tokens, cache_creation_tokens,
                        latency_ms, first_token_ms, request_headers, response_headers,
                        request_size, response_size, truncated
                 FROM request_logs WHERE id = ?1",
            )?;
            stmt.query_row(params![id], request_log_from_row)
                .optional()?
        };
        let Some(entry) = entry else {
            return Ok(None);
        };
        let bodies: Option<(Option<String>, Option<String>)> = conn
            .query_row(
                "SELECT request_body, response_body FROM request_bodies WHERE log_id = ?1",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        Ok(Some(RequestLogDetail {
            entry,
            request_body: bodies.as_ref().and_then(|b| b.0.clone()),
            response_body: bodies.as_ref().and_then(|b| b.1.clone()),
        }))
    }

    /// Delete rows older than `retain_days`; bodies cascade. Returns removed count.
    pub fn prune_request_logs(&self, retain_days: u32) -> Result<usize> {
        let cutoff = (Utc::now() - chrono::Duration::days(retain_days as i64)).to_rfc3339();
        let conn = self.conn.lock().expect("store mutex poisoned");
        let n = conn.execute("DELETE FROM request_logs WHERE ts < ?1", params![cutoff])?;
        Ok(n)
    }

    /// Delete every logged request (GUI "clear log" action).
    pub fn clear_request_logs(&self) -> Result<usize> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        Ok(conn.execute("DELETE FROM request_logs", [])?)
    }

    /// Load the log capture config (defaults when the key is absent/corrupt).
    pub fn load_log_config(&self) -> Result<LogConfig> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let value: Option<String> = conn
            .query_row(
                "SELECT value FROM gateway_settings WHERE key = ?1",
                params![LOG_CONFIG_KEY],
                |r| r.get(0),
            )
            .optional()?;
        Ok(match value {
            Some(json) => serde_json::from_str(&json).unwrap_or_default(),
            None => LogConfig::default(),
        })
    }

    /// Persist the log capture config (also used by the GUI via its own
    /// connection — same table, so keep the key/format in sync).
    pub fn save_log_config(&self, cfg: &LogConfig) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let json = serde_json::to_string(cfg)?;
        conn.execute(
            "INSERT INTO gateway_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            params![LOG_CONFIG_KEY, json],
        )?;
        Ok(())
    }

    /// The `model_pricing` mirror, written by the GUI's seeder from the bundled
    /// snapshot and refreshed from the Hub. The gateway resolves prices in
    /// memory, so this is what feeds a Hub price update into cost recording.
    ///
    /// An empty result means the seeder has never run — callers must fall back
    /// to the bundled table rather than read it as "nothing is priced".
    pub fn load_model_pricing(&self) -> Result<Vec<ModelPriceEntry>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT model_id, display_name, input, output,
                    cache_read, cache_creation, currency
             FROM model_pricing",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ModelPriceEntry {
                model_id: r.get(0)?,
                display_name: r.get(1)?,
                input: r.get(2)?,
                output: r.get(3)?,
                cache_read: r.get(4)?,
                cache_creation: r.get(5)?,
                currency: r.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Test-only seeding hook. The GUI owns every production write to
    /// `model_pricing` (it shares the file through its own connection), so the
    /// gateway has no upsert of its own — but a test needs one to prove that
    /// the table it reads is the table that gets served.
    #[cfg(test)]
    pub fn upsert_model_pricing(&self, e: &ModelPriceEntry) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO model_pricing (model_id, display_name, input, output,
                                        cache_read, cache_creation, currency, source)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'test')
             ON CONFLICT(model_id) DO UPDATE SET
                display_name = ?2, input = ?3, output = ?4,
                cache_read = ?5, cache_creation = ?6, currency = ?7",
            params![
                e.model_id,
                e.display_name,
                e.input,
                e.output,
                e.cache_read,
                e.cache_creation,
                e.currency
            ],
        )?;
        Ok(())
    }

    // ---- provider health (P1 failover groundwork) ------------------------

    pub fn upsert_health(&self, h: &HealthRecord) -> Result<()> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        conn.execute(
            "INSERT INTO provider_health (provider_id, status, last_latency_ms,
                                          last_check_at, consecutive_failures)
             VALUES (?1, ?2, ?3, ?4, ?5)
             ON CONFLICT(provider_id) DO UPDATE SET
                status = ?2, last_latency_ms = ?3, last_check_at = ?4,
                consecutive_failures = ?5",
            params![
                h.provider_id,
                h.status,
                h.last_latency_ms,
                h.last_check_at,
                h.consecutive_failures,
            ],
        )?;
        Ok(())
    }

    pub fn get_health(&self, provider_id: &str) -> Result<Option<HealthRecord>> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let mut stmt = conn.prepare(
            "SELECT provider_id, status, last_latency_ms, last_check_at, consecutive_failures
             FROM provider_health WHERE provider_id = ?1",
        )?;
        let h = stmt
            .query_row(params![provider_id], health_from_row)
            .optional()?;
        Ok(h)
    }

    // ---- metrics ---------------------------------------------------------

    pub fn metrics(&self) -> Result<StoreMetrics> {
        let conn = self.conn.lock().expect("store mutex poisoned");
        let count = |sql: &str| -> Result<i64> { Ok(conn.query_row(sql, [], |row| row.get(0))?) };
        Ok(StoreMetrics {
            providers: count("SELECT COUNT(*) FROM providers")?,
            bindings: count("SELECT COUNT(*) FROM agent_bindings")?,
            placeholder_keys: count("SELECT COUNT(*) FROM placeholder_keys")?,
            usage_rows: count("SELECT COUNT(*) FROM usage")?,
            request_log_rows: count("SELECT COUNT(*) FROM request_logs")?,
        })
    }
}

fn provider_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Provider> {
    let protocol_str: String = row.get(2)?;
    let billing_str: String = row.get(6)?;
    Ok(Provider {
        id: row.get(0)?,
        name: row.get(1)?,
        protocol: Protocol::parse_str(&protocol_str).unwrap_or(Protocol::Anthropic),
        base_url: row.get(3)?,
        api_path: row.get(4)?,
        endpoints: Vec::new(),
        api_key: row.get(5)?,
        billing: Billing::parse_str(&billing_str).unwrap_or(Billing::Metered),
        period_limit: row.get(7)?,
        limit_unit: row.get(8)?,
        plan_query: row.get(9)?,
        plan_limits: row.get(10)?,
        timeout_secs: row.get(11)?,
        retries: row.get(12)?,
        headers: row.get(13)?,
        reset_period: row.get(14)?,
        enabled: row.get::<_, i64>(15)? != 0,
        created_at: row.get(16)?,
        updated_at: row.get(17)?,
    })
}

/// Additional per-protocol endpoints of one provider, ordered by protocol for
/// stable output.
fn read_endpoints(conn: &Connection, provider_id: &str) -> Result<Vec<ProviderEndpoint>> {
    let mut stmt = conn.prepare(
        "SELECT protocol, base_url, api_path FROM provider_endpoints
         WHERE provider_id = ?1 ORDER BY protocol",
    )?;
    let rows = stmt.query_map(params![provider_id], |row| {
        let protocol_str: String = row.get(0)?;
        Ok(ProviderEndpoint {
            protocol: Protocol::parse_str(&protocol_str).unwrap_or(Protocol::OpenAI),
            base_url: row.get(1)?,
            api_path: row.get(2)?,
        })
    })?;
    let mut out = Vec::new();
    for row in rows {
        out.push(row?);
    }
    Ok(out)
}

/// Replace-all write of a provider's additional endpoints (inside the caller's
/// transaction; rows cascade away with the provider).
fn write_endpoints(
    tx: &rusqlite::Transaction<'_>,
    provider_id: &str,
    endpoints: &[ProviderEndpoint],
) -> Result<()> {
    tx.execute(
        "DELETE FROM provider_endpoints WHERE provider_id = ?1",
        params![provider_id],
    )?;
    for e in endpoints {
        tx.execute(
            "INSERT INTO provider_endpoints (provider_id, protocol, base_url, api_path)
             VALUES (?1, ?2, ?3, ?4)",
            params![provider_id, e.protocol.as_str(), e.base_url, e.api_path],
        )?;
    }
    Ok(())
}

fn binding_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Binding> {
    Ok(Binding {
        agent: row.get(0)?,
        provider_id: row.get(1)?,
        priority: row.get(2)?,
        weight: row.get(3)?,
        win_start: row.get(4)?,
        win_end: row.get(5)?,
        enabled: row.get::<_, i64>(6)? != 0,
    })
}

fn health_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<HealthRecord> {
    Ok(HealthRecord {
        provider_id: row.get(0)?,
        status: row.get(1)?,
        last_latency_ms: row.get(2)?,
        last_check_at: row.get(3)?,
        consecutive_failures: row.get(4)?,
    })
}

fn request_log_from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<RequestLogEntry> {
    Ok(RequestLogEntry {
        id: row.get(0)?,
        ts: row.get(1)?,
        method: row.get(2)?,
        path: row.get(3)?,
        query: row.get(4)?,
        agent: row.get(5)?,
        attribution: row.get(6)?,
        provider_id: row.get(7)?,
        model: row.get(8)?,
        status_code: row.get(9)?,
        error_kind: row.get(10)?,
        error_message: row.get(11)?,
        session_id: row.get(12)?,
        is_streaming: row.get::<_, i64>(13)? != 0,
        input_tokens: row.get(14)?,
        output_tokens: row.get(15)?,
        cache_read_tokens: row.get(16)?,
        cache_creation_tokens: row.get(17)?,
        latency_ms: row.get(18)?,
        first_token_ms: row.get(19)?,
        request_headers: row.get(20)?,
        response_headers: row.get(21)?,
        request_size: row.get(22)?,
        response_size: row.get(23)?,
        truncated: row.get::<_, i64>(24)? != 0,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_provider(id: &str, protocol: Protocol) -> Provider {
        Provider {
            id: id.to_string(),
            name: format!("prov-{id}"),
            protocol,
            base_url: "https://api.example.com".to_string(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some("sk-upstream".to_string()),
            billing: Billing::Metered,
            period_limit: Some(50.0),
            limit_unit: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
            reset_period: Some("monthly".to_string()),
            enabled: true,
            created_at: now_rfc3339(),
            updated_at: now_rfc3339(),
        }
    }

    fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(dir.path().join("kiwano.db")).expect("open store");
        (dir, store)
    }

    #[test]
    fn migration_creates_tables_and_wal() {
        let (_dir, store) = temp_store();
        let conn = store.conn.lock().unwrap();
        let version: i32 = conn
            .query_row("PRAGMA user_version", [], |r| r.get(0))
            .unwrap();
        assert_eq!(version, SCHEMA_VERSION);

        let journal: String = conn
            .query_row("PRAGMA journal_mode", [], |r| r.get(0))
            .unwrap();
        assert_eq!(journal, "wal");

        let tables: Vec<String> = {
            let mut stmt = conn
                .prepare("SELECT name FROM sqlite_master WHERE type='table' ORDER BY name")
                .unwrap();
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).unwrap();
            rows.map(|r| r.unwrap()).collect()
        };
        for expected in [
            "providers",
            "agent_strategies",
            "agent_bindings",
            "placeholder_keys",
            "usage",
            "provider_health",
            "request_logs",
            "request_bodies",
            "gateway_settings",
            "provider_endpoints",
        ] {
            assert!(tables.iter().any(|t| t == expected), "missing {expected}");
        }

        // Migration must be idempotent.
        drop(conn);
        let (_dir2, store2) = temp_store();
        store2.metrics().expect("re-migrate ok");
    }

    /// v3 → v4 rebuild: existing providers/bindings survive, gemini accepted.
    #[test]
    fn migration_v4_rebuilds_providers_with_gemini_protocol() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v3 database (pre-Gemini CHECK) with data in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-ant', 'Old', 'anthropic', 'https://api.anthropic.com', 'metered', 't0', 't0');
                 INSERT INTO agent_bindings (agent, provider_id, priority, weight, enabled)
                 VALUES ('claude', 'p-ant', 0, 1, 1);
                 INSERT INTO api_keys (provider_id, api_key, enabled, created_at)
                 VALUES ('p-ant', 'sk-x', 1, 't0');
                 PRAGMA user_version = 3;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let got = store.get_provider("p-ant").unwrap().expect("provider kept");
        assert_eq!(got.protocol, Protocol::Anthropic);
        assert_eq!(got.limit_unit, None);
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);
        assert_eq!(store.list_api_keys("p-ant").unwrap().len(), 1);

        // v4 widened the CHECK: gemini providers are accepted now.
        let gem = sample_provider("p-gem", Protocol::Gemini);
        store.insert_provider(&gem).unwrap();
        assert_eq!(
            store.get_provider("p-gem").unwrap().unwrap().protocol,
            Protocol::Gemini
        );
    }

    /// v8 → v9 rebuild: providers survive with widened limit_unit CHECK
    /// (legacy 'cny' uppercased, units preserved), plan_query backfilled NULL,
    /// model_pricing created, usage/request_logs gain cost columns.
    #[test]
    fn migration_v9_rebuilds_providers_with_plan_query_and_cost_columns() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v8 database (pre-pricing) with data in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(MIGRATION_V4).unwrap();
            conn.execute_batch(MIGRATION_V5).unwrap();
            conn.execute_batch(MIGRATION_V7).unwrap();
            conn.execute_batch(MIGRATION_V8).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        period_limit, limit_unit, created_at, updated_at)
                 VALUES ('p-cny', 'Old', 'anthropic', 'https://api.example.com', 'metered',
                         10.0, 'cny', 't0', 't0'),
                        ('p-req', 'Old2', 'openai', 'https://api2.example.com', 'metered',
                         100.0, 'requests', 't0', 't0');
                 PRAGMA user_version = 8;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        let cny = store.get_provider("p-cny").unwrap().expect("provider kept");
        // Legacy lowercase currency unit is uppercased; 'requests' preserved.
        assert_eq!(cny.limit_unit.as_deref(), Some("CNY"));
        assert_eq!(cny.plan_query, None);
        assert_eq!(
            store
                .get_provider("p-req")
                .unwrap()
                .unwrap()
                .limit_unit
                .as_deref(),
            Some("requests")
        );
        // Bindings survive the rebuild.
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p-cny".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        assert_eq!(store.bindings_for_agent("claude").unwrap().len(), 1);

        // New v9 shapes exist: model_pricing table + cost columns + 3-letter
        // currency units accepted.
        let conn = store.conn.lock().unwrap();
        let has_table: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM sqlite_master WHERE type='table' AND name='model_pricing'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(has_table, 1);
        conn.execute_batch(
            "INSERT INTO model_pricing (model_id, display_name, input, output)
             VALUES ('m-x', 'X', '1', '2');",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO usage (ts, agent, provider_id, status, cost, cost_currency)
             VALUES ('t0', 'claude', 'p-cny', 'ok', 0.5, 'USD')",
            [],
        )
        .unwrap();
        drop(conn);

        // Currency limit units pass the widened CHECK; plan_query round-trips.
        let mut p = sample_provider("p-usd", Protocol::OpenAI);
        p.limit_unit = Some("USD".to_string());
        p.plan_query = Some(r#"{"template":"kimi"}"#.to_string());
        store.insert_provider(&p).unwrap();
        let got = store.get_provider("p-usd").unwrap().expect("usd provider");
        assert_eq!(got.limit_unit.as_deref(), Some("USD"));
        assert_eq!(got.plan_query.as_deref(), Some(r#"{"template":"kimi"}"#));
    }

    /// v9 → v10: rows survive and gain a NULL plan_limits column; the percent
    /// limits JSON round-trips through insert/update/get.
    #[test]
    fn migration_v10_adds_plan_limits_column() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");

        // Hand-build a v9 database with a provider in it.
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(MIGRATION_V3).unwrap();
            conn.execute_batch(MIGRATION_V4).unwrap();
            conn.execute_batch(MIGRATION_V5).unwrap();
            conn.execute_batch(MIGRATION_V7).unwrap();
            conn.execute_batch(MIGRATION_V8).unwrap();
            conn.execute_batch(MIGRATION_V9).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing,
                                        period_limit, limit_unit, created_at, updated_at)
                 VALUES ('p-old', 'Old', 'openai', 'https://api.example.com', 'metered',
                         10.0, 'requests', 't0', 't0');
                 PRAGMA user_version = 9;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        // v9 row survives with its data; plan_limits reads as NULL.
        let old = store.get_provider("p-old").unwrap().expect("kept");
        assert_eq!(old.limit_unit.as_deref(), Some("requests"));
        assert_eq!(old.plan_limits, None);

        // Percent limits round-trip; NULL clears.
        let mut p = sample_provider("p-plan", Protocol::Anthropic);
        p.billing = Billing::Subscription;
        p.plan_limits = Some(r#"{"five_hour":20,"weekly":60}"#.to_string());
        store.insert_provider(&p).unwrap();
        let got = store
            .get_provider("p-plan")
            .unwrap()
            .expect("plan provider");
        assert_eq!(
            got.plan_limits.as_deref(),
            Some(r#"{"five_hour":20,"weekly":60}"#)
        );
        p.plan_limits = None;
        store.update_provider(&p).unwrap();
        assert_eq!(
            store.get_provider("p-plan").unwrap().unwrap().plan_limits,
            None
        );
    }

    /// v7: additional per-protocol endpoints round-trip with the provider row
    /// and cascade away on delete.
    #[test]
    fn provider_endpoints_roundtrip() {
        let (_dir, store) = temp_store();

        let mut p = sample_provider("p-dual", Protocol::OpenAI);
        p.endpoints = vec![ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://api.example.com/anthropic".into(),
            api_path: None,
        }];
        store.insert_provider(&p).unwrap();

        let got = store.get_provider("p-dual").unwrap().unwrap();
        assert_eq!(got.endpoints.len(), 1);
        assert_eq!(got.endpoints[0].protocol, Protocol::Anthropic);
        assert_eq!(
            got.endpoints[0].base_url,
            "https://api.example.com/anthropic"
        );
        // list reads them too
        assert_eq!(store.list_providers().unwrap()[0].endpoints.len(), 1);

        // update replaces the whole set
        let mut updated = got.clone();
        updated.endpoints = vec![
            ProviderEndpoint {
                protocol: Protocol::Anthropic,
                base_url: "https://api.example.com/anthropic/v2".into(),
                api_path: None,
            },
            ProviderEndpoint {
                protocol: Protocol::Gemini,
                base_url: "https://api.example.com/gemini".into(),
                api_path: None,
            },
        ];
        store.update_provider(&updated).unwrap();
        let got = store.get_provider("p-dual").unwrap().unwrap();
        assert_eq!(got.endpoints.len(), 2);
        assert_eq!(
            got.endpoints[0].base_url,
            "https://api.example.com/anthropic/v2"
        );

        // empty list clears every additional endpoint
        let mut cleared = got.clone();
        cleared.endpoints.clear();
        store.update_provider(&cleared).unwrap();
        assert!(store
            .get_provider("p-dual")
            .unwrap()
            .unwrap()
            .endpoints
            .is_empty());

        // rows cascade away with the provider
        let mut p2 = sample_provider("p-cascade", Protocol::OpenAI);
        p2.endpoints = vec![ProviderEndpoint {
            protocol: Protocol::Anthropic,
            base_url: "https://x.example.com/anthropic".into(),
            api_path: None,
        }];
        store.insert_provider(&p2).unwrap();
        store.delete_provider("p-cascade").unwrap();
        let conn = store.conn.lock().unwrap();
        let n: i64 = conn
            .query_row("SELECT COUNT(*) FROM provider_endpoints", [], |r| r.get(0))
            .unwrap();
        assert_eq!(n, 0);
    }

    /// v8: per-provider advanced forwarding settings (timeout / retries /
    /// custom headers) round-trip through insert, get, list, and update.
    #[test]
    fn provider_advanced_settings_roundtrip() {
        let (_dir, store) = temp_store();

        let mut p = sample_provider("p-adv", Protocol::OpenAI);
        p.timeout_secs = Some(120);
        p.retries = Some(2);
        p.headers = Some(r#"{"api-key":"azure-key"}"#.to_string());
        store.insert_provider(&p).unwrap();

        // get reads all three
        let got = store.get_provider("p-adv").unwrap().unwrap();
        assert_eq!(got.timeout_secs, Some(120));
        assert_eq!(got.retries, Some(2));
        assert_eq!(got.headers.as_deref(), Some(r#"{"api-key":"azure-key"}"#));
        // list reads them too (everything here is positional SQL)
        let listed = &store.list_providers().unwrap()[0];
        assert_eq!(listed.timeout_secs, Some(120));
        assert_eq!(listed.retries, Some(2));
        assert_eq!(
            listed.headers.as_deref(),
            Some(r#"{"api-key":"azure-key"}"#)
        );

        // update clears them (None = cleared, not "keep")
        let mut cleared = got.clone();
        cleared.timeout_secs = None;
        cleared.retries = None;
        cleared.headers = None;
        store.update_provider(&cleared).unwrap();
        let got = store.get_provider("p-adv").unwrap().unwrap();
        assert_eq!(got.timeout_secs, None);
        assert_eq!(got.retries, None);
        assert_eq!(got.headers, None);
    }

    /// A database stamped v5 by an early dev build but missing the v3 api_keys
    /// table, the v4 gemini providers shape, and the v5 request-log tables is
    /// repaired on open (version bumped to SCHEMA_VERSION, objects created,
    /// data kept).
    #[test]
    fn migrate_repairs_database_stamped_ahead_of_schema() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("kiwano.db");
        {
            let conn = Connection::open(&db).unwrap();
            conn.execute_batch(MIGRATION_V1).unwrap();
            conn.execute_batch(MIGRATION_V2).unwrap();
            conn.execute_batch(
                "INSERT INTO providers (id, name, protocol, base_url, billing, created_at, updated_at)
                 VALUES ('p-ant', 'Old', 'anthropic', 'https://api.anthropic.com', 'metered', 't0', 't0');
                 PRAGMA user_version = 5;",
            )
            .unwrap();
        }

        let store = Store::open(&db).unwrap();
        {
            let conn = store.conn.lock().unwrap();
            let version: i32 = conn
                .query_row("PRAGMA user_version", [], |r| r.get(0))
                .unwrap();
            assert_eq!(version, SCHEMA_VERSION);

            let n: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM sqlite_master
                     WHERE type='table'
                       AND name IN ('api_keys','request_logs','request_bodies','gateway_settings')",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert_eq!(n, 4, "missing tables repaired");

            // providers rebuilt to the gemini-capable shape
            let ddl: String = conn
                .query_row(
                    "SELECT sql FROM sqlite_master WHERE name='providers'",
                    [],
                    |r| r.get(0),
                )
                .unwrap();
            assert!(ddl.contains("gemini"));
        }
        assert_eq!(
            store
                .get_provider("p-ant")
                .unwrap()
                .expect("provider kept")
                .name,
            "Old"
        );
    }

    #[test]
    fn provider_crud_roundtrip() {
        let (_dir, store) = temp_store();
        let p = sample_provider("p1", Protocol::Anthropic);
        store.insert_provider(&p).unwrap();

        let got = store.get_provider("p1").unwrap().expect("exists");
        assert_eq!(got, p);

        let mut updated = p.clone();
        updated.name = "renamed".to_string();
        updated.enabled = false;
        updated.updated_at = String::new(); // triggers fresh updated_at
        store.update_provider(&updated).unwrap();
        let got = store.get_provider("p1").unwrap().unwrap();
        assert_eq!(got.name, "renamed");
        assert!(!got.enabled);
        assert_ne!(got.updated_at, p.updated_at);

        let all = store.list_providers().unwrap();
        assert_eq!(all.len(), 1);

        // Duplicate insert must fail (primary key).
        assert!(store.insert_provider(&p).is_err());

        assert!(store.delete_provider("p1").unwrap());
        assert!(store.get_provider("p1").unwrap().is_none());
        assert!(!store.delete_provider("p1").unwrap());
    }

    #[test]
    fn strategy_and_binding_selection() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("a", Protocol::Anthropic))
            .unwrap();
        store
            .insert_provider(&sample_provider("b", Protocol::OpenAI))
            .unwrap();

        store
            .upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        let s = store.get_strategy("claude").unwrap().unwrap();
        assert_eq!(s.kind, StrategyType::Single);
        assert_eq!(s.agent, "claude");

        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "b".into(),
                priority: 1,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "a".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();

        // Single strategy picks the priority-0 primary.
        assert_eq!(store.primary_provider_id("claude").unwrap().unwrap(), "a");

        let bindings = store.bindings_for_agent("claude").unwrap();
        assert_eq!(bindings.len(), 2);
        assert_eq!(bindings[0].provider_id, "a");

        // Disabling the primary demotes it; next candidate takes over.
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "a".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: false,
            })
            .unwrap();
        assert_eq!(store.primary_provider_id("claude").unwrap().unwrap(), "b");

        assert!(store.delete_binding("claude", "a").unwrap());
        assert!(!store.delete_binding("claude", "a").unwrap());

        // Unknown agent has no binding.
        assert!(store.primary_provider_id("codex").unwrap().is_none());
    }

    #[test]
    fn deleting_provider_cascades_bindings() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p", Protocol::Anthropic))
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store.delete_provider("p").unwrap();
        assert!(store.bindings_for_agent("claude").unwrap().is_empty());
    }

    #[test]
    fn placeholder_key_lookup() {
        let (_dir, store) = temp_store();
        store
            .upsert_placeholder_key("kw-ag-claude-abc123", "claude")
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-codex-xyz789", "codex")
            .unwrap();

        assert_eq!(
            store.agent_for_key("kw-ag-claude-abc123").unwrap(),
            Some("claude".to_string())
        );
        assert_eq!(
            store.agent_for_key("kw-ag-codex-xyz789").unwrap(),
            Some("codex".to_string())
        );
        assert_eq!(store.agent_for_key("kw-ag-unknown").unwrap(), None);

        // Upsert rebinds the agent.
        store
            .upsert_placeholder_key("kw-ag-claude-abc123", "gemini")
            .unwrap();
        assert_eq!(
            store.agent_for_key("kw-ag-claude-abc123").unwrap(),
            Some("gemini".to_string())
        );

        assert_eq!(store.list_placeholder_keys().unwrap().len(), 2);
        assert!(store.delete_placeholder_key("kw-ag-claude-abc123").unwrap());
        assert!(!store.delete_placeholder_key("kw-ag-claude-abc123").unwrap());
    }

    #[test]
    fn usage_record_and_aggregates() {
        let (_dir, store) = temp_store();
        let mk = |ts: &str, provider: &str, input: i64, output: i64| UsageRecord {
            ts: ts.to_string(),
            agent: "claude".to_string(),
            provider_id: provider.to_string(),
            model: Some("claude-sonnet-4-5".to_string()),
            input_tokens: input,
            output_tokens: output,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(120),
            status: "ok".to_string(),
            cost: None,
            cost_currency: None,
        };

        store
            .record_usage(&mk("2026-09-06T10:00:00+00:00", "p1", 100, 200))
            .unwrap();
        store
            .record_usage(&mk("2026-09-06T11:00:00+00:00", "p1", 10, 20))
            .unwrap();
        store
            .record_usage(&mk("2026-09-07T10:00:00+00:00", "p2", 7, 3))
            .unwrap();
        store
            .record_usage(&UsageRecord {
                ts: "2026-09-07T12:00:00+00:00".into(),
                agent: "codex".into(),
                provider_id: "p1".into(),
                model: None,
                input_tokens: 5,
                output_tokens: 5,
                cache_read_tokens: 40,
                cache_creation_tokens: 2,
                latency_ms: None,
                status: "error".into(),
                cost: None,
                cost_currency: None,
            })
            .unwrap();

        let totals = store.usage_totals(None, None, None).unwrap();
        assert_eq!(totals.requests, 4);
        assert_eq!(totals.input_tokens, 122);
        assert_eq!(totals.output_tokens, 228);
        assert_eq!(totals.cache_read_tokens, 40);
        assert_eq!(totals.cache_creation_tokens, 2);

        let agent_totals = store.usage_totals(Some("claude"), None, None).unwrap();
        assert_eq!(agent_totals.requests, 3);
        assert_eq!(agent_totals.input_tokens, 117);

        // Provider filter spans agents (2 claude + 1 codex rows on p1).
        let provider_totals = store.usage_totals(None, Some("p1"), None).unwrap();
        assert_eq!(provider_totals.requests, 3);
        // Combined agent + provider filter ANDs both conditions.
        let combined = store
            .usage_totals(Some("claude"), Some("p1"), None)
            .unwrap();
        assert_eq!(combined.requests, 2);

        let since_totals = store
            .usage_totals(None, None, Some("2026-09-07T00:00:00+00:00"))
            .unwrap();
        assert_eq!(since_totals.requests, 2);

        let by_provider = store.usage_by_provider(Some("claude"), None, None).unwrap();
        assert_eq!(by_provider.len(), 2);
        assert_eq!(by_provider[0].provider_id, "p1");
        assert_eq!(by_provider[0].totals.requests, 2);
        assert_eq!(by_provider[0].totals.output_tokens, 220);

        let one_provider = store.usage_by_provider(None, Some("p2"), None).unwrap();
        assert_eq!(one_provider.len(), 1);
        assert_eq!(one_provider[0].provider_id, "p2");

        // tz 0 = UTC: the timestamps below are already UTC dates, so the
        // buckets are unchanged. A non-zero offset is covered by the dashboard
        // test in vm.rs, where the window and the chart must agree on it.
        let daily = store.usage_daily(Some("claude"), None, None, 0).unwrap();
        assert_eq!(daily.len(), 2);
        assert_eq!(daily[0].day, "2026-09-06");
        assert_eq!(daily[1].day, "2026-09-07");
        assert_eq!(daily[1].totals.requests, 1);

        let empty = store.usage_totals(Some("gemini"), None, None).unwrap();
        assert_eq!(empty.requests, 0);
        assert_eq!(empty.input_tokens, 0);
    }

    #[test]
    fn health_upsert_and_get() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p1", Protocol::OpenAI))
            .unwrap();

        assert!(store.get_health("p1").unwrap().is_none());

        store
            .upsert_health(&HealthRecord {
                provider_id: "p1".into(),
                status: "healthy".into(),
                last_latency_ms: Some(88),
                last_check_at: Some(now_rfc3339()),
                consecutive_failures: 0,
            })
            .unwrap();
        let h = store.get_health("p1").unwrap().unwrap();
        assert_eq!(h.status, "healthy");
        assert_eq!(h.last_latency_ms, Some(88));

        store
            .upsert_health(&HealthRecord {
                provider_id: "p1".into(),
                status: "down".into(),
                last_latency_ms: None,
                last_check_at: Some(now_rfc3339()),
                consecutive_failures: 3,
            })
            .unwrap();
        let h = store.get_health("p1").unwrap().unwrap();
        assert_eq!(h.status, "down");
        assert_eq!(h.consecutive_failures, 3);
    }

    #[test]
    fn metrics_counts() {
        let (_dir, store) = temp_store();
        store
            .insert_provider(&sample_provider("p1", Protocol::Anthropic))
            .unwrap();
        store
            .upsert_placeholder_key("kw-ag-claude-abc", "claude")
            .unwrap();
        store
            .upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: "p1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        store
            .record_usage(&UsageRecord {
                ts: now_rfc3339(),
                agent: "claude".into(),
                provider_id: "p1".into(),
                model: None,
                input_tokens: 1,
                output_tokens: 1,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
            })
            .unwrap();

        let m = store.metrics().unwrap();
        assert_eq!(m.providers, 1);
        assert_eq!(m.bindings, 1);
        assert_eq!(m.placeholder_keys, 1);
        assert_eq!(m.usage_rows, 1);
    }

    fn sample_log(ts: &str, agent: Option<&str>, status: i64) -> RequestLogNew {
        RequestLogNew {
            ts: ts.to_string(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: agent.map(str::to_string),
            attribution: agent.map(|_| "key".to_string()),
            provider_id: agent.map(|_| "p1".to_string()),
            model: Some("claude-sonnet-4-5".into()),
            status_code: status,
            error_kind: (status >= 400).then(|| "upstream_error".to_string()),
            error_message: (status >= 400).then(|| "boom".to_string()),
            session_id: Some("sess-1".into()),
            is_streaming: false,
            input_tokens: 10,
            output_tokens: 20,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(88),
            first_token_ms: None,
            request_headers: Some(r#"{"content-type":"application/json"}"#.into()),
            response_headers: None,
            request_body: Some(r#"{"model":"claude-sonnet-4-5"}"#.into()),
            response_body: Some(r#"{"ok":true}"#.into()),
            request_size: 28,
            response_size: 12,
            truncated: false,
            cost: None,
            cost_currency: None,
        }
    }

    #[test]
    fn request_log_insert_list_detail_roundtrip() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log("2026-09-07T11:00:00+00:00", Some("codex"), 502))
            .unwrap();
        // Pre-attribution failure: no agent/provider at all.
        store
            .insert_request_log(&sample_log("2026-09-07T12:00:00+00:00", None, 404))
            .unwrap();

        let (rows, total) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(total, 3);
        assert_eq!(rows.len(), 3);
        assert_eq!(rows[0].status_code, 404); // newest first

        let detail = store.get_request_log(rows[0].id).unwrap().unwrap();
        assert_eq!(detail.entry.status_code, 404);
        assert_eq!(detail.entry.error_kind.as_deref(), Some("upstream_error"));
        assert_eq!(
            detail.request_body.as_deref(),
            Some(r#"{"model":"claude-sonnet-4-5"}"#)
        );
        assert_eq!(detail.response_body.as_deref(), Some(r#"{"ok":true}"#));

        // Filters.
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    agent: Some("claude"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 1);
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    status: Some("error"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 2); // 502 + 404
        let (_, n) = store
            .list_request_logs(
                1,
                10,
                RequestLogFilter {
                    status: Some("ok"),
                    ..Default::default()
                },
            )
            .unwrap();
        assert_eq!(n, 1);

        // Pagination.
        let (rows, _) = store
            .list_request_logs(2, 2, RequestLogFilter::default())
            .unwrap();
        assert_eq!(rows.len(), 1);

        // Bodyless insert (capture_bodies=false) still lists; detail has no bodies.
        let mut bodyless = sample_log("2026-09-07T13:00:00+00:00", Some("pi"), 200);
        bodyless.request_body = None;
        bodyless.response_body = None;
        let id = store.insert_request_log(&bodyless).unwrap();
        let detail = store.get_request_log(id).unwrap().unwrap();
        assert_eq!(detail.request_body, None);
        assert_eq!(detail.response_body, None);

        assert!(store.get_request_log(9999).unwrap().is_none());
    }

    #[test]
    fn count_request_logs_windows_by_ts() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-09-01T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log(
                "2026-09-07T10:00:00+00:00",
                Some("claude"),
                503,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log("2026-09-08T10:00:00+00:00", None, 404))
            .unwrap();

        assert_eq!(store.count_request_logs(None, None, None).unwrap(), 3);
        // Failures count too — the dashboard headline uses this.
        assert_eq!(
            store
                .count_request_logs(None, None, Some("2026-09-07T00:00:00Z"))
                .unwrap(),
            2
        );
        assert_eq!(
            store
                .count_request_logs(None, None, Some("2026-09-09T00:00:00Z"))
                .unwrap(),
            0
        );
        // Agent / provider filters: the agentless row (no provider either) is
        // only counted in the unfiltered totals.
        assert_eq!(
            store
                .count_request_logs(Some("claude"), None, None)
                .unwrap(),
            2
        );
        assert_eq!(store.count_request_logs(None, Some("p1"), None).unwrap(), 2);
    }

    #[test]
    fn request_log_prune_and_clear() {
        let (_dir, store) = temp_store();
        store
            .insert_request_log(&sample_log(
                "2026-08-01T10:00:00+00:00",
                Some("claude"),
                200,
            ))
            .unwrap();
        store
            .insert_request_log(&sample_log(now_rfc3339().as_str(), Some("claude"), 200))
            .unwrap();

        // Old row (plus its bodies) goes; fresh row stays.
        assert_eq!(store.prune_request_logs(30).unwrap(), 1);
        let (rows, total) = store
            .list_request_logs(1, 10, RequestLogFilter::default())
            .unwrap();
        assert_eq!(total, 1);
        let detail = store.get_request_log(rows[0].id).unwrap().unwrap();
        assert_eq!(
            detail.request_body.as_deref(),
            Some(r#"{"model":"claude-sonnet-4-5"}"#)
        );

        assert_eq!(store.clear_request_logs().unwrap(), 1);
        assert_eq!(
            store
                .list_request_logs(1, 10, RequestLogFilter::default())
                .unwrap()
                .1,
            0
        );
    }

    #[test]
    fn log_config_roundtrip_with_defaults() {
        let (_dir, store) = temp_store();
        assert_eq!(store.load_log_config().unwrap(), LogConfig::default());

        store
            .save_log_config(&LogConfig {
                enabled: false,
                capture_bodies: false,
                retain_days: 7,
                max_body_bytes: 1024,
            })
            .unwrap();
        assert_eq!(
            store.load_log_config().unwrap(),
            LogConfig {
                enabled: false,
                capture_bodies: false,
                retain_days: 7,
                max_body_bytes: 1024
            }
        );

        // Corrupt JSON falls back to defaults instead of breaking the gateway.
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE gateway_settings SET value = 'not-json' WHERE key = ?1",
            params![LOG_CONFIG_KEY],
        )
        .unwrap();
        drop(conn);
        assert_eq!(store.load_log_config().unwrap(), LogConfig::default());
    }
}
