//! View-model layer: maps the gateway `Store` rows to the frontend contract
//! in `src/api/types.ts` (field names must match exactly — serde default
//! snake_case). The UI is mock-free; this is the single source of mapping.
//!
//! Owns an auxiliary SQLite connection on the same database file for reads
//! the gateway store does not expose (average latency, per-provider daily
//! sparkline) plus a GUI-scoped `app_settings` table.

use std::collections::HashMap;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use kiwano_gateway::store::{
    Billing, Binding, HealthRecord, Provider, RequestLogDetail, RequestLogEntry, RequestLogFilter,
    Store, Strategy, StrategyType, UsageTotals,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const AGENTS: [(&str, &str); 9] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
    ("grokbuild", "Grok Build"),
    ("claude-desktop", "Claude Desktop"),
    ("opencode", "OpenCode"),
    ("openclaw", "OpenClaw"),
    ("hermes", "Hermes"),
    ("pi", "Pi"),
];

/// Additive-mode agents: their native config keeps multiple providers
/// coexisting, so takeover writes a gateway-pointed provider entry and selects
/// it, instead of replacing an exclusive provider slot like the other five.
pub const ADDITIVE_AGENTS: [&str; 4] = ["opencode", "openclaw", "hermes", "pi"];

const PALETTE: [&str; 6] = [
    "#4D6BFE", "#615CED", "#3859FF", "#F55036", "#6467F2", "#0F9D58",
];

fn palette_color(name: &str) -> &'static str {
    let h: u64 = name.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
    PALETTE[(h as usize) % PALETTE.len()]
}

fn logo_char(name: &str) -> String {
    name.chars()
        .next()
        .unwrap_or('?')
        .to_uppercase()
        .to_string()
}

/// Token formatting, mirroring `src/lib/format.ts`.
pub fn fmt_tokens(v: i64) -> String {
    if v >= 1_000_000 {
        let m = v as f64 / 1_000_000.0;
        if m >= 10.0 {
            format!("{}M", m.round() as i64)
        } else {
            format!("{}M", (m * 10.0).round() / 10.0)
        }
    } else if v >= 1_000 {
        format!("{}k", (v as f64 / 1_000.0).round() as i64)
    } else {
        v.to_string()
    }
}

// ── UTC date helpers (no chrono dependency; RFC3339 UTC keeps lexicographic
//    ordering, which is exactly what the store's `ts >= ?` filters expect) ──

pub(crate) fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

/// Days-since-epoch → (y, m, d), Howard Hinnant's civil_from_days.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

pub(crate) fn rfc3339(epoch_secs: i64) -> String {
    let days = epoch_secs.div_euclid(86_400);
    let secs = epoch_secs.rem_euclid(86_400);
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

fn day_key(epoch_secs: i64) -> String {
    let (y, m, d) = civil_from_days(epoch_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `MM-DD` label for the dashboard trend axis.
fn mmdd(day: &str) -> String {
    day.get(5..10).unwrap_or(day).to_string()
}

/// Start of the current reset period (RFC3339 UTC, for `ts >= ?` filters)
/// plus a dedup key. Returns (since, period_key); reset_period NULL = no
/// reset → (None, "all"). Day math: 1970-01-01 was a Thursday, so
/// `(days + 3) % 7 == 0` lands on Monday.
fn period_start(epoch_secs: i64, reset_period: Option<&str>) -> (Option<String>, String) {
    let days = epoch_secs.div_euclid(86_400);
    let (y, m, _) = civil_from_days(days);
    match reset_period {
        None => (None, "all".into()),
        Some("weekly") => {
            let monday = days - (days + 3).rem_euclid(7);
            let (wy, wm, wd) = civil_from_days(monday);
            let key = format!("{wy:04}-{wm:02}-{wd:02}");
            (Some(format!("{key}T00:00:00Z")), key)
        }
        Some("yearly") => {
            let key = format!("{y:04}");
            (Some(format!("{key}-01-01T00:00:00Z")), key)
        }
        // monthly and any unexpected values all fall back to monthly
        _ => {
            let key = format!("{y:04}-{m:02}");
            (Some(format!("{key}-01T00:00:00Z")), key)
        }
    }
}

// ── Auxiliary connection (same DB file, GUI-scoped tables + extra reads) ──

pub struct Aux {
    pub conn: Mutex<Connection>,
}

impl Aux {
    pub fn open(path: impl AsRef<std::path::Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        // The gateway sidecar holds the same file with a 5s busy timeout; the
        // GUI writes settings concurrently, so mirror it to survive lock races.
        conn.busy_timeout(std::time::Duration::from_secs(5))?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_tables(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             )",
            [],
        )?;
        // takeover backups (tech.md §4.3-3): files = JSON [[path, content], ...]
        conn.execute(
            "CREATE TABLE IF NOT EXISTS takeover_backups (
                 agent         TEXT PRIMARY KEY,
                 files         TEXT NOT NULL,
                 backed_up_at  TEXT NOT NULL
             )",
            [],
        )?;
        // Hub catalog cache (tech.md §3 Hub sync): single-row cache, payload = CatalogListVm JSON
        conn.execute(
            "CREATE TABLE IF NOT EXISTS hub_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
             )",
            [],
        )?;
        Ok(())
    }

    pub fn save_takeover_backup(
        &self,
        agent: &str,
        files: &[(String, String)],
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let json = serde_json::to_string(files).expect("serialize backup files");
        conn.execute(
            "INSERT INTO takeover_backups (agent, files, backed_up_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(agent) DO UPDATE SET files = ?2, backed_up_at = ?3",
            rusqlite::params![agent, json, rfc3339(unix_now())],
        )?;
        Ok(())
    }

    pub fn load_takeover_backup(&self, agent: &str) -> Option<(String, Vec<(String, String)>)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let (ts, json) = conn
            .query_row(
                "SELECT backed_up_at, files FROM takeover_backups WHERE agent = ?1",
                [agent],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()?;
        let files: Vec<(String, String)> = serde_json::from_str(&json).ok()?;
        Some((ts, files))
    }

    pub fn delete_takeover_backup(&self, agent: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute("DELETE FROM takeover_backups WHERE agent = ?1", [agent])?;
        Ok(())
    }

    pub fn load_settings_json(&self) -> Option<serde_json::Value> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = 'ui'",
            [],
            |row| row.get::<_, String>(0),
        )
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
    }

    pub fn save_settings_json(&self, v: &serde_json::Value) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES ('ui', ?1)
             ON CONFLICT(key) DO UPDATE SET value = ?1",
            [v.to_string()],
        )?;
        Ok(())
    }

    /// Generic KV read (app_settings table; used for cost-alert dedup etc.).
    pub fn get_setting(&self, key: &str) -> Option<String> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT value FROM app_settings WHERE key = ?1",
            [key],
            |row| row.get(0),
        )
        .ok()
    }

    pub fn set_setting(&self, key: &str, value: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO app_settings (key, value) VALUES (?1, ?2)
             ON CONFLICT(key) DO UPDATE SET value = ?2",
            [key, value],
        )?;
        Ok(())
    }

    /// Hub catalog cache: single-row upsert (RFC3339 synced_at).
    pub fn save_hub_cache(&self, payload: &str, synced_at: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO hub_cache (id, payload, synced_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = ?1, synced_at = ?2",
            rusqlite::params![payload, synced_at],
        )?;
        Ok(())
    }

    /// `(payload, synced_at)`; None when never synced.
    pub fn load_hub_cache(&self) -> Option<(String, String)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT payload, synced_at FROM hub_cache WHERE id = 1",
            [],
            |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
        )
        .ok()
    }

    /// Average `latency_ms` over a window, optionally per provider.
    /// `from`/`to` are RFC3339 (store ts strings compare lexicographically).
    pub fn avg_latency(
        &self,
        provider: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Option<i64> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut sql =
            String::from("SELECT AVG(latency_ms) FROM usage WHERE latency_ms IS NOT NULL");
        if provider.is_some() {
            sql.push_str(" AND provider_id = ?1");
        }
        if from.is_some() {
            sql.push_str(&format!(
                " AND ts >= ?{}",
                if provider.is_some() { 2 } else { 1 }
            ));
        }
        if to.is_some() {
            let base = 1 + provider.is_some() as i32 + from.is_some() as i32;
            sql.push_str(&format!(" AND ts < ?{base}"));
        }
        let mut stmt = conn.prepare(&sql).ok()?;
        let map = |r: &rusqlite::Row| r.get::<_, Option<f64>>(0);
        let avg: Option<f64> = match (provider, from, to) {
            (Some(p), Some(f), Some(t)) => stmt.query_row(rusqlite::params![p, f, t], map).ok()?,
            (Some(p), Some(f), None) => stmt.query_row(rusqlite::params![p, f], map).ok()?,
            (Some(p), None, Some(t)) => stmt.query_row(rusqlite::params![p, t], map).ok()?,
            (Some(p), None, None) => stmt.query_row(rusqlite::params![p], map).ok()?,
            (None, Some(f), Some(t)) => stmt.query_row(rusqlite::params![f, t], map).ok()?,
            (None, Some(f), None) => stmt.query_row(rusqlite::params![f], map).ok()?,
            (None, None, Some(t)) => stmt.query_row(rusqlite::params![t], map).ok()?,
            (None, None, None) => stmt.query_row([], map).ok()?,
        };
        avg.map(|a| a.round() as i64)
    }

    /// Per-UTC-day token totals (input+output) for one provider.
    pub fn provider_daily(&self, provider_id: &str, since: &str) -> Vec<(String, i64)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut stmt = match conn.prepare(
            "SELECT SUBSTR(ts, 1, 10) AS day,
                    SUM(input_tokens + output_tokens)
             FROM usage WHERE provider_id = ?1 AND ts >= ?2
             GROUP BY day ORDER BY day ASC",
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };
        let map = |r: &rusqlite::Row| -> rusqlite::Result<(String, i64)> {
            Ok((r.get(0)?, r.get::<_, Option<i64>>(1)?.unwrap_or(0)))
        };
        let rows = stmt.query_map(rusqlite::params![provider_id, since], map);
        match rows {
            Ok(rows) => rows.filter_map(|r| r.ok()).collect(),
            Err(_) => Vec::new(),
        }
    }
}

