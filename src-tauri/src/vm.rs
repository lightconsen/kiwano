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
    Billing, Binding, HealthRecord, Provider, Store, StrategyType, UsageTotals,
};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

pub const AGENTS: [(&str, &str); 3] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
];

const PALETTE: [&str; 6] = ["#4D6BFE", "#615CED", "#3859FF", "#F55036", "#6467F2", "#0F9D58"];

fn palette_color(name: &str) -> &'static str {
    let h: u64 = name.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
    PALETTE[(h as usize) % PALETTE.len()]
}

fn logo_char(name: &str) -> String {
    name.chars().next().unwrap_or('?').to_uppercase().to_string()
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
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z", secs / 3600, secs / 60 % 60, secs % 60)
}

fn day_key(epoch_secs: i64) -> String {
    let (y, m, d) = civil_from_days(epoch_secs.div_euclid(86_400));
    format!("{y:04}-{m:02}-{d:02}")
}

/// `MM-DD` label for the dashboard trend axis.
fn mmdd(day: &str) -> String {
    day.get(5..10).unwrap_or(day).to_string()
}

// ── Auxiliary connection (same DB file, GUI-scoped tables + extra reads) ──

pub struct Aux {
    pub conn: Mutex<Connection>,
}

impl Aux {
    pub fn open(path: impl AsRef<std::path::Path>) -> rusqlite::Result<Self> {
        let conn = Connection::open(path)?;
        conn.pragma_update(None, "journal_mode", "WAL")?;
        Self::init_tables(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    #[cfg(test)]
    pub fn open_in_memory() -> rusqlite::Result<Self> {
        let conn = Connection::open_in_memory()?;
        Self::init_tables(&conn)?;
        Ok(Self { conn: Mutex::new(conn) })
    }

    fn init_tables(conn: &Connection) -> rusqlite::Result<()> {
        conn.execute(
            "CREATE TABLE IF NOT EXISTS app_settings (
                 key   TEXT PRIMARY KEY,
                 value TEXT NOT NULL
             )",
            [],
        )?;
        // 接管备份（tech.md §4.3-3）：files = JSON [[path, content], ...]
        conn.execute(
            "CREATE TABLE IF NOT EXISTS takeover_backups (
                 agent         TEXT PRIMARY KEY,
                 files         TEXT NOT NULL,
                 backed_up_at  TEXT NOT NULL
             )",
            [],
        )?;
        // Hub 目录缓存（tech.md §三 Hub 同步）：单行缓存，payload = CatalogListVm JSON
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

    pub fn save_takeover_backup(&self, agent: &str, files: &[(String, String)]) -> rusqlite::Result<()> {
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

    /// Hub 目录缓存：单行 upsert（RFC3339 synced_at）。
    pub fn save_hub_cache(&self, payload: &str, synced_at: &str) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO hub_cache (id, payload, synced_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = ?1, synced_at = ?2",
            rusqlite::params![payload, synced_at],
        )?;
        Ok(())
    }

    /// `(payload, synced_at)`；从未同步过则 None。
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
        let mut sql = String::from(
            "SELECT AVG(latency_ms) FROM usage WHERE latency_ms IS NOT NULL",
        );
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

/// 手动/启动时 Hub 同步的结果（UI 反馈用）。
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
    pub telemetry: bool,
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
            telemetry: false,
            hub_logged_in: false,
            hub_url: default_hub_url(),
        }
    }
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

    // agent → bindings (to read priorities for 备用 #N badges)
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
                        badge = Some(format!("备用 #{pr}"));
                    }
                }
            }
            agents.sort();
            let is_current = agents.iter().any(|a| primary.get(a).map(String::as_str) == Some(&p.id));

            let note = if backup_for_any {
                Some("故障转移队列".to_string())
            } else if !agents.is_empty() {
                Some(format!("{} 个 Agent", agents.len()))
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
        kiwano_gateway::store::Protocol::OpenAI => "OpenAI 兼容".to_string(),
        kiwano_gateway::store::Protocol::Anthropic => "Anthropic".to_string(),
    }
}

fn health_vm(store: &Store, p: &Provider) -> HealthVm {
    let rec = store.get_health(&p.id).ok().flatten();
    match rec {
        Some(HealthRecord { status, last_latency_ms, .. }) => match status.as_str() {
            "healthy" => HealthVm { state: "ok".into(), latency_ms: last_latency_ms, note: None },
            "degraded" => HealthVm { state: "idle".into(), latency_ms: last_latency_ms, note: None },
            "down" => HealthVm { state: "off".into(), latency_ms: last_latency_ms, note: Some("故障".into()) },
            _ => derive_health(p, last_latency_ms),
        },
        None => derive_health(p, None),
    }
}

fn derive_health(p: &Provider, latency: Option<i64>) -> HealthVm {
    if p.enabled {
        HealthVm { state: "idle".into(), latency_ms: latency, note: None }
    } else {
        HealthVm { state: "off".into(), latency_ms: None, note: Some("未启用".into()) }
    }
}

