//! View-model layer: maps the gateway `Store` rows to the frontend contract
//! in `src/api/types.ts` (field names must match exactly — serde default
//! snake_case). The UI is mock-free; this is the single source of mapping.
//!
//! Owns an auxiliary SQLite connection on the same database file for reads
//! the gateway store does not expose (average latency, per-provider daily
//! sparkline) plus a GUI-scoped `app_settings` table.

use std::collections::{HashMap, HashSet};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use kiwano_gateway::store::{
    Billing, Binding, HealthRecord, Provider, RequestLogDetail, RequestLogEntry, RequestLogFilter,
    Store, Strategy, StrategyType, UsageTotals, EXPORT_ROW_CAP,
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

/// Categorical colours for charts. Spread around the hue wheel and held at a
/// lightness that reads on both themes — the letter-avatar palette above is
/// blue-heavy, which is fine behind a white glyph and useless for slices that
/// have to be told apart.
const CHART_COLORS: [&str; 8] = [
    "#4D6BFE", // blue
    "#0F9D58", // green
    "#F55036", // orange-red
    "#9333EA", // purple
    "#0EA5E9", // cyan
    "#EAB308", // amber
    "#EC4899", // pink
    "#14B8A6", // teal
];

/// One colour per id, distinct within the list: the id picks the starting slot
/// (so a provider keeps its colour while the roster holds still) and a taken
/// slot steps to the next free one. Only past eight entries do colours repeat.
fn chart_palette(ids: &[String]) -> Vec<&'static str> {
    let mut taken = [false; CHART_COLORS.len()];
    ids.iter()
        .map(|id| {
            let h: u64 = id.bytes().map(|b| (b as u64).wrapping_mul(31)).sum();
            let mut i = (h as usize) % CHART_COLORS.len();
            for _ in 0..CHART_COLORS.len() {
                if !taken[i] {
                    break;
                }
                i = (i + 1) % CHART_COLORS.len();
            }
            taken[i] = true;
            CHART_COLORS[i]
        })
        .collect()
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

/// `YYYY-MM-DDTHH` — the bucket key `Store::usage_hourly` groups by, from local
/// time already shifted by the caller's offset.
fn hour_key(local_secs: i64) -> String {
    let (y, m, d) = civil_from_days(local_secs.div_euclid(86_400));
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}",
        local_secs.rem_euclid(86_400) / 3600
    )
}

/// Inverse of `civil_from_days` (Howard Hinnant's `days_from_civil`): the day
/// index of a calendar date, so a period boundary can be turned back into the
/// instant a filter needs.
fn days_from_civil(y: i64, m: u32, d: u32) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

/// `MM-DD` label for the dashboard trend axis.
fn mmdd(day: &str) -> String {
    day.get(5..10).unwrap_or(day).to_string()
}

/// `HH:00` label for the dashboard trend axis when it plots hours.
fn hh00(hour: &str) -> String {
    hour.get(11..13)
        .map(|h| format!("{h}:00"))
        .unwrap_or_else(|| hour.to_string())
}

// Day boundaries are the user's, not UTC's: a UTC+8 user's "today" runs from
// 08:00 local yesterday to 08:00 today if we bucket by UTC. Everything that
// asks "which day is this in" goes through these two, with the offset the
// frontend reports (`getTimezoneOffset()`, negated to mean "east of UTC").

/// The local calendar date (`YYYY-MM-DD`) containing a unix timestamp.
fn local_day_key(offset_minutes: i64, epoch_secs: i64) -> String {
    day_key(epoch_secs + offset_minutes * 60)
}

/// The stored UTC offset (minutes east of UTC) the frontend keeps current.
fn tz_offset(aux: &Aux) -> i64 {
    ui_settings(aux).tz_offset_minutes
}

/// The first instant of that local day, as the RFC3339 UTC value a `ts >=`
/// filter needs — stored timestamps are UTC, so the boundary has to be too.
fn local_day_start(offset_minutes: i64, epoch_secs: i64) -> String {
    let local = epoch_secs + offset_minutes * 60;
    local_day_start_from(offset_minutes, local.div_euclid(86_400))
}

/// Same, from a local day index (which is what the calendar math produces).
fn local_day_start_from(offset_minutes: i64, local_day: i64) -> String {
    rfc3339(local_day * 86_400 - offset_minutes * 60)
}

// ── "In use" helpers: which candidate would serve a request issued right now,
//    mirroring the gateway's strategy selection (strategy/mod.rs) minus its
//    runtime state (circuit breakers, roundrobin sticky sessions) ──

/// Minutes-of-day in the local timezone (timewindow windows are local).
fn local_minutes_now() -> u32 {
    use chrono::Timelike;
    let t = chrono::Local::now().time();
    t.hour() * 60 + t.minute()
}

/// Whether `now_min` falls inside an "HH:MM" window; inclusive bounds, and a
/// start later than the end wraps midnight (same semantics as the gateway).
fn in_window(now_min: u32, start: &str, end: &str) -> bool {
    let parse = |s: &str| -> Option<u32> {
        let (h, m) = s.trim().split_once(':')?;
        let (h, m) = (h.parse::<u32>().ok()?, m.parse::<u32>().ok()?);
        (h < 24 && m < 60).then_some(h * 60 + m)
    };
    match (parse(start), parse(end)) {
        (Some(s), Some(e)) => {
            if s <= e {
                now_min >= s && now_min <= e
            } else {
                now_min >= s || now_min <= e
            }
        }
        _ => false,
    }
}

/// Whether the quota config puts `provider_id` over threshold for the current
/// UTC day — counted exactly like the gateway's select_quota (requests, or
/// input+output tokens; cache reads excluded). No/invalid config → under.
fn quota_over_threshold(store: &Store, aux: &Aux, config: Option<&str>, provider_id: &str) -> bool {
    #[derive(Deserialize)]
    struct QuotaCfg {
        limit: f64,
        #[serde(default = "default_quota_unit")]
        unit: String,
    }
    fn default_quota_unit() -> String {
        "requests".into()
    }
    let Some(cfg) = config.and_then(|c| serde_json::from_str::<QuotaCfg>(c).ok()) else {
        return false;
    };
    let since = local_day_start(tz_offset(aux), unix_now());
    let Ok(t) = store.usage_totals_for_provider(provider_id, Some(&since)) else {
        return false;
    };
    let consumed = if cfg.unit == "tokens" {
        (t.input_tokens + t.output_tokens) as f64
    } else {
        t.requests as f64
    };
    consumed >= cfg.limit
}

/// Start of the current reset period (RFC3339 UTC, for `ts >= ?` filters)
/// plus a dedup key. Returns (since, period_key); reset_period NULL = no
/// reset → (None, "all"). Day math: 1970-01-01 was a Thursday, so
/// `(days + 3) % 7 == 0` lands on Monday.
fn period_start(
    epoch_secs: i64,
    reset_period: Option<&str>,
    tz_offset_minutes: i64,
) -> (Option<String>, String) {
    // Reset periods follow the user's day too: a daily limit resetting at
    // 00:00 UTC is 08:00 for a UTC+8 user.
    let days = (epoch_secs + tz_offset_minutes * 60).div_euclid(86_400);
    let (y, m, _) = civil_from_days(days);
    match reset_period {
        None => (None, "all".into()),
        Some("weekly") => {
            let monday = days - (days + 3).rem_euclid(7);
            let (wy, wm, wd) = civil_from_days(monday);
            let key = format!("{wy:04}-{wm:02}-{wd:02}");
            (Some(local_day_start_from(tz_offset_minutes, monday)), key)
        }
        Some("yearly") => {
            let key = format!("{y:04}");
            (
                Some(local_day_start_from(
                    tz_offset_minutes,
                    days_from_civil(y, 1, 1),
                )),
                key,
            )
        }
        // monthly and any unexpected values all fall back to monthly
        _ => {
            let key = format!("{y:04}-{m:02}");
            (
                Some(local_day_start_from(
                    tz_offset_minutes,
                    days_from_civil(y, m, 1),
                )),
                key,
            )
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
        // Hub pricing cache: single-row, self-describing so the seed gate is
        // one read. A new table (not a column) because `CREATE TABLE IF NOT
        // EXISTS` reaches existing databases for free, whereas added columns
        // would need a migration framework this half of the DB does not have.
        conn.execute(
            "CREATE TABLE IF NOT EXISTS hub_models_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 version   INTEGER NOT NULL,
                 sha256    TEXT NOT NULL,
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
        files: &[crate::takeover::BackupFile],
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

    pub fn load_takeover_backup(
        &self,
        agent: &str,
    ) -> Option<(String, Vec<crate::takeover::BackupFile>)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let (ts, json) = conn
            .query_row(
                "SELECT backed_up_at, files FROM takeover_backups WHERE agent = ?1",
                [agent],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
            )
            .ok()?;
        // Earlier builds stored a bare [path, content] array, which cannot say
        // whether the file existed. Reading those as "existed" keeps the old
        // restore behaviour for backups already on disk. Refusing them instead
        // would make `disable` report "never taken over" and leave the agent
        // pointed at the gateway — the one outcome worth avoiding here.
        let files = serde_json::from_str::<Vec<crate::takeover::BackupFile>>(&json)
            .or_else(|_| {
                serde_json::from_str::<Vec<(String, String)>>(&json).map(|legacy| {
                    legacy
                        .into_iter()
                        .map(|(path, content)| crate::takeover::BackupFile {
                            path,
                            content,
                            existed: true,
                        })
                        .collect()
                })
            })
            .ok()?;
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

    /// Drop a KV entry (plan-limit enforcement markers, expired dedup…).
    pub fn delete_setting(&self, key: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let n = conn.execute("DELETE FROM app_settings WHERE key = ?1", [key])?;
        Ok(n > 0)
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

    /// Refresh only the cache timestamp, leaving the payload untouched — the
    /// conditional-sync path (manifest sha matched, nothing to re-download).
    /// Returns false when there is no cache row (never synced).
    pub fn touch_hub_synced_at(&self, synced_at: &str) -> rusqlite::Result<bool> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let n = conn.execute(
            "UPDATE hub_cache SET synced_at = ?1 WHERE id = 1",
            rusqlite::params![synced_at],
        )?;
        Ok(n > 0)
    }

    /// Hub pricing cache: single-row upsert. `payload` is the remote
    /// models.json **verbatim** — re-serializing would break the sha256 that
    /// the seed gate compares against the manifest.
    pub fn save_hub_models_cache(
        &self,
        version: i64,
        payload: &str,
        sha256: &str,
        synced_at: &str,
    ) -> rusqlite::Result<()> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, ?1, ?2, ?3, ?4)
             ON CONFLICT(id) DO UPDATE SET
                 version = ?1, sha256 = ?2, payload = ?3, synced_at = ?4",
            rusqlite::params![version, sha256, payload, synced_at],
        )?;
        Ok(())
    }

    /// `(version, payload, sha256, synced_at)`; None when never fetched.
    pub fn load_hub_models_cache(&self) -> Option<(i64, String, String, String)> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        conn.query_row(
            "SELECT version, payload, sha256, synced_at FROM hub_models_cache WHERE id = 1",
            [],
            |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                ))
            },
        )
        .ok()
    }

    /// Average `latency_ms` over a window, optionally per provider and/or
    /// agent. `from`/`to` are RFC3339 (store ts strings compare
    /// lexicographically).
    pub fn avg_latency(
        &self,
        provider: Option<&str>,
        agent: Option<&str>,
        from: Option<&str>,
        to: Option<&str>,
    ) -> Option<i64> {
        let conn = self.conn.lock().expect("aux mutex poisoned");
        let mut sql =
            String::from("SELECT AVG(latency_ms) FROM usage WHERE latency_ms IS NOT NULL");
        // Owned params: binding trait objects to borrowed &str inside `if let`
        // scopes fights the borrows (same as Store::usage_filters)
        let mut params: Vec<String> = Vec::new();
        if let Some(p) = provider {
            params.push(p.to_string());
            sql.push_str(&format!(" AND provider_id = ?{}", params.len()));
        }
        if let Some(a) = agent {
            params.push(a.to_string());
            sql.push_str(&format!(" AND agent = ?{}", params.len()));
        }
        if let Some(f) = from {
            params.push(f.to_string());
            sql.push_str(&format!(" AND ts >= ?{}", params.len()));
        }
        if let Some(t) = to {
            params.push(t.to_string());
            sql.push_str(&format!(" AND ts < ?{}", params.len()));
        }
        let mut stmt = conn.prepare(&sql).ok()?;
        let avg: Option<f64> = stmt
            .query_row(rusqlite::params_from_iter(params), |r| {
                r.get::<_, Option<f64>>(0)
            })
            .ok()?;
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
    pub used: f64,
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
    /// Currency of `cost` — the provider's own, never converted. Absent when
    /// no usage row carried a price.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub cost_currency: Option<String>,
    pub latency_ms: Option<i64>,
    pub quota: Option<QuotaVm>,
    pub spark: Option<Vec<f64>>,
}