// ── VM types (serde field names mirror src/api/types.ts verbatim) ──

#[derive(Serialize)]
pub struct HealthVm {
    pub state: String,
    pub latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

#[derive(Serialize)]
pub struct QuotaVm {
    pub used: i64,
    pub limit: f64,
    pub unit: String,
    pub resets_at: Option<String>,
}

#[derive(Serialize)]
pub struct UsageVm {
    pub requests: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost: Option<f64>,
    pub latency_ms: Option<i64>,
    pub quota: Option<QuotaVm>,
    pub spark: Option<Vec<f64>>,
}

#[derive(Serialize)]
pub struct ProviderVm {
    pub id: String,
    pub name: String,
    pub logo_char: String,
    pub logo_color: String,
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub logo_border: bool,
    pub endpoint: String,
    pub protocol: String,
    pub endpoint_note: String,
    pub billing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_price: Option<String>,
    pub enabled: bool,
    pub agents: Vec<String>,
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_badge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_note: Option<String>,
    pub health: HealthVm,
    pub usage: Option<UsageVm>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEntryVm {
    pub id: String,
    pub name: String,
    pub logo_char: String,
    pub logo_color: String,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub logo_border: bool,
    /// Brand-mark key in the frontend icon registry (cc-switch port);
    /// absent entries fall back to the letter avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    /// Protocol fingerprint (anthropic | openai | gemini); entries predate the
    /// multi-protocol catalog, so older payloads default to openai.
    #[serde(default = "default_catalog_protocol")]
    pub protocol: String,
    pub tag: String,
    pub tag_label: String,
    pub rating: f64,
    pub endpoint: String,
    pub price_line: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_note: Option<String>,
    pub billing: String,
    pub users: String,
    pub blurb: String,
    pub added: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_offer: Option<String>,
    pub models: Vec<String>,
}

#[derive(Serialize, Deserialize)]
pub struct CatalogListVm {
    pub total: i64,
    pub entries: Vec<CatalogEntryVm>,
}

fn default_catalog_protocol() -> String {
    "openai".to_string()
}

/// Result of a manual/startup Hub sync (for UI feedback).
#[derive(Serialize)]
pub struct SyncReportVm {
    pub fetched: i64,
    pub synced_at: String,
    pub hub_url: String,
}

#[derive(Serialize)]
pub struct TrendVm {
    pub date: String,
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Serialize)]
pub struct ProviderDistVm {
    pub name: String,
    pub color: String,
    pub pct: i64,
    pub cost: f64,
}

#[derive(Serialize)]
pub struct AgentDistVm {
    pub agent: String,
    pub label: String,
    pub requests: i64,
    pub tokens: String,
    pub cost: f64,
}

#[derive(Serialize)]
pub struct DashboardVm {
    pub window: String,
    pub requests: i64,
    pub requests_delta_pct: i64,
    pub input_tokens: i64,
    pub cache_read_tokens: i64,
    pub output_tokens: i64,
    pub cost: f64,
    pub latency_ms: i64,
    pub latency_delta_pct: i64,
    pub trend: Vec<TrendVm>,
    pub by_provider: Vec<ProviderDistVm>,
    pub by_agent: Vec<AgentDistVm>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct TakeoverVm {
    pub agent: String,
    pub label: String,
    pub placeholder_key: Option<String>,
    pub enabled: bool,
    /// Additive-mode agent (config keeps multiple providers; takeover writes a
    /// gateway entry and selects it) rather than exclusive-switch mode.
    #[serde(default)]
    pub additive: bool,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct SettingsVm {
    pub language: String,
    pub theme: String,
    pub autostart: bool,
    pub close_to_tray: bool,
    pub gateway_listen: String,
    pub takeovers: Vec<TakeoverVm>,
    pub auto_failover: bool,
    pub request_logs: bool,
    /// Request-log retention in days (mirrored into gateway_settings for the sidecar).
    #[serde(default = "default_retain_days")]
    pub log_retention_days: u32,
    pub telemetry: bool,
    /// Cost-alert toggle (spec §4.1 P1): system notification when usage hits the per-period limit
    #[serde(default = "default_true")]
    pub cost_alert: bool,
    pub hub_logged_in: bool,
    #[serde(default = "default_hub_url")]
    pub hub_url: String,
}

pub(crate) fn default_hub_url() -> String {
    crate::sync::DEFAULT_HUB_URL.into()
}

impl Default for SettingsVm {
    fn default() -> Self {
        Self {
            language: "zh-CN".into(),
            theme: "dark".into(),
            autostart: true,
            close_to_tray: true,
            gateway_listen: "127.0.0.1:8317".into(),
            takeovers: Vec::new(),
            auto_failover: true,
            request_logs: true,
            log_retention_days: default_retain_days(),
            telemetry: false,
            cost_alert: true,
            hub_logged_in: false,
            hub_url: default_hub_url(),
        }
    }
}

pub(crate) fn default_true() -> bool {
    true
}

pub(crate) fn default_retain_days() -> u32 {
    30
}

#[derive(Serialize)]
pub struct GatewayStatusVm {
    pub running: bool,
    pub port: u16,
}

#[derive(Serialize)]
pub struct FooterStatsVm {
    pub today_requests: i64,
    pub today_cost: f64,
    pub hub_synced: bool,
    pub version: String,
}

#[derive(Deserialize)]
pub struct BillingConfigInput {
    pub limit_value: Option<f64>,
    #[allow(dead_code)]
    pub limit_unit: Option<String>,
    pub reset_period: Option<String>,
}

#[derive(Deserialize)]
pub struct NewProviderInput {
    pub name: String,
    #[allow(dead_code)]
    pub api_key: String,
    pub endpoint: String,
    pub protocol: String,
    #[allow(dead_code)]
    pub model_default: String,
    pub billing: String,
    pub billing_config: BillingConfigInput,
    pub agents: Vec<String>,
}

// ── Billing mapping (UI plan/payg/unl ↔ DB subscription/metered/unlimited) ──

fn billing_to_db(ui: &str) -> Billing {
    match ui {
        "plan" => Billing::Subscription,
        "unl" => Billing::Unlimited,
        _ => Billing::Metered,
    }
}

fn billing_to_ui(db: Billing) -> &'static str {
    match db {
        Billing::Subscription => "plan",
        Billing::Unlimited => "unl",
        Billing::Metered => "payg",
    }
}

// ── Sparkline normalization: y coords in the 80×14 viewBox, 1..13 ──

fn normalize_spark(values: &[i64]) -> Option<Vec<f64>> {
    let max = values.iter().max().copied()?;
    if values.is_empty() {
        return None;
    }
    Some(
        values
            .iter()
            .map(|v| {
                if max == 0 {
                    12.0
                } else {
                    (12.0 - 10.0 * (*v as f64 / max as f64)).clamp(2.0, 12.0)
                }
            })
            .collect(),
    )
}

// ── Provider view assembly ──

pub fn build_provider_vms(store: &Store, aux: &Aux) -> Result<Vec<ProviderVm>, String> {
    let providers = store.list_providers().map_err(e2s)?;
    if providers.is_empty() {
        return Ok(Vec::new());
    }

    let now = unix_now();
    let since7 = rfc3339(now - 7 * 86_400);

    // agent → primary provider id (single strategy)
    let mut primary: HashMap<String, String> = HashMap::new();
    for agent in store.bound_agents().map_err(e2s)? {
        if let Some(id) = store.primary_provider_id(&agent).map_err(e2s)? {
            primary.insert(agent, id);
        }
    }

    // agent → bindings (to read priorities for the backup #N badges)
    let mut bindings_by_agent: HashMap<String, Vec<Binding>> = HashMap::new();
    for agent in primary.keys() {
        if let Ok(bs) = store.bindings_for_agent(agent) {
            bindings_by_agent.insert(agent.clone(), bs);
        }
    }

    // provider → 7d usage totals
    let mut usage_by_id: HashMap<String, UsageTotals> = HashMap::new();
    for pu in store.usage_by_provider(None, Some(&since7)).map_err(e2s)? {
        usage_by_id.insert(pu.provider_id, pu.totals);
    }

    let vms = providers
        .into_iter()
        .map(|p| {
            let mut agents: Vec<String> = Vec::new();
            let mut badge: Option<String> = None;
            let mut backup_for_any = false;
            for (agent, _) in primary.iter() {
                let is_bound = bindings_by_agent
                    .get(agent)
                    .is_some_and(|bs| bs.iter().any(|b| b.provider_id == p.id));
                if !is_bound {
                    continue;
                }
                agents.push(agent.clone());
                if primary.get(agent).map(String::as_str) != Some(&p.id) {
                    backup_for_any = true;
                    if badge.is_none() {
                        let pr = bindings_by_agent
                            .get(agent)
                            .and_then(|bs| {
                                bs.iter()
                                    .find(|b| b.provider_id == p.id)
                                    .map(|b| b.priority)
                            })
                            .unwrap_or(1);
                        badge = Some(format!("Standby #{pr}"));
                    }
                }
            }
            agents.sort();
            let is_current = agents
                .iter()
                .any(|a| primary.get(a).map(String::as_str) == Some(&p.id));

            let note = if backup_for_any {
                Some("Failover queue".to_string())
            } else if !agents.is_empty() {
                Some(format!("{} agent(s)", agents.len()))
            } else {
                None
            };

            let health = health_vm(store, &p);
            let usage = usage_vm(aux, &p, usage_by_id.get(&p.id), &since7);

            ProviderVm {
                id: p.id.clone(),
                name: p.name.clone(),
                logo_char: logo_char(&p.name),
                logo_color: palette_color(&p.name).to_string(),
                logo_border: false,
                endpoint: display_endpoint(&p),
                protocol: p.protocol.as_str().to_string(),
                endpoint_note: endpoint_note(&p),
                billing: billing_to_ui(p.billing).to_string(),
                plan_price: None, // no price metadata until Hub price tables land
                enabled: p.enabled,
                agents,
                is_current,
                status_badge: badge,
                agents_note: note,
                health,
                usage,
            }
        })
        .collect();

    Ok(vms)
}

// ── Agent strategy views (tech.md §4.7: strategy types + candidate ordering) ──

/// UI projection of agent_strategies + agent_bindings.
#[derive(Serialize)]
pub struct BindingVm {
    pub provider_id: String,
    pub provider_name: String,
    pub logo_char: String,
    pub logo_color: String,
    pub priority: i64,
    pub weight: i64,
    pub enabled: bool,
}

#[derive(Serialize)]
pub struct AgentRouteVm {
    pub agent: String,
    /// single | failover | roundrobin | timewindow | quota
    pub strategy: String,
    /// Strategy JSON payload (quota: {"limit","unit"}; null otherwise)
    pub config: Option<String>,
    /// Candidates in ascending priority order (index 0 = primary)
    pub bindings: Vec<BindingVm>,
}

/// One row per Agent (only agents with bindings); strategy defaults to single.
pub fn build_agent_routes(store: &Store) -> Result<Vec<AgentRouteVm>, String> {
    let providers: HashMap<String, Provider> = store
        .list_providers()
        .map_err(e2s)?
        .into_iter()
        .map(|p| (p.id.clone(), p))
        .collect();

    let mut routes = Vec::new();
    for agent in store.bound_agents().map_err(e2s)? {
        let strategy = store
            .get_strategy(&agent)
            .map_err(e2s)?
            .unwrap_or(Strategy {
                agent: agent.clone(),
                kind: StrategyType::Single,
                config: None,
            });
        let bindings = store
            .bindings_for_agent(&agent)
            .map_err(e2s)?
            .into_iter()
            .map(|b| {
                let name = providers
                    .get(&b.provider_id)
                    .map(|p| p.name.clone())
                    .unwrap_or_else(|| b.provider_id.clone());
                BindingVm {
                    logo_char: logo_char(&name),
                    logo_color: palette_color(&name).to_string(),
                    provider_name: name,
                    provider_id: b.provider_id,
                    priority: b.priority,
                    weight: b.weight,
                    enabled: b.enabled,
                }
            })
            .collect();
        routes.push(AgentRouteVm {
            strategy: strategy.kind.as_str().to_string(),
            config: strategy.config,
            agent,
            bindings,
        });
    }
    Ok(routes)
}

/// Update an Agent's strategy type (+ optional JSON config); unknown types error.
pub fn set_agent_strategy(
    store: &Store,
    agent: &str,
    strategy: &str,
    config: Option<&str>,
) -> Result<(), String> {
    let kind = StrategyType::from_str(strategy)
        .ok_or_else(|| format!("unknown strategy type: {strategy}"))?;
    store.upsert_strategy(agent, kind, config).map_err(e2s)?;
    Ok(())
}

/// Candidate reorder: given a provider_id order → rewrite priority 0..n
/// (weight/window/enabled bits preserved). Unlisted bindings stay; unknown
/// provider_ids error.
pub fn reorder_agent_bindings(
    store: &Store,
    agent: &str,
    provider_ids: &[String],
) -> Result<(), String> {
    let existing: HashMap<String, Binding> = store
        .bindings_for_agent(agent)
        .map_err(e2s)?
        .into_iter()
        .map(|b| (b.provider_id.clone(), b))
        .collect();
    for (i, pid) in provider_ids.iter().enumerate() {
        let Some(mut b) = existing.get(pid).cloned() else {
            return Err(format!("provider {pid} is not bound to {agent}"));
        };
        b.priority = i as i64;
        store.upsert_binding(&b).map_err(e2s)?;
    }
    Ok(())
}

fn e2s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn display_endpoint(p: &Provider) -> String {
    let stripped = p
        .base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match &p.api_path {
        Some(path) if !path.is_empty() => format!("{stripped}{path}"),
        _ => stripped.to_string(),
    }
}

fn endpoint_note(p: &Provider) -> String {
    match p.protocol {
        kiwano_gateway::store::Protocol::OpenAI => "OpenAI-compatible".to_string(),
        kiwano_gateway::store::Protocol::Anthropic => "Anthropic".to_string(),
        kiwano_gateway::store::Protocol::Gemini => "Gemini API".to_string(),
    }
}

fn health_vm(store: &Store, p: &Provider) -> HealthVm {
    let rec = store.get_health(&p.id).ok().flatten();
    match rec {
        Some(HealthRecord {
            status,
            last_latency_ms,
            ..
        }) => match status.as_str() {
            "healthy" => HealthVm {
                state: "ok".into(),
                latency_ms: last_latency_ms,
                note: None,
            },
            "degraded" => HealthVm {
                state: "idle".into(),
                latency_ms: last_latency_ms,
                note: None,
            },
            "down" => HealthVm {
                state: "off".into(),
                latency_ms: last_latency_ms,
                note: Some("Error".into()),
            },
            _ => derive_health(p, last_latency_ms),
        },
        None => derive_health(p, None),
    }
}

fn derive_health(p: &Provider, latency: Option<i64>) -> HealthVm {
    if p.enabled {
        HealthVm {
            state: "idle".into(),
            latency_ms: latency,
            note: None,
        }
    } else {
        HealthVm {
            state: "off".into(),
            latency_ms: None,
            note: Some("Disabled".into()),
        }
    }
}

/// Normalize the user-entered per-period limit unit (tech.md §2.4 A). With a
/// limit set but no unit chosen, fall back to the legacy behavior of counting
/// "requests"; with no limit the unit is meaningless and stored as NULL.
fn normalize_limit_unit(unit: Option<&str>, has_limit: bool) -> Option<String> {
    if !has_limit {
        return None;
    }
    Some(
        match unit {
            Some("wan_tokens") => "wan_tokens",
            Some("cny") => "cny",
            _ => "requests",
        }
        .to_string(),
    )
}

fn usage_vm(
    aux: &Aux,
    p: &Provider,
    totals: Option<&UsageTotals>,
    since7: &str,
) -> Option<UsageVm> {
    let t = totals?;
    let quota = match (p.billing, p.limit_unit.as_deref()) {
        // CNY amount limits can't be estimated without a price table (wired
        // up when Hub price tables land); ring not shown
        (Billing::Subscription, Some("cny")) => None,
        (Billing::Subscription, unit) => p.period_limit.map(|limit| {
            let used = match unit {
                Some("wan_tokens") => {
                    (t.input_tokens
                        + t.output_tokens
                        + t.cache_read_tokens
                        + t.cache_creation_tokens) as f64
                        / 10_000.0
                }
                _ => t.requests as f64,
            };
            QuotaVm {
                used: used as i64,
                limit,
                unit: unit.unwrap_or("requests").to_string(),
                resets_at: None, // reset-cycle tracking lands with the quota strategy (P2)
            }
        }),
        _ => None,
    };
    let spark = match quota {
        None => normalize_spark(
            &aux.provider_daily(&p.id, since7)
                .into_iter()
                .map(|(_, v)| v)
                .collect::<Vec<_>>(),
        ),
        Some(_) => None,
    };
    Some(UsageVm {
        requests: t.requests,
        input_tokens: t.input_tokens,
        cache_read_tokens: t.cache_read_tokens,
        output_tokens: t.output_tokens,
        cost: None, // cost estimation needs per-provider price tables (Hub, P1)
        latency_ms: aux.avg_latency(Some(&p.id), Some(since7), None),
        quota,
        spark,
    })
}

// ── Mutations (called from commands; each ends with an admin /reload) ──

pub(crate) fn slug(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() {
        "provider".into()
    } else {
        trimmed.to_string()
    }
}

pub fn add_provider(store: &Store, input: &NewProviderInput) -> Result<ProviderVm, String> {
    let now = rfc3339(unix_now());
    let id = format!(
        "{}-{}",
        slug(&input.name),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let reset_period = match input.billing_config.reset_period.as_deref() {
        Some("monthly") | Some("weekly") | Some("yearly") => {
            input.billing_config.reset_period.clone()
        }
        _ => None,
    };
    let provider = Provider {
        id: id.clone(),
        name: input.name.trim().to_string(),
        protocol: kiwano_gateway::store::Protocol::from_str(&input.protocol)
            .unwrap_or(kiwano_gateway::store::Protocol::OpenAI),
        base_url: input.endpoint.trim().to_string(),
        api_path: None,
        api_key: Some(input.api_key.clone()),
        billing: billing_to_db(&input.billing),
        period_limit: input.billing_config.limit_value,
        limit_unit: normalize_limit_unit(
            input.billing_config.limit_unit.as_deref(),
            input.billing_config.limit_value.is_some(),
        ),
        reset_period,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    store.insert_provider(&provider).map_err(e2s)?;

    // compute view fields before partially moving `provider`
    let vm_name = provider.name.clone();
    let vm_endpoint = display_endpoint(&provider);
    let vm_note = endpoint_note(&provider);
    let vm_protocol = provider.protocol.as_str().to_string();
    let vm_billing = billing_to_ui(provider.billing).to_string();
    let vm_health = derive_health(&provider, None);

    for agent in &input.agents {
        store
            .upsert_strategy(agent, StrategyType::Single, None)
            .map_err(e2s)?;
        // "Save & Enable" → becomes the primary for the chosen agents; the
        // previous primary is demoted to backup #1.
        let prev = store.primary_provider_id(agent).map_err(e2s)?;
        store
            .upsert_binding(&Binding {
                agent: agent.clone(),
                provider_id: id.clone(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(e2s)?;
        if let Some(prev) = prev {
            if prev != id {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: prev,
                        priority: 1,
                        weight: 1,
                        win_start: None,
                        win_end: None,
                        enabled: true,
                    })
                    .map_err(e2s)?;
            }
        }
    }
    Ok(ProviderVm {
        id,
        name: vm_name,
        logo_char: logo_char(&input.name),
        logo_color: palette_color(&input.name).to_string(),
        logo_border: false,
        endpoint: vm_endpoint,
        protocol: vm_protocol,
        endpoint_note: vm_note,
        billing: vm_billing,
        plan_price: None,
        enabled: true,
        agents: input.agents.clone(),
        is_current: !input.agents.is_empty(),
        status_badge: None,
        agents_note: (!input.agents.is_empty()).then(|| format!("{} agent(s)", input.agents.len())),
        health: vm_health,
        usage: None,
    })
}

/// Enable = make this provider the primary of every agent it is bound to.
pub fn enable_provider(store: &Store, id: &str) -> Result<(), String> {
    let agents: Vec<String> = store
        .bound_agents()
        .map_err(e2s)?
        .into_iter()
        .filter(|a| {
            store
                .bindings_for_agent(a)
                .map(|bs| bs.iter().any(|b| b.provider_id == id))
                .unwrap_or(false)
        })
        .collect();
    for agent in agents {
        let mut others: Vec<String> = store
            .bindings_for_agent(&agent)
            .map_err(e2s)?
            .into_iter()
            .map(|b| b.provider_id)
            .filter(|p| p != id)
            .collect();
        store
            .upsert_binding(&Binding {
                agent: agent.clone(),
                provider_id: id.to_string(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(e2s)?;
        for (i, p) in others.drain(..).enumerate() {
            store
                .upsert_binding(&Binding {
                    agent: agent.clone(),
                    provider_id: p,
                    priority: i as i64 + 1,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(e2s)?;
        }
    }
    Ok(())
}

/// Update provider: rewrite the providers row + rebind agents (the new set
/// becomes primary; removed ones are unbound). An empty api_key means keep
/// the existing key. Returns the refreshed VM (re-aggregated so badges and
/// notes stay consistent).
pub fn update_provider(
    store: &Store,
    aux: &Aux,
    id: &str,
    input: &NewProviderInput,
) -> Result<ProviderVm, String> {
    let mut p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider `{id}` not found"))?;

    p.name = input.name.trim().to_string();
    p.base_url = input.endpoint.trim().to_string();
    p.protocol = kiwano_gateway::store::Protocol::from_str(&input.protocol)
        .unwrap_or(kiwano_gateway::store::Protocol::OpenAI);
    p.billing = billing_to_db(&input.billing);
    p.period_limit = input.billing_config.limit_value;
    p.limit_unit = normalize_limit_unit(
        input.billing_config.limit_unit.as_deref(),
        input.billing_config.limit_value.is_some(),
    );
    p.reset_period = match input.billing_config.reset_period.as_deref() {
        Some("monthly") | Some("weekly") | Some("yearly") => {
            input.billing_config.reset_period.clone()
        }
        _ => None,
    };
    if !input.api_key.trim().is_empty() {
        p.api_key = Some(input.api_key.clone());
    }
    p.updated_at = rfc3339(unix_now());
    store.update_provider(&p).map_err(e2s)?;

    // Rebind: unbind old agents not in the new set; new agents use the same
    // primary logic as add
    let new_set: std::collections::HashSet<&str> =
        input.agents.iter().map(String::as_str).collect();
    let old_agents: Vec<String> = store
        .bound_agents()
        .map_err(e2s)?
        .into_iter()
        .filter(|a| {
            store
                .bindings_for_agent(a)
                .map(|bs| bs.iter().any(|b| b.provider_id == id))
                .unwrap_or(false)
        })
        .collect();
    for agent in &old_agents {
        if !new_set.contains(agent.as_str()) {
            store.delete_binding(agent, id).map_err(e2s)?;
        }
    }
    for agent in &input.agents {
        store
            .upsert_strategy(agent, StrategyType::Single, None)
            .map_err(e2s)?;
        let prev = store.primary_provider_id(agent).map_err(e2s)?;
        store
            .upsert_binding(&Binding {
                agent: agent.clone(),
                provider_id: id.to_string(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(e2s)?;
        if let Some(prev) = prev {
            if prev != id {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: prev,
                        priority: 1,
                        weight: 1,
                        win_start: None,
                        win_end: None,
                        enabled: true,
                    })
                    .map_err(e2s)?;
            }
        }
    }

    let vms = build_provider_vms(store, aux)?;
    vms.into_iter()
        .find(|v| v.id == id)
        .ok_or_else(|| "provider vanished after update".to_string())
}

/// Delete a provider. If it is some Agent's primary, the next-best candidate
/// in that Agent's list is promoted automatically.
pub fn delete_provider(store: &Store, id: &str) -> Result<bool, String> {
    let mut affected: Vec<String> = Vec::new();
    for a in store.bound_agents().map_err(e2s)? {
        if store.primary_provider_id(&a).map_err(e2s)?.as_deref() == Some(id) {
            affected.push(a);
        }
    }
    let deleted = store.delete_provider(id).map_err(e2s)?;
    if !deleted {
        return Ok(false);
    }
    for agent in affected {
        let remaining = store.bindings_for_agent(&agent).map_err(e2s)?;
        if let Some(next) = remaining.first() {
            let next_id = next.provider_id.clone();
            store
                .upsert_binding(&Binding {
                    agent: agent.clone(),
                    provider_id: next_id.clone(),
                    priority: 0,
                    weight: 1,
                    win_start: None,
                    win_end: None,
                    enabled: true,
                })
                .map_err(e2s)?;
            for (i, b) in remaining
                .iter()
                .filter(|b| b.provider_id != next_id)
                .enumerate()
            {
                store
                    .upsert_binding(&Binding {
                        agent: agent.clone(),
                        provider_id: b.provider_id.clone(),
                        priority: i as i64 + 1,
                        weight: b.weight,
                        win_start: b.win_start.clone(),
                        win_end: b.win_end.clone(),
                        enabled: b.enabled,
                    })
                    .map_err(e2s)?;
            }
        }
    }
    Ok(true)
}

// ── Settings ──

/// Read UI settings (used by Rust-side logic like tray/autostart; the
/// store-free part of `build_settings`).
pub(crate) fn ui_settings(aux: &Aux) -> SettingsVm {
    aux.load_settings_json()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default()
}

pub fn build_settings(store: &Store, aux: &Aux) -> Result<SettingsVm, String> {
    let mut s: SettingsVm = ui_settings(aux);
    s.takeovers = AGENTS
        .iter()
        .map(|(agent, label)| {
            let keys = store.list_placeholder_keys().map_err(e2s)?;
            let key = keys
                .iter()
                .find(|k| k.agent == *agent)
                .map(|k| k.key.clone());
            Ok(TakeoverVm {
                agent: agent.to_string(),
                label: label.to_string(),
                enabled: key.is_some(),
                placeholder_key: key,
                additive: ADDITIVE_AGENTS.contains(agent),
            })
        })
        .collect::<Result<Vec<_>, String>>()?;
    Ok(s)
}

pub fn update_settings(
    store: &Store,
    aux: &Aux,
    patch: &serde_json::Value,
) -> Result<SettingsVm, String> {
    let mut merged = aux
        .load_settings_json()
        .unwrap_or_else(|| serde_json::to_value(SettingsVm::default()).expect("default settings"));
    if let (Some(obj), Some(p)) = (merged.as_object_mut(), patch.as_object()) {
        for (k, v) in p {
            // takeovers are managed through set_agent_takeover, not this patch
            if k != "takeovers" {
                obj.insert(k.clone(), v.clone());
            }
        }
    }
    aux.save_settings_json(&merged).map_err(e2s)?;
    // Request-log capture config lives in the shared gateway_settings table:
    // the sidecar reads it at startup and on /reload, so keep both copies in
    // sync whenever the UI patches one of these keys.
    if patch.get("request_logs").is_some() || patch.get("log_retention_days").is_some() {
        let mut cfg = store.load_log_config().unwrap_or_default();
        if let Some(v) = patch.get("request_logs").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        if let Some(v) = patch.get("log_retention_days").and_then(|v| v.as_u64()) {
            if let Ok(days) = u32::try_from(v) {
                cfg.retain_days = days;
            }
        }
        store.save_log_config(&cfg).map_err(e2s)?;
    }
    build_settings(store, aux)
}

// ── Request logs (request_logs + request_bodies, migration V5) ──

#[derive(Serialize)]
pub struct RequestLogListVm {
    pub rows: Vec<RequestLogEntry>,
    pub total: i64,
}

pub fn list_request_logs(
    store: &Store,
    page: i64,
    page_size: i64,
    agent: Option<&str>,
    provider_id: Option<&str>,
    status: Option<&str>,
) -> Result<RequestLogListVm, String> {
    let (rows, total) = store
        .list_request_logs(
            page,
            page_size,
            RequestLogFilter {
                agent,
                provider_id,
                status,
            },
        )
        .map_err(e2s)?;
    Ok(RequestLogListVm { rows, total })
}

/// Detail view (metadata + bodies); re-exported for the command signature.
pub use kiwano_gateway::store::RequestLogDetail as RequestLogDetailVm;

pub fn get_request_log(store: &Store, id: i64) -> Result<Option<RequestLogDetail>, String> {
    store.get_request_log(id).map_err(e2s)
}

pub fn clear_request_logs(store: &Store) -> Result<(), String> {
    store.clear_request_logs().map_err(e2s).map(drop)
}

pub fn set_agent_takeover(
    store: &Store,
    aux: &Aux,
    agent: &str,
    enabled: bool,
    data_port: u16,
    home: &std::path::Path,
) -> Result<(), String> {
    if !AGENTS.iter().any(|(a, _)| *a == agent) {
        return Err(format!("unknown agent: {agent}"));
    }
    if enabled {
        // First-takeover import: pick up the provider the agent is currently
        // using and bind it as the agent's sole candidate, so the gateway has
        // a route on day one (official-login/blank configs yield no creds —
        // the onboarding guide steers those users to manual entry). Import or
        // binding failures never block the takeover itself.
        if let Some(creds) = crate::creds::read_current_creds(agent, home) {
            match import_current_provider(store, &creds) {
                Ok(provider_id) => {
                    if store.bindings_for_agent(agent).map_err(e2s)?.is_empty() {
                        store
                            .upsert_strategy(agent, StrategyType::Single, None)
                            .map_err(e2s)?;
                        store
                            .upsert_binding(&Binding {
                                agent: agent.to_string(),
                                provider_id,
                                priority: 0,
                                weight: 1,
                                win_start: None,
                                win_end: None,
                                enabled: true,
                            })
                            .map_err(e2s)?;
                    }
                }
                Err(e) => eprintln!("kiwano: current-provider import skipped: {e}"),
            }
        }
        let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
        let key = format!("kw-ag-{agent}-{rand}");
        store.upsert_placeholder_key(&key, agent).map_err(e2s)?;
        // Rewrite the Agent config (backup → base_url → placeholder key); on
        // failure roll back the key registration to stay consistent
        if let Err(e) = crate::takeover::enable(aux, agent, &key, data_port, &home) {
            let _ = store.delete_placeholder_key(&key);
            return Err(e);
        }
    } else {
        crate::takeover::disable(aux, agent, &home)?;
        for k in store.list_placeholder_keys().map_err(e2s)? {
            if k.agent == agent {
                store.delete_placeholder_key(&k.key).map_err(e2s)?;
            }
        }
    }
    Ok(())
}

/// Find-or-create a provider for the agent's current credentials: dedup by
/// base_url (trailing slash ignored) reuses the existing row — that shared
/// provider then also serves other agents; otherwise insert a new PAYG row
/// named after the config's provider key (or the URL host).
fn import_current_provider(store: &Store, creds: &crate::creds::CurrentCreds) -> Result<String, String> {
    let base = creds.base_url.trim().trim_end_matches('/');
    for p in store.list_providers().map_err(e2s)? {
        if p.base_url.trim().trim_end_matches('/') == base {
            return Ok(p.id);
        }
    }
    let now = rfc3339(unix_now());
    let name = creds.name.clone().unwrap_or_else(|| {
        let host = crate::creds::host_of(&creds.base_url);
        if host.is_empty() { "Imported provider".into() } else { host }
    });
    let provider = Provider {
        id: format!("{}-{}", slug(&name), &uuid::Uuid::new_v4().simple().to_string()[..6]),
        name,
        protocol: kiwano_gateway::store::Protocol::from_str(creds.protocol)
            .unwrap_or(kiwano_gateway::store::Protocol::OpenAI),
        base_url: base.to_string(),
        api_path: None,
        api_key: Some(creds.api_key.clone()),
        billing: kiwano_gateway::store::Billing::Metered,
        period_limit: None,
        limit_unit: None,
        reset_period: None,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    let id = provider.id.clone();
    store.insert_provider(&provider).map_err(e2s)?;
    Ok(id)
}

// ── Multi-key rotation (spec §4.1 P1: auto-rotate multiple API keys per provider) ──

#[derive(Serialize)]
pub struct ApiKeyVm {
    pub id: i64,
    /// Full key (local app; the frontend handles masking)
    pub api_key: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

pub fn list_api_keys(store: &Store, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
    store
        .list_api_keys(provider_id)
        .map(|rows| {
            rows.into_iter()
                .map(|r| ApiKeyVm {
                    id: r.id,
                    api_key: r.api_key,
                    label: r.label,
                    enabled: r.enabled,
                    created_at: r.created_at,
                })
                .collect()
        })
        .map_err(e2s)
}

/// Append a rotation key (providers.api_key is the primary key, always first in the pool).
pub fn add_api_key(
    store: &Store,
    provider_id: &str,
    api_key: &str,
    label: Option<&str>,
) -> Result<ApiKeyVm, String> {
    let key = api_key.trim();
    if key.is_empty() {
        return Err("API key must not be empty".into());
    }
    store
        .get_provider(provider_id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider `{provider_id}` not found"))?;
    let id = store
        .insert_api_key(
            provider_id,
            key,
            label.map(str::trim).filter(|s| !s.is_empty()),
        )
        .map_err(e2s)?;
    Ok(ApiKeyVm {
        id,
        api_key: key.to_string(),
        label: label
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(String::from),
        enabled: true,
        created_at: rfc3339(unix_now()),
    })
}

pub fn delete_api_key(store: &Store, id: i64) -> Result<bool, String> {
    store.delete_api_key(id).map_err(e2s)
}

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

#[derive(Serialize)]
pub struct UsageAlertVm {
    pub provider_id: String,
    pub provider_name: String,
    pub used: i64,
    pub limit: f64,
    /// requests | wan_tokens
    pub unit: String,
}

/// Check whether enabled providers' usage this period has reached the
/// user-set per-period limit (period_limit). Hits not yet notified this
/// period are recorded under a dedup key and returned (the frontend turns
/// them into system notifications). CNY amount limits have no price table
/// yet (Hub price tables, P1) and are skipped to avoid false positives.
pub fn check_usage_alerts(store: &Store, aux: &Aux) -> Result<Vec<UsageAlertVm>, String> {
    if !ui_settings(aux).cost_alert {
        return Ok(Vec::new());
    }
    let now = unix_now();
    let mut alerts = Vec::new();
    for p in store.list_providers().map_err(e2s)? {
        let (Some(limit), _) = (p.period_limit, p.limit_unit.as_deref()) else {
            continue;
        };
        if !p.enabled || limit <= 0.0 {
            continue;
        }
        let unit = match p.limit_unit.as_deref() {
            Some("wan_tokens") => "wan_tokens",
            Some("cny") => continue,
            // NULL normalizes to requests (same source as the ring percentage, compatible with v1 rows)
            _ => "requests",
        };
        let (since, period_key) = period_start(now, p.reset_period.as_deref());
        let t = store
            .usage_totals_for_provider(&p.id, since.as_deref())
            .map_err(e2s)?;
        let used = match unit {
            "wan_tokens" => {
                (t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_creation_tokens)
                    as f64
                    / 10_000.0
            }
            _ => t.requests as f64,
        };
        if used < limit {
            continue;
        }
        // Notify at most once per reset period (app_settings KV dedup)
        let dedup_key = format!("alert_sent:{}", p.id);
        if aux.get_setting(&dedup_key).as_deref() == Some(period_key.as_str()) {
            continue;
        }
        aux.set_setting(&dedup_key, &period_key).map_err(e2s)?;
        alerts.push(UsageAlertVm {
            provider_id: p.id,
            provider_name: p.name,
            used: used as i64,
            limit,
            unit: unit.to_string(),
        });
    }
    Ok(alerts)
}

// ── Dashboard ──

pub fn build_dashboard(store: &Store, aux: &Aux, window: &str) -> Result<DashboardVm, String> {
    let now = unix_now();
    let (window, since, days) = match window {
        "today" => ("today", day_key(now) + "T00:00:00Z", 1),
        "30d" => ("30d", rfc3339(now - 30 * 86_400), 30),
        _ => ("7d", rfc3339(now - 7 * 86_400), 7),
    };

    let cur = store.usage_totals(None, Some(&since)).map_err(e2s)?;
    let providers = store.list_providers().map_err(e2s)?;
    let name_by_id: HashMap<String, String> = providers
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();

    // trend: daily totals zero-filled over the window (30d buckets by 5 days)
    let mut daily: HashMap<String, UsageTotals> = HashMap::new();
    for d in store.usage_daily(None, Some(&since)).map_err(e2s)? {
        daily.insert(d.day, d.totals);
    }
    let mut trend = Vec::new();
    if window == "today" {
        let t = daily.get(&day_key(now)).cloned().unwrap_or_default();
        trend.push(TrendVm {
            date: mmdd(&day_key(now)),
            requests: t.requests,
            tokens: t.input_tokens + t.output_tokens,
        });
    } else {
        let bucket = if window == "30d" { 5 } else { 1 };
        let today_days = now.div_euclid(86_400);
        let mut b_req = 0i64;
        let mut b_tok = 0i64;
        for i in (0..days).rev() {
            let key = day_key((today_days - i) * 86_400);
            let t = daily.get(&key).cloned().unwrap_or_default();
            b_req += t.requests;
            b_tok += t.input_tokens + t.output_tokens;
            let is_bucket_end = (days - 1 - i) % bucket == bucket - 1 || i == 0;
            if is_bucket_end {
                trend.push(TrendVm {
                    date: mmdd(&key),
                    requests: b_req,
                    tokens: b_tok,
                });
                b_req = 0;
                b_tok = 0;
            }
        }
    }

    // provider distribution
    let total_req = cur.requests.max(1);
    let mut by_provider: Vec<ProviderDistVm> = store
        .usage_by_provider(None, Some(&since))
        .map_err(e2s)?
        .into_iter()
        .filter(|pu| pu.totals.requests > 0)
        .map(|pu| {
            let name = name_by_id
                .get(&pu.provider_id)
                .cloned()
                .unwrap_or(pu.provider_id.clone());
            ProviderDistVm {
                name,
                color: palette_color(&pu.provider_id).to_string(),
                pct: (pu.totals.requests * 100 / total_req) as i64,
                cost: 0.0,
            }
        })
        .collect();
    by_provider.sort_by(|a, b| b.pct.cmp(&a.pct));

    let mut by_agent = Vec::new();
    for (agent, label) in AGENTS {
        let t = store.usage_totals(Some(agent), Some(&since)).map_err(e2s)?;
        if t.requests > 0 {
            by_agent.push(AgentDistVm {
                agent: agent.to_string(),
                label: label.to_string(),
                requests: t.requests,
                tokens: fmt_tokens(t.input_tokens + t.output_tokens),
                cost: 0.0,
            });
        }
    }

    let latency = aux.avg_latency(None, Some(&since), None).unwrap_or(0);
    let latency_delta_pct = 0; // prev-window latency comparison lands with cost tables

    Ok(DashboardVm {
        window: window.to_string(),
        requests: cur.requests,
        requests_delta_pct: 0, // prev-window deltas land with cost tables (P1)
        input_tokens: cur.input_tokens,
        cache_read_tokens: cur.cache_read_tokens,
        output_tokens: cur.output_tokens,
        cost: 0.0,
        latency_ms: latency,
        latency_delta_pct,
        trend,
        by_provider,
        by_agent,
    })
}

pub fn build_footer_stats(store: &Store, aux: &Aux) -> Result<FooterStatsVm, String> {
    let today = day_key(unix_now());
    let since = format!("{today}T00:00:00Z");
    let t = store.usage_totals(None, Some(&since)).map_err(e2s)?;
    // hub_synced = catalog synced today (first 10 chars of the cache
    // timestamp are the date)
    let hub_synced = aux
        .load_hub_cache()
        .map(|(_, ts)| ts.starts_with(&today))
        .unwrap_or(false);
    Ok(FooterStatsVm {
        today_requests: t.requests,
        today_cost: 0.0,
        hub_synced,
        version: "v0.1.0 · MVP".into(),
    })
}

/// Catalog shelf: Hub cache first; fall back to the bundled static
/// catalog.json when never synced or on parse failure.
pub fn load_catalog(aux: &Aux) -> CatalogListVm {
    if let Some((payload, _)) = aux.load_hub_cache() {
        if let Ok(list) = serde_json::from_str::<CatalogListVm>(&payload) {
            return list;
        }
    }
    let entries: Vec<CatalogEntryVm> =
        serde_json::from_str(include_str!("catalog.json")).expect("catalog.json is valid");
    // Total mirrors the bundled listing size; a Hub sync replaces both.
    CatalogListVm {
        total: entries.len() as i64,
        entries,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kiwano_gateway::store::UsageRecord;

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    fn provider(id: &str, name: &str, billing: Billing) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            protocol: kiwano_gateway::store::Protocol::OpenAI,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            api_key: Some("sk-test".into()),
            billing,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            enabled: true,
            created_at: "2026-09-07T00:00:00Z".into(),
            updated_at: "2026-09-07T00:00:00Z".into(),
        }
    }

    #[test]
    fn fmt_tokens_matches_frontend() {
        assert_eq!(fmt_tokens(6_200_000), "6.2M");
        assert_eq!(fmt_tokens(8_800_000), "8.8M");
        assert_eq!(fmt_tokens(200_000), "200k");
        assert_eq!(fmt_tokens(31), "31");
        assert_eq!(fmt_tokens(15_000_000), "15M");
    }

    #[test]
    fn billing_mapping_roundtrip() {
        assert_eq!(billing_to_ui(billing_to_db("plan")), "plan");
        assert_eq!(billing_to_ui(billing_to_db("payg")), "payg");
        assert_eq!(billing_to_ui(billing_to_db("unl")), "unl");
    }

    #[test]
    fn import_current_provider_dedups_by_base_url() {
        let s = store();
        let creds = |base: &str, name: Option<&str>| crate::creds::CurrentCreds {
            base_url: base.into(),
            api_key: "sk-x".into(),
            name: name.map(String::from),
            protocol: "openai",
        };
        // trailing-slash variants dedup to one row
        let id1 = import_current_provider(&s, &creds("https://api.deepseek.com/v1/", Some("deepseek"))).unwrap();
        let id2 = import_current_provider(&s, &creds("https://api.deepseek.com/v1", Some("deepseek"))).unwrap();
        assert_eq!(id1, id2);
        let list = s.list_providers().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "deepseek");
        assert_eq!(list[0].api_key.as_deref(), Some("sk-x"));
        // no declared name → URL host
        let id3 = import_current_provider(&s, &creds("https://api.x.ai/v1", None)).unwrap();
        let p = s.get_provider(&id3).unwrap().unwrap();
        assert_eq!(p.name, "api.x.ai");
        assert_ne!(id1, id3);
    }

    #[test]
    fn provider_vm_maps_catalog_shape() {
        let s = store();
        s.insert_provider(&provider("deepseek-1", "DeepSeek", Billing::Metered))
            .unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "deepseek-1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux).unwrap();
        assert_eq!(vms.len(), 1);
        let vm = &vms[0];
        let json = serde_json::to_value(vm).unwrap();
        assert_eq!(json["billing"], "payg");
        assert_eq!(json["logo_char"], "D");
        assert_eq!(json["is_current"], true);
        assert_eq!(json["agents"][0], "claude");
        assert_eq!(json["agents_note"], "1 agent(s)");
        assert_eq!(json["endpoint"], "deepseek-1.example.com");
        assert_eq!(json["endpoint_note"], "OpenAI-compatible");
    }

    #[test]
    fn backup_binding_gets_badge_and_note() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux).unwrap();
        let alpha = vms.iter().find(|v| v.id == "a1").unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(alpha.is_current);
        assert!(!beta.is_current);
        assert_eq!(beta.status_badge.as_deref(), Some("Standby #1"));
        assert_eq!(beta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn enable_provider_promotes_to_primary() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Metered))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        enable_provider(&s, "b1").unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some("b1")
        );
        // Alpha demoted, still bound (failover groundwork)
        let bs = s.bindings_for_agent("claude").unwrap();
        let alpha = bs.iter().find(|b| b.provider_id == "a1").unwrap();
        assert_eq!(alpha.priority, 1);
    }

    #[test]
    fn add_provider_becomes_primary_and_demotes_prev() {
        let s = store();
        s.insert_provider(&provider("old1", "Old", Billing::Metered))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "old1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let input = NewProviderInput {
            name: "New Guy".into(),
            api_key: "sk-x".into(),
            endpoint: "https://api.new.example.com".into(),
            protocol: "openai".into(),
            model_default: "new-chat".into(),
            billing: "plan".into(),
            billing_config: BillingConfigInput {
                limit_value: Some(460.0),
                limit_unit: Some("requests".into()),
                reset_period: Some("monthly".into()),
            },
            agents: vec!["codex".into()],
        };
        let vm = add_provider(&s, &input).unwrap();
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some(vm.id.as_str())
        );
        assert_eq!(vm.billing, "plan");
        let bs = s.bindings_for_agent("codex").unwrap();
        let old = bs.iter().find(|b| b.provider_id == "old1").unwrap();
        assert_eq!(old.priority, 1);
        // plan + limit → request-unit quota
        let s2_quota = serde_json::to_value(&vm).unwrap();
        assert!(s2_quota["is_current"].as_bool().unwrap());
    }

    #[test]
    fn update_provider_rebinds_and_keeps_key_when_blank() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "P One", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("p2", "P Two", Billing::Metered))
            .unwrap();
        for id in ["p1", "p2"] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: id.into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p2".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let input = NewProviderInput {
            name: "P One Renamed".into(),
            api_key: "".into(), // blank = keep existing key
            endpoint: "https://p1.example.com/v2".into(),
            protocol: "openai".into(),
            model_default: String::new(),
            billing: "unl".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
            },
            agents: vec!["codex".into()], // rebind: claude dropped
        };
        let vm = update_provider(&s, &aux, "p1", &input).unwrap();
        assert_eq!(vm.name, "P One Renamed");
        assert_eq!(vm.billing, "unl");

        let p = s.get_provider("p1").unwrap().unwrap();
        assert_eq!(p.api_key.as_deref(), Some("sk-test")); // kept
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some("p1")
        );
        // claude binding removed; claude's primary falls back to p2
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some("p2")
        );
        assert!(!s
            .bindings_for_agent("claude")
            .unwrap()
            .iter()
            .any(|b| b.provider_id == "p1"));
    }

