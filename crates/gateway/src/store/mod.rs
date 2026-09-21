//! SQLite persistence for the Kiwano gateway (tech.md §2.3 / §4.7).
//!
//! SQLite is the single source of truth (tech.md §4.2): the Tauri app writes
//! provider/binding configuration, the gateway reads it and writes usage.
//! The database file is opened in WAL mode so both processes can share it;
//! `busy_timeout` guards against transient cross-process lock contention.
//!
//! The file is also the home of every upstream credential (see `Provider`), so
//! it is owner-only by construction — see `harden_permissions`.
//!
//! The module is split by table and domain. Every `pub` item keeps the path it
//! had when this was one file (`store::Provider`, `store::list_providers`): the
//! facade below re-exports it, because `crates/core`, `crates/cli` and
//! `app/src-tauri` all still name it that way.
//!
//! `time` is a leaf — it reads no store and no setting, so any module may
//! import it without an ordering worry. `migrations` owns the schema and the
//! open contract (`Store::open` → `configure_and_migrate` → `migrate`, then
//! `harden_permissions`); `types` owns the row structs everything else reads.
//!
//! `Store` itself is defined here rather than in a submodule, because every
//! module in the tree adds an `impl Store` block to it and `conn` is the field
//! they all reach through — `pub(crate)`, so a test can drive raw SQL against
//! the same connection.

use rusqlite::Connection;
use std::path::PathBuf;
use std::sync::Mutex;

pub mod agents;
pub mod config;
pub mod health;
pub mod keys;
pub mod logs;
pub mod metrics;
pub mod migrations;
pub mod permissions;
pub mod pricing;
pub mod providers;
pub mod routing;
pub mod settings;
pub mod time;
pub mod types;
pub mod usage;

// ── the public surface, re-exported so every `store::x` path still resolves ──

pub use config::{
    CompatShimConfig, LogConfig, StreamTimeouts, COMPAT_SHIM_CONFIG_KEY, LOG_CONFIG_KEY,
    STREAM_TIMEOUTS_KEY,
};
pub use logs::{
    RequestLogDetail, RequestLogEntry, RequestLogExportRow, RequestLogFilter, RequestLogNew,
    EXPORT_ROW_CAP,
};
pub use metrics::StoreMetrics;
pub use migrations::SCHEMA_VERSION;
pub use time::{now_rfc3339, rfc3339_from_unix, unix_now};
pub use types::{
    AgentLimit, ApiKeyRow, Billing, Binding, CostBucket, CustomAgent, DailyUsage, PlaceholderKey,
    Protocol, Provider, ProviderCostBucket, ProviderEndpoint, ProviderHealth, ProviderUsage,
    Strategy, StrategyType, TrafficStats, UsageRecord, UsageTotals,
};

/// Thin handle around a SQLite connection (WAL, shared with the Tauri app).
pub struct Store {
    pub(crate) conn: Mutex<Connection>,
    #[allow(dead_code)]
    path: Option<PathBuf>,
}

#[cfg(test)]
pub(crate) mod test_support {
    use crate::store::time::now_rfc3339;
    use crate::store::types::{Billing, Protocol, Provider};
    use crate::store::{RequestLogNew, Store};

    pub(crate) fn sample_provider(id: &str, protocol: Protocol) -> Provider {
        Provider {
            id: id.to_string(),
            name: format!("prov-{id}"),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol,
            base_url: "https://api.example.com".to_string(),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some("sk-upstream".to_string()),
            model_default: None,
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

    pub(crate) fn temp_store() -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().expect("tempdir");
        let store = Store::open(dir.path().join("kiwano.db")).expect("open store");
        (dir, store)
    }

    pub(crate) fn sample_log(ts: &str, agent: Option<&str>, status: i64) -> RequestLogNew {
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
            reasoning_tokens: 0,
            usage_missing: false,
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
            cost_off_peak: None,
            request_notes: None,
        }
    }
}