/// Advanced forwarding settings echoed back to the modal for edit prefill.
#[derive(Serialize, Clone)]
pub struct ProviderAdvancedVm {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    pub headers: std::collections::BTreeMap<String, String>,
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
    /// Additional per-protocol endpoints (primary excluded).
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<ProviderEndpointVm>,
    pub billing: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_price: Option<String>,
    /// Raw limit unit (requests | wan_tokens | ISO currency) for edit prefill.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub limit_unit: Option<String>,
    pub enabled: bool,
    pub agents: Vec<String>,
    /// Agents this provider would serve a request for right now (per-agent
    /// slice of the strategy serving map). The All tab badges the collapsed
    /// `is_current`; an agent tab badges membership here instead, so a
    /// provider serving another agent does not read as in-use locally.
    pub serving_agents: Vec<String>,
    pub is_current: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub status_badge: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub agents_note: Option<String>,
    pub health: HealthVm,
    pub usage: Option<UsageVm>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub advanced: Option<ProviderAdvancedVm>,
    /// Token-plan quota query JSON (edit prefill); None = not configured.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_query: Option<serde_json::Value>,
    /// Plan-mode percent limits JSON `{"five_hour":20,"weekly":60}` (edit
    /// prefill); None = not set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub plan_limits: Option<serde_json::Value>,
}

/// Catalog-side billing vocabulary (`plan` | `payg` | `unl`).
///
/// Serialized as the bare lowercase tag, so the wire shape is unchanged. An
/// unrecognized tag is *not* silently coerced to `payg`: it is preserved
/// verbatim in `Other` and written back byte-identically, which keeps the hub
/// cache round-trip stable. One bad row must not fail a whole sync — the
/// catalog is the primary resource and a failed sync would strand the user on
/// the bundled snapshot forever.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum CatalogBilling {
    Plan,
    Payg,
    Unl,
    /// Unrecognized tag, kept verbatim for lossless round-tripping.
    Other(String),
}

impl CatalogBilling {
    pub fn as_str(&self) -> &str {
        match self {
            CatalogBilling::Plan => "plan",
            CatalogBilling::Payg => "payg",
            CatalogBilling::Unl => "unl",
            CatalogBilling::Other(raw) => raw,
        }
    }

    /// Inverse of `as_str`; `None` on an unrecognized tag (`Other` is what the
    /// `From<String>` conversion falls back to).
    pub fn parse_str(s: &str) -> Option<Self> {
        match s {
            "plan" => Some(CatalogBilling::Plan),
            "payg" => Some(CatalogBilling::Payg),
            "unl" => Some(CatalogBilling::Unl),
            _ => None,
        }
    }
}

impl From<String> for CatalogBilling {
    fn from(raw: String) -> Self {
        CatalogBilling::parse_str(&raw).unwrap_or(CatalogBilling::Other(raw))
    }
}