    #[test]
    fn delete_provider_promotes_next_candidate() {
        let s = store();
        s.insert_provider(&provider("main", "Main", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("backup", "Backup", Billing::Metered))
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "main".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "backup".into(),
            priority: 1,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        assert!(delete_provider(&s, "main").unwrap());
        assert_eq!(s.get_provider("main").unwrap(), None);
        // backup promoted to primary
        assert_eq!(
            s.primary_provider_id("codex").unwrap().as_deref(),
            Some("backup")
        );
        let bs = s.bindings_for_agent("codex").unwrap();
        assert_eq!(bs.len(), 1);
        assert_eq!(bs[0].priority, 0);
    }

    #[test]
    fn delete_unknown_provider_is_noop() {
        let s = store();
        assert!(!delete_provider(&s, "nope").unwrap());
    }

    #[test]
    fn settings_roundtrip_and_merge() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let v0 = build_settings(&s, &aux).unwrap();
        assert_eq!(v0.language, "zh-CN");
        assert!(v0.takeovers.iter().all(|t| !t.enabled));

        let patch =
            serde_json::json!({ "language": "en", "telemetry": true, "takeovers": "ignored" });
        let v1 = update_settings(&s, &aux, &patch).unwrap();
        assert_eq!(v1.language, "en");
        assert!(v1.telemetry);
        // takeovers untouched by patch
        assert!(v1.takeovers.iter().all(|t| !t.enabled));

        // takeover appears in settings and persists (tmp home, real config untouched)
        let tmp = tempfile::tempdir().unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path()).unwrap_err(); // no ~/.claude/settings.json → rejected and no key left behind
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != "claude"));

        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path()).unwrap();
        let v2 = build_settings(&s, &aux).unwrap();
        let claude = v2.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(claude.enabled);
        assert!(claude
            .placeholder_key
            .as_deref()
            .unwrap_or_default()
            .starts_with("kw-ag-claude-"));
        // config actually rewritten + restored via the escape hatch
        let env: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(env["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
        let v3 = build_settings(&s, &aux).unwrap();
        assert!(
            !v3.takeovers
                .iter()
                .find(|t| t.agent == "claude")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn dashboard_shape_with_usage() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered))
            .unwrap();
        let now = rfc3339(unix_now());
        s.record_usage(&kiwano_gateway::store::UsageRecord {
            ts: now.clone(),
            agent: "claude".into(),
            provider_id: "p1".into(),
            model: None,
            input_tokens: 1000,
            output_tokens: 500,
            cache_read_tokens: 100,
            cache_creation_tokens: 0,
            latency_ms: Some(1200),
            status: "ok".into(),
        })
        .unwrap();
        // The aux connection is a separate in-memory DB in tests (one shared
        // file in production); mirror the usage row so avg-latency reads see it.
        {
            let c = aux.conn.lock().unwrap();
            c.execute(
                "CREATE TABLE usage (
                     id INTEGER PRIMARY KEY AUTOINCREMENT,
                     ts TEXT NOT NULL, agent TEXT NOT NULL, provider_id TEXT NOT NULL,
                     model TEXT, input_tokens INTEGER NOT NULL DEFAULT 0,
                     output_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_read_tokens INTEGER NOT NULL DEFAULT 0,
                     cache_creation_tokens INTEGER NOT NULL DEFAULT 0,
                     latency_ms INTEGER, status TEXT NOT NULL DEFAULT 'ok')",
                [],
            )
            .unwrap();
            c.execute(
                "INSERT INTO usage (ts, agent, provider_id, input_tokens, output_tokens,
                                    cache_read_tokens, latency_ms)
                 VALUES (?1, 'claude', 'p1', 1000, 500, 100, 1200)",
                rusqlite::params![now],
            )
            .unwrap();
        }
        let d = build_dashboard(&s, &aux, "7d").unwrap();
        assert_eq!(d.requests, 1);
        assert_eq!(d.input_tokens, 1000);
        assert_eq!(d.latency_ms, 1200);
        assert_eq!(d.by_agent[0].tokens, "2k");
        assert_eq!(d.by_provider[0].pct, 100);
        assert!(d.trend.iter().map(|t| t.requests).sum::<i64>() >= 1);
    }

    #[test]
    fn rfc3339_and_day_helpers() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(mmdd("2026-09-07"), "09-07");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
    }

    #[test]
    fn agent_routes_roundtrip_strategy_and_reorder() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Metered))
            .unwrap();
        for (pid, pr) in [("a1", 0), ("b1", 1)] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: pid.into(),
                priority: pr,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }

        // default strategy is single
        let routes = build_agent_routes(&s).unwrap();
        assert_eq!(routes.len(), 1);
        let r = &routes[0];
        assert_eq!(r.agent, "claude");
        assert_eq!(r.strategy, "single");
        assert_eq!(r.bindings.len(), 2);
        assert_eq!(r.bindings[0].provider_name, "Alpha");
        assert_eq!(r.bindings[0].logo_char, "A");

        // change strategy + reorder → priorities rewritten, weights preserved
        set_agent_strategy(&s, "claude", "failover", None).unwrap();
        assert!(set_agent_strategy(&s, "claude", "bogus", None).is_err());
        reorder_agent_bindings(&s, "claude", &["b1".into(), "a1".into()]).unwrap();
        assert!(reorder_agent_bindings(&s, "claude", &["nope".into()]).is_err());

        let routes = build_agent_routes(&s).unwrap();
        let r = &routes[0];
        assert_eq!(r.strategy, "failover");
        assert_eq!(r.bindings[0].provider_id, "b1");
        assert_eq!(r.bindings[0].priority, 0);
        assert_eq!(r.bindings[1].provider_id, "a1");
        assert_eq!(r.bindings[1].priority, 1);

        // quota config passes through
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":50,"unit":"requests"}"#),
        )
        .unwrap();
        let r = &build_agent_routes(&s).unwrap()[0];
        assert_eq!(r.strategy, "quota");
        assert_eq!(
            r.config.as_deref(),
            Some(r#"{"limit":50,"unit":"requests"}"#)
        );
    }

    fn usage_row(provider_id: &str) -> UsageRecord {
        UsageRecord {
            ts: rfc3339(unix_now()),
            agent: "claude".into(),
            provider_id: provider_id.into(),
            model: None,
            input_tokens: 1_000,
            output_tokens: 100,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: None,
            status: "ok".into(),
        }
    }

    #[test]
    fn cost_alert_fires_once_per_period_and_respects_toggle() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("kimi-1", "Kimi", Billing::Subscription);
        p.period_limit = Some(100.0);
        p.limit_unit = Some("requests".into());
        p.reset_period = Some("monthly".into());
        s.insert_provider(&p).unwrap();

        // 40/100 → below threshold
        for _ in 0..40 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }
        assert!(check_usage_alerts(&s, &aux).unwrap().is_empty());

        // 100/100 → threshold hit
        for _ in 0..60 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].provider_id, "kimi-1");
        assert_eq!(alerts[0].unit, "requests");

        // same-period dedup: the second check returns nothing
        assert!(check_usage_alerts(&s, &aux).unwrap().is_empty());

        // toggle off → silent
        let patch = serde_json::json!({ "cost_alert": false });
        update_settings(&s, &aux, &patch).unwrap();
        assert!(check_usage_alerts(&s, &aux).unwrap().is_empty());
    }

    #[test]
    fn cost_alert_skips_unlimited_rows_and_cny_unit() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        let mut payg = provider("ds-1", "DeepSeek", Billing::Metered);
        payg.period_limit = Some(5.0); // NULL unit normalizes to requests
        s.insert_provider(&payg).unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("cny".into()); // no price table, no estimate
        s.insert_provider(&cny).unwrap();

        // 6 requests each: payg hits the threshold and alerts; cny is skipped
        for _ in 0..6 {
            s.record_usage(&usage_row("ds-1")).unwrap();
            s.record_usage(&usage_row("glm-1")).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].provider_id, "ds-1");
    }

    #[test]
    fn period_start_keys() {
        // 2026-09-07T12:34:56Z (Monday)
        let t = 1_788_784_496_i64;
        let (since, key) = period_start(t, Some("monthly"));
        assert_eq!(since.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(key, "2026-09");
        let (since, key) = period_start(t, Some("weekly"));
        assert_eq!(since.as_deref(), Some("2026-09-07T00:00:00Z"));
        assert_eq!(key, "2026-09-07");
        let (since, key) = period_start(t, Some("yearly"));
        assert_eq!(since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(key, "2026");
        // no reset → all-time totals
        let (since, key) = period_start(t, None);
        assert_eq!(since, None);
        assert_eq!(key, "all");
    }
}
