//! The Settings object: the whole blob the Settings screen reads and patches.
//!
//! Moved here from `kiwano_core::vm::settings` when the daemon took over the
//! read and the write (`migrate.local.md` §10.16). Both derive both directions:
//! the daemon serves the blob and the client parses it back — and a patch is
//! sent the same way it came.

use crate::agents::CustomAgentVm;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TakeoverVm {
    pub agent: String,
    pub label: String,
    pub placeholder_key: Option<String>,
    pub enabled: bool,
    /// The files a takeover rewrites for this agent, in the form a reader would
    /// write them (`~/.claude/settings.json`). Read out of the same function the
    /// takeover itself writes through, so what the screen shows is the file that
    /// would actually change — not a second list to keep in step. Empty for an
    /// agent whose paths cannot be resolved on this platform (claude-desktop
    /// outside macOS).
    #[serde(default)]
    pub config_paths: Vec<String>,
    /// Additive-mode agent (config keeps multiple providers; takeover writes a
    /// gateway entry and selects it) rather than exclusive-switch mode.
    #[serde(default)]
    pub additive: bool,
    /// The protocols this agent's clients speak ([`AGENT_PROTOCOLS`]). On this
    /// row because it is *the* per-agent row the screen already reads — `additive`
    /// above is a fact about the agent rather than about takeover state, and this
    /// is the same kind of thing. A **label**: nothing routes or validates by it.
    #[serde(default)]
    pub protocols: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsVm {
    pub language: String,
    pub theme: String,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub gateway_listen: String,
    pub takeovers: Vec<TakeoverVm>,
    /// Agents the user defined (migration v16). Separate from `takeovers`
    /// because the two answer different questions: a takeover says "we rewrote
    /// this agent's config", and a custom agent has no config to rewrite.
    #[serde(default)]
    pub custom_agents: Vec<CustomAgentVm>,
    pub auto_failover: bool,
    pub request_logs: bool,
    /// Compat shim on the gateway's passthrough path (mirrored into
    /// gateway_settings for the sidecar). Default on: each rule is narrow
    /// enough that the honest choices are on and off, and off reproduces the
    /// 400s the shim exists to absorb.
    #[serde(default = "default_true")]
    pub compat_shim: bool,
    /// Outbound credential detection on the gateway's data plane (mirrored into
    /// gateway_settings for the sidecar). `"alert"` or `"off"`; default
    /// `"alert"`, because the pass reports and never blocks — an install that
    /// ignores this switch loses nothing by it.
    #[serde(default = "default_dlp_mode")]
    pub dlp_mode: String,
    /// Request-log retention in days (mirrored into gateway_settings for the
    /// sidecar); 0 keeps every row, which is the default — capture is the
    /// point of the feature, and a retention nobody chose is a silent cap on
    /// it. Same spelling as the stream timeouts below.
    #[serde(default)]
    pub log_retention_days: u32,
    /// Per-body capture cap in bytes (mirrored into gateway_settings); 0
    /// stores every byte, which is the default. The cap exists to bound two
    /// things at once — the row on disk and the buffer a streaming response is
    /// held in until it ends — so an install that sets none should know it is
    /// buying completeness with memory.
    #[serde(default)]
    pub log_max_body_bytes: u32,
    /// How long an upstream stream may take to its first byte before the
    /// gateway abandons it (mirrored into gateway_settings; 0 disables).
    #[serde(default = "default_stream_first_byte_secs")]
    pub stream_first_byte_secs: u32,
    /// How long a stream may stay silent between bytes before the gateway
    /// abandons it (0 disables).
    #[serde(default = "default_stream_idle_secs")]
    pub stream_idle_secs: u32,
    /// Cost-alert toggle (spec §4.1 P1): system notification when usage hits the per-period limit
    #[serde(default = "default_true")]
    pub cost_alert: bool,
    pub hub_logged_in: bool,
    #[serde(default = "default_hub_url")]
    pub hub_url: String,
    /// Preferred display currency (ISO code). Only the Dashboard converts into
    /// it, via the Hub's published exchange rates — rolling per-currency
    /// buckets into one number is what it is for. Everywhere else an amount
    /// keeps the currency it was priced in.
    #[serde(default = "default_preferred_currency")]
    pub preferred_currency: String,
    /// Auto-check for app updates at startup (opt-out; silent, notification only).
    #[serde(default = "default_true")]
    pub auto_check_update: bool,
    /// Version the user closed in the update banner. Remembered so one release
    /// does not re-announce itself on every launch — a newer one will show.
    #[serde(default)]
    pub dismissed_update: Option<String>,
    /// Minutes east of UTC (UTC+8 → 480). Every day boundary in the UI — the
    /// dashboard's "today", its daily chart buckets, the footer, and usage-alert
    /// reset periods — is the user's day, not UTC's. The frontend keeps it
    /// current; 0 (UTC) is the fallback for a settings blob written before this
    /// existed.
    #[serde(default)]
    pub tz_offset_minutes: i64,
    /// The Models list's sort choice, remembered across sessions: `"<key>:<dir>"`
    /// with key in name|protocol|tag|price and dir 1|-1. None means the default
    /// order (providers you already have, then tag rank, then name). A plain
    /// string rather than a struct on purpose — a value written by a build that
    /// knows a sort key this one does not simply falls back to the default.
    #[serde(default)]
    pub shelf_sort: Option<String>,
    /// Which Models view the user last chose: `"model"` for the one grouped by
    /// model, anything else (None) for the provider table. Remembered for the
    /// same reason as `shelf_sort`, only more so — the shelf is remounted after
    /// every mutation, so a view held in component state would snap back the
    /// moment the user added a provider while browsing models.
    #[serde(default)]
    pub shelf_view: Option<String>,
    // ── Features panel (docs/request-logs-applications.md): every flag is an
    // opt-in, default off. None of them reach the gateway's forward path, so
    // none are mirrored into gateway_settings. ──
    /// Alert when the month's current spend slope projects past a provider's
    /// currency limit, before the limit is actually hit.
    #[serde(default)]
    pub feat_cost_forecast: bool,
    /// Alert on error-rate spikes, latency outliers and traffic bursts,
    /// measured against the same hour's trailing-7-day baseline.
    #[serde(default)]
    pub feat_anomaly_alerts: bool,
    /// Alert when an agent hits its own per-period budget (agent_limits) —
    /// the enforcement is the gateway's, this is the missing notification.
    #[serde(default)]
    pub feat_agent_limit_alerts: bool,
    /// Let agents query their own aggregate stats over MCP (`kiwano mcp`).
    /// Aggregates only; bodies never leave the machine.
    #[serde(default)]
    pub feat_mcp_self_query: bool,
    /// Allow `kiwano rules apply` to append insights-derived rules to the
    /// agent's CLAUDE.md/AGENTS.md (backed up, reversible).
    #[serde(default)]
    pub feat_rule_injection: bool,
    /// Add tuning suggestions (retry budgets, route health) to the insights
    /// report. Advice only — nothing is reconfigured.
    #[serde(default)]
    pub feat_tuning_advice: bool,
    /// Enable the cache-shaping offline experiment (`kiwano cache-experiment`).
    #[serde(default)]
    pub feat_cache_experiment: bool,
}

pub fn default_preferred_currency() -> String {
    "CNY".into()
}

pub fn default_hub_url() -> String {
    "https://hub.kiwano.cc/catalog.json".into()
}

pub fn default_true() -> bool {
    true
}

pub fn default_dlp_mode() -> String {
    "alert".to_string()
}

pub fn default_stream_first_byte_secs() -> u32 {
    120
}

pub fn default_stream_idle_secs() -> u32 {
    120
}

impl Default for SettingsVm {
    fn default() -> Self {
        Self {
            // Not a locale: the UI resolves this one from the OS. Defaulting to
            // a concrete language would decide the reader's language for them
            // on first run, and an existing install carries whatever it stored.
            language: "system".into(),
            theme: "dark".into(),
            autostart: true,
            close_to_tray: true,
            gateway_listen: "127.0.0.1:8317".into(),
            takeovers: Vec::new(),
            custom_agents: Vec::new(),
            auto_failover: true,
            request_logs: true,
            compat_shim: true,
            dlp_mode: default_dlp_mode(),
            log_retention_days: 0,
            log_max_body_bytes: 0,
            stream_first_byte_secs: default_stream_first_byte_secs(),
            stream_idle_secs: default_stream_idle_secs(),
            cost_alert: true,
            hub_logged_in: false,
            hub_url: default_hub_url(),
            preferred_currency: default_preferred_currency(),
            auto_check_update: true,
            dismissed_update: None,
            tz_offset_minutes: 0,
            shelf_sort: None,
            shelf_view: None,
            feat_cost_forecast: false,
            feat_anomaly_alerts: false,
            feat_agent_limit_alerts: false,
            feat_mcp_self_query: false,
            feat_rule_injection: false,
            feat_tuning_advice: false,
            feat_cache_experiment: false,
        }
    }
}