fn usage_vm(aux: &Aux, p: &Provider, totals: Option<&UsageTotals>, since7: &str) -> Option<UsageVm> {
    let t = totals?;
    let quota = match p.billing {
        Billing::Subscription => p.period_limit.map(|limit| QuotaVm {
            used: t.requests,
            limit,
            unit: "requests".into(),
            resets_at: None, // reset-cycle tracking lands with the quota strategy (P2)
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
        .map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' })
        .collect();
    let trimmed = s.trim_matches('-');
    if trimmed.is_empty() { "provider".into() } else { trimmed.to_string() }
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
        store.upsert_strategy(agent, StrategyType::Single, None).map_err(e2s)?;
        // "保存并启用" → becomes the primary for the chosen agents; the
        // previous primary is demoted to 备用 #1.
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
        agents_note: (!input.agents.is_empty())
            .then(|| format!("{} 个 Agent", input.agents.len())),
        health: vm_health,
        usage: None,
    })
}

/// 启用 = make this provider the primary of every agent it is bound to.
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

/// 更新供应商：改 providers 行 + 重绑 agents（新集合 = 主选，被移除的解绑）。
/// api_key 留空表示保持原 Key 不变。返回刷新后的 VM（重新聚合，保证徽章/备注一致）。
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

    // 重绑：旧集合中不在新集合的解绑；新集合走 add 同款主选逻辑
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
        store.upsert_strategy(agent, StrategyType::Single, None).map_err(e2s)?;
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

/// 删除供应商。若它是某 Agent 的主选，自动把该 Agent 候选集中最优的下一个提升为主选。
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
            for (i, b) in remaining.iter().filter(|b| b.provider_id != next_id).enumerate() {
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

/// 读取 UI 设置（托盘/自启等 Rust 侧逻辑用；`build_settings` 的无 store 部分）。
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
            let key = keys.iter().find(|k| k.agent == *agent).map(|k| k.key.clone());
            Ok(TakeoverVm {
                agent: agent.to_string(),
                label: label.to_string(),
                enabled: key.is_some(),
                placeholder_key: key,
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
    build_settings(store, aux)
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
        let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
        let key = format!("kw-ag-{agent}-{rand}");
        store.upsert_placeholder_key(&key, agent).map_err(e2s)?;
        // 改写 Agent 配置（备份→base_url→占位 Key）；失败回滚 Key 登记保持一致
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
    let name_by_id: HashMap<String, String> =
        providers.iter().map(|p| (p.id.clone(), p.name.clone())).collect();

    // trend: daily totals zero-filled over the window (30d buckets by 5 days)
    let mut daily: HashMap<String, UsageTotals> = HashMap::new();
    for d in store.usage_daily(None, Some(&since)).map_err(e2s)? {
        daily.insert(d.day, d.totals);
    }
    let mut trend = Vec::new();
    if window == "today" {
        let t = daily.get(&day_key(now)).cloned().unwrap_or_default();
        trend.push(TrendVm { date: mmdd(&day_key(now)), requests: t.requests, tokens: t.input_tokens + t.output_tokens });
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
                trend.push(TrendVm { date: mmdd(&key), requests: b_req, tokens: b_tok });
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
            let name = name_by_id.get(&pu.provider_id).cloned().unwrap_or(pu.provider_id.clone());
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
    // hub_synced = 今天同步过目录（缓存时间戳前 10 位即日期）
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

/// 货架目录：Hub 缓存优先，未同步/解析失败时回退随包静态 catalog.json。
pub fn load_catalog(aux: &Aux) -> CatalogListVm {
    if let Some((payload, _)) = aux.load_hub_cache() {
        if let Ok(list) = serde_json::from_str::<CatalogListVm>(&payload) {
            return list;
        }
    }
    let entries: Vec<CatalogEntryVm> =
        serde_json::from_str(include_str!("catalog.json")).expect("catalog.json is valid");
    // Hub total (42) is the catalog listing size; bundled set is the top picks.
    CatalogListVm { total: 42, entries }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn provider_vm_maps_catalog_shape() {
        let s = store();
        s.insert_provider(&provider("deepseek-1", "DeepSeek", Billing::Metered)).unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None).unwrap();
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
        assert_eq!(json["agents_note"], "1 个 Agent");
        assert_eq!(json["endpoint"], "deepseek-1.example.com");
        assert_eq!(json["endpoint_note"], "OpenAI 兼容");
    }

    #[test]
    fn backup_binding_gets_badge_and_note() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered)).unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription)).unwrap();
        s.upsert_binding(&Binding { agent: "claude".into(), provider_id: "a1".into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();
        s.upsert_binding(&Binding { agent: "claude".into(), provider_id: "b1".into(), priority: 1, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux).unwrap();
        let alpha = vms.iter().find(|v| v.id == "a1").unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(alpha.is_current);
        assert!(!beta.is_current);
        assert_eq!(beta.status_badge.as_deref(), Some("备用 #1"));
        assert_eq!(beta.agents_note.as_deref(), Some("故障转移队列"));
    }

    #[test]
    fn enable_provider_promotes_to_primary() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered)).unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Metered)).unwrap();
        s.upsert_binding(&Binding { agent: "claude".into(), provider_id: "a1".into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();
        s.upsert_binding(&Binding { agent: "claude".into(), provider_id: "b1".into(), priority: 1, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();

        enable_provider(&s, "b1").unwrap();
        assert_eq!(s.primary_provider_id("claude").unwrap().as_deref(), Some("b1"));
        // Alpha demoted, still bound (failover groundwork)
        let bs = s.bindings_for_agent("claude").unwrap();
        let alpha = bs.iter().find(|b| b.provider_id == "a1").unwrap();
        assert_eq!(alpha.priority, 1);
    }

    #[test]
    fn add_provider_becomes_primary_and_demotes_prev() {
        let s = store();
        s.insert_provider(&provider("old1", "Old", Billing::Metered)).unwrap();
        s.upsert_binding(&Binding { agent: "codex".into(), provider_id: "old1".into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();

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
        assert_eq!(s.primary_provider_id("codex").unwrap().as_deref(), Some(vm.id.as_str()));
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
        s.insert_provider(&provider("p1", "P One", Billing::Metered)).unwrap();
        s.insert_provider(&provider("p2", "P Two", Billing::Metered)).unwrap();
        for id in ["p1", "p2"] {
            s.upsert_binding(&Binding { agent: "claude".into(), provider_id: id.into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();
        }
        s.upsert_binding(&Binding { agent: "claude".into(), provider_id: "p2".into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();

        let input = NewProviderInput {
            name: "P One Renamed".into(),
            api_key: "".into(), // blank = keep existing key
            endpoint: "https://p1.example.com/v2".into(),
            protocol: "openai".into(),
            model_default: String::new(),
            billing: "unl".into(),
            billing_config: BillingConfigInput { limit_value: None, limit_unit: None, reset_period: None },
            agents: vec!["codex".into()], // rebind: claude dropped
        };
        let vm = update_provider(&s, &aux, "p1", &input).unwrap();
        assert_eq!(vm.name, "P One Renamed");
        assert_eq!(vm.billing, "unl");

        let p = s.get_provider("p1").unwrap().unwrap();
        assert_eq!(p.api_key.as_deref(), Some("sk-test")); // kept
        assert_eq!(s.primary_provider_id("codex").unwrap().as_deref(), Some("p1"));
        // claude binding removed; claude's primary falls back to p2
        assert_eq!(s.primary_provider_id("claude").unwrap().as_deref(), Some("p2"));
        assert!(!s.bindings_for_agent("claude").unwrap().iter().any(|b| b.provider_id == "p1"));
    }

    #[test]
    fn delete_provider_promotes_next_candidate() {
        let s = store();
        s.insert_provider(&provider("main", "Main", Billing::Metered)).unwrap();
        s.insert_provider(&provider("backup", "Backup", Billing::Metered)).unwrap();
        s.upsert_binding(&Binding { agent: "codex".into(), provider_id: "main".into(), priority: 0, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();
        s.upsert_binding(&Binding { agent: "codex".into(), provider_id: "backup".into(), priority: 1, weight: 1, win_start: None, win_end: None, enabled: true }).unwrap();

        assert!(delete_provider(&s, "main").unwrap());
        assert_eq!(s.get_provider("main").unwrap(), None);
        // backup promoted to primary
        assert_eq!(s.primary_provider_id("codex").unwrap().as_deref(), Some("backup"));
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

        let patch = serde_json::json!({ "language": "en", "telemetry": true, "takeovers": "ignored" });
        let v1 = update_settings(&s, &aux, &patch).unwrap();
        assert_eq!(v1.language, "en");
        assert!(v1.telemetry);
        // takeovers untouched by patch
        assert!(v1.takeovers.iter().all(|t| !t.enabled));

        // takeover appears in settings and persists（tmp home，不碰真实配置）
        let tmp = tempfile::tempdir().unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path()).unwrap_err(); // 无 ~/.claude/settings.json → 拒绝且不留 Key
        assert!(s.list_placeholder_keys().unwrap().iter().all(|k| k.agent != "claude"));

        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path()).unwrap();
        let v2 = build_settings(&s, &aux).unwrap();
        let claude = v2.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(claude.enabled);
        assert!(claude.placeholder_key.as_deref().unwrap_or_default().starts_with("kw-ag-claude-"));
        // 配置被真实改写 + 还原逃生门
        let env: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(env["env"]["ANTHROPIC_BASE_URL"], "http://127.0.0.1:8317");
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path()).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
        let v3 = build_settings(&s, &aux).unwrap();
        assert!(!v3.takeovers.iter().find(|t| t.agent == "claude").unwrap().enabled);
    }

    #[test]
    fn dashboard_shape_with_usage() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered)).unwrap();
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
}