impl From<CatalogBilling> for String {
    fn from(b: CatalogBilling) -> Self {
        b.as_str().to_string()
    }
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
    /// Hub-relative logo path ("logos/<id>.<ext>"); the frontend resolves it
    /// against hub_url. Absent = fall back to `icon` / letter avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// Protocol fingerprint (anthropic | openai | gemini); entries predate the
    /// multi-protocol catalog, so older payloads default to openai.
    #[serde(default = "default_catalog_protocol")]
    pub protocol: String,
    /// Additional per-protocol endpoints of the same vendor service (migration
    /// v7 merged the former per-protocol variant entries into one brand row).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<CatalogEndpointVm>,
    pub tag: String,
    pub tag_label: String,
    pub rating: f64,
    pub endpoint: String,
    pub price_line: String,
    /// The currency this provider bills in. Its price rows — and therefore its
    /// spending limit — are denominated in it. Catalogs published before the
    /// field existed fall back to USD, the price table's base currency.
    #[serde(default = "default_catalog_currency")]
    pub currency: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub price_note: Option<String>,
    /// Billing mode; unknown Hub tags survive as `CatalogBilling::Other`.
    pub billing: CatalogBilling,
    pub users: String,
    pub blurb: String,
    pub added: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub free_offer: Option<String>,
    pub models: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogEndpointVm {
    pub protocol: String,
    pub endpoint: String,
    #[serde(default)]
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
    /// Conditional sync: the manifest sha256 matched the cached catalog, so
    /// catalog.json was not re-downloaded. `synced_at` still refreshed — the
    /// app confirmed it is current, which is what the footer badge claims.
    pub unchanged: bool,
    /// Version of the price table now cached; None when the Hub offers no
    /// pricing (unreachable, malformed, or absent — the bundled table stands).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub pricing_version: Option<i64>,
    /// The pricing half was already current, so models.json was not fetched.
    pub pricing_unchanged: bool,
}

#[derive(Serialize)]
pub struct TrendVm {
    pub date: String,
    pub requests: i64,
    pub tokens: i64,
}

#[derive(Serialize)]
pub struct ProviderDistVm {
    pub id: String,
    pub name: String,
    pub color: String,
    /// Requests attributed to this provider in the window — what `pct` is a
    /// share of, so a chart can size its segments without re-deriving them.
    pub requests: i64,
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

/// One select option of the dashboard's provider/agent filters.
#[derive(Serialize)]
pub struct FilterOptionVm {
    pub id: String,
    pub label: String,
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
    /// Filter select options: providers/agents with traffic in the window,
    /// computed independent of the active filter (otherwise the option list
    /// would collapse to the current selection).
    pub filter_providers: Vec<FilterOptionVm>,
    pub filter_agents: Vec<FilterOptionVm>,
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
    /// Preferred display currency (ISO code); per-provider amounts convert
    /// into it via the bundled exchange rates.
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
}

pub(crate) fn default_preferred_currency() -> String {
    "CNY".into()
}

/// Provider currency assumed when a catalog entry does not declare one — the
/// same default `generate.mjs` applies on the Hub side.
pub(crate) fn default_catalog_currency() -> String {
    "USD".into()
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
            preferred_currency: default_preferred_currency(),
            auto_check_update: true,
            dismissed_update: None,
            tz_offset_minutes: 0,
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
    /// Tokens consumed today (input + output) — the footer's headline metric;
    /// cost stays out of the status bar until price tables land (P1).
    pub today_tokens: i64,
    pub hub_synced: bool,
    pub version: String,
}

/// Plan-mode percent limits (modal form): utilization ceilings over the
/// vendor's rolling 5h / weekly windows. Both optional; both absent = none.
#[derive(Deserialize, Clone)]
pub struct PlanLimitsInput {
    pub five_hour: Option<f64>,
    pub weekly: Option<f64>,
}

#[derive(Deserialize)]
pub struct BillingConfigInput {
    pub limit_value: Option<f64>,
    #[allow(dead_code)]
    pub limit_unit: Option<String>,
    pub reset_period: Option<String>,
    pub plan_limits: Option<PlanLimitsInput>,
}

/// Serialize the percent limits into the `providers.plan_limits` JSON shape.
/// Non-positive / absent percents are dropped; an empty object reads as NULL.
fn plan_limits_json(input: Option<&PlanLimitsInput>) -> Option<String> {
    let input = input?;
    let mut obj = serde_json::Map::new();
    if let Some(pct) = input.five_hour.filter(|p| *p > 0.0 && *p <= 100.0) {
        obj.insert("five_hour".into(), serde_json::json!(pct));
    }
    if let Some(pct) = input.weekly.filter(|p| *p > 0.0 && *p <= 100.0) {
        obj.insert("weekly".into(), serde_json::json!(pct));
    }
    if obj.is_empty() {
        None
    } else {
        Some(serde_json::Value::Object(obj).to_string())
    }
}

/// Per-provider advanced forwarding settings (timeout / retries / custom
/// headers), edited in the provider modal's Advanced section. Custom header
/// names/values are sanitized before they reach the gateway.
#[derive(Deserialize)]
pub struct AdvancedInput {
    pub timeout_secs: Option<i64>,
    pub retries: Option<i64>,
    /// Header name → value; serialized to a JSON object column.
    pub headers: Option<std::collections::BTreeMap<String, String>>,
}

/// An additional per-protocol endpoint of a provider (migration v7): the
/// gateway forwards natively here when an inbound request speaks `protocol`.
#[derive(Serialize)]
pub struct ProviderEndpointVm {
    pub protocol: String,
    pub endpoint: String,
}

#[derive(Deserialize)]
pub struct NewEndpointInput {
    pub protocol: String,
    pub endpoint: String,
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
    /// Additional per-protocol endpoints; unknown protocol strings are
    /// skipped (defaulting one to openai could collide with the primary).
    #[serde(default)]
    pub endpoints: Vec<NewEndpointInput>,
    /// Advanced forwarding settings. Absent in an update = keep existing
    /// (mirrors the empty-api_key semantics); a present object is an
    /// authoritative snapshot whose null fields clear values.
    #[serde(default)]
    pub advanced: Option<AdvancedInput>,
    /// Token-plan quota query `{"template":"kimi","fields":{...}}`. Absent in
    /// an update = keep existing; null clears; a present object replaces.
    #[serde(default)]
    pub plan_query: Option<serde_json::Value>,
}

// ── Billing mapping (UI plan/payg/unl ↔ DB subscription/metered/unlimited) ──

/// Map a UI billing tag onto the store vocabulary. Unrecognized tags are an
/// error, never a silent `Metered` fallback (an unknown tag would otherwise
/// persist as a wrong billing mode and mis-shape the quota columns).
fn billing_to_db(ui: &str) -> Result<Billing, String> {
    match ui {
        "plan" => Ok(Billing::Subscription),
        "unl" => Ok(Billing::Unlimited),
        "payg" => Ok(Billing::Metered),
        other => Err(format!(
            "unknown billing \"{other}\" (expected plan|payg|unl)"
        )),
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

    // agent → bindings (to read priorities for the backup #N badges) and the
    // active strategy kind (to classify non-head candidates below)
    let mut bindings_by_agent: HashMap<String, Vec<Binding>> = HashMap::new();
    let mut strategy_by_agent: HashMap<String, StrategyType> = HashMap::new();
    for agent in primary.keys() {
        if let Ok(bs) = store.bindings_for_agent(agent) {
            bindings_by_agent.insert(agent.clone(), bs);
        }
        if let Ok(Some(st)) = store.get_strategy(agent) {
            strategy_by_agent.insert(agent.clone(), st.kind);
        }
    }

    // agent → provider ids that would serve a request issued right now under
    // the active strategy (the "In use" badge). Mirrors the gateway's strategy
    // selection; its runtime state (breaker health, roundrobin sticky sessions)
    // is process-local and invisible here, so those two degrade to the
    // deterministic first choice / full rotation.
    let mut serving: HashMap<String, HashSet<String>> = HashMap::new();
    for agent in store.bound_agents().map_err(e2s)? {
        let enabled: Vec<Binding> = store
            .bindings_for_agent(&agent)
            .map_err(e2s)?
            .into_iter()
            .filter(|b| b.enabled)
            .collect();
        let Some(head) = enabled.first().map(|b| b.provider_id.clone()) else {
            continue;
        };
        let strategy = store
            .get_strategy(&agent)
            .map_err(e2s)?
            .unwrap_or(Strategy {
                agent: agent.clone(),
                kind: StrategyType::Single,
                config: None,
            });
        let ids: HashSet<String> = match strategy.kind {
            // every candidate takes rotation turns → all of them serve
            StrategyType::Roundrobin => enabled.iter().map(|b| b.provider_id.clone()).collect(),
            // the candidate whose local window matches now; none → the head
            StrategyType::Timewindow => {
                let now = local_minutes_now();
                let hit = enabled.iter().find(|b| {
                    matches!(
                        (b.win_start.as_deref(), b.win_end.as_deref()),
                        (Some(s), Some(e)) if in_window(now, s, e)
                    )
                });
                HashSet::from([hit.map(|b| b.provider_id.clone()).unwrap_or(head)])
            }
            // under threshold → primary; over → first backup in line
            StrategyType::Quota => {
                let over = quota_over_threshold(store, aux, strategy.config.as_deref(), &head);
                HashSet::from([if over {
                    enabled
                        .get(1)
                        .map(|b| b.provider_id.clone())
                        .unwrap_or(head)
                } else {
                    head
                }])
            }
            // single / failover: the head (failover degradation is breaker runtime)
            _ => HashSet::from([head]),
        };
        serving.insert(agent, ids);
    }

    // provider → 7d usage totals
    let mut usage_by_id: HashMap<String, UsageTotals> = HashMap::new();
    for pu in store
        .usage_by_provider(None, None, Some(&since7))
        .map_err(e2s)?
    {
        usage_by_id.insert(pu.provider_id, pu.totals);
    }

    let vms = providers
        .into_iter()
        .map(|p| {
            let mut agents: Vec<String> = Vec::new();
            let mut serving_agents: Vec<String> = Vec::new();
            let mut backup_for_any = false;
            for agent in primary.keys() {
                let is_bound = bindings_by_agent
                    .get(agent)
                    .is_some_and(|bs| bs.iter().any(|b| b.provider_id == p.id));
                if !is_bound {
                    continue;
                }
                agents.push(agent.clone());
                if serving.get(agent).is_some_and(|ids| ids.contains(&p.id)) {
                    serving_agents.push(agent.clone());
                }
                if primary.get(agent).map(String::as_str) == Some(&p.id) {
                    continue;
                }
                // Non-head. Whether that marks the provider as a failover-queue
                // member (the Agent-column note) depends on the strategy: a
                // roundrobin tail takes rotation turns and a windowed
                // timewindow tail serves its own window — neither queues. A
                // windowless timewindow tail is never picked at all, and
                // single/failover/quota tails queue. No "Standby" badge here:
                // next to "In use" it read as a contradiction.
                let binding = bindings_by_agent
                    .get(agent)
                    .and_then(|bs| bs.iter().find(|b| b.provider_id == p.id));
                let windowed =
                    binding.is_some_and(|b| b.win_start.is_some() && b.win_end.is_some());
                let standby = match strategy_by_agent.get(agent) {
                    Some(StrategyType::Roundrobin) => false,
                    Some(StrategyType::Timewindow) => !windowed,
                    _ => true,
                };
                if standby {
                    backup_for_any = true;
                }
            }
            agents.sort();
            serving_agents.sort();
            let is_current = !serving_agents.is_empty();

            let note = if backup_for_any {
                Some("Failover queue".to_string())
            } else if !agents.is_empty() {
                Some(format!("{} agent(s)", agents.len()))
            } else {
                None
            };

            let health = health_vm(store, &p);
            let usage = usage_vm(store, aux, &p, usage_by_id.get(&p.id), &since7);

            ProviderVm {
                id: p.id.clone(),
                name: p.name.clone(),
                logo_char: logo_char(&p.name),
                logo_color: palette_color(&p.name).to_string(),
                logo_border: false,
                endpoint: display_endpoint(&p),
                protocol: p.protocol.as_str().to_string(),
                endpoint_note: endpoint_note(&p),
                endpoints: vm_endpoints(&p),
                billing: billing_to_ui(p.billing).to_string(),
                plan_price: crate::plan_quota::plan_monthly_price(p.plan_query.as_deref()),
                limit_unit: p.limit_unit.clone(),
                plan_limits: p
                    .plan_limits
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok()),
                enabled: p.enabled,
                agents,
                serving_agents,
                is_current,
                status_badge: None,
                agents_note: note,
                health,
                usage,
                advanced: advanced_vm(&p),
                plan_query: p
                    .plan_query
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok()),
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
    /// Local "HH:MM" window bounds (timewindow strategy); null = no window.
    pub win_start: Option<String>,
    pub win_end: Option<String>,
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
                    win_start: b.win_start,
                    win_end: b.win_end,
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
    let kind = StrategyType::parse_str(strategy)
        .ok_or_else(|| format!("unknown strategy type: {strategy}"))?;
    store.upsert_strategy(agent, kind, config).map_err(e2s)?;
    // Entering roundrobin: seed the weights as an even split of 100 (2
    // candidates → 50/50, 3 → 34/33/33, remainder to the head of the queue)
    // instead of leaving every candidate at 1, so the rotation starts balanced.
    if kind == StrategyType::Roundrobin {
        let bindings = store.bindings_for_agent(agent).map_err(e2s)?;
        let n = bindings.len();
        for (i, mut b) in bindings.into_iter().enumerate() {
            let w = ((100 / n) + if i < 100 % n { 1 } else { 0 }).max(1) as i64;
            if b.weight != w {
                b.weight = w;
                store.upsert_binding(&b).map_err(e2s)?;
            }
        }
    }
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

/// Patch one binding's strategy parameters (weight for roundrobin, the local
/// "HH:MM" window for timewindow). Unspecified fields keep their value; a
/// binding without a window is the timewindow fallback candidate.
pub fn update_agent_binding(
    store: &Store,
    agent: &str,
    provider_id: &str,
    weight: Option<i64>,
    win_start: Option<String>,
    win_end: Option<String>,
) -> Result<(), String> {
    let mut b = store
        .bindings_for_agent(agent)
        .map_err(e2s)?
        .into_iter()
        .find(|b| b.provider_id == provider_id)
        .ok_or_else(|| format!("provider {provider_id} is not bound to {agent}"))?;
    if let Some(w) = weight {
        b.weight = w.max(1);
    }
    // Both bounds are set/cleared together: a half window would never match.
    if win_start.is_some() || win_end.is_some() {
        let (s, e) = (
            win_start.filter(|v| !v.is_empty()),
            win_end.filter(|v| !v.is_empty()),
        );
        match (s, e) {
            (Some(s), Some(e)) => {
                b.win_start = Some(s);
                b.win_end = Some(e);
            }
            _ => {
                b.win_start = None;
                b.win_end = None;
            }
        }
    }
    store.upsert_binding(&b).map_err(e2s)?;
    Ok(())
}

/// Bind a provider to an agent as a new candidate: appended at the tail of
/// the queue (primary keeps its place). Binding an already-bound provider is
/// a no-op so the call stays idempotent. Takes effect on the gateway via
/// after_mutation's /reload.
pub fn add_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    if store.get_provider(provider_id).map_err(e2s)?.is_none() {
        return Err(format!("unknown provider: {provider_id}"));
    }
    let existing = store.bindings_for_agent(agent).map_err(e2s)?;
    if existing.iter().any(|b| b.provider_id == provider_id) {
        return Ok(());
    }
    let next_priority = existing.iter().map(|b| b.priority).max().unwrap_or(-1) + 1;
    store
        .upsert_binding(&Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
            priority: next_priority,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .map_err(e2s)?;
    Ok(())
}

/// Remove one agent's binding of a provider (other agents keep theirs).
/// Unbinding the last candidate is allowed: the route then has zero
/// candidates and requests fail cleanly with NoBinding until re-bound.
pub fn remove_agent_binding(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    let removed = store.delete_binding(agent, provider_id).map_err(e2s)?;
    if !removed {
        return Err(format!("provider {provider_id} is not bound to {agent}"));
    }
    Ok(())
}

/// Copy another agent's whole route onto this one: strategy kind + config
/// plus the ordered candidate list (priority, weight, time windows). The
/// target's existing route is replaced; providers are shared, not moved —
/// the source agent keeps its own bindings. Weights come over as-is
/// (upsert_strategy directly, no roundrobin even-split reseed).
pub fn apply_agent_route(store: &Store, target: &str, source: &str) -> Result<(), String> {
    if target == source {
        return Err("cannot copy an agent's route onto itself".to_string());
    }
    let strategy = store
        .get_strategy(source)
        .map_err(e2s)?
        .ok_or_else(|| format!("{source} has no route to copy"))?;
    let bindings = store.bindings_for_agent(source).map_err(e2s)?;
    if bindings.is_empty() {
        return Err(format!("{source} has no candidates to copy"));
    }
    store
        .upsert_strategy(target, strategy.kind, strategy.config.as_deref())
        .map_err(e2s)?;
    for b in store.bindings_for_agent(target).map_err(e2s)? {
        store.delete_binding(target, &b.provider_id).map_err(e2s)?;
    }
    for b in bindings {
        store
            .upsert_binding(&Binding {
                agent: target.to_string(),
                ..b
            })
            .map_err(e2s)?;
    }
    Ok(())
}

fn e2s(e: impl std::fmt::Display) -> String {
    e.to_string()
}

fn display_base(base_url: &str, api_path: &Option<String>) -> String {
    let stripped = base_url
        .trim_start_matches("https://")
        .trim_start_matches("http://");
    match api_path {
        Some(path) if !path.is_empty() => format!("{stripped}{path}"),
        _ => stripped.to_string(),
    }
}

fn display_endpoint(p: &Provider) -> String {
    display_base(&p.base_url, &p.api_path)
}

fn protocol_label(p: kiwano_gateway::store::Protocol) -> &'static str {
    match p {
        kiwano_gateway::store::Protocol::OpenAI => "OpenAI-compatible",
        kiwano_gateway::store::Protocol::Anthropic => "Anthropic",
        kiwano_gateway::store::Protocol::Gemini => "Gemini API",
    }
}

fn endpoint_note(p: &Provider) -> String {
    let mut note = protocol_label(p.protocol).to_string();
    // Additional endpoints surface in the same subtitle: "OpenAI-compatible · +Anthropic".
    for e in &p.endpoints {
        let tag = match e.protocol {
            kiwano_gateway::store::Protocol::OpenAI => "OpenAI",
            kiwano_gateway::store::Protocol::Anthropic => "Anthropic",
            kiwano_gateway::store::Protocol::Gemini => "Gemini",
        };
        note.push_str(" · +");
        note.push_str(tag);
    }
    note
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
/// Units are the two counting units plus any 3-letter currency code (stored
/// uppercase; the v9 CHECK constraint enforces the same shape).
fn normalize_limit_unit(unit: Option<&str>, has_limit: bool) -> Option<String> {
    if !has_limit {
        return None;
    }
    match unit {
        Some("wan_tokens") => Some("wan_tokens".into()),
        Some("requests") => Some("requests".into()),
        Some(u) => {
            let code = u.trim().to_ascii_uppercase();
            if code.len() == 3 && code.chars().all(|c| c.is_ascii_alphabetic()) {
                Some(code)
            } else {
                Some("requests".into())
            }
        }
        None => Some("requests".into()),
    }
}

/// Cost of one provider, in the currency its usage was priced in.
///
/// No conversion: a provider bills in one currency and this number is read
/// beside that provider's own limits. Should usage ever be priced in more than
/// one currency (a price-table currency change mid-period), the currency
/// carrying the most money names the total — the alternatives are folding
/// other currencies in at a rate nobody asked for, or inventing a second line
/// for a case that does not occur in practice. Unpriced rows (`None`)
/// contribute nothing, exactly as they did when the sum was converted.
fn provider_cost(buckets: &[(Option<String>, f64)]) -> (Option<f64>, Option<String>) {
    let mut per_currency: HashMap<&str, f64> = HashMap::new();
    for (currency, cost) in buckets {
        let Some(currency) = currency.as_deref() else {
            continue;
        };
        *per_currency.entry(currency).or_default() += cost;
    }
    match per_currency.into_iter().max_by(|a, b| a.1.total_cmp(&b.1)) {
        Some((currency, total)) => (Some(total), Some(currency.to_string())),
        None => (None, None),
    }
}

/// Usage cell for one provider.
fn usage_vm(
    store: &Store,
    aux: &Aux,
    p: &Provider,
    totals: Option<&UsageTotals>,
    since7: &str,
) -> Option<UsageVm> {
    let t = totals?;
    // The cost stays in the currency this provider's usage was priced in: it
    // is read next to that provider's own limits, and converting it into the
    // user's display currency made the two disagree. Rolling several
    // providers into one number is the Dashboard's job, and converting there
    // is what the display currency is for.
    let cost_buckets = store
        .usage_cost_by_currency(None, Some(&p.id), Some(since7))
        .unwrap_or_default();
    let (cost, cost_currency) = provider_cost(&cost_buckets);
    let quota = match (p.billing, p.limit_unit.as_deref()) {
        // A subscription's period limit and a metered provider's spending cap
        // are the same arithmetic: this period's usage against a number in the
        // provider's own unit. Building it only for Subscription left the
        // pay-as-you-go branches above unreachable — the Apps list drew no ring
        // and its tooltip said "no limit set" while a limit sat in the row.
        // Unlimited has nothing to measure, so it stays None.
        (Billing::Subscription | Billing::Metered, unit) => p.period_limit.map(|limit| {
            let (used, unit) = match unit {
                Some("wan_tokens") => (
                    (t.input_tokens
                        + t.output_tokens
                        + t.cache_read_tokens
                        + t.cache_creation_tokens) as f64
                        / 10_000.0,
                    "wan_tokens",
                ),
                // A currency limit rings against the period's cost as recorded:
                // the limit is denominated in the provider's own currency (the
                // price table's), so no rate is involved. Converting would make
                // the threshold move with the exchange rate.
                Some(u) if u.len() == 3 => (cost.unwrap_or(0.0), u),
                _ => (t.requests as f64, "requests"),
            };
            QuotaVm {
                used: (used * 100.0).round() / 100.0,
                limit,
                unit: unit.to_string(),
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
        cost: cost.map(|c| (c * 1e6).round() / 1e6),
        cost_currency,
        latency_ms: aux.avg_latency(Some(&p.id), None, Some(since7), None),
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

/// Map the advanced input to the three store columns: timeout clamps to
/// 1..=3600 (else unset = gateway defaults), retries to 0..=5 (0 = "no retry"
/// stored as NULL), headers serialize to a sanitized JSON object (dropping
/// empty names/values; empty object → NULL).
fn advanced_columns(adv: &AdvancedInput) -> (Option<i64>, Option<i64>, Option<String>) {
    let timeout_secs = adv.timeout_secs.filter(|s| (1..=3600).contains(s));
    let retries = adv
        .retries
        .filter(|r| (0..=5).contains(r))
        .filter(|&r| r > 0);
    let headers = adv
        .headers
        .as_ref()
        .map(|map| {
            let sanitized: serde_json::Map<String, serde_json::Value> = map
                .iter()
                .filter(|(k, v)| !k.trim().is_empty() && !v.is_empty())
                .map(|(k, v)| (k.trim().to_string(), serde_json::Value::String(v.clone())))
                .collect();
            (!sanitized.is_empty()).then(|| serde_json::Value::Object(sanitized).to_string())
        })
        .unwrap_or(None);
    (timeout_secs, retries, headers)
}

/// Parse a provider row's advanced columns back into the VM (for edit prefill).
fn advanced_vm(p: &Provider) -> Option<ProviderAdvancedVm> {
    if p.timeout_secs.is_none() && p.retries.is_none() && p.headers.is_none() {
        return None;
    }
    let headers = p
        .headers
        .as_deref()
        .and_then(|raw| {
            serde_json::from_str::<std::collections::BTreeMap<String, String>>(raw).ok()
        })
        .unwrap_or_default();
    Some(ProviderAdvancedVm {
        timeout_secs: p.timeout_secs,
        retries: p.retries,
        headers,
    })
}

/// Map the user-supplied additional endpoints to store rows; unknown protocol
/// strings are skipped (defaulting one to openai could collide with the
/// primary's protocol in provider_endpoints' PK).
fn input_endpoints(input: &NewProviderInput) -> Vec<kiwano_gateway::store::ProviderEndpoint> {
    input
        .endpoints
        .iter()
        .filter_map(|e| {
            kiwano_gateway::store::Protocol::parse_str(&e.protocol).map(|p| {
                kiwano_gateway::store::ProviderEndpoint {
                    protocol: p,
                    base_url: e.endpoint.trim().to_string(),
                    api_path: None,
                }
            })
        })
        .collect()
}

fn vm_endpoints(p: &Provider) -> Vec<ProviderEndpointVm> {
    p.endpoints
        .iter()
        .map(|e| ProviderEndpointVm {
            protocol: e.protocol.as_str().to_string(),
            endpoint: display_base(&e.base_url, &e.api_path),
        })
        .collect()
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
    let (timeout_secs, retries, adv_headers) = input
        .advanced
        .as_ref()
        .map(advanced_columns)
        .unwrap_or((None, None, None));
    let plan_query_json = input
        .plan_query
        .as_ref()
        .filter(|v| !v.is_null())
        .map(|v| v.to_string());
    // Plan rows carry percent limits in plan_limits; the legacy
    // number+unit+reset-cycle columns are left NULL (v10 form dropped them).
    let billing = billing_to_db(&input.billing)?;
    let is_plan = billing == kiwano_gateway::store::Billing::Subscription;
    let provider = Provider {
        id: id.clone(),
        name: input.name.trim().to_string(),
        protocol: kiwano_gateway::store::Protocol::parse_str(&input.protocol)
            .unwrap_or(kiwano_gateway::store::Protocol::OpenAI),
        base_url: input.endpoint.trim().to_string(),
        api_path: None,
        endpoints: input_endpoints(input),
        api_key: Some(input.api_key.clone()),
        billing,
        period_limit: if is_plan {
            None
        } else {
            input.billing_config.limit_value
        },
        limit_unit: if is_plan {
            None
        } else {
            normalize_limit_unit(
                input.billing_config.limit_unit.as_deref(),
                input.billing_config.limit_value.is_some(),
            )
        },
        plan_query: plan_query_json,
        plan_limits: if is_plan {
            plan_limits_json(input.billing_config.plan_limits.as_ref())
        } else {
            None
        },
        reset_period: if is_plan { None } else { reset_period },
        timeout_secs,
        retries,
        headers: adv_headers,
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
    let vm_endpoints = vm_endpoints(&provider);
    let vm_billing = billing_to_ui(provider.billing).to_string();
    let vm_health = derive_health(&provider, None);
    let vm_advanced = advanced_vm(&provider);

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
        endpoints: vm_endpoints,
        billing: vm_billing,
        plan_price: None,
        limit_unit: None,
        plan_query: input.plan_query.clone(),
        plan_limits: None,
        enabled: true,
        agents: input.agents.clone(),
        // Optimistic: strategy serving is only computed by build_provider_vms;
        // the list refetch right after returns the real per-agent state.
        serving_agents: vec![],
        is_current: !input.agents.is_empty(),
        status_badge: None,
        agents_note: (!input.agents.is_empty()).then(|| format!("{} agent(s)", input.agents.len())),
        health: vm_health,
        usage: None,
        advanced: vm_advanced,
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
    p.protocol = kiwano_gateway::store::Protocol::parse_str(&input.protocol)
        .unwrap_or(kiwano_gateway::store::Protocol::OpenAI);
    p.endpoints = input_endpoints(input);
    p.billing = billing_to_db(&input.billing)?;
    // Plan rows carry percent limits in plan_limits and NULL the legacy
    // number+unit+reset-cycle columns (v10 form); payg keeps the old shape.
    let is_plan = p.billing == kiwano_gateway::store::Billing::Subscription;
    p.period_limit = if is_plan {
        None
    } else {
        input.billing_config.limit_value
    };
    p.limit_unit = if is_plan {
        None
    } else {
        normalize_limit_unit(
            input.billing_config.limit_unit.as_deref(),
            input.billing_config.limit_value.is_some(),
        )
    };
    p.plan_limits = if is_plan {
        plan_limits_json(input.billing_config.plan_limits.as_ref())
    } else {
        None
    };
    p.reset_period = if is_plan {
        None
    } else {
        match input.billing_config.reset_period.as_deref() {
            Some("monthly") | Some("weekly") | Some("yearly") => {
                input.billing_config.reset_period.clone()
            }
            _ => None,
        }
    };
    if !input.api_key.trim().is_empty() {
        p.api_key = Some(input.api_key.clone());
    }
    // Advanced: absent = keep existing (same semantics as an empty api_key);
    // a present object is an authoritative snapshot — null fields clear values.
    if let Some(adv) = &input.advanced {
        let (timeout_secs, retries, headers) = advanced_columns(adv);
        p.timeout_secs = timeout_secs;
        p.retries = retries;
        p.headers = headers;
    }
    // Plan quota query: same absent-keeps semantics; null clears.
    if let Some(pq) = &input.plan_query {
        p.plan_query = (!pq.is_null()).then(|| pq.to_string());
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

/// The Hub endpoint older builds shipped as the default. That host no longer
/// resolves, and `#[serde(default)]` cannot repair it: the value is already
/// stored, so the default never applies again. Anyone who ran a build from
/// before the domain change would keep failing to sync forever, and silently —
/// a failed sync only logs and falls back to the bundled catalog.
const LEGACY_HUB_URL: &str = "https://hub.kiwano.app/catalog.json";

/// Read UI settings (used by Rust-side logic like tray/autostart; the
/// store-free part of `build_settings`).
pub(crate) fn ui_settings(aux: &Aux) -> SettingsVm {
    let mut s: SettingsVm = aux
        .load_settings_json()
        .and_then(|v| serde_json::from_value(v).ok())
        .unwrap_or_default();
    // Heal the one value known to be a bygone default. An endpoint the user
    // chose — even a broken one — is left exactly as it is.
    if s.hub_url == LEGACY_HUB_URL {
        s.hub_url = default_hub_url();
    }
    s
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
            if k == "takeovers" {
                continue;
            }
            // preferred currency: normalize + validate the ISO code
            if k == "preferred_currency" {
                let Some(code) = v.as_str() else { continue };
                let code = code.trim().to_ascii_uppercase();
                if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                    return Err(format!("invalid currency code: {code}"));
                }
                obj.insert(k.clone(), serde_json::Value::String(code));
                continue;
            }
            obj.insert(k.clone(), v.clone());
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
    filter: RequestLogFilter<'_>,
) -> Result<RequestLogListVm, String> {
    let (rows, total) = store
        .list_request_logs(page, page_size, filter)
        .map_err(e2s)?;
    Ok(RequestLogListVm { rows, total })
}

#[derive(Serialize)]
pub struct RequestLogExportVm {
    pub rows_written: usize,
    /// The slice was larger than `EXPORT_ROW_CAP`, so the file is short of it.
    pub truncated: bool,
}

/// Every log row the filter matches, as CSV — what the Logs card's export
/// writes. Unpaged, unlike `list_request_logs`: the page size is a display
/// concern and must not cap what lands in the file.
pub fn export_request_logs_csv(
    store: &Store,
    path: &str,
    agent: Option<&str>,
    provider_id: Option<&str>,
    status: Option<&str>,
    from: Option<&str>,
    to: Option<&str>,
) -> Result<RequestLogExportVm, String> {
    // Ask for one row more than the cap will allow, so "exactly at the cap"
    // and "more than the cap" are distinguishable.
    let mut rows = store
        .export_request_logs(
            RequestLogFilter {
                agent,
                provider_id,
                status,
                from,
                to,
            },
            EXPORT_ROW_CAP + 1,
        )
        .map_err(e2s)?;
    let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
    rows.truncate(EXPORT_ROW_CAP as usize);
    crate::csv::write_csv(path, &rows).map_err(e2s)?;
    Ok(RequestLogExportVm {
        rows_written: rows.len(),
        truncated,
    })
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
        if let Err(e) = crate::takeover::enable(aux, agent, &key, data_port, home) {
            let _ = store.delete_placeholder_key(&key);
            return Err(e);
        }
    } else {
        crate::takeover::disable(aux, agent, home)?;
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
fn import_current_provider(
    store: &Store,
    creds: &crate::creds::CurrentCreds,
) -> Result<String, String> {
    let base = creds.base_url.trim().trim_end_matches('/');
    for p in store.list_providers().map_err(e2s)? {
        if p.base_url.trim().trim_end_matches('/') == base {
            return Ok(p.id);
        }
    }
    let now = rfc3339(unix_now());
    let name = creds.name.clone().unwrap_or_else(|| {
        let host = crate::creds::host_of(&creds.base_url);
        crate::creds::brand_name_for_host(&host)
            .map(String::from)
            .unwrap_or_else(|| {
                if host.is_empty() {
                    "Imported provider".into()
                } else {
                    host
                }
            })
    });
    let provider = Provider {
        id: format!(
            "{}-{}",
            slug(&name),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        ),
        name,
        protocol: kiwano_gateway::store::Protocol::parse_str(creds.protocol)
            .unwrap_or(kiwano_gateway::store::Protocol::OpenAI),
        base_url: base.to_string(),
        api_path: None,
        endpoints: Vec::new(),
        api_key: Some(creds.api_key.clone()),
        billing: kiwano_gateway::store::Billing::Metered,
        period_limit: None,
        limit_unit: None,
        reset_period: None,
        plan_query: None,
        plan_limits: None,
        timeout_secs: None,
        retries: None,
        headers: None,
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
    pub used: f64,
    pub limit: f64,
    /// requests | wan_tokens | 3-letter ISO currency code
    pub unit: String,
}

/// Check whether enabled providers' usage this period has reached the
/// user-set per-period limit (period_limit). Hits not yet notified this
/// period are recorded under a dedup key and returned (the frontend turns
/// them into system notifications). Currency limits compare the cost spent
/// this period (converted into the limit's currency) against the limit.
pub fn check_usage_alerts(store: &Store, aux: &Aux) -> Result<Vec<UsageAlertVm>, String> {
    if !ui_settings(aux).cost_alert {
        return Ok(Vec::new());
    }
    let now = unix_now();
    let mut alerts = Vec::new();
    for p in store.list_providers().map_err(e2s)? {
        let Some(limit) = p.period_limit else {
            continue;
        };
        if !p.enabled || limit <= 0.0 {
            continue;
        }
        // NULL unit normalizes to requests (same source as the ring
        // percentage, compatible with v1 rows).
        let unit = match p.limit_unit.as_deref() {
            Some("wan_tokens") => "wan_tokens",
            // Currency limit: compare spent cost (converted into the limit
            // currency) against the limit.
            Some(u) if u.len() == 3 => u,
            _ => "requests",
        };
        let (since, period_key) = period_start(now, p.reset_period.as_deref(), tz_offset(aux));
        let used = match unit {
            "wan_tokens" => {
                let t = store
                    .usage_totals_for_provider(&p.id, since.as_deref())
                    .map_err(e2s)?;
                (t.input_tokens + t.output_tokens + t.cache_read_tokens + t.cache_creation_tokens)
                    as f64
                    / 10_000.0
            }
            u if u.len() == 3 => {
                // A spending limit is denominated in the provider's own
                // currency — the one the price table quotes its models in — so
                // the period's cost compares directly. Converting here would
                // make the limit mean whatever the rate said that day.
                store
                    .usage_cost_by_currency(None, Some(&p.id), since.as_deref())
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|(currency, cost)| currency.as_deref().map(|_| cost))
                    .sum()
            }
            _ => {
                store
                    .usage_totals_for_provider(&p.id, since.as_deref())
                    .map_err(e2s)?
                    .requests as f64
            }
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
            used: (used * 100.0).round() / 100.0,
            limit,
            unit: unit.to_string(),
        });
    }
    Ok(alerts)
}

// ── Plan percent-limit enforcement (per-window utilization ceilings) ──

/// Aux KV marker written when the patrol disables a provider for exceeding a
/// plan percent limit (value = the exceeded window). Only a marker-bearing
/// provider is auto re-enabled, so a manual user disable is never overridden.
fn plan_limit_marker(provider_id: &str) -> String {
    format!("plan_limit_disabled:{provider_id}")
}

/// Flip a provider's enabled flag in place. Distinct from the route-takeover
/// `enable_provider` command (that one promotes the provider to primary).
fn set_provider_enabled(store: &Store, id: &str, enabled: bool) -> Result<bool, String> {
    let Some(mut p) = store.get_provider(id).map_err(e2s)? else {
        return Ok(false);
    };
    if p.enabled == enabled {
        return Ok(false);
    }
    p.enabled = enabled;
    p.updated_at = rfc3339(unix_now());
    store.update_provider(&p).map_err(e2s)?;
    Ok(true)
}

#[derive(Deserialize)]
struct PlanLimitsParsed {
    five_hour: Option<f64>,
    weekly: Option<f64>,
}

/// Live plan-quota report for enforcement, or None when there is nothing to
/// judge on this tick (transient network error, deterministic query failure
/// like a missing plan query, or auth problems). Enforcement stays dormant
/// rather than acting on a report it could not read.
fn plan_report_for_enforcement(
    store: &Store,
    aux: &Aux,
    provider_id: &str,
) -> Result<Option<crate::plan_quota::PlanQuotaReport>, String> {
    match crate::plan_quota::get_plan_quota_report(store, aux, provider_id, false) {
        Ok(report) if report.success => Ok(Some(report)),
        _ => Ok(None),
    }
}

/// First configured window whose tier utilization has reached its percent
/// ceiling (five_hour ↔ the `five_hour` tier, weekly ↔ the `weekly_limit`
/// tier). A window absent from the report counts as not over — no evidence,
/// no enforcement.
fn window_over(
    report: &crate::plan_quota::PlanQuotaReport,
    limits: &PlanLimitsParsed,
) -> Option<(&'static str, f64, f64)> {
    let tier_util = |name: &str| {
        report
            .tiers
            .iter()
            .find(|t| t.name == name)
            .map(|t| t.utilization)
    };
    if let Some(pct) = limits.five_hour {
        if let Some(util) = tier_util("five_hour") {
            if util >= pct {
                return Some(("five_hour", util, pct));
            }
        }
    }
    if let Some(pct) = limits.weekly {
        if let Some(util) = tier_util("weekly_limit") {
            if util >= pct {
                return Some(("weekly", util, pct));
            }
        }
    }
    None
}

/// Enforce the plan-mode percent limits (`providers.plan_limits`, JSON
/// `{"five_hour":20,"weekly":60}`): when a window tier's live plan-quota
/// utilization (5-minute cached report) reaches the configured percent, the
/// provider is disabled — dropped from the gateway route table once the
/// routes are reloaded. Once every configured window falls back under its
/// percent, the provider re-enables automatically. Only providers Kiwano
/// disabled carry the marker, so a manual user disable is never overridden.
///
/// Returns alerts (fired only on the under→over transition — disabling the
/// provider is itself the dedup) and a mutated flag telling the caller to
/// reload the gateway routes.
pub fn enforce_plan_limits(store: &Store, aux: &Aux) -> Result<(Vec<UsageAlertVm>, bool), String> {
    let mut alerts = Vec::new();
    let mut mutated = false;
    for p in store.list_providers().map_err(e2s)? {
        if p.billing != Billing::Subscription {
            continue;
        }
        let limits = p
            .plan_limits
            .as_deref()
            .and_then(|s| serde_json::from_str::<PlanLimitsParsed>(s).ok())
            .filter(|l| l.five_hour.is_some() || l.weekly.is_some());
        let marker = plan_limit_marker(&p.id);
        let disabled_by_us = aux
            .get_setting(&marker)
            .as_deref()
            .is_some_and(|v| !v.is_empty());

        // Percents removed (or row corrupted) while Kiwano had disabled it →
        // restore the provider and drop the marker.
        if disabled_by_us && limits.is_none() {
            mutated |= set_provider_enabled(store, &p.id, true)?;
            aux.delete_setting(&marker).map_err(e2s)?;
            continue;
        }
        let Some(limits) = limits else { continue };

        if !p.enabled {
            // Recovery pass: re-enable only what Kiwano disabled, once every
            // configured window is back under its percent.
            if !disabled_by_us {
                continue;
            }
            if let Some(report) = plan_report_for_enforcement(store, aux, &p.id)? {
                if window_over(&report, &limits).is_none() {
                    mutated |= set_provider_enabled(store, &p.id, true)?;
                    aux.delete_setting(&marker).map_err(e2s)?;
                }
            }
            continue;
        }

        let Some(report) = plan_report_for_enforcement(store, aux, &p.id)? else {
            continue;
        };
        let Some((window, util, pct)) = window_over(&report, &limits) else {
            continue;
        };
        mutated |= set_provider_enabled(store, &p.id, false)?;
        aux.set_setting(&marker, window).map_err(e2s)?;
        alerts.push(UsageAlertVm {
            provider_id: p.id,
            provider_name: p.name,
            used: (util * 10.0).round() / 10.0,
            limit: pct,
            unit: "plan_pct".to_string(),
        });
    }
    Ok((alerts, mutated))
}

// ── Dashboard ──

/// Dashboard aggregation. `provider_id`/`agent` narrow every stat (headline,
/// trend, distributions, latency) to that slice; None means all.
pub fn build_dashboard(
    store: &Store,
    aux: &Aux,
    window: &str,
    provider_id: Option<&str>,
    agent: Option<&str>,
) -> Result<DashboardVm, String> {
    let now = unix_now();
    let tz = tz_offset(aux);
    // Whole local calendar days, so a stat and its chart describe the same
    // span: 7 days is today plus the six before it, not a rolling 168 hours
    // (which would count the hours between 6 and 7 days back that the chart's
    // seven daily points cannot show).
    let (window, since, days) = match window {
        "today" => ("today", local_day_start(tz, now), 1),
        "30d" => ("30d", local_day_start(tz, now - 29 * 86_400), 30),
        _ => ("7d", local_day_start(tz, now - 6 * 86_400), 7),
    };

    let cur = store
        .usage_totals(agent, provider_id, Some(&since))
        .map_err(e2s)?;
    // Headline request count shares the Logs card's source (request_logs):
    // usage rows only cover forwarded requests, so failures before the forward
    // leg (no provider bound, protocol mismatch…) would vanish from the top
    // stat while the Logs card below still shows them. Token/cost/latency stay
    // usage-based — failed requests carry none.
    let requests = store
        .count_request_logs(agent, provider_id, Some(&since))
        .map_err(e2s)?;
    let providers = store.list_providers().map_err(e2s)?;
    let name_by_id: HashMap<String, String> = providers
        .iter()
        .map(|p| (p.id.clone(), p.name.clone()))
        .collect();

    // Cost rolls up per-currency buckets (each row's cost_currency) into the
    // user's preferred display currency via the effective price table's rates:
    // the Hub's when it has published any, so this agrees with the currency
    // selector instead of a snapshot compiled into the binary.
    let rates = crate::pricing::effective_doc(aux).0.exchange_rates;
    let pref = crate::pricing::preferred_currency(aux);
    let cost_of = |buckets: &[(Option<String>, f64)]| {
        crate::pricing::convert_cost_buckets(buckets, &pref, &rates)
    };

    // Headline cost for the window.
    let cost = cost_of(
        &store
            .usage_cost_by_currency(agent, provider_id, Some(&since))
            .map_err(e2s)?,
    );

    // Per-provider cost for the distribution card.
    let mut cost_by_pid: HashMap<String, f64> = HashMap::new();
    for (pid, currency, c) in store
        .usage_cost_by_provider(agent, Some(&since))
        .map_err(e2s)?
    {
        let c = match currency.as_deref() {
            Some(cur) => crate::pricing::convert_amount(c, cur, &pref, &rates),
            None => 0.0,
        };
        *cost_by_pid.entry(pid).or_default() += c;
    }

    let mut trend = Vec::new();
    if window == "today" {
        // "today" plots the day's hours, not one bar for the whole day: 24 local
        // hour buckets, zero-filled exactly like the daily axis so the chart
        // spans the same day the stat above it counts (and its bars still sum
        // to that stat).
        let mut hourly: HashMap<String, UsageTotals> = HashMap::new();
        for b in store
            .usage_hourly(agent, provider_id, Some(&since), tz)
            .map_err(e2s)?
        {
            hourly.insert(b.day, b.totals);
        }
        // The local day index, turned back into the 24 hour keys of that day.
        let today_days = (now + tz * 60).div_euclid(86_400);
        for h in 0..24 {
            let key = hour_key(today_days * 86_400 + h * 3_600);
            let t = hourly.get(&key).cloned().unwrap_or_default();
            trend.push(TrendVm {
                date: hh00(&key),
                requests: t.requests,
                tokens: t.input_tokens + t.output_tokens,
            });
        }
    } else {
        // One bar per local day, zero-filled: the chart draws exactly the days
        // the window selected, so its bars sum to the stat above it and each
        // label names the one day its own bar covers. Merging days (30d used to
        // draw six five-day blocks) made a bar mean something the axis could
        // not say.
        let mut daily: HashMap<String, UsageTotals> = HashMap::new();
        for d in store
            .usage_daily(agent, provider_id, Some(&since), tz)
            .map_err(e2s)?
        {
            daily.insert(d.day, d.totals);
        }
        // Local day index: the buckets have to be the same days the window
        // above selected, or the chart and its stat disagree again.
        let today_days = (now + tz * 60).div_euclid(86_400);
        for i in (0..days).rev() {
            let key = local_day_key(tz, (today_days - i) * 86_400);
            let t = daily.get(&key).cloned().unwrap_or_default();
            trend.push(TrendVm {
                date: mmdd(&key),
                requests: t.requests,
                tokens: t.input_tokens + t.output_tokens,
            });
        }
    }

    // provider distribution
    let total_req = cur.requests.max(1);
    let mut by_provider: Vec<ProviderDistVm> = store
        .usage_by_provider(agent, provider_id, Some(&since))
        .map_err(e2s)?
        .into_iter()
        .filter(|pu| pu.totals.requests > 0)
        .map(|pu| {
            let name = name_by_id
                .get(&pu.provider_id)
                .cloned()
                .unwrap_or(pu.provider_id.clone());
            ProviderDistVm {
                id: pu.provider_id.clone(),
                name,
                color: String::new(), // assigned below, once the list is fixed
                requests: pu.totals.requests,
                pct: pu.totals.requests * 100 / total_req,
                cost: (cost_by_pid.get(&pu.provider_id).copied().unwrap_or(0.0) * 1e6).round()
                    / 1e6,
            }
        })
        .collect();
    by_provider.sort_by_key(|p| std::cmp::Reverse(p.pct));
    // Colours last: they depend on the whole roster (see `chart_palette`), and
    // assigning them after the sort keeps the largest slice's slot stable.
    let ids: Vec<String> = by_provider.iter().map(|p| p.id.clone()).collect();
    for (entry, color) in by_provider.iter_mut().zip(chart_palette(&ids)) {
        entry.color = color.to_string();
    }

    let mut by_agent = Vec::new();
    for (name, label) in AGENTS {
        // The agent filter narrows this breakdown like every other panel,
        // leaving one row at 100% when one agent is selected. `name` is the
        // loop's, not the filter's, so skipping here is what applies it — a
        // table that kept every agent would total more than the headline.
        if agent.is_some_and(|a| a != name) {
            continue;
        }
        let t = store
            .usage_totals(Some(name), provider_id, Some(&since))
            .map_err(e2s)?;
        if t.requests > 0 {
            let buckets = store
                .usage_cost_by_currency(Some(name), provider_id, Some(&since))
                .unwrap_or_default();
            by_agent.push(AgentDistVm {
                agent: name.to_string(),
                label: label.to_string(),
                requests: t.requests,
                tokens: fmt_tokens(t.input_tokens + t.output_tokens),
                cost: (cost_of(&buckets) * 1e6).round() / 1e6,
            });
        }
    }

    let latency = aux
        .avg_latency(provider_id, agent, Some(&since), None)
        .unwrap_or(0);
    let latency_delta_pct = 0; // prev-window latency comparison lands with cost tables

    // Filter select options: who has traffic in the window, independent of
    // the active filter. The provider side reuses the same per-provider
    // aggregation (query already orders by request count DESC); the agent
    // side mirrors the by_agent loop without its provider narrowing.
    let filter_providers: Vec<FilterOptionVm> = store
        .usage_by_provider(None, None, Some(&since))
        .map_err(e2s)?
        .into_iter()
        .filter(|pu| pu.totals.requests > 0)
        .map(|pu| FilterOptionVm {
            id: pu.provider_id.clone(),
            label: name_by_id
                .get(&pu.provider_id)
                .cloned()
                .unwrap_or(pu.provider_id.clone()),
        })
        .collect();
    let filter_agents: Vec<FilterOptionVm> = AGENTS
        .iter()
        .filter_map(|(name, label)| {
            let t = store.usage_totals(Some(name), None, Some(&since)).ok()?;
            (t.requests > 0).then(|| FilterOptionVm {
                id: name.to_string(),
                label: label.to_string(),
            })
        })
        .collect();

    Ok(DashboardVm {
        window: window.to_string(),
        requests,
        requests_delta_pct: 0, // prev-window deltas land with cost tables (P1)
        input_tokens: cur.input_tokens,
        cache_read_tokens: cur.cache_read_tokens,
        output_tokens: cur.output_tokens,
        cost: (cost * 1e6).round() / 1e6,
        latency_ms: latency,
        latency_delta_pct,
        trend,
        by_provider,
        by_agent,
        filter_providers,
        filter_agents,
    })
}

pub fn build_footer_stats(
    store: &Store,
    aux: &Aux,
    version: &str,
) -> Result<FooterStatsVm, String> {
    let tz = tz_offset(aux);
    let now = unix_now();
    let today = local_day_key(tz, now);
    // The same boundary the dashboard's "today" window uses: local midnight as
    // the UTC instant a `ts >=` filter needs. Spelling it `{today}T00:00:00Z`
    // reads as *UTC* midnight, which for a UTC+8 user drops the day's first
    // eight hours — and, seen just after midnight, the whole day.
    let since = local_day_start(tz, now);
    let t = store.usage_totals(None, None, Some(&since)).map_err(e2s)?;
    // hub_synced = catalog synced today (first 10 chars of the cache
    // timestamp are the date)
    let hub_synced = aux
        .load_hub_cache()
        .map(|(_, ts)| ts.starts_with(&today))
        .unwrap_or(false);
    Ok(FooterStatsVm {
        today_requests: t.requests,
        today_tokens: t.input_tokens + t.output_tokens,
        hub_synced,
        version: version.to_string(),
    })
}

/// Catalog shelf: Hub cache first; fall back to the bundled static
/// catalog.json when never synced or on parse failure.
///
/// `added` is derived at read time from the local provider list: an entry
/// counts as added when a provider exists at its primary OR any of its
/// additional per-protocol endpoints. The static flags carried by
/// catalog.json / the Hub cache are ignored.
pub fn load_catalog(store: &Store, aux: &Aux) -> CatalogListVm {
    let mut list = if let Some((payload, _)) = aux.load_hub_cache() {
        serde_json::from_str::<CatalogListVm>(&payload).unwrap_or_else(|_| bundled_catalog())
    } else {
        bundled_catalog()
    };
    let keys: HashSet<String> = store
        .list_providers()
        .unwrap_or_default()
        .iter()
        .flat_map(|p| {
            let mut keys = vec![endpoint_key(&p.base_url)];
            keys.extend(p.endpoints.iter().map(|e| endpoint_key(&e.base_url)));
            keys
        })
        .collect();
    for e in &mut list.entries {
        let mut endpoints = vec![&e.endpoint];
        endpoints.extend(e.endpoints.iter().map(|x| &x.endpoint));
        e.added = endpoints
            .iter()
            .any(|url| keys.contains(&endpoint_key(url)));
    }
    list
}

fn bundled_catalog() -> CatalogListVm {
    let entries: Vec<CatalogEntryVm> =
        serde_json::from_str(include_str!("catalog.json")).expect("catalog.json is valid");
    // Total mirrors the bundled listing size; a Hub sync replaces both.
    CatalogListVm {
        total: entries.len() as i64,
        entries,
    }
}

/// Normalize an endpoint into its identity: host+path, lowercased, scheme
/// and trailing slashes stripped (config import merges by base_url too).
fn endpoint_key(s: &str) -> String {
    let t = s.trim().to_lowercase();
    let no_scheme = t
        .strip_prefix("https://")
        .or_else(|| t.strip_prefix("http://"))
        .unwrap_or(&t);
    no_scheme.trim_end_matches('/').to_string()
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
            endpoints: Vec::new(),
            api_key: Some("sk-test".into()),
            billing,
            period_limit: None,
            limit_unit: None,
            reset_period: None,
            plan_query: None,
            plan_limits: None,
            timeout_secs: None,
            retries: None,
            headers: None,
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

    /// Seed the plan-quota cache (Aux KV, same shape plan_quota.rs writes) so
    /// the patrol reads canned tier utilizations instead of hitting network.
    fn seed_quota_cache(aux: &Aux, provider_id: &str, five_hour_util: f64) {
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_millis() as i64;
        let report = serde_json::json!({
            "provider_id": provider_id,
            "template": "zhipu",
            "success": true,
            "error": null,
            "note": null,
            "tiers": [{
                "name": "five_hour",
                "utilization": five_hour_util,
                "resets_at": null,
                "used": null,
                "limit": null,
                "unit": null
            }],
            "queried_at": ts,
            "cached": true
        });
        aux.set_setting(
            &format!("plan_quota_cache:{provider_id}"),
            &serde_json::json!({ "ts": ts, "report": report }).to_string(),
        )
        .unwrap();
    }

    #[test]
    fn plan_limit_patrol_disables_and_recovers() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("plan-a", "PlanA", Billing::Subscription);
        p.plan_limits = Some(r#"{"five_hour":20}"#.into());
        s.insert_provider(&p).unwrap();

        // Over the ceiling: disabled + marker + one alert…
        seed_quota_cache(&aux, "plan-a", 85.0);
        let (alerts, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(mutated);
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].provider_id, "plan-a");
        assert_eq!(alerts[0].unit, "plan_pct");
        assert_eq!(alerts[0].used, 85.0);
        assert_eq!(alerts[0].limit, 20.0);
        assert!(!s.get_provider("plan-a").unwrap().unwrap().enabled);
        assert_eq!(
            aux.get_setting("plan_limit_disabled:plan-a").as_deref(),
            Some("five_hour")
        );

        // …and the next tick stays quiet (no re-alert, stays disabled).
        let (alerts, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(!mutated);
        assert!(alerts.is_empty());
        assert!(!s.get_provider("plan-a").unwrap().unwrap().enabled);

        // Utilization back under: re-enabled + marker cleared.
        seed_quota_cache(&aux, "plan-a", 10.0);
        let (alerts, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(mutated);
        assert!(alerts.is_empty());
        assert!(s.get_provider("plan-a").unwrap().unwrap().enabled);
        assert!(aux.get_setting("plan_limit_disabled:plan-a").is_none());
    }

    #[test]
    fn plan_limit_patrol_respects_manual_disable_and_skips_others() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        // User disabled this one by hand (no marker): never re-enabled.
        let mut manual = provider("plan-manual", "Manual", Billing::Subscription);
        manual.plan_limits = Some(r#"{"five_hour":20}"#.into());
        manual.enabled = false;
        s.insert_provider(&manual).unwrap();

        // Metered provider with percents is skipped entirely.
        let mut payg = provider("payg-a", "Payg", Billing::Metered);
        payg.plan_limits = Some(r#"{"five_hour":50}"#.into());
        s.insert_provider(&payg).unwrap();

        seed_quota_cache(&aux, "plan-manual", 90.0);
        seed_quota_cache(&aux, "payg-a", 90.0);
        let (alerts, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(!mutated);
        assert!(alerts.is_empty());
        assert!(!s.get_provider("plan-manual").unwrap().unwrap().enabled);
        assert!(s.get_provider("payg-a").unwrap().unwrap().enabled);
    }

    #[test]
    fn plan_limit_patrol_restores_when_percents_removed() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("plan-a", "PlanA", Billing::Subscription);
        p.plan_limits = Some(r#"{"five_hour":20}"#.into());
        s.insert_provider(&p).unwrap();

        // Kiwano disables…
        seed_quota_cache(&aux, "plan-a", 85.0);
        let (_, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(mutated);

        // …then the user clears the percents → restored on the next tick.
        let mut row = s.get_provider("plan-a").unwrap().unwrap();
        row.plan_limits = None;
        s.update_provider(&row).unwrap();
        let (_, mutated) = enforce_plan_limits(&s, &aux).unwrap();
        assert!(mutated);
        assert!(s.get_provider("plan-a").unwrap().unwrap().enabled);
        assert!(aux.get_setting("plan_limit_disabled:plan-a").is_none());
    }

    #[test]
    fn billing_mapping_roundtrip() {
        assert_eq!(billing_to_ui(billing_to_db("plan").unwrap()), "plan");
        assert_eq!(billing_to_ui(billing_to_db("payg").unwrap()), "payg");
        assert_eq!(billing_to_ui(billing_to_db("unl").unwrap()), "unl");
    }

    #[test]
    fn billing_to_db_rejects_unknown_tag() {
        // An unknown tag must never silently become payg/metered.
        let err = billing_to_db("per-token").unwrap_err();
        assert!(err.contains("per-token"), "{err}");
        assert!(err.contains("plan|payg|unl"), "{err}");
        assert!(billing_to_db("").is_err());
        assert!(billing_to_db("PAYG").is_err());
    }

    #[test]
    fn catalog_billing_roundtrip_known_and_unknown() {
        // Known tags map onto their variants and serialize back lowercase.
        for (raw, variant) in [
            ("plan", CatalogBilling::Plan),
            ("payg", CatalogBilling::Payg),
            ("unl", CatalogBilling::Unl),
        ] {
            assert_eq!(CatalogBilling::parse_str(raw), Some(variant.clone()));
            assert_eq!(CatalogBilling::from(raw.to_string()), variant);
            assert_eq!(variant.as_str(), raw);
            assert_eq!(
                serde_json::to_string(&variant).unwrap(),
                format!("\"{raw}\"")
            );
        }

        // Unknown tags survive verbatim instead of being coerced to payg.
        let other = CatalogBilling::from("per-token".to_string());
        assert_eq!(other, CatalogBilling::Other("per-token".into()));
        assert_eq!(CatalogBilling::parse_str("per-token"), None);
        assert_eq!(serde_json::to_string(&other).unwrap(), "\"per-token\"");
        assert_eq!(other.as_str(), "per-token");
        let back: CatalogBilling = serde_json::from_str("\"per-token\"").unwrap();
        assert_eq!(back, other);
    }

    #[test]
    fn catalog_entry_unknown_billing_survives_json_roundtrip() {
        // A hub cache payload with a bad row must deserialize and re-serialize
        // byte-identically (the sync gate compares bytes).
        let raw = r##"{"id":"x","name":"X","logo_char":"X","logo_color":"#000",
            "tag":"official","tag_label":"Official","rating":1.0,
            "endpoint":"https://x.example","price_line":"p","billing":"per-token",
            "users":"1","blurb":"b","added":false,"models":[]}"##;
        let entry: CatalogEntryVm = serde_json::from_str(raw).unwrap();
        assert_eq!(
            entry.billing,
            CatalogBilling::Other("per-token".to_string())
        );
        let json = serde_json::to_value(&entry).unwrap();
        assert_eq!(json["billing"], "per-token");

        let known: CatalogEntryVm =
            serde_json::from_str(&raw.replace("\"per-token\"", "\"payg\"")).unwrap();
        assert_eq!(known.billing, CatalogBilling::Payg);
        assert_eq!(
            serde_json::to_value(&known).unwrap()["billing"],
            serde_json::json!("payg")
        );
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
        let id1 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1/", Some("deepseek")))
                .unwrap();
        let id2 =
            import_current_provider(&s, &creds("https://api.deepseek.com/v1", Some("deepseek")))
                .unwrap();
        assert_eq!(id1, id2);
        let list = s.list_providers().unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "deepseek");
        assert_eq!(list[0].api_key.as_deref(), Some("sk-x"));
        // no declared name → brand name inferred from the host
        let id3 = import_current_provider(&s, &creds("https://api.x.ai/v1", None)).unwrap();
        let p = s.get_provider(&id3).unwrap().unwrap();
        assert_eq!(p.name, "xAI");
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
        assert_eq!(alpha.serving_agents, ["claude"]);
        assert!(!beta.is_current);
        assert!(beta.serving_agents.is_empty());
        // Standby badges are gone; the failover-queue role lives in the note
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn standby_flag_follows_strategy() {
        let s = store();
        for (id, name) in [
            ("a1", "Alpha"),
            ("b1", "Beta"),
            ("c1", "Gamma"),
            ("d1", "Delta"),
        ] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        s.upsert_strategy("codex", StrategyType::Roundrobin, None)
            .unwrap();
        s.upsert_strategy("gemini", StrategyType::Timewindow, None)
            .unwrap();
        s.upsert_strategy("hermes", StrategyType::Timewindow, None)
            .unwrap();
        let bind = |agent: &str, pid: &str, priority: i64, win: Option<(&str, &str)>| Binding {
            agent: agent.into(),
            provider_id: pid.into(),
            priority,
            weight: 1,
            win_start: win.map(|w| w.0.into()),
            win_end: win.map(|w| w.1.into()),
            enabled: true,
        };
        // roundrobin tail: takes rotation turns → not a standby
        s.upsert_binding(&bind("codex", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("codex", "b1", 1, None)).unwrap();
        // windowed timewindow tail: serves its own window → not a standby
        s.upsert_binding(&bind("gemini", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("gemini", "c1", 1, Some(("22:00", "06:00"))))
            .unwrap();
        // windowless timewindow tail: never picked → still a standby
        s.upsert_binding(&bind("hermes", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("hermes", "d1", 1, None)).unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux).unwrap();
        let beta = vms.iter().find(|v| v.id == "b1").unwrap();
        assert!(beta.is_current); // roundrobin serves every candidate
        assert_eq!(beta.serving_agents, ["codex"]);
        assert_eq!(beta.status_badge, None);
        assert_eq!(beta.agents_note.as_deref(), Some("1 agent(s)"));
        let gamma = vms.iter().find(|v| v.id == "c1").unwrap();
        assert_eq!(gamma.status_badge, None);
        let delta = vms.iter().find(|v| v.id == "d1").unwrap();
        assert!(!delta.is_current);
        assert_eq!(delta.status_badge, None);
        assert_eq!(delta.agents_note.as_deref(), Some("Failover queue"));
    }

    #[test]
    fn apply_agent_route_copies_strategy_and_candidates() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        s.insert_provider(&provider("b1", "Beta", Billing::Subscription))
            .unwrap();
        // source: roundrobin with tuned weights and a windowed tail
        s.upsert_strategy("codex", StrategyType::Roundrobin, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 60,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        s.upsert_binding(&Binding {
            agent: "codex".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 40,
            win_start: Some("22:00".into()),
            win_end: Some("06:00".into()),
            enabled: true,
        })
        .unwrap();
        // target: an unrelated failover route that gets replaced wholesale
        s.upsert_strategy("gemini", StrategyType::Failover, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "gemini".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        apply_agent_route(&s, "gemini", "gemini").unwrap_err();
        apply_agent_route(&s, "gemini", "claude").unwrap_err(); // no route
        apply_agent_route(&s, "gemini", "codex").unwrap();

        let st = s.get_strategy("gemini").unwrap().unwrap();
        assert_eq!(st.kind, StrategyType::Roundrobin);
        let bs = s.bindings_for_agent("gemini").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!((bs[0].provider_id.as_str(), bs[0].weight), ("a1", 60));
        assert_eq!((bs[1].provider_id.as_str(), bs[1].weight), ("b1", 40));
        // weights copied as-is, not re-seeded to an even split
        assert_eq!(bs[1].win_start.as_deref(), Some("22:00"));
        assert_eq!(bs[1].win_end.as_deref(), Some("06:00"));
        // the source agent keeps its own bindings
        assert_eq!(s.bindings_for_agent("codex").unwrap().len(), 2);
    }

    #[test]
    fn catalog_added_derives_from_provider_endpoints() {
        let aux = Aux::open_in_memory().unwrap();
        let s = store();
        // nothing added yet: the bundled static flags are ignored
        let empty = load_catalog(&s, &aux);
        assert!(!empty.entries.iter().any(|e| e.added));

        // add a provider on the merged Kimi entry's anthropic additional
        // endpoint (trailing slash variant) — the whole entry counts as added
        let mut p = provider("kfc", "Kimi For Coding", Billing::Metered);
        p.base_url = "https://api.kimi.com/coding/".into();
        s.insert_provider(&p).unwrap();

        let list = load_catalog(&s, &aux);
        let added: Vec<&str> = list
            .entries
            .iter()
            .filter(|e| e.added)
            .map(|e| e.id.as_str())
            .collect();
        // exactly the merged entry matches (via its alt endpoint); everything
        // else — including DeepSeek at a different endpoint — stays addable
        assert_eq!(added, ["kimi-for-coding"]);

        // a provider on the primary endpoint also marks the entry added
        let aux2 = Aux::open_in_memory().unwrap();
        let mut d = provider("ds", "DeepSeek", Billing::Metered);
        d.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&d).unwrap();
        let list2 = load_catalog(&s, &aux2);
        let added2: Vec<&str> = list2
            .entries
            .iter()
            .filter(|e| e.added)
            .map(|e| e.id.as_str())
            .collect();
        // catalog order: DeepSeek is the first bundled entry
        assert_eq!(added2, ["deepseek", "kimi-for-coding"]);
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
                plan_limits: None,
            },
            agents: vec!["codex".into()],
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
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
    fn add_provider_persists_endpoints() {
        let s = store();
        let input = NewProviderInput {
            name: "Qianfan".into(),
            api_key: "sk-x".into(),
            endpoint: "https://qianfan.baidubce.com/v2/tokenplan/personal".into(),
            protocol: "openai".into(),
            model_default: "qianfan-code-latest".into(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: vec![],
            endpoints: vec![
                NewEndpointInput {
                    protocol: "anthropic".into(),
                    endpoint: "  https://qianfan.baidubce.com/anthropic/coding  ".into(),
                },
                // unknown protocol → skipped, not defaulted (PK clash guard)
                NewEndpointInput {
                    protocol: "xml".into(),
                    endpoint: "https://x.example.com".into(),
                },
            ],
            advanced: None,
            plan_query: None,
        };
        add_provider(&s, &input).unwrap();

        let rows = s.list_providers().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].endpoints.len(), 1);
        assert_eq!(
            rows[0].endpoints[0].base_url,
            "https://qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(
            rows[0].endpoints[0].protocol,
            kiwano_gateway::store::Protocol::Anthropic
        );
    }

    #[test]
    fn provider_vm_carries_endpoints_and_note_suffix() {
        let s = store();
        let input = NewProviderInput {
            name: "Qianfan".into(),
            api_key: "sk-x".into(),
            endpoint: "https://qianfan.baidubce.com/v2/tokenplan/personal".into(),
            protocol: "openai".into(),
            model_default: "qianfan-code-latest".into(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: vec![],
            endpoints: vec![NewEndpointInput {
                protocol: "anthropic".into(),
                endpoint: "https://qianfan.baidubce.com/anthropic/coding".into(),
            }],
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &input).unwrap();
        assert_eq!(vm.endpoints.len(), 1);
        assert_eq!(vm.endpoints[0].protocol, "anthropic");
        // display_base strips the scheme (same as the primary endpoint field)
        assert_eq!(
            vm.endpoints[0].endpoint,
            "qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(vm.endpoint_note, "OpenAI-compatible · +Anthropic");
        // endpoints survive a fresh VM build from the store
        let aux = Aux::open_in_memory().unwrap();
        let vms = build_provider_vms(&s, &aux).unwrap();
        let loaded = vms.iter().find(|v| v.id == vm.id).unwrap();
        assert_eq!(loaded.endpoints.len(), 1);
        assert_eq!(loaded.endpoint_note, "OpenAI-compatible · +Anthropic");
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
                plan_limits: None,
            },
            agents: vec!["codex".into()], // rebind: claude dropped
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
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

    /// Seed `requests` rows `offset_days` back (at 23:00 UTC of that calendar
    /// day, so day-bucket boundaries are unambiguous), each with `tokens` in.
    fn seed_usage_at(s: &Store, offset_days: i64, requests: i64, tokens: i64) {
        let now = unix_now();
        let day = now.div_euclid(86_400) - offset_days;
        seed_usage_rows(s, day * 86_400 + 23 * 3600, requests, tokens);
    }

    /// `requests` rows starting at `first_secs`, as the gateway would have
    /// written them: a usage row and its request_logs twin.
    fn seed_usage_rows(s: &Store, first_secs: i64, requests: i64, tokens: i64) {
        for i in 0..requests {
            let ts = rfc3339(first_secs + i);
            s.record_usage(&kiwano_gateway::store::UsageRecord {
                ts: ts.clone(),
                agent: "claude".into(),
                provider_id: "demo-alpha".into(),
                model: Some("demo-model".into()),
                input_tokens: tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                status: "ok".into(),
                cost: Some(0.5),
                cost_currency: Some("USD".into()),
            })
            .unwrap();
            // The headline count reads request_logs, not usage: seed both, as
            // the gateway does for a forwarded request.
            s.insert_request_log(&kiwano_gateway::store::RequestLogNew {
                ts,
                method: "POST".into(),
                path: "/v1/messages".into(),
                query: None,
                agent: Some("claude".into()),
                attribution: Some("key".into()),
                provider_id: Some("demo-alpha".into()),
                model: Some("demo-model".into()),
                status_code: 200,
                error_kind: None,
                error_message: None,
                session_id: None,
                is_streaming: false,
                input_tokens: tokens,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                first_token_ms: None,
                request_headers: None,
                response_headers: None,
                request_body: None,
                response_body: None,
                request_size: 0,
                response_size: 0,
                truncated: false,
                cost: Some(0.5),
                cost_currency: Some("USD".into()),
            })
            .unwrap();
        }
    }

    #[test]
    fn dashboard_windows_cover_the_right_days() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        // Distinct magnitudes per age so a window that is too wide or too
        // narrow cannot cancel out: today 2, 3 days back 4, 10 days back 6,
        // 40 days back 8, plus one row 7 calendar days back.
        seed_usage_at(&s, 0, 2, 1_000);
        seed_usage_at(&s, 3, 4, 2_000);
        seed_usage_at(&s, 10, 6, 3_000);
        seed_usage_at(&s, 40, 8, 4_000);
        seed_usage_at(&s, 7, 1, 5_000);

        let d = |w: &str| build_dashboard(&s, &aux, w, None, None).unwrap();

        // Today: the current UTC day only — the 23:00 rows of the days before
        // it stay out, and so does the one from 40 days back. The chart splits
        // that day into its 24 hours, so the 23:00 rows land in the last bucket.
        let today = d("today");
        assert_eq!((today.requests, today.input_tokens), (2, 2_000));
        assert_eq!(today.trend.len(), 24, "today plots one bar per hour");
        assert_eq!(today.trend[23].date, "23:00");
        assert_eq!(today.trend[23].requests, 2);
        assert_eq!(today.trend[0].requests, 0, "an idle hour is still a bucket");
        assert_eq!(
            today.trend.iter().map(|p| p.requests).sum::<i64>(),
            today.requests,
            "the hourly bars cover the stat's whole window"
        );

        // 7d is today plus the six days before it, so the chart's seven points
        // cover exactly the same span as the stat above them.
        let week = d("7d");
        assert_eq!((week.requests, week.input_tokens), (6, 10_000));
        assert_eq!(week.trend.len(), 7);
        assert_eq!(
            week.trend.iter().map(|p| p.requests).sum::<i64>(),
            week.requests,
            "the chart covers the stat's whole window"
        );

        // 30d adds the 7- and 10-day-old groups (1 + 6) and still excludes the
        // 40-day-old one: 6 + 7 = 13 requests, 10k + 5k + 18k tokens.
        let month = d("30d");
        assert_eq!((month.requests, month.input_tokens), (13, 33_000));
        assert_eq!(month.trend.len(), 30, "30d is one bar per day");
        assert_eq!(month.trend.iter().map(|p| p.requests).sum::<i64>(), 13);
        // Bar i is the day 29 - i days ago, so each seeded group lands where its
        // own label says it does — no bar covering more than the day it names.
        assert_eq!(month.trend[29].requests, 2, "the last bar is today");
        assert_eq!(month.trend[26].requests, 4, "three days back");
        assert_eq!(month.trend[22].requests, 1, "seven days back");
        assert_eq!(month.trend[19].requests, 6, "ten days back");
        assert_eq!(
            month.trend[18].requests, 0,
            "and the quiet days are bars too"
        );

        // Cost is summed in the window and converted for display (default CNY).
        assert!((month.cost - month.requests as f64 * 0.5 * 7.1).abs() < 0.01);
    }

    #[test]
    fn footer_today_counts_from_local_midnight() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        update_settings(&s, &aux, &serde_json::json!({ "tz_offset_minutes": 480 })).unwrap();
        // One row a second after *local* midnight at UTC+8 — 16:00:01Z the day
        // before. It is the first moment of the user's day and the stretch a
        // UTC-midnight boundary silently dropped.
        let now = unix_now();
        let local_day = (now + 480 * 60).div_euclid(86_400);
        seed_usage_rows(&s, local_day * 86_400 - 480 * 60 + 1, 1, 1_000);

        let f = build_footer_stats(&s, &aux, "test").unwrap();
        assert_eq!(
            f.today_requests, 1,
            "00:00:01 local is today, not yesterday"
        );
        assert_eq!(f.today_tokens, 1_000);
    }

    #[test]
    fn day_boundaries_follow_the_configured_offset() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let utc_day = now.div_euclid(86_400);
        let local_day = (now + 480 * 60).div_euclid(86_400);
        // One row the two clocks date differently. Which way it can be built
        // depends on the hour, because the offset only opens a gap once one
        // date has rolled over and the other has not. While UTC's date still
        // matches the local one, the local day's first second (16:00Z the day
        // before) is what UTC calls yesterday; once UTC has caught up, a row in
        // UTC's morning is what the user's clock calls yesterday. Only one of
        // the two exists at any given moment — a fixed "23:00Z yesterday",
        // which is what this used to seed, is yesterday on *both* clocks for
        // the first eight hours of every local day, and the test failed there.
        let (row, in_utc, in_local) = if local_day == utc_day {
            (local_day * 86_400 - 480 * 60 + 1, 0, 1)
        } else {
            (utc_day * 86_400 + 3_600, 1, 0)
        };
        seed_usage_rows(&s, row, 1, 1_000);

        // UTC (the default, and what an older settings blob yields).
        assert_eq!(
            build_dashboard(&s, &aux, "today", None, None)
                .unwrap()
                .requests,
            in_utc
        );

        update_settings(&s, &aux, &serde_json::json!({ "tz_offset_minutes": 480 })).unwrap();
        let shifted = build_dashboard(&s, &aux, "today", None, None).unwrap();
        assert_eq!(shifted.requests, in_local, "the two clocks disagree");
        assert_eq!(shifted.trend.len(), 24);
        assert_eq!(
            shifted.trend.iter().filter(|t| t.requests > 0).count(),
            in_local as usize,
            "and the chart plots the day the stat counts"
        );
    }

    #[test]
    fn chart_palette_keeps_slices_distinct() {
        // More names than colours: a taken slot steps to the next free one, so
        // the slices stay distinguishable rather than sharing a hash.
        let ids: Vec<String> = (0..8).map(|i| format!("provider-{i}")).collect();
        let colors = chart_palette(&ids);
        let unique: std::collections::HashSet<_> = colors.iter().collect();
        assert_eq!(unique.len(), colors.len(), "each slice gets its own colour");

        // Stable: the same roster yields the same colours on every render.
        assert_eq!(chart_palette(&ids), colors);
    }

    #[test]
    fn provider_cost_stays_in_its_own_currency() {
        let (cost, currency) = provider_cost(&[
            (Some("USD".into()), 1.5),
            (Some("USD".into()), 0.5),
            (None, 9.0), // unpriced row: no currency, contributes nothing
        ]);
        assert_eq!(cost, Some(2.0));
        assert_eq!(currency.as_deref(), Some("USD"));

        // Mixed currencies: the one carrying the most money names the total.
        let (cost, currency) =
            provider_cost(&[(Some("USD".into()), 1.0), (Some("CNY".into()), 40.0)]);
        assert_eq!(cost, Some(40.0));
        assert_eq!(currency.as_deref(), Some("CNY"));

        assert_eq!(provider_cost(&[]), (None, None));
        assert_eq!(provider_cost(&[(None, 3.0)]), (None, None));
    }

    #[test]
    fn legacy_takeover_backup_still_loads() {
        let aux = Aux::open_in_memory().unwrap();
        // Builds before the `existed` field stored a bare [path, content] array.
        let legacy =
            serde_json::json!([["/tmp/kiwano-test/settings.json", "{\"a\":1}"]]).to_string();
        aux.conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO takeover_backups (agent, files, backed_up_at) VALUES (?1, ?2, ?3)",
                rusqlite::params!["claude", legacy, "2026-01-01T00:00:00Z"],
            )
            .unwrap();

        let (_, files) = aux
            .load_takeover_backup("claude")
            .expect("legacy backup loads");
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "/tmp/kiwano-test/settings.json");
        assert_eq!(files[0].content, "{\"a\":1}");
        // Read as "existed", so disabling still writes the content back instead
        // of deleting a file it cannot prove we created.
        assert!(files[0].existed);
    }

    #[test]
    fn legacy_hub_url_is_healed_on_load() {
        let aux = Aux::open_in_memory().unwrap();
        let mut v = serde_json::to_value(SettingsVm::default()).unwrap();

        // A settings blob written by a build from before the domain change.
        v["hub_url"] = serde_json::json!(LEGACY_HUB_URL);
        aux.save_settings_json(&v).unwrap();
        assert_eq!(ui_settings(&aux).hub_url, default_hub_url());

        // An endpoint the user picked is never rewritten, broken or not.
        v["hub_url"] = serde_json::json!("https://hub.example.com/catalog.json");
        aux.save_settings_json(&v).unwrap();
        assert_eq!(
            ui_settings(&aux).hub_url,
            "https://hub.example.com/catalog.json"
        );
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
            cost: None,
            cost_currency: None,
        })
        .unwrap();
        // Seed the request-log rows the headline counts: the forwarded request
        // above plus a pre-forward failure (usage tables never see the latter).
        let log = |status: i64, tokens: (i64, i64)| kiwano_gateway::store::RequestLogNew {
            ts: now.clone(),
            method: "POST".into(),
            path: "/v1/messages".into(),
            query: None,
            agent: Some("claude".into()),
            attribution: Some("key".into()),
            provider_id: Some("p1".into()),
            model: None,
            status_code: status,
            error_kind: None,
            error_message: None,
            session_id: None,
            is_streaming: false,
            input_tokens: tokens.0,
            output_tokens: tokens.1,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(1200),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: None,
            response_body: None,
            request_size: 0,
            response_size: 0,
            truncated: false,
            cost: None,
            cost_currency: None,
        };
        s.insert_request_log(&log(200, (1000, 500))).unwrap();
        s.insert_request_log(&log(503, (0, 0))).unwrap();
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
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // The headline reads request_logs (Logs-card source): both the
        // forwarded and the failed request count, while the usage-derived
        // totals stay limited to the forwarded one.
        assert_eq!(d.requests, 2);
        assert_eq!(d.input_tokens, 1000);
        assert_eq!(d.latency_ms, 1200);
        assert_eq!(d.by_agent[0].tokens, "2k");
        assert_eq!(d.by_provider[0].pct, 100);
        assert!(d.trend.iter().map(|t| t.requests).sum::<i64>() >= 1);

        // Filters narrow every stat to the matching slice — and zero out on
        // a provider with no traffic.
        let fp = build_dashboard(&s, &aux, "7d", Some("p1"), Some("claude")).unwrap();
        assert_eq!(fp.requests, 2);
        assert_eq!(fp.by_provider.len(), 1);
        assert_eq!(fp.by_provider[0].id, "p1");
        assert_eq!(fp.by_agent.len(), 1);
        let fo = build_dashboard(&s, &aux, "7d", Some("ghost"), None).unwrap();
        assert_eq!(fo.requests, 0);
        assert!(fo.by_provider.is_empty());
        assert!(fo.by_agent.is_empty());
    }

    #[test]
    fn export_writes_the_filtered_slice_as_csv() {
        let s = store();
        let now = unix_now();
        // seed_usage_rows writes the request_logs twin too, which is the table
        // the export reads.
        seed_usage_rows(&s, now - 90, 3, 1_000);
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.csv");
        let path = path.to_str().unwrap();

        let out = export_request_logs_csv(&s, path, None, None, None, None, None).unwrap();
        assert_eq!(out.rows_written, 3);
        assert!(!out.truncated);

        let text = std::fs::read_to_string(path).unwrap();
        assert!(text.starts_with('\u{feff}'), "Excel needs the BOM");
        assert_eq!(text.lines().count(), 4, "a header and one line per row");
        assert!(
            text.starts_with("\u{feff}id,ts,method,path"),
            "header first"
        );
        assert!(text.contains("claude"), "the row's agent is in there");

        // The same filter the table gets: a slice with no rows writes a
        // header-only file rather than the whole table.
        let empty =
            export_request_logs_csv(&s, path, Some("codex"), None, None, None, None).unwrap();
        assert_eq!(empty.rows_written, 0);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    #[test]
    fn a_spending_limit_shows_up_for_every_billing_that_has_one() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        let since7 = local_day_start(0, now - 6 * 86_400);

        // A metered provider with a 50 CNY cap and 30 CNY of cost this period.
        let mut metered = provider("payg-1", "Payg", Billing::Metered);
        metered.period_limit = Some(50.0);
        metered.limit_unit = Some("CNY".into());
        s.insert_provider(&metered).unwrap();
        s.record_usage(&kiwano_gateway::store::UsageRecord {
            ts: rfc3339(now - 60),
            agent: "claude".into(),
            provider_id: "payg-1".into(),
            model: Some("demo-model".into()),
            input_tokens: 1_000,
            output_tokens: 200,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            latency_ms: Some(214),
            status: "ok".into(),
            cost: Some(30.0),
            cost_currency: Some("CNY".into()),
        })
        .unwrap();
        let totals = s.usage_totals(None, Some("payg-1"), Some(&since7)).unwrap();
        let payg = usage_vm(&s, &aux, &metered, Some(&totals), &since7).unwrap();
        let q = payg.quota.expect("a payg cap is a quota too");
        assert_eq!((q.used, q.limit), (30.0, 50.0));
        assert_eq!(q.unit, "CNY", "denominated in the provider's own currency");

        // Unlimited has nothing to measure, limit or no limit.
        let mut unl = provider("unl-1", "Local", Billing::Unlimited);
        unl.period_limit = Some(50.0);
        s.insert_provider(&unl).unwrap();
        let totals = s.usage_totals(None, Some("unl-1"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &unl, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());

        // A metered provider with no cap has nothing to ring against, so the
        // card falls back to the usage trend.
        let bare = provider("payg-2", "Bare", Billing::Metered);
        s.insert_provider(&bare).unwrap();
        let totals = s.usage_totals(None, Some("payg-2"), Some(&since7)).unwrap();
        let vm = usage_vm(&s, &aux, &bare, Some(&totals), &since7).unwrap();
        assert!(vm.quota.is_none());
    }

    #[test]
    fn the_agent_filter_narrows_its_own_breakdown() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let now = unix_now();
        // Two agents with traffic, so the table has something it could fail to
        // leave out. Only the first gets the request_logs twin the headline
        // counts — the filtered slice's total is all this needs.
        seed_usage_rows(&s, now - 90, 3, 1_000);
        for i in 0..2 {
            s.record_usage(&kiwano_gateway::store::UsageRecord {
                ts: rfc3339(now - 60 - i),
                agent: "codex".into(),
                provider_id: "demo-alpha".into(),
                model: Some("demo-model".into()),
                input_tokens: 100,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: Some(100),
                status: "ok".into(),
                cost: None,
                cost_currency: None,
            })
            .unwrap();
        }

        let all = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        assert_eq!(all.by_agent.len(), 2, "both agents have traffic");

        let one = build_dashboard(&s, &aux, "7d", None, Some("claude")).unwrap();
        assert_eq!(one.by_agent.len(), 1, "the table narrows with the filter");
        assert_eq!(one.by_agent[0].agent, "claude");
        assert_eq!(one.by_agent[0].requests, 3);
        assert_eq!(
            one.by_agent.iter().map(|a| a.requests).sum::<i64>(),
            one.requests,
            "the breakdown totals the same slice the headline counts"
        );
    }

    #[test]
    fn rfc3339_and_day_helpers() {
        assert_eq!(day_key(0), "1970-01-01");
        assert_eq!(mmdd("2026-09-07"), "09-07");
        assert_eq!(rfc3339(0), "1970-01-01T00:00:00Z");
        // Hour keys match the `YYYY-MM-DDTHH` shape `usage_hourly` groups by,
        // and roll over at midnight like `day_key` does.
        assert_eq!(hour_key(0), "1970-01-01T00");
        assert_eq!(hour_key(7 * 3_600 + 59 * 60), "1970-01-01T07");
        assert_eq!(hour_key(86_400 + 3_600), "1970-01-02T01");
        assert_eq!(hh00("1970-01-01T07"), "07:00");
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

    #[test]
    fn roundrobin_strategy_seeds_even_weights() {
        let s = store();
        for (pid, name, pr) in [("a1", "Alpha", 0), ("b1", "Beta", 1), ("c1", "Gamma", 2)] {
            s.insert_provider(&provider(pid, name, Billing::Metered))
                .unwrap();
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

        // Entering roundrobin splits 100 across the candidates (remainder to
        // the head of the queue): 3 candidates → 34/33/33
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        let weights: Vec<i64> = build_agent_routes(&s).unwrap()[0]
            .bindings
            .iter()
            .map(|b| b.weight)
            .collect();
        assert_eq!(weights, vec![34, 33, 33]);

        // Other strategies leave the weights untouched
        set_agent_strategy(&s, "claude", "failover", None).unwrap();
        let weights: Vec<i64> = build_agent_routes(&s).unwrap()[0]
            .bindings
            .iter()
            .map(|b| b.weight)
            .collect();
        assert_eq!(weights, vec![34, 33, 33]);
    }

    #[test]
    fn in_use_badge_follows_strategy() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
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
        let in_use = |s: &Store| -> (bool, bool) {
            let vms = build_provider_vms(s, &aux).unwrap();
            let cur = |id: &str| vms.iter().find(|p| p.id == id).unwrap().is_current;
            (cur("a1"), cur("b1"))
        };

        // single: only the head serves
        assert_eq!(in_use(&s), (true, false));

        // roundrobin: every candidate takes rotation turns
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        assert_eq!(in_use(&s), (true, true));

        // timewindow: a window containing now moves the badge off the head
        use chrono::Timelike;
        let t = chrono::Local::now().time();
        let now = t.hour() * 60 + t.minute();
        let hhmm = |min: u32| format!("{:02}:{:02}", min / 60 % 24, min % 60);
        // [now-30, now+30] — wraps midnight safely near the day edges
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            win_start: Some(hhmm(now + 1440 - 30)),
            win_end: Some(hhmm(now + 30)),
            enabled: true,
        })
        .unwrap();
        set_agent_strategy(&s, "claude", "timewindow", None).unwrap();
        assert_eq!(in_use(&s), (false, true));

        // timewindow: no window matching now → the fallback head serves
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "b1".into(),
            priority: 1,
            weight: 1,
            // one-minute window later today — can never contain now
            win_start: Some(hhmm(now + 60)),
            win_end: Some(hhmm(now + 60)),
            enabled: true,
        })
        .unwrap();
        assert_eq!(in_use(&s), (true, false));

        // quota: head under threshold; over → first backup (windows ignored)
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":5,"unit":"requests"}"#),
        )
        .unwrap();
        assert_eq!(in_use(&s), (true, false));
        for _ in 0..5 {
            s.record_usage(&kiwano_gateway::store::UsageRecord {
                ts: rfc3339(unix_now()),
                agent: "claude".into(),
                provider_id: "a1".into(),
                model: None,
                input_tokens: 10,
                output_tokens: 0,
                cache_read_tokens: 0,
                cache_creation_tokens: 0,
                latency_ms: None,
                status: "ok".into(),
                cost: None,
                cost_currency: None,
            })
            .unwrap();
        }
        assert_eq!(in_use(&s), (false, true));
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
            cost: None,
            cost_currency: None,
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
    fn cost_alert_skips_unlimited_rows_and_converts_currency_limits() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        let mut payg = provider("ds-1", "DeepSeek", Billing::Metered);
        payg.period_limit = Some(5.0); // NULL unit normalizes to requests
        s.insert_provider(&payg).unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("CNY".into()); // currency limit: compares spent cost
        s.insert_provider(&cny).unwrap();

        // 6 rows each: payg hits the request threshold; the CNY limit compares
        // the period's spent cost (¥10/row → ¥60) against ¥50.
        for _ in 0..6 {
            s.record_usage(&usage_row("ds-1")).unwrap();
            let mut row = usage_row("glm-1");
            row.cost = Some(10.0);
            row.cost_currency = Some("CNY".into());
            s.record_usage(&row).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux).unwrap();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].provider_id, "ds-1");
        assert_eq!(alerts[1].provider_id, "glm-1");
        assert_eq!(alerts[1].unit, "CNY");
        assert!((alerts[1].used - 60.0).abs() < 1e-6);
    }

    #[test]
    fn currency_limits_convert_with_the_hub_rate_table() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        // A Hub price table whose CNY rate is nothing like the bundled one
        // (the bundled table quotes several CNY per USD).
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 2.0 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("CNY".into());
        s.insert_provider(&cny).unwrap();

        let mut row = usage_row("glm-1");
        row.cost = Some(10.0);
        row.cost_currency = Some("USD".into());
        s.record_usage(&row).unwrap();

        // 10 USD is 20 CNY at the Hub's rate — under the 50 CNY limit, so no
        // alert. Converting with the bundled table instead would put it at 71
        // and fire one, which is what makes "no alert" a real assertion.
        let alerts = check_usage_alerts(&s, &aux).unwrap();
        assert!(
            alerts.iter().all(|a| a.provider_id != "glm-1"),
            "10 USD at 2.0 CNY/USD is 20 CNY, under the 50 CNY limit"
        );
    }

    #[test]
    fn period_start_keys() {
        // 2026-09-07T12:34:56Z (Monday)
        let t = 1_788_784_496_i64;
        let (since, key) = period_start(t, Some("monthly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-01T00:00:00Z"));
        assert_eq!(key, "2026-09");
        let (since, key) = period_start(t, Some("weekly"), 0);
        assert_eq!(since.as_deref(), Some("2026-09-07T00:00:00Z"));
        assert_eq!(key, "2026-09-07");
        let (since, key) = period_start(t, Some("yearly"), 0);
        assert_eq!(since.as_deref(), Some("2026-01-01T00:00:00Z"));
        assert_eq!(key, "2026");
        // no reset → all-time totals
        let (since, key) = period_start(t, None, 0);
        assert_eq!(since, None);
        assert_eq!(key, "all");
    }
}
