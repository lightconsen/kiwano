//! Provider token-plan quota queries (ported from cc-switch's coding_plan
//! service).
//!
//! A Subscription provider may carry a `plan_query` JSON blob —
//! `{"template": "<id>", "fields": {...}}` — selecting one of the known
//! quota endpoints (Kimi / Zhipu / Zhipu team / MiniMax / ZenMux /
//! OpenCode Go / Volcengine). `get_plan_quota` executes the template's
//! HTTP call with the provider's API key (plus per-template credential
//! fields), parses the usage windows into utilization tiers, and caches
//! the report in the shared `app_settings` KV for 5 minutes.
//!
//! Error channels (same contract as cc-switch): transient transport
//! failures (network / read interruption) return `Err` so the frontend
//! can retry and keep the last good value; deterministic failures (bad
//! key, business error, unknown shape) return a report with
//! `success: false` carrying the user-facing reason.
//!
//! The module is split by vendor. Every `pub` item keeps the path it had when
//! this was one file (`plan_quota::PlanQuotaReport`): the facade below
//! re-exports it, because `crates/cli`, `app/src-tauri` and the sibling
//! modules of this crate all still name it that way.
//!
//! `types` owns the wire contract; `json` and `http` are leaves — they read no
//! store and no vendor, so any adapter may import them without an ordering
//! worry. `kimi`, `zhipu`, `minimax`, `zenmux`, `opencode_go` and `volcengine`
//! each own one endpoint and its parser, `dispatch` is the template table that
//! names them, and `cache` owns the `app_settings` KV plus the refresh step
//! that fills it.

pub mod cache;
pub mod dispatch;
pub mod http;
pub mod json;
pub mod kimi;
pub mod minimax;
pub mod opencode_go;
pub mod types;
pub mod volcengine;
pub mod zenmux;
pub mod zhipu;

// ── the public surface, re-exported so every `plan_quota::x` path still resolves ──

pub use cache::{cache_write, cached_report, get_plan_quota_report};
pub use dispatch::plan_monthly_price;
pub use types::{PlanQuotaReport, PlanTierVm, QuotaOutcome};
