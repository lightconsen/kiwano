//! View-model layer: maps the gateway `Store` rows to the frontend contract
//! in `src/api/types.ts` (field names must match exactly — serde default
//! snake_case). The UI is mock-free; this is the single source of mapping.
//!
//! Owns an auxiliary SQLite connection on the same database file for reads
//! the gateway store does not expose (average latency, per-provider daily
//! sparkline) plus a GUI-scoped `app_settings` table.

use crate::detect::ShellVars;
use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};

use kiwano_adapters::model_pricing::{DeclaredPrice, DeclaredPrices};
use kiwanod::store::{
    AgentLimit, Billing, Binding, Provider, RequestLogDetail, RequestLogEntry, RequestLogExportRow,
    RequestLogFilter, Store, Strategy, StrategyType, UsageTotals, EXPORT_ROW_CAP,
};
use serde::{Deserialize, Serialize};

pub const AGENTS: [(&str, &str); 14] = [
    ("claude", "Claude Code"),
    ("codex", "Codex"),
    ("gemini", "Gemini CLI"),
    ("grokbuild", "Grok Build"),
    ("claude-desktop", "Claude Desktop"),
    ("opencode", "OpenCode"),
    ("openclaw", "OpenClaw"),
    ("hermes", "Hermes"),
    ("pi", "Pi"),
    ("workbuddy", "WorkBuddy"),
    ("codebuddy", "CodeBuddy Code"),
    ("kimi", "Kimi Code CLI"),
    ("qwen", "Qwen Code"),
    ("cline", "Cline"),
];

/// Whether `id` names a built-in agent — one whose *config* this app knows how
/// to rewrite or detect.
///
/// The distinction matters and is easy to blur: everything that reaches into an
/// agent's files (`takeover`, `creds`, `import`, `detect`) must only ever see a
/// built-in, while anything that *lists* agents (the Apps segments, the
/// dashboard's breakdowns, the provider dialog's multi-select) shows built-ins
/// and user-defined ones together.
pub fn is_builtin_agent(id: &str) -> bool {
    AGENTS.iter().any(|(a, _)| *a == id)
}

/// A user-defined agent's view: a name for a route, the key its traffic is
/// attributed by, and nothing else — there is no config file to report on.
#[derive(Serialize, Deserialize, Clone)]
pub struct CustomAgentVm {
    pub id: String,
    pub label: String,
    pub note: Option<String>,
    /// Always present: the key is minted with the agent and deleted with it, so
    /// unlike a built-in's (read out of its live config), this one cannot be
    /// stale — the row *is* the truth here, and it is the same row the gateway
    /// attributes by.
    pub placeholder_key: Option<String>,
    /// What this agent's clients speak, chosen when it was defined. `None` for
    /// one defined before the field existed — "not said", which is why the UI
    /// reads it as such rather than showing a protocol nobody picked.
    pub protocol: Option<String>,
}

/// Every agent the UI should offer, built-ins first (registry order), then the
/// user's own in the order they were created.
fn list_agents(store: &Store) -> Result<Vec<(String, String)>, String> {
    let mut out: Vec<(String, String)> = AGENTS
        .iter()
        .map(|(id, label)| (id.to_string(), label.to_string()))
        .collect();
    for a in store.list_custom_agents().map_err(e2s)? {
        out.push((a.id, a.label));
    }
    Ok(out)
}

/// The id stem for a user-defined agent: the label's slug, or `custom` when the
/// label has nothing slug-able in it (an all-CJK name). The check is on the
/// label, not on `slug`'s output, so `slug`'s own empty-name fallback ("provider")
/// cannot leak into an agent id.
fn agent_id_stem(label: &str) -> String {
    if label.chars().any(|c| c.is_ascii_alphanumeric()) {
        slug(label)
    } else {
        "custom".to_string()
    }
}

/// One prompt round trip against a provider, for the Apps screen's Test button.
#[derive(Serialize, Deserialize, Clone)]
pub struct PromptLatencyVm {
    pub provider_id: String,
    /// The model the ping was sent with — it decides the number as much as the
    /// network does, so the UI can say which one was measured.
    pub model: String,
    pub latency_ms: u64,
    pub status: u16,
    /// The upstream's own words when it refused, so a failure reads as one.
    pub error: Option<String>,
}

/// What model to ping a provider with: its own default when it has one,
/// otherwise the model its catalog entry publishes a price for — the one model
/// we know the vendor serves.
fn prompt_test_model(store: &Store, aux: &Aux, p: &Provider) -> Option<String> {
    if let Some(m) = p
        .model_default
        .as_deref()
        .map(str::trim)
        .filter(|m| !m.is_empty())
    {
        return Some(m.to_string());
    }
    let catalog = load_catalog(store, aux);
    let entry = catalog
        .entries
        .iter()
        .find(|e| Some(e.id.as_str()) == p.catalog_id.as_deref())?;
    entry.price_ref.as_ref().map(|r| r.model_id.clone())
}

/// Send one prompt through a provider and time it.
///
/// The URL is composed by the gateway's own function (`compose_upstream`), so
/// the test measures the endpoint the gateway would actually use; the ping is a
/// real completion, because a models-list GET answers from a different code path
/// and a different cache and says nothing about what a request costs in time.
pub async fn test_provider_latency(
    store: &Store,
    aux: &Aux,
    id: &str,
) -> Result<PromptLatencyVm, String> {
    let p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider not found: {id}"))?;
    let model = prompt_test_model(store, aux, &p).ok_or_else(|| {
        "no model to test with — set a default model on this provider".to_string()
    })?;
    let key = p
        .api_key
        .as_deref()
        .map(str::trim)
        .filter(|k| !k.is_empty())
        .ok_or_else(|| "this provider has no API key".to_string())?;
    let protocol = p.protocol.as_str();
    let path = if protocol == "anthropic" {
        "/v1/messages"
    } else {
        "/v1/chat/completions"
    };
    let url = kiwanod::server::data::compose_upstream(&p.base_url, p.api_path.as_deref(), path);
    let probe = crate::sidecar::probe_prompt(protocol, &url, key, &model).await;
    // What the click measured outlives the click: it is recorded as this
    // provider's verdict, so the Status column can show it. This is the one
    // measurement that works when the provider has no traffic of its own and the
    // prober has not been round — or is not running at all, which happens
    // whenever the daemon up is a build from before the prober existed. The
    // number on the button stays a readout of *this* click; the cell shows the
    // standing verdict, which this just became.
    let (status, latency_ms, error) = test_verdict(&probe);
    store
        .upsert_provider_health(id, status, latency_ms, "test", error.as_deref())
        .map_err(e2s)?;
    let probe = probe?;
    Ok(PromptLatencyVm {
        provider_id: id.to_string(),
        model,
        latency_ms: probe.latency_ms,
        status: probe.status,
        error: probe.error,
    })
}

/// What the latency test's result means as a health verdict.
///
/// Any HTTP answer proves reachability — a 401 is the vendor saying no to the
/// key, not the network saying nothing — so a refusal is `reachable` with the
/// vendor's own message beside it, and only a transport failure is `down`. The
/// distinction is the whole reason `provider_health` carries an `error` column:
/// the two read differently on the row, and conflating them sends the reader to
/// the wrong end of the problem.
fn test_verdict(
    probe: &Result<crate::sidecar::PromptProbe, String>,
) -> (&'static str, i64, Option<String>) {
    match probe {
        Ok(p) => ("reachable", p.latency_ms as i64, p.error.clone()),
        Err(e) => ("down", 0, Some(e.clone())),
    }
}

/// Create a user-defined agent: a row, a placeholder key, and a `single`
/// strategy — which is all an agent *is* to the gateway, whose route table is
/// built from those tables and never from the registry.
///
/// The id is derived from the label and never changes (bindings, strategies,
/// keys and usage rows reference it); renaming moves the label only. A label the
/// user repeats gets its own agent: the suffix is what makes that possible.
pub fn add_custom_agent(
    store: &Store,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("an agent needs a name".to_string());
    }
    let protocol = normalize_agent_protocol(protocol)?;
    let id = format!(
        "{}-{}",
        agent_id_stem(label),
        &uuid::Uuid::new_v4().simple().to_string()[..6]
    );
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    store
        .insert_custom_agent(&kiwanod::store::CustomAgent {
            id: id.clone(),
            label: label.to_string(),
            note: note.map(str::to_string),
            protocol: protocol.clone(),
            created_at: rfc3339(unix_now()),
        })
        .map_err(e2s)?;
    // Its key, in the same shape a takeover mints — the gateway's attribution
    // does not care where the row came from.
    let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
    let key = format!("kw-ag-{id}-{rand}");
    store.upsert_placeholder_key(&key, &id).map_err(e2s)?;
    // A route with no strategy still routes (the engine reads `single` for a
    // missing row), but writing it here is what makes the agent's tab show the
    // strategy it actually has rather than an implicit default.
    store
        .upsert_strategy(&id, StrategyType::Single, None)
        .map_err(e2s)?;
    Ok(CustomAgentVm {
        id,
        label: label.to_string(),
        note: note.map(str::to_string),
        protocol,
        placeholder_key: Some(key),
    })
}

/// Rename a user-defined agent.
///
/// Only its label and note move: the id is what bindings, routes, keys and
/// usage rows point at, so a rename must not touch it — the agent keeps its
/// route and its history, and only the name its user reads changes.
pub fn update_custom_agent(
    store: &Store,
    id: &str,
    label: &str,
    note: Option<&str>,
    protocol: Option<&str>,
) -> Result<CustomAgentVm, String> {
    let label = label.trim();
    if label.is_empty() {
        return Err("an agent needs a name".to_string());
    }
    let note = note.map(str::trim).filter(|n| !n.is_empty());
    let protocol = normalize_agent_protocol(protocol)?;
    if !store
        .update_custom_agent_label(id, label, note, protocol.as_deref())
        .map_err(e2s)?
    {
        return Err(format!("no such custom agent: {id}"));
    }
    // The same shape `add_custom_agent` returns, key included: the dialog that
    // shows an agent's settings reads it from here too.
    let placeholder_key = store
        .list_placeholder_keys()
        .map_err(e2s)?
        .into_iter()
        .find(|k| k.agent == id)
        .map(|k| k.key);
    Ok(CustomAgentVm {
        id: id.to_string(),
        label: label.to_string(),
        note: note.map(str::to_string),
        protocol,
        placeholder_key,
    })
}

/// A protocol as it is stored: one of the three words, or None for "not said".
///
/// An unknown word is a refusal rather than a row — the field is a label the UI
/// renders and the CLI prints, so a typo in the database would be a word no
/// reader recognises. Empty trims to None: clearing the choice is how a user
/// says they would rather not say.
fn normalize_agent_protocol(protocol: Option<&str>) -> Result<Option<String>, String> {
    match protocol.map(str::trim).filter(|p| !p.is_empty()) {
        None => Ok(None),
        Some(p) => match kiwanod::store::Protocol::parse_str(p) {
            Some(parsed) => Ok(Some(parsed.as_str().to_string())),
            None => Err(format!(
                "unknown protocol: {p} — one of anthropic, openai, gemini"
            )),
        },
    }
}

/// Delete a user-defined agent, and everything that was only about it: its
/// bindings, its strategy, its key, its row. `usage` and `request_logs` are
/// history and stay — the same line the provider deletion draws.
pub fn remove_custom_agent(store: &Store, id: &str) -> Result<(), String> {
    if store.get_custom_agent(id).map_err(e2s)?.is_none() {
        return Err(format!("no such custom agent: {id}"));
    }
    for b in store.bindings_for_agent(id).map_err(e2s)? {
        store.delete_binding(id, &b.provider_id).map_err(e2s)?;
    }
    store.delete_strategy(id).map_err(e2s)?;
    for k in store.list_placeholder_keys().map_err(e2s)? {
        if k.agent == id {
            store.delete_placeholder_key(&k.key).map_err(e2s)?;
        }
    }
    store.delete_custom_agent(id).map_err(e2s)?;
    Ok(())
}

/// The protocols each built-in agent's own clients speak, in the vocabulary
/// `kiwanod::store::Protocol` uses (`"anthropic"`, `"openai"`, `"gemini"`).
///
/// Read off what this app already writes into each tool's config — that is the
/// wire format the tool then reads (`wire_api = "responses"` for Codex, `api:
/// "openai-completions"` for OpenClaw and WorkBuddy, `providers[].type =
/// "openai"` for Kimi, and so on through the rewriters), so it is evidence
/// rather than a catalogue of what each vendor also offers.
///
/// A parallel table keyed by the ids in [`AGENTS`] rather than a third field on
/// it: that registry is destructured as `(agent, label)` in a dozen places, and
/// its neighbours here (`ADDITIVE_AGENTS`, `REBUILDABLE_AGENTS`, `CLI_AGENTS`)
/// are the same shape for the same reason.
///
/// **A label.** Nothing routes, validates or filters by it: the gateway learns
/// an inbound's protocol from the path it was called on
/// (`gateway::protocol::classify_path`), and that is unchanged.
pub const AGENT_PROTOCOLS: [(&str, &[&str]); 14] = [
    ("claude", &["anthropic"]),
    ("codex", &["openai"]),
    ("gemini", &["gemini"]),
    ("grokbuild", &["openai"]),
    ("claude-desktop", &["anthropic"]),
    ("opencode", &["openai"]),
    ("openclaw", &["openai"]),
    ("hermes", &["openai"]),
    ("pi", &["openai"]),
    ("workbuddy", &["openai"]),
    ("codebuddy", &["openai"]),
    ("kimi", &["openai"]),
    ("qwen", &["openai"]),
    ("cline", &["openai"]),
];

/// The protocols `agent` speaks, or an empty slice for an id nobody knows — a
/// user-defined agent is not in that table, and "we have no idea" is the honest
/// answer for one.
pub fn agent_protocols(agent: &str) -> &'static [&'static str] {
    AGENT_PROTOCOLS
        .iter()
        .find(|(id, _)| *id == agent)
        .map(|(_, protocols)| *protocols)
        .unwrap_or(&[])
}

/// Additive-mode agents: their native config keeps multiple providers
/// coexisting, so takeover writes a gateway-pointed provider entry and selects
/// it, instead of replacing an exclusive provider slot like the other five.
pub const ADDITIVE_AGENTS: [&str; 8] = [
    "opencode",
    "openclaw",
    "hermes",
    "pi",
    "workbuddy",
    "codebuddy",
    "kimi",
    "qwen",
];

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

pub fn unix_now() -> i64 {
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

pub fn rfc3339(epoch_secs: i64) -> String {
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

/// Minutes-of-day on the user's clock (timewindow windows are the user's local
/// time). Reads the stored `tz_offset_minutes` rather than the host's zone, so
/// the badge and the gateway — which reads the same field — agree on the hour.
fn local_minutes_now(tz_offset_minutes: i64) -> u32 {
    use chrono::Timelike;
    let local = chrono::Utc::now() + chrono::Duration::minutes(tz_offset_minutes);
    let t = local.time();
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

// ── Auxiliary connection (moved to `crate::auxiliary`, re-exported here) ──

pub use crate::auxiliary::Aux;

// ── VM types (serde field names mirror src/api/types.ts verbatim) ──

#[derive(Serialize)]
pub struct HealthVm {
    /// `ok` | `idle` | `off` | `error` — what the dot is drawn from.
    pub state: String,
    pub latency_ms: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Where `latency_ms` came from: `traffic` (this provider's own requests in
    /// the window) or `probe` (the gateway's reachability check). Absent when
    /// there is no number, and the two must not be read as one: one is a real
    /// round trip with the user's key, the other is an unsigned hello.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    /// When the probe ran, for the cell's tooltip. `probe` only: a traffic
    /// average covers a window rather than an instant.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub checked_at: Option<String>,
    /// What the endpoint said when it said no — the vendor's own message for a
    /// refused key, the transport error when nothing answered. Its presence is
    /// what makes the cell read as "the key" rather than as "no answer": a 401
    /// is the vendor responding, which reachability alone cannot tell apart from
    /// silence.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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
    /// Needed for the cache hit rate's denominator: input-side tokens are
    /// new input plus cache reads plus cache writes, and a rate that forgets
    /// the writes overstates every hit.
    pub cache_creation_tokens: i64,
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
    /// The catalog entry this provider was added from, when it came from the
    /// shelf. The dialog needs it to reach the entry again: the entry is the
    /// authority on the endpoints the provider answers on and on the currency it
    /// bills in, and a stored row can be missing both.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub catalog_id: Option<String>,
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
    /// The model the add/edit form collected as this provider's default, for
    /// edit prefill. Absent when never set.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_default: Option<String>,
    pub enabled: bool,
    pub agents: Vec<String>,
    /// Agents this provider would serve a request for right now (per-agent
    /// slice of the strategy serving map). The All tab badges the collapsed
    /// `is_current`; an agent tab badges membership here instead, so a
    /// provider serving another agent does not read as in-use locally.
    pub serving_agents: Vec<String>,
    /// Agents for which this provider is the quota strategy's configured first
    /// backup while that agent's primary is over its threshold. Not "in use":
    /// which backup actually serves depends on the gateway's breakers at
    /// request time, so the UI badges this as first-in-line instead.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub fallback_agents: Vec<String>,
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
    /// The prices the user declared for this provider:
    /// `{"currency":"CNY","models":[…]}`. Absent = none declared, so its
    /// requests are priced from the Hub's table (or recorded unpriced when the
    /// Hub knows nothing about the model either). Edit prefill: the form that
    /// collected them is the only one that can correct them.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prices: Option<serde_json::Value>,
}

/// Catalog-side billing vocabulary (`plan` | `payg` | `unl`).
///
/// Serialized as the bare lowercase tag, so the wire shape is unchanged. An
/// unrecognized tag is *not* silently coerced to `payg`: it is preserved
/// verbatim in `Other` and written back byte-identically, which keeps the hub
/// cache round-trip stable. One bad row must not fail a whole sync — the
/// catalog is the primary resource, and a sync that refuses a payload over one
/// odd tag leaves the shelf on whatever it had.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "String", into = "String")]
pub enum CatalogBilling {
    Plan,
    Payg,
    Unl,
    /// A vendor that charges both ways at **one address**: Anthropic sells an API
    /// (metered) and Pro/Max (subscription), and both answer on
    /// `api.anthropic.com`. A local provider row can only be one of them, so this
    /// value never reaches the database — the user settles it when adding, and
    /// [`billing_to_db`] refuses it with a message that says so.
    ///
    /// It has to be a tag of its own rather than a second catalog entry because
    /// [`catalog_id_for`] matches a local provider to an entry by endpoint and
    /// gives up when two entries share one: a second Anthropic entry would leave
    /// every hand-added Anthropic provider unlinked, and unlinked ones are priced
    /// from someone else's rates.
    Both,
    /// Unrecognized tag, kept verbatim for lossless round-tripping.
    Other(String),
}

impl CatalogBilling {
    pub fn as_str(&self) -> &str {
        match self {
            CatalogBilling::Plan => "plan",
            CatalogBilling::Payg => "payg",
            CatalogBilling::Unl => "unl",
            CatalogBilling::Both => "both",
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
            "both" => Some(CatalogBilling::Both),
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
    /// Letter-avatar colour, derived at load time from `name` with the same
    /// palette the locally-added providers use. The Hub used to curate a brand
    /// colour per provider; those are not derivable (they were hand-picked), so
    /// dropping the field traded them for one consistent palette everywhere.
    #[serde(default)]
    pub logo_color: String,
    /// Hub-relative logo path ("logos/<id>.<ext>"); the frontend resolves it
    /// against hub_url. Absent while the setting loads, and in the bundled
    /// snapshot, which falls back to the letter avatar.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub logo: Option<String>,
    /// Primary endpoint protocol — derived at load time from the first entry of
    /// `endpoints`. The Hub publishes one list, primary first.
    #[serde(default)]
    pub protocol: String,
    /// Primary endpoint URL — derived at load time, see `protocol`.
    #[serde(default)]
    pub endpoint: String,
    /// The models the primary endpoint serves — derived at load time, see
    /// `protocol`. It seeds the add modal's model picker.
    #[serde(default)]
    pub models: Vec<String>,
    /// Every endpoint *after* the primary. The Hub publishes them together with
    /// the primary in one list; the split happens here because every screen
    /// reads the primary from its own fields.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub endpoints: Vec<CatalogEndpointVm>,
    pub tag: String,
    pub rating: f64,
    /// One line of prose about the provider — what the card shows under its
    /// name. It supersedes the old price_line/price_note/users/blurb/free_offer
    /// fields; absent for the entries that have nothing to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub desc: Option<String>,
    /// The price of the model that represents this provider, projected at build
    /// time from the data repo's `flagship` flag. Absent for a provider that
    /// prices no model at all (eleven of them), which falls back to `desc`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub price_ref: Option<CatalogPriceRefVm>,
    /// The currency this provider bills in. Its price rows — and therefore its
    /// spending limit — are denominated in it. Catalogs published before the
    /// field existed fall back to USD, the price table's base currency.
    #[serde(default = "default_catalog_currency")]
    pub currency: String,
    /// The provider's own site (`https://deepseek.com/`). Published since the
    /// Hub started carrying it; nothing local can stand in for it, so the field
    /// only ever comes from there.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub website: Option<String>,
    /// How to read this provider's plan usage, when the vendor's own endpoint
    /// can answer with nothing but the provider's key: `{"template": "<id>"}`,
    /// and never anything else — credentials are the user's, and a catalog is
    /// public.
    ///
    /// Only four entries carry it. `billing == Plan` does **not** imply it: the
    /// other fourteen plan providers publish no such endpoint, or want a second
    /// credential. That distinction is the whole point of the field — it is what
    /// separates "bills by plan" from "can be asked how much of the plan is
    /// spent", and only the latter is worth offering a ceiling for.
    ///
    /// Declaring it here is not bookkeeping: `sync` re-serializes the parsed
    /// catalog before caching it, so a field this struct does not name is gone by
    /// the end of the first sync and no frontend can read it afterwards.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plan_query: Option<serde_json::Value>,
    /// Billing mode; unknown Hub tags survive as `CatalogBilling::Other`.
    pub billing: CatalogBilling,
    /// Derived at load time from the local provider list (same endpoint =
    /// added). The Hub does not publish it.
    #[serde(default)]
    pub added: bool,
}

/// One model's price, as shown for a provider on the Models list: the terms a
/// shopper compares before adding a provider. Figures are per million tokens in
/// TEXT decimal form, the same shape the price table uses — the frontend parses
/// them where it needs arithmetic.
#[derive(Serialize, Deserialize, Clone)]
pub struct CatalogPriceRefVm {
    pub model_id: String,
    /// The model's display name, e.g. "Claude Opus 5". Shown beside the figures
    /// so a row does not read as a bare pair of numbers.
    pub display_name: String,
    pub input: String,
    pub output: String,
    /// ISO-4217 code these figures are denominated in — the provider's own
    /// currency, so the frontend converts rather than assumes.
    pub currency: String,
    /// The rates in force outside `peak_hours`, when the provider publishes a
    /// schedule. The figures above are then the **peak** ones, and the panel
    /// says so — a reader who is only shown the peak would compare the wrong
    /// number.
    ///
    /// The Hub copies these from the entry's flagship model and publishes the
    /// two together or not at all, so they are present exactly when the entry's
    /// flagship is priced by time of day.
    /// The rate band that applies once a request's input passes `over`, on the
    /// same terms. This projection and the mirror's row are two independent
    /// paths to the same figures — the shelf reads this one for its price cell
    /// and the dialog reads the mirror — so a band has to be declared on both or
    /// half the screens would show it and half would not.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub off_peak: Option<kiwano_adapters::model_pricing::OffPeakRates>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub peak_hours: Option<kiwano_adapters::model_pricing::PeakHours>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub long_context: Option<kiwano_adapters::model_pricing::LongContextRates>,
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
    /// pricing (unreachable, malformed, or absent — the previous cache, if any,
    /// stands).
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
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
}

#[derive(Serialize)]
pub struct AgentDistVm {
    pub agent: String,
    pub label: String,
    pub requests: i64,
    pub tokens: String,
    pub cost: f64,
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
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
    /// The peak premium: what these requests would have cost had they all run
    /// at their rows' off-peak rates. Zero for a model with no schedule — and,
    /// deliberately, for traffic that was already off-peak, where the discount
    /// was simply taken. Not a "saving" the user could bank: it prices the same
    /// tokens at the same rows' other rate.
    pub cost_off_peak: f64,
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

#[derive(Serialize, Deserialize, Clone)]
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

/// Provider currency assumed when a catalog entry does not declare one — the
/// same default `generate.mjs` applies on the Hub side.
pub fn default_catalog_currency() -> String {
    "USD".into()
}

pub fn default_hub_url() -> String {
    crate::sync::DEFAULT_HUB_URL.into()
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

pub fn default_true() -> bool {
    true
}

/// Mirrors `kiwanod::store::StreamTimeouts::default`, which is what the gateway
/// falls back to when the settings row is absent. Kept in step by the round-trip
/// test below rather than by hoping.
pub fn default_stream_first_byte_secs() -> u32 {
    120
}

pub fn default_stream_idle_secs() -> u32 {
    120
}

#[derive(Serialize)]
pub struct GatewayStatusVm {
    pub running: bool,
    pub port: u16,
    /// Providers the gateway is refusing to route, with the reason it gave.
    ///
    /// Read from the gateway rather than recomputed here: the block is decided
    /// there, and a card that worked out its own answer could disagree with the
    /// process actually turning requests away.
    pub blocked: Vec<BlockedProviderVm>,
}

#[derive(Serialize)]
pub struct BlockedProviderVm {
    pub id: String,
    pub reason: String,
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

/// The prices a user declared for a provider, as the modal form sends them: one
/// currency for every figure, one row per model.
///
/// Asked for on a pay-as-you-go provider the form is the user's own (a hand-added
/// one, or an edit) because the Hub prices the models of *its* catalog entries —
/// a provider that names none has no published price to be costed at, so the user
/// is the only one who can say what it charges. These figures are also what its
/// spending limit is measured against.
#[derive(Deserialize)]
pub struct ProviderPricesInput {
    /// ISO code the figures are denominated in. The form fills it from the same
    /// picker the spending limit uses: both are about what this provider bills.
    pub currency: String,
    #[serde(default)]
    pub models: Vec<ProviderPriceInput>,
}

/// One model's declared rates, per million tokens, as the user typed them.
#[derive(Deserialize)]
pub struct ProviderPriceInput {
    pub model_id: String,
    /// Kept as text: every rate in the price table is a TEXT decimal, and
    /// re-printing one from the parsed f64 would rewrite what the user wrote.
    pub input: String,
    pub output: String,
    /// Blank is zero — "this vendor charges nothing for that bucket", which is
    /// what the form's hint says. Charging the input rate instead would invent a
    /// charge and overstate the spend a limit is measured against.
    #[serde(default)]
    pub cache_read: Option<String>,
    #[serde(default)]
    pub cache_creation: Option<String>,
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
    /// The model the form collected as this provider's default.
    ///
    /// It was carried here for a long time and read by nothing — stored in no
    /// column and returned by no view, so reopening a provider always showed an
    /// empty box. Persisted since v14 (`providers.model_default`), which is what
    /// lets the edit dialog show what the add dialog asked for.
    ///
    /// Remembered rather than consulted: nothing picks a model from it when
    /// routing, because the model a request uses is the one the agent sent. Empty
    /// stores `NULL`.
    pub model_default: String,
    pub billing: String,
    pub billing_config: BillingConfigInput,
    /// Agents to bind this provider to.
    ///
    /// Expected when adding — a new provider nothing serves is a dead row — and
    /// **absent when editing**, where the bindings belong to the Apps screen's
    /// agent tabs. The distinction has to be in the type: a plain `Vec` cannot
    /// tell "no agents" from "not speaking about agents", and the difference is
    /// whether an edit leaves the bindings alone or unbinds the lot.
    #[serde(default)]
    pub agents: Option<Vec<String>>,
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
    /// The prices the user declared for this provider. Absent in an update =
    /// keep what is stored (same semantics as an empty `api_key`); a present
    /// bundle is an authoritative snapshot, so an empty model list clears the
    /// column — which is what switching a provider off pay-as-you-go does.
    #[serde(default)]
    pub prices: Option<ProviderPricesInput>,
    /// The Hub catalog entry this provider is being added from, when the add
    /// came from the shelf. Prices are published per catalog entry, and a local
    /// row's own id is `<slug>-<hex>`, so this is what lets a forwarded request
    /// be costed at its provider's own rate rather than the general one.
    ///
    /// Absent (the hand-added form, and every edit) means "no catalog entry":
    /// on add the provider prices at the general rate, and on update the stored
    /// value is kept — an edit must not silently unlink the provider from its
    /// price row.
    #[serde(default)]
    pub catalog_id: Option<String>,
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
        // Known, and still refused: the catalog says this vendor charges two
        // ways, and a local row holds one. A distinct message because "unknown
        // billing" would send whoever reads it looking for a broken catalog.
        "both" => {
            Err("billing \"both\" must be resolved to plan or payg before saving".to_string())
        }
        other => Err(format!(
            "unknown billing \"{other}\" (expected plan|payg|unl)"
        )),
    }
}

pub fn billing_to_ui(db: Billing) -> &'static str {
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

/// The bound agents whose config actually points at the gateway *right now*.
///
/// Turning a takeover off drops that agent's route (`set_agent_takeover`), so
/// what is left here is the odd row: an agent whose config was reverted behind
/// Kiwano's back by another tool, a store written by a build that kept dormant
/// routes, a binding an import landed on an agent that was never taken over.
/// In every one of them the agent's traffic goes to its own provider rather than
/// to this gateway, so a provider must not read as bound to it — or, worse, as
/// *in use* by it, which is what the list claimed for every binding row.
///
/// Recognition is by the live file (`kw-ag-<agent>-…` in the config the
/// takeover wrote), never by the `placeholder_keys` table: a key row can
/// outlive its rewrite. The takeover panel reads the same files and counts
/// *more* agents than this on purpose — it also accepts a restorable backup,
/// which is a claim about being able to undo a takeover, not about traffic
/// arriving here.
fn live_bound_agents(store: &Store, home: &Path, vars: &ShellVars) -> Result<Vec<String>, String> {
    let custom: Vec<String> = store
        .list_custom_agents()
        .map_err(e2s)?
        .into_iter()
        .map(|a| a.id)
        .collect();
    let bound = store.bound_agents().map_err(e2s)?;
    Ok(bound
        .into_iter()
        // A user-defined agent routes as long as it exists: it has no config
        // file for the evidence to be missing from, and asking for one would
        // report its providers as unbound — the falsehood this whole helper was
        // added to remove, wearing a new hat.
        .filter(|agent| {
            custom.contains(agent)
                || crate::takeover::live_placeholder_key(agent, home, vars).is_some()
        })
        .collect())
}

/// Provider rows for the Apps screen. `home` roots the agent config files the
/// takeover state is read from, so a caller that knows its own tree (the CLI's
/// `--home`) does not have to settle for `$HOME`.
pub fn build_provider_vms(
    store: &Store,
    aux: &Aux,
    home: &Path,
    vars: &ShellVars,
) -> Result<Vec<ProviderVm>, String> {
    let providers = store.list_providers().map_err(e2s)?;
    if providers.is_empty() {
        return Ok(Vec::new());
    }

    let now = unix_now();
    let since7 = rfc3339(now - 7 * 86_400);
    // The window the prober scopes itself by: a provider with a request in it is
    // one whose latency this build can already show, so the probe skips it and
    // the row shows the traffic number instead. The two must agree, or a row
    // would show a probe while the prober considered it spoken for.
    let since_day = rfc3339(now - 86_400);

    // Only agents that route through this gateway have a say in the badges:
    // everything below — the agent column, "In use", the agent-count note —
    // is a claim about live traffic, and a dormant route carries none.
    let live: Vec<String> = live_bound_agents(store, home, vars)?;

    // agent → primary provider id (single strategy)
    let mut primary: HashMap<String, String> = HashMap::new();
    for agent in live.iter() {
        if let Some(id) = store.primary_provider_id(agent).map_err(e2s)? {
            primary.insert(agent.clone(), id);
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
    // Providers the user parked: their bindings stay (the route is their intent,
    // and re-enabling restores it), but the gateway's route table drops them
    // (`RouteTable::load` checks `p.enabled`), so nothing may read as served.
    let parked: HashSet<&str> = providers
        .iter()
        .filter(|p| !p.enabled)
        .map(|p| p.id.as_str())
        .collect();
    let mut serving: HashMap<String, HashSet<String>> = HashMap::new();
    // The quota strategy's configured first backup, while that agent's primary
    // is over its threshold. Kept apart from `serving`: "first in line" is not
    // "in use" — which backup actually serves depends on the gateway's breakers
    // at request time, and the gateway/CLI reader that says "In use" here would
    // guess wrong. The All tab badges it distinctly; agent tabs likewise.
    let mut fallback: HashMap<String, HashSet<String>> = HashMap::new();
    for agent in live.iter() {
        let enabled: Vec<Binding> = store
            .bindings_for_agent(agent)
            .map_err(e2s)?
            .into_iter()
            .filter(|b| b.enabled && !parked.contains(b.provider_id.as_str()))
            .collect();
        let Some(head) = enabled.first().map(|b| b.provider_id.clone()) else {
            continue;
        };
        let strategy = store
            .get_strategy(agent.as_str())
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
                let now = local_minutes_now(tz_offset(aux));
                let hit = enabled.iter().find(|b| {
                    matches!(
                        (b.win_start.as_deref(), b.win_end.as_deref()),
                        (Some(s), Some(e)) if in_window(now, s, e)
                    )
                });
                HashSet::from([hit.map(|b| b.provider_id.clone()).unwrap_or(head)])
            }
            // under threshold → the primary serves; over → the primary does not
            // serve and the first backup is only *first in line*, so it lands in
            // `fallback` rather than `serving` — the gateway's breakers decide
            // which backup (if any) actually takes a request, and the UI must
            // not claim one it has not observed.
            StrategyType::Quota => {
                let over = quota_over_threshold(store, aux, strategy.config.as_deref(), &head);
                if over {
                    if let Some(backup) = enabled.get(1) {
                        fallback.insert(agent.clone(), HashSet::from([backup.provider_id.clone()]));
                    }
                    HashSet::new()
                } else {
                    HashSet::from([head])
                }
            }
            // single / failover: the head (failover degradation is breaker runtime)
            _ => HashSet::from([head]),
        };
        serving.insert(agent.clone(), ids);
    }

    // provider → its last probe verdict, read whole: the rows below all come
    // from one pass over the table rather than a lookup each.
    let health_by_id: HashMap<String, kiwanod::store::ProviderHealth> = store
        .list_provider_health()
        .map_err(e2s)?
        .into_iter()
        .map(|h| (h.provider_id.clone(), h))
        .collect();

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
            let mut fallback_agents: Vec<String> = Vec::new();
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
                if fallback.get(agent).is_some_and(|ids| ids.contains(&p.id)) {
                    fallback_agents.push(agent.clone());
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
            fallback_agents.sort();
            let is_current = !serving_agents.is_empty();

            let note = if backup_for_any {
                Some("Failover queue".to_string())
            } else if !agents.is_empty() {
                Some(format!("{} agent(s)", agents.len()))
            } else {
                None
            };

            let health = health_vm(aux, &p, &since_day, health_by_id.get(&p.id));
            let usage = usage_vm(store, aux, &p, usage_by_id.get(&p.id), &since7);

            ProviderVm {
                id: p.id.clone(),
                name: p.name.clone(),
                logo_char: logo_char(&p.name),
                logo_color: palette_color(&p.name).to_string(),
                logo_border: false,
                catalog_id: p.catalog_id.clone(),
                endpoint: display_endpoint(&p),
                protocol: p.protocol.as_str().to_string(),
                endpoint_note: endpoint_note(&p),
                endpoints: vm_endpoints(&p),
                billing: billing_to_ui(p.billing).to_string(),
                plan_price: kiwanod::plan_quota::plan_monthly_price(p.plan_query.as_deref()),
                limit_unit: p.limit_unit.clone(),
                model_default: p.model_default.clone(),
                plan_limits: p
                    .plan_limits
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok()),
                prices: p
                    .prices
                    .as_deref()
                    .and_then(|s| serde_json::from_str(s).ok()),
                enabled: p.enabled,
                agents,
                serving_agents,
                fallback_agents,
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
    /// The agent's own ceilings, one per window, empty when it has none. Not part
    /// of the strategy — they hold under every one of them — but read with the
    /// route because that is the fetch the agent's tab already makes.
    #[serde(default)]
    pub limits: Vec<AgentLimitVm>,
}

/// One window of an agent's spend ceiling, as the Apps screen edits it.
///
/// An agent holds a list of these: a day's ceiling and a month's answer different
/// questions, and being over either is being over. The screen edits the list as a
/// set, which is why it is a `Vec` at every layer rather than a struct with a
/// window per field.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentLimitVm {
    /// `day` | `weekly` | `monthly` | `yearly` | `all` — the window this ceiling
    /// is measured over.
    pub period: String,
    pub period_limit: f64,
    /// `requests` (default), `wan_tokens`, or a 3-letter currency code.
    pub limit_unit: Option<String>,
}

impl AgentLimitVm {
    /// Read stored rows as the screen sees them.
    pub fn from_store(limits: Vec<AgentLimit>) -> Vec<Self> {
        limits
            .into_iter()
            .map(|l| AgentLimitVm {
                period: l.period,
                period_limit: l.period_limit,
                limit_unit: l.limit_unit,
            })
            .collect()
    }
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
            limits: AgentLimitVm::from_store(store.agent_limits_for(&agent).map_err(e2s)?),
            agent,
            bindings,
        });
    }
    Ok(routes)
}

/// A path as the screen should print it: `~` for the home tree, absolute for
/// anything outside it (a `HERMES_HOME` pointing elsewhere is a real path, and
/// rewriting it to `~/…` would name a file that does not exist).
///
/// Joined rather than formatted, so the separator this *adds* is the platform's:
/// gluing a literal `~/` to a Windows path produced `~/.claude\settings.json`, a
/// spelling neither platform uses. What it does not do is rewrite the separators
/// already inside `path` — a path built component by component (as every one of
/// these is) already spells them natively, and one that does not is somebody's
/// own text, which is not this function's to re-punctuate.
fn display_path(path: &Path, home: &Path) -> String {
    match path.strip_prefix(home) {
        Ok(rest) => Path::new("~").join(rest).display().to_string(),
        Err(_) => path.display().to_string(),
    }
}

/// Set or clear one agent's own ceiling. `None` clears it — an absent row is the
/// absence of a limit, which is what the gateway reads as "no ceiling".
pub fn set_agent_limits(
    store: &Store,
    agent: &str,
    limits: Vec<AgentLimitVm>,
) -> Result<(), String> {
    let now = kiwanod::store::now_rfc3339();
    let rows: Vec<AgentLimit> = limits
        .into_iter()
        // A window of zero is not a window of nothing: the gateway reads it as
        // "no ceiling at all" (see `limits::agent_limit_usage`), so storing one
        // would show the user a limit that does not exist. The screen sends what
        // its fields hold, including an empty or half-typed one, and this is where
        // that is dropped rather than being written down. `is_finite` first, for
        // the reason the quota config does it: a NaN compares false against
        // everything.
        .filter(|l| l.period_limit.is_finite() && l.period_limit > 0.0 && !l.period.is_empty())
        .map(|l| AgentLimit {
            agent: agent.to_string(),
            period: l.period,
            period_limit: l.period_limit,
            limit_unit: l.limit_unit,
            created_at: now.clone(),
            updated_at: now.clone(),
        })
        .collect();
    // An empty set is how the screen says "no limit"; `replace` writes it as the
    // absence of rows.
    store.replace_agent_limits(agent, &rows).map_err(e2s)
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

fn protocol_label(p: kiwanod::store::Protocol) -> &'static str {
    match p {
        kiwanod::store::Protocol::OpenAI => "OpenAI-compatible",
        kiwanod::store::Protocol::Anthropic => "Anthropic",
        kiwanod::store::Protocol::Gemini => "Gemini",
    }
}

fn endpoint_note(p: &Provider) -> String {
    let mut note = protocol_label(p.protocol).to_string();
    // Additional endpoints surface in the same subtitle: "OpenAI-compatible · +Anthropic".
    for e in &p.endpoints {
        let tag = match e.protocol {
            kiwanod::store::Protocol::OpenAI => "OpenAI",
            kiwanod::store::Protocol::Anthropic => "Anthropic",
            kiwanod::store::Protocol::Gemini => "Gemini",
        };
        note.push_str(" · +");
        note.push_str(tag);
    }
    note
}

/// What the row says about a provider nobody has just asked.
///
/// The gateway used to keep a background probe's verdict here (every 30s, one
/// HTTP request per provider, written to `provider_health`), and the row showed
/// it as a green dot and a latency. That signal never routed anything — the
/// breaker, fed by real traffic, is what decides — so it was a continuous
/// background request per provider for a badge, and it is gone. An enabled
/// provider now reads as neutral rather than as healthy, which is the honest
/// answer to "is it up?" when the way to find out is to ask it: the row's Test
/// button measures one, and the request log is where failures show up.
/// What the Status column shows, and where the number came from.
///
/// Two sources, in this order, because they answer the same question with
/// different authority:
///
/// 1. **The provider's own requests** inside `since`. A round trip through the
///    gateway, with the user's key, to the model they actually route to — the
///    number is already in the usage table, so showing it costs nothing and it
///    is the most honest of the two.
/// 2. **The prober's verdict**, for a provider with nothing of its own to
///    measure (just added, or idle since yesterday). An unsigned GET: it says
///    whether something answers at that endpoint, never whether the key works.
///    The `source` field is what keeps the two apart downstream.
///
/// A parked provider answers neither question — it is out of every route, and
/// that is the fact worth showing.
fn health_vm(
    aux: &Aux,
    p: &Provider,
    since: &str,
    probe: Option<&kiwanod::store::ProviderHealth>,
) -> HealthVm {
    if !p.enabled {
        return HealthVm {
            state: "off".into(),
            latency_ms: None,
            note: Some("Disabled".into()),
            source: None,
            checked_at: None,
            error: None,
        };
    }
    if let Some(ms) = aux.avg_latency(Some(&p.id), None, Some(since), None) {
        return HealthVm {
            state: "ok".into(),
            latency_ms: Some(ms),
            note: None,
            source: Some("traffic".into()),
            checked_at: None,
            error: None,
        };
    }
    match probe {
        // Answered. The number is a round trip, and `source` says whose: the
        // prober's unsigned GET, or the Apps screen's own test — which sent a
        // real prompt with the provider's key, and is therefore the stronger
        // claim of the two.
        Some(h) if h.status == "reachable" => HealthVm {
            // A refusal is reachability too: the vendor answered, and what it
            // said was no. The cell reads that as the key rather than as
            // silence, which is the difference the tooltip carries.
            state: if h.error.is_some() {
                "error".into()
            } else {
                "ok".into()
            },
            latency_ms: h.latency_ms,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        Some(h) => HealthVm {
            // No answer at all. Not a latency to print but a fact to show: the
            // endpoint did not respond when it was last asked.
            state: "error".into(),
            latency_ms: None,
            note: None,
            source: Some(h.source.clone()),
            checked_at: Some(h.checked_at.clone()),
            error: h.error.clone(),
        },
        // Nothing measured it yet — a provider added a moment ago, or one whose
        // first probe has not come round. Nothing to say, which is what the
        // blank cell has always meant.
        None => HealthVm {
            state: "idle".into(),
            latency_ms: None,
            note: None,
            source: None,
            checked_at: None,
            error: None,
        },
    }
}

/// The currencies a spending limit may be denominated in.
///
/// The Hub's rate table, because that is what makes the limit comparable with the
/// costs it is measured against: `convert_cost_buckets` needs a rate for both
/// sides, and a currency without one is **added** to the others at 1:1 — the bug
/// that once let a `¥50` limit mean nothing in particular.
///
/// A machine that has never synced has no table at all, and then it is the two
/// currencies the Hub publishes rates against. Empty is not "anything goes": the
/// reason for the rule is that no rate exists, and that is true of every third
/// currency as well.
pub fn known_limit_currencies(store: &Store) -> Vec<String> {
    let rates = store.hub_exchange_rates();
    if rates.is_empty() {
        vec!["USD".to_string(), "CNY".to_string()]
    } else {
        let mut codes: Vec<String> = rates.into_keys().collect();
        codes.sort();
        codes
    }
}

/// Normalize the user-entered per-period limit unit (tech.md §2.4 A).
///
/// With a limit set but no unit chosen, fall back to the legacy behavior of
/// counting "requests"; with no limit the unit is meaningless and stored as NULL.
/// Units are the two counting units plus a currency code from `known` (stored
/// uppercase; the v9 CHECK constraint enforces the same shape).
///
/// A currency this machine cannot price against is **refused** rather than
/// dropped: falling back to `requests` would quietly turn a money ceiling into a
/// request count, which is a different limit, not a smaller one.
pub fn normalize_limit_unit(
    unit: Option<&str>,
    has_limit: bool,
    known: &[String],
) -> Result<Option<String>, String> {
    if !has_limit {
        return Ok(None);
    }
    match unit {
        Some("wan_tokens") => Ok(Some("wan_tokens".into())),
        Some("requests") => Ok(Some("requests".into())),
        Some(u) => {
            let code = u.trim().to_ascii_uppercase();
            // Not a currency code at all: the legacy reading, count requests.
            if code.len() != 3 || !code.chars().all(|c| c.is_ascii_alphabetic()) {
                return Ok(Some("requests".into()));
            }
            if is_known_currency(&code, known) {
                Ok(Some(code))
            } else {
                Err(format!(
                    "a limit cannot be in {code}: this machine has no rate for it, and the limit is \
                     measured against costs priced in other currencies. Known: {}",
                    if known.is_empty() {
                        "none (the Hub has never been synced)".to_string()
                    } else {
                        known.join(", ")
                    }
                ))
            }
        }
        None => Ok(Some("requests".into())),
    }
}

/// Whether this machine can convert amounts in `code`.
///
/// One rule for two callers: the unit a spending limit is denominated in and the
/// currency declared prices are written in. Both end up compared against usage
/// the machine costs in whatever currency the price table names, and
/// `convert_amount` passes an unknown currency through unchanged rather than
/// inventing a rate — so an amount in one would be compared against a limit in
/// another as though the numbers meant the same thing.
fn is_known_currency(code: &str, known: &[String]) -> bool {
    known.iter().any(|k| k.eq_ignore_ascii_case(code))
}

/// One declared rate, validated, with a blank meaning zero.
///
/// The text is trimmed and kept as written: `0.80` and `0.8` are the same rate,
/// and the one the reader sees back should be the one they typed. Anything that
/// is not a non-negative number is refused — the price table parses these as f64
/// at cost time, so a stray character would otherwise turn into a NaN (or a
/// zero) in the recorded cost of every request to that provider.
fn declared_rate(raw: &str, model_id: &str, what: &str) -> Result<String, String> {
    let text = raw.trim();
    if text.is_empty() {
        return Ok("0".to_string());
    }
    let value: f64 = text
        .parse()
        .map_err(|_| format!("the {what} rate of `{model_id}` is not a number: `{text}`"))?;
    if !value.is_finite() || value < 0.0 {
        return Err(format!(
            "the {what} rate of `{model_id}` must be zero or more: `{text}`"
        ));
    }
    Ok(text.to_string())
}

/// Validate the declared prices and serialize them into the `providers.prices`
/// blob.
///
/// `None` (the field absent from the request) is "not speaking about prices": an
/// update keeps what is stored, exactly as it does for `advanced` and
/// `plan_query`. `Some` is an authoritative snapshot, so a bundle that holds no
/// model clears the column.
///
/// Both refusals below are refusals rather than silent drops, because either
/// would leave the provider quietly mispriced:
/// - **A currency this machine cannot convert** — the figures are what the
///   spending limit is measured against, and they are recorded in this currency.
/// - **A rate that is not a non-negative number** — see `declared_rate`.
///
/// A row with a blank model id is dropped (that is the form's empty tail row)
/// and a duplicated model id keeps the first, the same rule the modal applies to
/// a duplicated protocol when it saves the endpoint list.
pub fn normalize_declared_prices(
    input: Option<&ProviderPricesInput>,
    known: &[String],
) -> Result<Option<String>, String> {
    let Some(input) = input else {
        return Ok(None);
    };
    let currency = input.currency.trim().to_ascii_uppercase();
    if currency.len() != 3 || !currency.chars().all(|c| c.is_ascii_alphabetic()) {
        return Err(format!("`{currency}` is not a currency code"));
    }
    if !is_known_currency(&currency, known) {
        return Err(format!(
            "prices cannot be in {currency}: this machine has no rate for it, and the spending \
             limit is measured against costs priced in other currencies. Known: {}",
            if known.is_empty() {
                "none (the Hub has never been synced)".to_string()
            } else {
                known.join(", ")
            }
        ));
    }
    let mut seen: HashSet<String> = HashSet::new();
    let mut models = Vec::new();
    for row in &input.models {
        let model_id = row.model_id.trim();
        if model_id.is_empty() {
            continue;
        }
        // Case-insensitive, because that is how the table keys a model: two rows
        // differing only in case would collide there and one would win by write
        // order rather than by anything the user chose.
        if !seen.insert(model_id.to_ascii_lowercase()) {
            continue;
        }
        models.push(DeclaredPrice {
            model_id: model_id.to_string(),
            input: declared_rate(&row.input, model_id, "input")?,
            output: declared_rate(&row.output, model_id, "output")?,
            cache_read: declared_rate(
                row.cache_read.as_deref().unwrap_or(""),
                model_id,
                "cache read",
            )?,
            cache_creation: declared_rate(
                row.cache_creation.as_deref().unwrap_or(""),
                model_id,
                "cache write",
            )?,
        });
    }
    if models.is_empty() {
        return Ok(None);
    }
    Ok(Some(DeclaredPrices { currency, models }.to_json()))
}

/// The declared prices of an imported provider, validated for *this* machine.
///
/// A shared config carries the blob the exporting install wrote (the shape
/// `normalize_declared_prices` produces), and it arrives the same way a limit's
/// unit does: written where that currency could be converted, read where it may
/// not be. Same rule, then — refused rather than re-denominated, since dropping
/// it would let the provider's requests be costed by the Hub's table instead
/// without anyone having said so.
///
/// The one exception is a blob this build cannot read. That one says nothing to
/// honour, so there is no statement to refuse: it is treated as "none declared",
/// which is both the honest reading of a field written in a shape this build does
/// not know and the state most providers are in anyway.
pub fn import_declared_prices(
    raw: Option<&str>,
    known: &[String],
) -> Result<Option<String>, String> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let Some(parsed) = DeclaredPrices::parse(raw) else {
        return Ok(None);
    };
    // Back through the input shape, so an imported blob is held to exactly the
    // rule the form is — a hand-edited share file included.
    let input = ProviderPricesInput {
        currency: parsed.currency,
        models: parsed
            .models
            .into_iter()
            .map(|m| ProviderPriceInput {
                model_id: m.model_id,
                input: m.input,
                output: m.output,
                cache_read: Some(m.cache_read),
                cache_creation: Some(m.cache_creation),
            })
            .collect(),
    };
    normalize_declared_prices(Some(&input), known)
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
        cache_creation_tokens: t.cache_creation_tokens,
        output_tokens: t.output_tokens,
        cost: cost.map(|c| (c * 1e6).round() / 1e6),
        cost_currency,
        latency_ms: aux.avg_latency(Some(&p.id), None, Some(since7), None),
        quota,
        spark,
    })
}

// ── Mutations (called from commands; each ends with an admin /reload) ──

pub fn slug(name: &str) -> String {
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
/// The stored form of an endpoint somebody typed: an absolute URL.
///
/// The dialog is shown `display_endpoint` — the scheme stripped, `api_path`
/// folded in — and hands that back on save, so opening a provider and saving it
/// again rewrote `https://api.deepseek.com` as `api.deepseek.com` and broke
/// routing for that provider from then on. Nothing downstream puts the scheme
/// back: the gateway composes the upstream URL by concatenation
/// (`server::data::compose_upstream`) and `reqwest` refuses a relative one, so
/// every request to it fails at the transport layer while the row still reads
/// like a working provider. The invariant is kept here, at the one function
/// every writer of a provider's endpoints goes through.
///
/// The scheme is `http://` for a loopback host and `https://` otherwise. A local
/// server — Ollama on 11434, an LM Studio port — is both the common case and the
/// one where the obvious guess is wrong, and it is not a case that corrects
/// itself by trying: a TLS handshake against a plaintext listener fails before
/// anything can say why.
fn absolute_endpoint(raw: &str) -> String {
    let endpoint = raw.trim();
    if endpoint.contains("://") {
        return endpoint.to_string();
    }
    let local = matches!(
        crate::creds::host_of(endpoint)
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1" | "[::1]"
    );
    format!("{}{endpoint}", if local { "http://" } else { "https://" })
}

fn input_endpoints(input: &NewProviderInput) -> Vec<kiwanod::store::ProviderEndpoint> {
    input
        .endpoints
        .iter()
        .filter_map(|e| {
            kiwanod::store::Protocol::parse_str(&e.protocol).map(|p| {
                kiwanod::store::ProviderEndpoint {
                    protocol: p,
                    base_url: absolute_endpoint(&e.endpoint),
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

pub fn add_provider(
    store: &Store,
    aux: &Aux,
    input: &NewProviderInput,
) -> Result<ProviderVm, String> {
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
    let is_plan = billing == kiwanod::store::Billing::Subscription;
    let mut provider = Provider {
        id: id.clone(),
        name: input.name.trim().to_string(),
        // Which catalog entry this came from, if it was added from the shelf.
        // An empty string is the frontend's "nothing selected"; storing it
        // would be a provider id that names no row.
        catalog_id: input
            .catalog_id
            .as_deref()
            .map(str::trim)
            .filter(|id| !id.is_empty())
            .map(str::to_string),
        protocol: kiwanod::store::Protocol::parse_str(&input.protocol)
            .unwrap_or(kiwanod::store::Protocol::OpenAI),
        base_url: absolute_endpoint(&input.endpoint),
        api_path: None,
        endpoints: input_endpoints(input),
        api_key: Some(input.api_key.clone()),
        // Collected by the form since it existed, and until now dropped on the
        // floor (see `NewProviderInput::model_default`). Empty is `None`: a
        // cleared box is "not set", not the empty string.
        model_default: Some(input.model_default.trim().to_string()).filter(|m| !m.is_empty()),
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
                &known_limit_currencies(store),
            )?
        },
        plan_query: plan_query_json,
        plan_limits: if is_plan {
            plan_limits_json(input.billing_config.plan_limits.as_ref())
        } else {
            None
        },
        // What this provider charges, when the user said. Validated here rather
        // than in the gateway: a rate that will not parse is a mistake in the
        // form, and the person who made it is the one who can fix it.
        prices: normalize_declared_prices(input.prices.as_ref(), &known_limit_currencies(store))?,
        reset_period: if is_plan { None } else { reset_period },
        timeout_secs,
        retries,
        headers: adv_headers,
        enabled: true,
        created_at: now.clone(),
        updated_at: now,
    };
    // Nobody named an entry — a hand-added provider, or the CLI — so infer it
    // from the endpoint where that is unambiguous. Worth doing at all because
    // the row's link is what prices its requests at its own rate rather than at
    // whichever entry sorts first (`link_providers` has the long version).
    if provider.catalog_id.is_none() {
        provider.catalog_id = catalog_id_for(&catalog_snapshot(aux).entries, &provider);
    }
    store.insert_provider(&provider).map_err(e2s)?;

    // compute view fields before partially moving `provider`
    let vm_name = provider.name.clone();
    let vm_catalog_id = provider.catalog_id.clone();
    let vm_model_default = provider.model_default.clone();
    let vm_endpoint = display_endpoint(&provider);
    let vm_note = endpoint_note(&provider);
    let vm_protocol = provider.protocol.as_str().to_string();
    let vm_endpoints = vm_endpoints(&provider);
    let vm_billing = billing_to_ui(provider.billing).to_string();
    // Nothing has measured this provider yet — it was created a line ago — so
    // `None` is what the Status column starts as, and the prober fills it in.
    let vm_health = health_vm(aux, &provider, &rfc3339(unix_now() - 86_400), None);
    let vm_advanced = advanced_vm(&provider);
    let vm_prices = provider
        .prices
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok());

    for agent in input.agents.iter().flatten() {
        // "Save & Enable" → becomes the primary for the chosen agents.
        store
            .upsert_strategy(agent, StrategyType::Single, None)
            .map_err(e2s)?;
        bind_as_primary(store, agent, &id)?;
    }
    Ok(ProviderVm {
        id,
        name: vm_name,
        logo_char: logo_char(&input.name),
        logo_color: palette_color(&input.name).to_string(),
        logo_border: false,
        catalog_id: vm_catalog_id,
        endpoint: vm_endpoint,
        protocol: vm_protocol,
        endpoint_note: vm_note,
        endpoints: vm_endpoints,
        billing: vm_billing,
        plan_price: None,
        limit_unit: None,
        model_default: vm_model_default,
        plan_query: input.plan_query.clone(),
        plan_limits: None,
        prices: vm_prices,
        enabled: true,
        agents: input.agents.clone().unwrap_or_default(),
        // Optimistic: strategy serving is only computed by build_provider_vms;
        // the list refetch right after returns the real per-agent state.
        serving_agents: vec![],
        fallback_agents: vec![],
        is_current: input.agents.as_ref().is_some_and(|a| !a.is_empty()),
        status_badge: None,
        agents_note: input
            .agents
            .as_ref()
            .filter(|a| !a.is_empty())
            .map(|a| format!("{} agent(s)", a.len())),
        health: vm_health,
        usage: None,
        advanced: vm_advanced,
    })
}

/// Park a provider, or put it back: `enabled` decides whether this row may
/// serve, and touches no route.
///
/// This is the rung between "in the route" and "deleted". A disabled provider
/// keeps its row, its key, its prices and its usage history, and the gateway's
/// route table skips it (`router::RouteTable::load`) — so every agent bound to it
/// falls through to its next candidate exactly as if the row were gone, without
/// anything being lost. Deleting stays the way to be rid of a provider; this is
/// the way to stop using one for a while.
///
/// The old `enable_provider` did something else with the word: it promoted a
/// provider to primary everywhere it was bound. `providers use --agent` is that
/// operation, one agent at a time, and it keeps its name.
pub fn set_provider_enabled(store: &Store, id: &str, enabled: bool) -> Result<(), String> {
    let mut p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider not found: {id}"))?;
    if p.enabled == enabled {
        return Ok(());
    }
    p.enabled = enabled;
    p.updated_at = rfc3339(unix_now());
    store.update_provider(&p).map_err(e2s)
}

/// Make `provider_id` the primary for `agent`: priority 0, every other binding
/// reindexed after it in the order it already had.
///
/// The full reindex is the point. Demoting only the previous primary can leave
/// two bindings claiming priority 1, and `bindings_for_agent` is
/// priority-ordered — so the ambiguity would reach the strategy engine rather
/// than stop here.
///
/// The one place that writes "this provider is now the primary". `add_provider`'s
/// Save & Enable, `update_provider` and the CLI's `providers use` /
/// `providers add --bind` all land here.
pub fn bind_as_primary(store: &Store, agent: &str, provider_id: &str) -> Result<(), String> {
    let mut others: Vec<String> = store
        .bindings_for_agent(agent)
        .map_err(e2s)?
        .into_iter()
        .map(|b| b.provider_id)
        .filter(|p| p != provider_id)
        .collect();
    store
        .upsert_binding(&Binding {
            agent: agent.to_string(),
            provider_id: provider_id.to_string(),
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
                agent: agent.to_string(),
                provider_id: p,
                priority: i as i64 + 1,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .map_err(e2s)?;
    }
    Ok(())
}

/// Update provider: rewrite the providers row + rebind agents (the new set
/// becomes primary; removed ones are unbound). An empty api_key means keep
/// the existing key. Returns the refreshed VM (re-aggregated so badges and
/// notes stay consistent), read against `home` like [`build_provider_vms`].
pub fn update_provider(
    store: &Store,
    aux: &Aux,
    home: &Path,
    id: &str,
    input: &NewProviderInput,
    vars: &ShellVars,
) -> Result<ProviderVm, String> {
    let mut p = store
        .get_provider(id)
        .map_err(e2s)?
        .ok_or_else(|| format!("provider `{id}` not found"))?;

    p.name = input.name.trim().to_string();
    p.base_url = absolute_endpoint(&input.endpoint);
    p.protocol = kiwanod::store::Protocol::parse_str(&input.protocol)
        .unwrap_or(kiwanod::store::Protocol::OpenAI);
    p.endpoints = input_endpoints(input);
    // Authoritative, like `endpoints`: the form is the only caller and always
    // sends it, so clearing the box means "no default" rather than "leave it".
    // Empty stores NULL.
    p.model_default = Some(input.model_default.trim().to_string()).filter(|m| !m.is_empty());
    p.billing = billing_to_db(&input.billing)?;
    // Plan rows carry percent limits in plan_limits and NULL the legacy
    // number+unit+reset-cycle columns (v10 form); payg keeps the old shape.
    let is_plan = p.billing == kiwanod::store::Billing::Subscription;
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
            &known_limit_currencies(store),
        )?
    };
    p.plan_limits = if is_plan {
        plan_limits_json(input.billing_config.plan_limits.as_ref())
    } else {
        None
    };
    // Declared prices: same absent-keeps semantics as `advanced` below, and a
    // present bundle is the snapshot — including an empty one, which is how a
    // provider that leaves pay-as-you-go stops carrying prices.
    if input.prices.is_some() {
        p.prices =
            normalize_declared_prices(input.prices.as_ref(), &known_limit_currencies(store))?;
    }
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
    // The catalog entry this provider was added from: absent or empty keeps
    // what is stored. The edit form has no catalog picker, and dropping the
    // link on an unrelated edit would quietly change which price it is costed
    // at — the one thing this column exists to pin down.
    if let Some(catalog_id) = input.catalog_id.as_deref().map(str::trim) {
        if !catalog_id.is_empty() {
            p.catalog_id = Some(catalog_id.to_string());
        }
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

    // Bindings: absent leaves them exactly as they are, which is what an edit
    // sends. They belong to the Apps screen's agent tabs — that screen binds,
    // unbinds and sets the strategy — and this loop is not a neutral rewrite of
    // "the same set": every agent it names is promoted to *this* provider as
    // primary, and has its strategy flattened to Single. Running it on an
    // unrelated save is how editing a provider's timeout silently reordered an
    // agent's failover queue.
    if let Some(agents) = &input.agents {
        // Unbind old agents not in the new set; new ones use the same primary
        // logic as add.
        let new_set: std::collections::HashSet<&str> = agents.iter().map(String::as_str).collect();
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
        for agent in agents {
            store
                .upsert_strategy(agent, StrategyType::Single, None)
                .map_err(e2s)?;
            bind_as_primary(store, agent, id)?;
        }
    }

    let vms = build_provider_vms(store, aux, home, vars)?;
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
/// a failed sync only logs, and the shelf stays empty.
const LEGACY_HUB_URL: &str = "https://hub.kiwano.app/catalog.json";

/// Read UI settings (used by Rust-side logic like tray/autostart; the
/// store-free part of `build_settings`).
pub fn ui_settings(aux: &Aux) -> SettingsVm {
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

pub fn build_settings(store: &Store, aux: &Aux, vars: &ShellVars) -> Result<SettingsVm, String> {
    build_settings_with_home(store, aux, &kiwano_adapters::config::get_home_dir(), vars)
}

/// [`build_settings`] against an explicit home directory.
///
/// Takeover state is read out of the agent's live config files, so the caller
/// has to say which tree to read. Production passes `$HOME`; tests pass a temp
/// root, which is what keeps "is claude taken over" from depending on whether
/// the developer running the suite happens to have taken claude over.
pub fn build_settings_with_home(
    store: &Store,
    aux: &Aux,
    home: &std::path::Path,
    vars: &ShellVars,
) -> Result<SettingsVm, String> {
    let mut s: SettingsVm = ui_settings(aux);
    // A takeover *is* its rewrite: the agent's own config carries our
    // placeholder key, which is also how its requests get attributed. Neither
    // of the other two possible claims counts here — not the `placeholder_keys`
    // table (a row can outlive its rewrite), and not the backup (it is the
    // escape hatch, and it outlives the rewrite by design). Counting the backup
    // answered "is this agent ours?" with "we could undo a takeover", which is
    // how an agent reads as taken over while its traffic goes straight to its
    // own provider — and its provider reads as in use.
    s.takeovers = AGENTS
        .iter()
        .map(|(agent, label)| {
            let key = crate::takeover::live_placeholder_key(agent, home, vars);
            TakeoverVm {
                agent: agent.to_string(),
                label: label.to_string(),
                // The key is read out of the file, never out of the table, so
                // what the UI offers to copy is what the agent actually sends.
                enabled: key.is_some(),
                placeholder_key: key,
                additive: ADDITIVE_AGENTS.contains(agent),
                protocols: agent_protocols(agent)
                    .iter()
                    .map(|p| (*p).to_string())
                    .collect(),
                config_paths: crate::takeover::takeover_paths(agent, home, vars)
                    .unwrap_or_default()
                    .iter()
                    .map(|p| display_path(p, home))
                    .collect(),
            }
        })
        .collect();
    // The user's own agents, with the key their traffic is attributed by. The
    // key comes out of the table here — where a built-in's comes out of its
    // config file — because for these there is no file to read: the row *is*
    // the agent, and it is deleted with it, so it cannot outlive anything.
    let keys = store.list_placeholder_keys().map_err(e2s)?;
    s.custom_agents = store
        .list_custom_agents()
        .map_err(e2s)?
        .into_iter()
        .map(|a| CustomAgentVm {
            placeholder_key: keys.iter().find(|k| k.agent == a.id).map(|k| k.key.clone()),
            id: a.id,
            label: a.label,
            note: a.note,
            protocol: a.protocol,
        })
        .collect();
    Ok(s)
}

pub fn update_settings(
    store: &Store,
    aux: &Aux,
    patch: &serde_json::Value,
    vars: &ShellVars,
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
    if patch.get("request_logs").is_some()
        || patch.get("log_retention_days").is_some()
        || patch.get("log_max_body_bytes").is_some()
    {
        let mut cfg = store.load_log_config().unwrap_or_default();
        if let Some(v) = patch.get("request_logs").and_then(|v| v.as_bool()) {
            cfg.enabled = v;
        }
        // Zero is how the patch says "keep every row": the field is a count of
        // days, and the only count that means *no* retention is none of them.
        // Inside the config the two states stay distinct — `None` is a
        // decision, zero would be a bug — but a JSON patch has one number to
        // say it with, and `0` is the number every other time limit here uses.
        if let Some(v) = patch.get("log_retention_days").and_then(|v| v.as_u64()) {
            if let Ok(days) = u32::try_from(v) {
                cfg.retain_days = (days > 0).then_some(days);
            }
        }
        if let Some(v) = patch.get("log_max_body_bytes").and_then(|v| v.as_u64()) {
            if let Ok(bytes) = usize::try_from(v) {
                cfg.max_body_bytes = (bytes > 0).then_some(bytes);
            }
        }
        store.save_log_config(&cfg).map_err(e2s)?;
    }
    // Same contract for the streaming timeouts: the sidecar reads them at
    // startup and on /reload, so the two copies move together.
    if patch.get("stream_first_byte_secs").is_some() || patch.get("stream_idle_secs").is_some() {
        let mut cfg = store.load_stream_timeouts().unwrap_or_default();
        if let Some(v) = patch.get("stream_first_byte_secs").and_then(|v| v.as_u64()) {
            cfg.first_byte_secs = v;
        }
        if let Some(v) = patch.get("stream_idle_secs").and_then(|v| v.as_u64()) {
            cfg.idle_secs = v;
        }
        store.save_stream_timeouts(&cfg).map_err(e2s)?;
    }
    build_settings(store, aux, vars)
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
///
/// `include_bodies` is the export's own choice, not a capture setting: bodies
/// are always recorded, and the file either carries them or does not. It also
/// picks the read — the bodyless one never touches `request_bodies` at all,
/// so a metadata-only export of a large log does not load every body.
pub fn export_request_logs_csv(
    store: &Store,
    path: &str,
    filter: RequestLogFilter<'_>,
    include_bodies: bool,
) -> Result<RequestLogExportVm, String> {
    // Ask for one row more than the cap will allow, so "exactly at the cap"
    // and "more than the cap" are distinguishable.
    let (mut rows, truncated) = if include_bodies {
        let rows = store
            .export_request_logs_with_bodies(filter, EXPORT_ROW_CAP + 1)
            .map_err(e2s)?;
        let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
        (rows, truncated)
    } else {
        let rows = store
            .export_request_logs(filter, EXPORT_ROW_CAP + 1)
            .map_err(e2s)?;
        let truncated = rows.len() as i64 > EXPORT_ROW_CAP;
        (
            rows.into_iter()
                .map(RequestLogExportRow::from_entry)
                .collect(),
            truncated,
        )
    };
    rows.truncate(EXPORT_ROW_CAP as usize);
    crate::csv::write_csv(path, &rows, include_bodies).map_err(e2s)?;
    Ok(RequestLogExportVm {
        rows_written: rows.len(),
        truncated,
    })
}

/// Detail view (metadata + bodies); re-exported for the command signature.
pub use kiwanod::store::RequestLogDetail as RequestLogDetailVm;

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
    vars: &ShellVars,
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
            // The import leaves the link empty (the agent's config knows
            // nothing about our catalog), and this is the one path where the
            // provider starts carrying traffic before any backfill pass runs —
            // takeover is followed immediately by real requests.
            let _ = link_providers(store, aux);
        }
        let rand = &uuid::Uuid::new_v4().simple().to_string()[..4];
        let key = format!("kw-ag-{agent}-{rand}");
        store.upsert_placeholder_key(&key, agent).map_err(e2s)?;
        // Rewrite the Agent config (backup → base_url → placeholder key); on
        // failure roll back the key registration to stay consistent
        if let Err(e) = crate::takeover::enable(aux, agent, &key, data_port, home, vars) {
            let _ = store.delete_placeholder_key(&key);
            return Err(e);
        }
    } else {
        // The provider the gateway serves for this agent, handed to restore as
        // the rebuild tier: losing the backup must not strand the agent at
        // loopback if there is a provider to point it back at.
        let fallback = rebuild_route(store, agent)?;
        let report = crate::takeover::disable(aux, agent, home, fallback.as_ref(), vars)?;
        // A restore that could not hand the original config back changed the
        // agent's config in a way the user did not ask for: it is not an error
        // (the agent is whole and no longer points at loopback), but it must be
        // visible in the log rather than only in the degraded UI state.
        if report.outcome == crate::takeover::RestoreOutcome::RebuiltFromProvider {
            eprintln!(
                "kiwano: {agent} takeover backup was unusable; its config was rebuilt from the gateway's current provider"
            );
        }
        // A cleanup step that did not finish is a log line too, not a failure:
        // the agent is already whole, and failing the command would tell the
        // user the restore broke when it did not.
        if let Some(warning) = report.warning {
            eprintln!("kiwano: {agent} takeover restore: {warning}");
        }
        // The registration is dropped whatever the restore outcome: leave it
        // and the UI keeps offering a key the config no longer carries.
        for k in store.list_placeholder_keys().map_err(e2s)? {
            if k.agent == agent {
                store.delete_placeholder_key(&k.key).map_err(e2s)?;
            }
        }
        // …and so does the route, for the same reason one step further out: an
        // agent that has its own config back sends us nothing, so its candidate
        // list and strategy are stale the moment the restore lands — invisible
        // in the agent tab (which shows the takeover onboarding again) but not
        // in the Apps screen, which kept reading the rows as "bound" and even
        // as "In use" (see `live_bound_agents`). The *providers* stay: they are
        // the user's own rows, with their keys, plans and usage history, and
        // they are what a later takeover re-imports and binds again.
        for b in store.bindings_for_agent(agent).map_err(e2s)? {
            store.delete_binding(agent, &b.provider_id).map_err(e2s)?;
        }
        store.delete_strategy(agent).map_err(e2s)?;
    }
    Ok(())
}

/// The route restore can rebuild an agent's config from: its primary provider,
/// when that provider carries a key and does not itself point at the gateway.
///
/// `None` is a real answer — the strip tier then applies. Only the agents the
/// takeover module can actually rewrite from a provider get looked up at all:
/// the additive agents' `kiwano-gateway` entry and Claude Desktop's
/// configLibrary profile are kiwano-authored projections with no faithful
/// provider-side rebuild.
fn rebuild_route(
    store: &Store,
    agent: &str,
) -> Result<Option<crate::takeover::ProviderRoute>, String> {
    if !crate::takeover::REBUILDABLE_AGENTS.contains(&agent) {
        return Ok(None);
    }
    let Some(id) = store.primary_provider_id(agent).map_err(e2s)? else {
        return Ok(None);
    };
    let Some(provider) = store.get_provider(&id).map_err(e2s)? else {
        return Ok(None);
    };
    let Some(api_key) = provider
        .api_key
        .map(|k| k.trim().to_string())
        .filter(|k| !k.is_empty())
    else {
        return Ok(None);
    };
    // A provider whose own endpoint is the gateway (a card created from an
    // already taken-over config, say) would rebuild the agent straight back
    // onto loopback, which is the one outcome restore must never produce.
    let base_url = join_provider_url(&provider.base_url, provider.api_path.as_deref());
    if kiwano_adapters::codex_config::is_loopback_gateway_url(&base_url) {
        return Ok(None);
    }
    Ok(Some(crate::takeover::ProviderRoute { base_url, api_key }))
}

/// `base_url` with the provider's optional `api_path` prefix appended — the URL
/// the gateway itself forwards to, and therefore the one an agent rebuilt onto
/// this provider has to hold.
fn join_provider_url(base_url: &str, api_path: Option<&str>) -> String {
    let base = base_url.trim().trim_end_matches('/');
    match api_path.map(str::trim).filter(|p| !p.is_empty()) {
        Some(path) => format!(
            "{base}/{}",
            path.trim_start_matches('/').trim_end_matches('/')
        ),
        None => base.to_string(),
    }
}

/// Find-or-create a provider for the agent's current credentials: dedup by
/// base_url (trailing slash ignored) reuses the existing row — that shared
/// provider then also serves other agents; otherwise insert a new PAYG row
/// named after the config's provider key (or the URL host).
fn import_current_provider(
    store: &Store,
    creds: &crate::creds::CurrentCreds,
) -> Result<String, String> {
    // An agent's config is another tool's file, and it can hold a bare host —
    // which is how a provider ends up stored as `api.deepseek.com` and unrouted.
    // Normalizing before the dedup lookup also means a config that gains its
    // scheme later still matches the row it made without one.
    let base = absolute_endpoint(creds.base_url.trim_end_matches('/'));
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
        // Imported from another manager: no Hub catalog entry behind it.
        catalog_id: None,
        model_default: None,
        id: format!(
            "{}-{}",
            slug(&name),
            &uuid::Uuid::new_v4().simple().to_string()[..6]
        ),
        name,
        protocol: kiwanod::store::Protocol::parse_str(creds.protocol)
            .unwrap_or(kiwanod::store::Protocol::OpenAI),
        base_url: base.to_string(),
        api_path: None,
        endpoints: Vec::new(),
        api_key: Some(creds.api_key.clone()),
        billing: kiwanod::store::Billing::Metered,
        period_limit: None,
        limit_unit: None,
        reset_period: None,
        plan_query: None,
        plan_limits: None,
        prices: None,
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
    /// Masked for display — never the key itself. The UI only ever needs to
    /// tell two entries apart, and every copy of the plaintext we do not hand
    /// out is one less copy sitting in a webview heap.
    pub masked: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    pub enabled: bool,
    pub created_at: String,
}

/// Display form of a key: a recognisable head and tail, never a usable secret.
///
/// The previous frontend-side masker printed the key verbatim when it was 12
/// characters or shorter, which is exactly the case where a mask matters most.
/// There is no length at which this one returns the whole key.
fn mask_key(key: &str) -> String {
    let chars: Vec<char> = key.chars().collect();
    let n = chars.len();
    if n > 12 {
        format!(
            "{}…{}",
            chars[..6].iter().collect::<String>(),
            chars[n - 4..].iter().collect::<String>()
        )
    } else if n > 4 {
        format!("…{}", chars[n - 4..].iter().collect::<String>())
    } else {
        "•".repeat(n)
    }
}

pub fn list_api_keys(store: &Store, provider_id: &str) -> Result<Vec<ApiKeyVm>, String> {
    store
        .list_api_keys(provider_id)
        .map(|rows| {
            rows.into_iter()
                .map(|r| ApiKeyVm {
                    id: r.id,
                    masked: mask_key(&r.api_key),
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
        masked: mask_key(key),
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

/// Alerts for providers that have spent their allowance, which the frontend
/// turns into system notifications.
///
/// Two ways to be over, and both are raised here: an **amount** limit
/// (`period_limit` — requests, tokens, or the provider's own currency) and a
/// **plan window** ceiling (`plan_limits`, measured against what the provider's
/// endpoint reports). Both are read through `kiwanod::limits`, the same code the
/// gateway enforces with, so a notice can never describe a different number from
/// the block.
///
/// Notification only — the acting half is `kiwanod::limits::evaluate`, which runs
/// whether or not the user wants to be told.
///
/// `mark` controls the once-per-identity dedup write. The app passes `true` — it
/// raises one notification per provider per reset period, and recording that is
/// the point of the key. A read-only caller passes `false`, because consuming
/// the dedup would suppress the notification the app is about to raise: a
/// `--json` poll from a script must not silently eat the user's alert.
pub fn check_usage_alerts(
    store: &Store,
    aux: &Aux,
    mark: bool,
) -> Result<Vec<UsageAlertVm>, String> {
    if !ui_settings(aux).cost_alert {
        return Ok(Vec::new());
    }
    let mut alerts = Vec::new();
    for p in store.list_providers().map_err(e2s)? {
        if !p.enabled {
            continue;
        }
        if let Some(pl) = kiwanod::limits::period_limit_usage(store, &p).map_err(e2s)? {
            // Notify at most once per reset period (app_settings KV dedup).
            let key = format!("alert_sent:{}", p.id);
            if pl.used >= pl.limit && first_notice(aux, &key, &pl.period_key, mark)? {
                alerts.push(UsageAlertVm {
                    provider_id: p.id.clone(),
                    provider_name: p.name.clone(),
                    used: (pl.used * 100.0).round() / 100.0,
                    limit: pl.limit,
                    unit: pl.unit,
                });
            }
        }
        // The percent half — a plan window whose live utilization has reached the
        // ceiling the user set for it. These are the same numbers the gateway
        // blocks on (`limits::evaluate`), read through the same cached report and
        // the same `window_over`, so the notice and the block agree. Until now
        // nothing raised it: the UI had the branch and the strings, and no
        // producer, so a plan ceiling took the provider out of service silently.
        if p.billing == Billing::Subscription {
            // Bound to locals rather than chained: the hit borrows the report,
            // so the report has to outlive it in this scope.
            let limits = kiwanod::limits::PlanLimits::parse(p.plan_limits.as_deref());
            let report = kiwanod::plan_quota::cached_report(store, &p.id);
            let hit = match (limits.as_ref(), report.as_ref()) {
                (Some(limits), Some(report)) => kiwanod::limits::window_over(report, limits),
                _ => None,
            };
            if let Some(hit) = hit {
                // One notice per window, and a fresh one once it rolls over.
                let identity = match hit.resets_at {
                    Some(at) => format!("{}@{at}", hit.window),
                    // The endpoint does not say when the window resets, so
                    // re-arm daily: a window that stays over should not notify
                    // again on every poll, but it must not go quiet forever.
                    None => format!(
                        "{}@{}",
                        hit.window,
                        kiwanod::limits::period_start(
                            unix_now(),
                            Some("day"),
                            store.ui_tz_offset_minutes()
                        )
                        .1
                    ),
                };
                let key = format!("alert_sent:plan:{}", p.id);
                if first_notice(aux, &key, &identity, mark)? {
                    alerts.push(UsageAlertVm {
                        provider_id: p.id.clone(),
                        provider_name: p.name.clone(),
                        used: (hit.util * 100.0).round() / 100.0,
                        limit: hit.pct,
                        unit: "plan_pct".into(),
                    });
                }
            }
        }
    }
    Ok(alerts)
}

/// Record `identity` under `key`, answering whether this is the first time it
/// has been seen. With `mark` false the answer is given without consuming the
/// dedup, so a read-only caller cannot eat the alert the app is about to raise.
fn first_notice(aux: &Aux, key: &str, identity: &str, mark: bool) -> Result<bool, String> {
    if aux.get_setting(key).as_deref() == Some(identity) {
        return Ok(false);
    }
    if mark {
        aux.set_setting(key, identity).map_err(e2s)?;
    }
    Ok(true)
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
    // user's preferred display currency via the Hub's published rates, so this
    // agrees with the currency selector. Before the first sync there are no
    // rates and the buckets are summed as-is.
    let rates = crate::pricing::effective_rates(aux);
    let pref = crate::pricing::preferred_currency(aux);
    let cost_of = |buckets: &[(Option<String>, f64)]| {
        crate::pricing::convert_cost_buckets(buckets, &pref, &rates)
    };

    // Headline cost for the window, with the off-peak equivalent of the same
    // rows beside it. Both sums come from one query over one row set, which is
    // what makes their difference "what running at peak cost you" rather than a
    // comparison of two different populations.
    let headline = store
        .usage_cost_with_off_peak_by_currency(agent, provider_id, Some(&since))
        .map_err(e2s)?;
    let pairs = |pick: fn(&kiwanod::store::CostBucket) -> f64| -> Vec<(Option<String>, f64)> {
        headline
            .iter()
            .map(|b| (b.currency.clone(), pick(b)))
            .collect()
    };
    let cost = cost_of(&pairs(|b| b.cost));
    let cost_off_peak = (cost_of(&pairs(|b| b.cost_off_peak)) * 1e6).round() / 1e6;

    // Per-provider cost for the distribution card.
    let mut cost_by_pid: HashMap<String, f64> = HashMap::new();
    let mut off_peak_by_pid: HashMap<String, f64> = HashMap::new();
    for b in store
        .usage_cost_by_provider(agent, Some(&since))
        .map_err(e2s)?
    {
        let convert = |c: f64| match b.currency.as_deref() {
            Some(cur) => crate::pricing::convert_amount(c, cur, &pref, &rates),
            None => 0.0,
        };
        *cost_by_pid.entry(b.provider_id.clone()).or_default() += convert(b.cost);
        *off_peak_by_pid.entry(b.provider_id).or_default() += convert(b.cost_off_peak);
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
                cost_off_peak: (off_peak_by_pid.get(&pu.provider_id).copied().unwrap_or(0.0) * 1e6)
                    .round()
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

    // Who gets a row: the registry in its own order (which is the order this
    // table has always used), then the user's own agents, then anything else
    // with traffic in the window. That last group is an agent whose route was
    // deleted: its usage rows stay, and its traffic should not disappear from
    // the page just because the name did.
    let mut roster: Vec<(String, String)> = list_agents(store)?;
    for id in store.usage_agents(Some(&since)).map_err(e2s)? {
        if !roster.iter().any(|(a, _)| *a == id) {
            roster.push((id.clone(), id));
        }
    }

    let mut by_agent = Vec::new();
    for (name, label) in &roster {
        // The agent filter narrows this breakdown like every other panel,
        // leaving one row at 100% when one agent is selected. `name` is the
        // loop's, not the filter's, so skipping here is what applies it — a
        // table that kept every agent would total more than the headline.
        if agent.is_some_and(|a| a != name.as_str()) {
            continue;
        }
        let t = store
            .usage_totals(Some(name), provider_id, Some(&since))
            .map_err(e2s)?;
        if t.requests > 0 {
            let buckets = store
                .usage_cost_with_off_peak_by_currency(Some(name), provider_id, Some(&since))
                .unwrap_or_default();
            let off_peak = cost_of(
                &buckets
                    .iter()
                    .map(|b| (b.currency.clone(), b.cost_off_peak))
                    .collect::<Vec<_>>(),
            );
            by_agent.push(AgentDistVm {
                agent: name.clone(),
                label: label.clone(),
                requests: t.requests,
                tokens: fmt_tokens(t.input_tokens + t.output_tokens),
                cost: (cost_of(
                    &buckets
                        .iter()
                        .map(|b| (b.currency.clone(), b.cost))
                        .collect::<Vec<_>>(),
                ) * 1e6)
                    .round()
                    / 1e6,
                cost_off_peak: (off_peak * 1e6).round() / 1e6,
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
    let filter_agents: Vec<FilterOptionVm> = roster
        .iter()
        .filter_map(|(name, label)| {
            let t = store.usage_totals(Some(name), None, Some(&since)).ok()?;
            (t.requests > 0).then(|| FilterOptionVm {
                id: name.clone(),
                label: label.clone(),
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
        cost_off_peak,
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

/// Catalog shelf, read from the Hub cache. Nothing else.
///
/// There used to be a bundled `catalog.json` compiled into the binary as the
/// offline fallback, and it is gone deliberately. It was a build-time copy of a
/// separate repository that publishes on its own schedule, so it went stale by
/// construction — and it went a whole schema migration stale without anything
/// noticing, which is how it was found. A copy nobody remembers to refresh is a
/// worse answer than no copy: what it produced was a shelf that silently showed
/// the *old* shape (no prices, no descriptions) to exactly the users least able
/// to tell why.
///
/// An empty cache is now an empty list, and the shelf says so rather than
/// reporting it as "no matches" — see the empty state in `Shelf.tsx`.
///
/// `added` is derived at read time from the local provider list: an entry
/// counts as added when a provider exists at its primary OR any of its
/// additional per-protocol endpoints. The static flags carried by the Hub cache
/// are ignored.
/// Fill what the Hub does not publish, and split the endpoint list.
///
/// The Hub sends source-of-truth data only: identity, classification, billing,
/// and one endpoint list with the primary first. Two things this view model
/// carries are the app's business, so they are derived on the way in — the
/// letter-avatar colour (one palette shared with locally-added providers), and
/// the primary endpoint hoisted into its own fields (`protocol` / `endpoint` /
/// `models`), which is the shape every screen reads.
///
/// Idempotent: a payload that already carries `endpoint` was normalized by an
/// older build — or was cached before the Hub switched to one endpoint list —
/// so its `endpoints` are extras and stay untouched.
fn normalize_catalog_entry(e: &mut CatalogEntryVm) {
    e.logo_color = palette_color(&e.name).to_string();
    if e.endpoint.is_empty() && !e.endpoints.is_empty() {
        let primary = e.endpoints.remove(0);
        e.protocol = primary.protocol;
        e.endpoint = primary.endpoint;
        e.models = primary.models;
    }
}

/// The cached Hub catalog, parsed and normalized.
///
/// One reader for every path that has to see the same entries: the shelf, and
/// the endpoint matching that links a provider to its entry and marks the
/// shelf's "already added" badge. Normalizing here is what makes the primary
/// endpoint matchable at all — `normalize_catalog_entry` is what moves it out
/// of `endpoints`, and a matcher that saw the raw list would be looking at the
/// extras only.
fn catalog_snapshot(aux: &Aux) -> CatalogListVm {
    // No cache, or a cache we cannot parse, is an empty shelf. A parse failure
    // used to fall back to the bundled copy, which meant a corrupted cache was
    // answered with stale data instead of being visibly broken; the next sync
    // repairs this either way.
    let mut list = aux
        .load_hub_cache()
        .and_then(|(payload, _)| serde_json::from_str::<CatalogListVm>(&payload).ok())
        .unwrap_or(CatalogListVm {
            total: 0,
            entries: Vec::new(),
        });
    for e in &mut list.entries {
        normalize_catalog_entry(e);
    }
    list
}

/// Every endpoint key a local provider answers on: its base URL and each
/// additional per-protocol endpoint.
fn provider_endpoint_keys(p: &Provider) -> Vec<String> {
    std::iter::once(&p.base_url)
        .chain(p.endpoints.iter().map(|e| &e.base_url))
        .map(|u| endpoint_key(u))
        .collect()
}

/// The credential stored for `provider_id`, but **only** when `endpoint` is one
/// of the endpoints that provider already answers on.
///
/// The add/edit form deliberately keeps the key out of the webview: it shows a
/// masked placeholder and sends a blank when the user has not typed a new one
/// (blank = keep the stored key). So a Test or a Fetch pressed from the edit
/// dialog arrives with nothing to authenticate with, and both used to fail —
/// the probe reported `auth`, and the model list refused outright — for a
/// provider whose key was sitting right there.
///
/// The endpoint has to match, and that is the point rather than a nicety: the
/// URL field is editable, so reading the stored key for whatever address is in
/// the box would turn a Test button into a way to post someone's credential to a
/// host of the typer's choosing. Matching the stored endpoints keeps the button
/// meaning "does my saved configuration still work".
pub fn stored_key_for(store: &Store, provider_id: &str, endpoint: &str) -> Option<String> {
    let p = store.get_provider(provider_id).ok().flatten()?;
    let wanted = endpoint_key(endpoint);
    let known = provider_endpoint_keys(&p);
    known.contains(&wanted).then_some(p.api_key)?
}

/// Every endpoint key a catalog entry advertises, its primary first.
fn catalog_endpoint_keys(e: &CatalogEntryVm) -> Vec<String> {
    std::iter::once(&e.endpoint)
        .chain(e.endpoints.iter().map(|x| &x.endpoint))
        .map(|u| endpoint_key(u))
        .collect()
}

/// Does a local endpoint name the same place as a catalog one?
///
/// Equal keys, or whichever is shorter being a *path* prefix of the other: the
/// user typed a bare host (`api.deepseek.com`) where the entry names a deeper
/// path (`api.deepseek.com/anthropic`), or imported an agent config that
/// carries the path (`api.deepseek.com/v1`) where the entry names the bare
/// host. Same host, same service.
///
/// The `/` boundary is what keeps a host from swallowing its neighbours:
/// `api.deepseek.com` matches neither `api.deepseek.com.evil.com` nor
/// `api.deepseek.com:8443`. A non-default port staying distinct is deliberate —
/// a replica on another port is not the vendor's endpoint.
///
/// An empty key matches nothing: `strip_prefix("")` would otherwise make it a
/// prefix of every key.
fn endpoint_matches(a: &str, b: &str) -> bool {
    if a.is_empty() || b.is_empty() {
        return false;
    }
    if a == b {
        return true;
    }
    let (short, long) = if a.len() < b.len() { (a, b) } else { (b, a) };
    long.strip_prefix(short)
        .is_some_and(|rest| rest.starts_with('/'))
}

/// The catalog entry a provider's endpoints point at, when exactly one does.
///
/// The provider's extra per-protocol endpoints are part of the set, mirroring
/// `added`: an entry whose second-protocol URL is what the user typed is the
/// same entry. Protocol itself is not part of the test — filtering on it would
/// let the badge and this link disagree about which entry a URL names.
///
/// Two or more matching entries resolve to `None` rather than to a guess: two
/// entries on one host mean the endpoint cannot tell them apart, and picking one
/// is the silent mis-pricing this exists to prevent. A provider that matches
/// nothing — self-hosted, an aggregator the Hub does not list — is the same
/// `None`, and that is an answer, not a failure.
fn catalog_id_for(entries: &[CatalogEntryVm], p: &Provider) -> Option<String> {
    let local = provider_endpoint_keys(p);
    let matching = |e: &&CatalogEntryVm| {
        catalog_endpoint_keys(e)
            .iter()
            .any(|cat| local.iter().any(|l| endpoint_matches(l, cat)))
    };
    let mut found = entries.iter().filter(matching);
    let first = found.next()?;
    if found.next().is_some() {
        return None;
    }
    Some(first.id.clone())
}

/// Fill in the catalog link on providers that have none.
///
/// The link is what prices a request at its own provider's rate, and it is
/// written when a provider is added from the shelf — and only then, so
/// everything hand-added, added from the CLI, imported, or present before the
/// column existed has none. With the Hub pricing per entry, an unlinked
/// provider is costed at *another* entry's rate, so the link is inferred from
/// the endpoint wherever that is unambiguous ([`catalog_id_for`]).
///
/// Only `NULL`s are considered: a link is a fact that decides a price, so it is
/// never re-derived, never overwritten, never cleared — the same rule
/// `update_provider` follows for an edit that carries no catalog id. The row's
/// own `updated_at` rides along, so a backfill does not read as a user edit.
///
/// Returns how many rows were linked: a caller with a running gateway needs to
/// know whether the daemon's in-memory copy just went stale.
pub fn link_providers(store: &Store, aux: &Aux) -> Result<usize, String> {
    let list = catalog_snapshot(aux);
    // Never synced: nothing to infer from, and nothing to write.
    if list.entries.is_empty() {
        return Ok(0);
    }
    let mut linked = 0;
    for mut p in store.list_providers().map_err(e2s)? {
        if p.catalog_id.is_some() {
            continue;
        }
        let Some(id) = catalog_id_for(&list.entries, &p) else {
            continue; // a custom endpoint, or an ambiguous one
        };
        p.catalog_id = Some(id);
        store.update_provider(&p).map_err(e2s)?;
        linked += 1;
    }
    Ok(linked)
}

pub fn load_catalog(store: &Store, aux: &Aux) -> CatalogListVm {
    // The badge and the link answer the same question — "is this entry one the
    // user already has?" — so they match endpoints by the same rule. A
    // provider whose link was inferred is then not offered for adding a second
    // time, which is what it would otherwise be.
    let local: Vec<String> = store
        .list_providers()
        .unwrap_or_default()
        .iter()
        .flat_map(provider_endpoint_keys)
        .collect();
    let mut list = catalog_snapshot(aux);
    for e in &mut list.entries {
        let catalog = catalog_endpoint_keys(e);
        e.added = catalog
            .iter()
            .any(|cat| local.iter().any(|l| endpoint_matches(l, cat)));
    }
    list
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
    use kiwanod::store::UsageRecord;

    /// No shell environment: these tests root every agent file at the temp home
    /// they injected, and the variables have their own tests in `takeover`.
    fn no_vars() -> ShellVars {
        ShellVars::new()
    }

    fn store() -> Store {
        Store::open_in_memory().expect("in-memory store")
    }

    /// An `Aux` with nothing cached, so nothing can be inferred from a catalog
    /// — what the tests that are not about the link want.
    fn linkless_aux() -> Aux {
        Aux::open_in_memory().unwrap()
    }

    /// An agent config tree carrying the gateway's placeholder key for each
    /// named agent — the live evidence `build_provider_vms` reads before a
    /// binding counts as a route. Named agents get the key in the file their
    /// takeover writes; every other agent stays dormant, as it would be with no
    /// takeover ever enabled.
    fn live_home(agents: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for agent in agents {
            let rel = match *agent {
                "claude" => ".claude/settings.json",
                // auth.json rather than config.toml: the Codex config is read
                // through the parsed detector, the rest by a value scan.
                "codex" => ".codex/auth.json",
                "grokbuild" => ".grok/config.toml",
                "opencode" => ".config/opencode/opencode.json",
                "openclaw" => ".openclaw/openclaw.json",
                "hermes" => ".hermes/config.yaml",
                "pi" => ".pi/agent/settings.json",
                other => panic!("no agent-config fixture for {other}"),
            };
            let path = dir.path().join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, format!("kw-ag-{agent}-test")).unwrap();
        }
        dir
    }

    fn provider(id: &str, name: &str, billing: Billing) -> Provider {
        Provider {
            id: id.into(),
            name: name.into(),
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            protocol: kiwanod::store::Protocol::OpenAI,
            base_url: format!("https://{id}.example.com"),
            api_path: None,
            endpoints: Vec::new(),
            api_key: Some("sk-test".into()),
            model_default: None,
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

    /// A masked key is a display string, not a credential — there is no input
    /// length at which it hands the key back.
    #[test]
    fn mask_key_never_returns_the_whole_key() {
        assert_eq!(mask_key("sk-live-abcdefghijklmnop"), "sk-liv…mnop");
        // The frontend masker this replaced printed keys of 12 characters or
        // fewer verbatim — the case where masking matters most.
        assert_eq!(mask_key("123456789012"), "…9012");
        assert_eq!(mask_key("sk-short"), "…hort");
        assert_eq!(mask_key("abc"), "•••");
        assert_eq!(mask_key(""), "");
    }

    /// The provider's rotating keys travel to the webview for the edit form.
    /// They used to arrive in the clear; assert the plaintext is gone from the
    /// serialized VM, since that is the payload the IPC layer actually sends.
    #[test]
    fn api_keys_do_not_reach_the_frontend_in_the_clear() {
        let s = store();
        s.insert_provider(&provider("p1", "P", Billing::Metered))
            .unwrap();
        let key = "sk-live-abcdefghijklmnop";
        add_api_key(&s, "p1", key, Some("backup")).unwrap();

        let keys = list_api_keys(&s, "p1").unwrap();
        assert_eq!(keys.len(), 1);
        assert_eq!(keys[0].masked, "sk-liv…mnop");
        assert_eq!(keys[0].label.as_deref(), Some("backup"));

        let json = serde_json::to_string(&keys).unwrap();
        assert!(!json.contains(key), "plaintext key in VM payload: {json}");

        // add_api_key echoes the new row back to the UI; it must mask too.
        let added = add_api_key(&s, "p1", key, None).unwrap();
        assert!(!serde_json::to_string(&added).unwrap().contains(key));
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

        // `both` is a tag we *know*, and still refuse: the catalog is telling us
        // the vendor charges two ways, and the local row holds one. The message
        // has to point at the choice rather than at the catalog — "unknown
        // billing" would send the reader looking for a data defect.
        let err = billing_to_db("both").unwrap_err();
        assert!(err.contains("resolved"), "{err}");
        assert!(err.contains("plan or payg"), "{err}");
        assert!(!err.contains("unknown"), "{err}");
    }

    #[test]
    fn catalog_billing_roundtrip_known_and_unknown() {
        // Known tags map onto their variants and serialize back lowercase.
        for (raw, variant) in [
            ("plan", CatalogBilling::Plan),
            ("payg", CatalogBilling::Payg),
            ("unl", CatalogBilling::Unl),
            ("both", CatalogBilling::Both),
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
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
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

    /// A binding whose agent never took the gateway over is not a route: the
    /// provider list must leave that agent out of the agent column, out of
    /// "In use", and out of the count — the route stays in the store for the
    /// day the takeover is re-enabled, but no traffic reaches us until then.
    #[test]
    fn dormant_agent_bindings_are_not_counted() {
        let s = store();
        s.insert_provider(&provider("p1", "P One", Billing::Metered))
            .unwrap();
        for agent in ["claude", "codex"] {
            s.upsert_binding(&Binding {
                agent: agent.into(),
                provider_id: "p1".into(),
                priority: 0,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }
        let aux = Aux::open_in_memory().unwrap();

        // claude is routed through the gateway, codex is not.
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert_eq!(vm.agents, ["claude"]);
        assert_eq!(vm.serving_agents, ["claude"]);
        assert!(vm.is_current);
        assert_eq!(vm.agents_note.as_deref(), Some("1 agent(s)"));

        // Nothing taken over at all: the provider reads as unbound rather than
        // as serving an agent that has its own config back.
        let none = live_home(&[]);
        let vms = build_provider_vms(&s, &aux, none.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert!(vm.agents.is_empty());
        assert!(vm.serving_agents.is_empty());
        assert!(!vm.is_current);
        assert_eq!(vm.agents_note, None);
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
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
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
        s.upsert_strategy("opencode", StrategyType::Timewindow, None)
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
        s.upsert_binding(&bind("opencode", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("opencode", "c1", 1, Some(("22:00", "06:00"))))
            .unwrap();
        // windowless timewindow tail: never picked → still a standby
        s.upsert_binding(&bind("hermes", "a1", 0, None)).unwrap();
        s.upsert_binding(&bind("hermes", "d1", 1, None)).unwrap();
        let aux = Aux::open_in_memory().unwrap();
        let home = live_home(&["codex", "opencode", "hermes"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
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
        s.upsert_strategy("opencode", StrategyType::Failover, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "opencode".into(),
            provider_id: "a1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        apply_agent_route(&s, "opencode", "opencode").unwrap_err();
        apply_agent_route(&s, "opencode", "claude").unwrap_err(); // no route
        apply_agent_route(&s, "opencode", "codex").unwrap();

        let st = s.get_strategy("opencode").unwrap().unwrap();
        assert_eq!(st.kind, StrategyType::Roundrobin);
        let bs = s.bindings_for_agent("opencode").unwrap();
        assert_eq!(bs.len(), 2);
        assert_eq!((bs[0].provider_id.as_str(), bs[0].weight), ("a1", 60));
        assert_eq!((bs[1].provider_id.as_str(), bs[1].weight), ("b1", 40));
        // weights copied as-is, not re-seeded to an even split
        assert_eq!(bs[1].win_start.as_deref(), Some("22:00"));
        assert_eq!(bs[1].win_end.as_deref(), Some("06:00"));
        // the source agent keeps its own bindings
        assert_eq!(s.bindings_for_agent("codex").unwrap().len(), 2);
    }

    /// Binding appends to the tail of the queue: the standing primary keeps
    /// priority 0, the new candidate takes the next free slot, and a provider
    /// that is already bound is left exactly where it was — the command is
    /// idempotent so a second click cannot reorder the queue.
    #[test]
    fn add_agent_binding_appends_a_candidate_and_never_duplicates_one() {
        let s = store();
        for (id, name) in [("a1", "Alpha"), ("b1", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        add_agent_binding(&s, "claude", "a1").unwrap();
        add_agent_binding(&s, "claude", "b1").unwrap();
        let priorities = |s: &Store| -> Vec<(String, i64)> {
            s.bindings_for_agent("claude")
                .unwrap()
                .into_iter()
                .map(|b| (b.provider_id, b.priority))
                .collect()
        };
        assert_eq!(
            priorities(&s),
            vec![("a1".to_string(), 0), ("b1".to_string(), 1)]
        );
        let fresh = s.bindings_for_agent("claude").unwrap();
        assert!(
            fresh.iter().all(|b| b.enabled && b.weight == 1),
            "a new candidate joins enabled, at an even weight"
        );

        // Re-binding the primary leaves it at 0.
        add_agent_binding(&s, "claude", "a1").unwrap();
        assert_eq!(
            priorities(&s),
            vec![("a1".to_string(), 0), ("b1".to_string(), 1)]
        );

        // An unknown provider is refused rather than bound to nothing.
        assert!(add_agent_binding(&s, "claude", "ghost").is_err());
        assert_eq!(s.bindings_for_agent("claude").unwrap().len(), 2);
    }

    /// Unbinding takes one candidate off one agent's route and touches nothing
    /// else: another agent's binding of the same provider stays, and an empty
    /// route is a legal state (requests fail cleanly with NoBinding until
    /// something is bound again) rather than a reason to keep a stale row.
    #[test]
    fn remove_agent_binding_touches_one_route_only() {
        let s = store();
        s.insert_provider(&provider("a1", "Alpha", Billing::Metered))
            .unwrap();
        for agent in ["claude", "codex"] {
            add_agent_binding(&s, agent, "a1").unwrap();
        }

        remove_agent_binding(&s, "claude", "a1").unwrap();
        assert!(s.bindings_for_agent("claude").unwrap().is_empty());
        assert!(s.primary_provider_id("claude").unwrap().is_none());
        assert_eq!(s.bindings_for_agent("codex").unwrap().len(), 1);

        // Removing what is not bound is an error, not a silent success — the
        // caller has to be able to tell "it is gone" from "it never was".
        assert!(remove_agent_binding(&s, "claude", "a1").is_err());
        assert!(remove_agent_binding(&s, "claude", "ghost").is_err());
    }

    /// A binding's strategy parameters are patched one field at a time, except
    /// the window: its two bounds are set or cleared together, because half a
    /// window can never match and would quietly turn a rotating candidate into
    /// one that never serves.
    #[test]
    fn update_agent_binding_patches_weight_and_window_together() {
        let s = store();
        for (id, name) in [("a1", "Alpha"), ("b1", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        add_agent_binding(&s, "claude", "a1").unwrap();
        add_agent_binding(&s, "claude", "b1").unwrap();
        let binding = |s: &Store| -> Binding {
            s.bindings_for_agent("claude")
                .unwrap()
                .into_iter()
                .find(|b| b.provider_id == "b1")
                .unwrap()
        };

        update_agent_binding(
            &s,
            "claude",
            "b1",
            Some(7),
            Some("22:00".into()),
            Some("06:00".into()),
        )
        .unwrap();
        let b = binding(&s);
        assert_eq!(b.weight, 7);
        assert_eq!(b.win_start.as_deref(), Some("22:00"));
        assert_eq!(b.win_end.as_deref(), Some("06:00"));

        // A patch that does not mention the window leaves it alone.
        update_agent_binding(&s, "claude", "b1", Some(3), None, None).unwrap();
        let b = binding(&s);
        assert_eq!(b.weight, 3);
        assert_eq!(b.win_start.as_deref(), Some("22:00"));

        // A weight below 1 is floored: 0 would take the candidate out of a
        // weighted rotation while still sitting in the queue.
        update_agent_binding(&s, "claude", "b1", Some(0), None, None).unwrap();
        assert_eq!(binding(&s).weight, 1);

        // Half a window — one bound, or an empty string — clears the pair.
        update_agent_binding(&s, "claude", "b1", None, Some("22:00".into()), None).unwrap();
        let b = binding(&s);
        assert!(b.win_start.is_none() && b.win_end.is_none(), "{b:?}");

        // The other candidate was never touched by any of it.
        let a1 = s
            .bindings_for_agent("claude")
            .unwrap()
            .into_iter()
            .find(|b| b.provider_id == "a1")
            .unwrap();
        assert_eq!((a1.weight, a1.win_start), (1, None));

        // Patching something that is not bound is an error.
        assert!(update_agent_binding(&s, "claude", "ghost", Some(1), None, None).is_err());
    }

    // ── provider ↔ catalog link ──────────────────────────────────────────

    /// A catalog covering the shapes the link has to tell apart: an entry whose
    /// endpoint names a deeper path than a user types, an entry advertising two
    /// paths, a bare host, and two entries sharing one host — which must stay
    /// unlinked, because the endpoint cannot say which of them is meant.
    const LINK_CATALOG: &str = r#"{"total":5,"entries":[
        {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com/anthropic"}]},
        {"id":"kimi-for-coding","name":"Kimi For Coding","tag":"official","rating":4,
         "billing":"plan","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.kimi.com/coding/v1"},
                      {"protocol":"anthropic","endpoint":"https://api.kimi.com/coding"}]},
        {"id":"bare-host","name":"Bare Host","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.bare.example"}]},
        {"id":"host-one","name":"Host One","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.example.com/one"}]},
        {"id":"host-two","name":"Host Two","tag":"third","rating":3,
         "billing":"payg","currency":"USD",
         "endpoints":[{"protocol":"openai","endpoint":"https://api.example.com/two"}]}
    ]}"#;

    fn catalog_aux() -> Aux {
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_cache(LINK_CATALOG, "2026-09-07T00:00:00Z")
            .unwrap();
        aux
    }

    /// The link as stored. The inference writes the row, so this is where the
    /// answer can be read — `ProviderVm` deliberately does not carry it.
    fn stored_catalog_id(s: &Store, id: &str) -> Option<String> {
        s.get_provider(id)
            .unwrap()
            .expect("provider row")
            .catalog_id
    }

    fn stored_base(s: &Store, id: &str) -> String {
        s.get_provider(id).unwrap().expect("provider row").base_url
    }

    /// A stored endpoint is an absolute URL, whatever the form it was typed in.
    ///
    /// The dialog is handed `display_endpoint` — scheme stripped, `api_path`
    /// folded in — and hands it back on save, and nothing downstream adds a
    /// scheme: the gateway concatenates and `reqwest` refuses a relative URL. So
    /// opening a provider and saving it again used to leave a row that reads like
    /// a working provider and fails every request.
    #[test]
    fn a_saved_endpoint_keeps_its_scheme() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        // Typed as the dialog shows it. `absolute_endpoint` is the only thing
        // between that and the row, and this is the case that was broken.
        let mut bare = catalog_input("DeepSeek", "api.deepseek.com");
        bare.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "api.deepseek.com/anthropic".into(),
        }];
        let vm = add_provider(&s, &aux, &bare).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");
        let extra = s.get_provider(&vm.id).unwrap().unwrap().endpoints;
        assert_eq!(extra[0].base_url, "https://api.deepseek.com/anthropic");

        // …and the edit the dialog sends back is the same stripped form, which
        // is what used to strip the scheme off an already-correct row.
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &bare,
            &no_vars(),
        )
        .unwrap();
        assert_eq!(stored_base(&s, &vm.id), "https://api.deepseek.com");

        // A local server is plain HTTP: guessing https there fails at the
        // handshake, before anything can say why.
        let local = catalog_input("Ollama", "localhost:11434");
        let vm = add_provider(&s, &aux, &local).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://localhost:11434");
        let loopback = catalog_input("Local", "127.0.0.1:1234");
        let vm = add_provider(&s, &aux, &loopback).unwrap();
        assert_eq!(stored_base(&s, &vm.id), "http://127.0.0.1:1234");

        // An absolute URL is left exactly as it is, http and https alike.
        for (typed, stored) in [
            ("https://api.moonshot.cn", "https://api.moonshot.cn"),
            ("http://relay.internal", "http://relay.internal"),
        ] {
            let vm = add_provider(&s, &aux, &catalog_input("Typed", typed)).unwrap();
            assert_eq!(stored_base(&s, &vm.id), stored, "{typed}");
        }
    }

    /// The smallest input `add_provider` takes, so a test can vary the endpoint
    /// and leave everything else alone.
    fn catalog_input(name: &str, endpoint: &str) -> NewProviderInput {
        NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
            name: name.into(),
            api_key: "sk-test".into(),
            endpoint: endpoint.into(),
            protocol: "openai".into(),
            model_default: String::new(),
            billing: "payg".into(),
            billing_config: BillingConfigInput {
                limit_value: None,
                limit_unit: None,
                reset_period: None,
                plan_limits: None,
            },
            agents: None,
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
        }
    }

    /// A pay-as-you-go provider with a spending limit in `unit`.
    fn limited_input(unit: &str) -> NewProviderInput {
        let mut input = catalog_input("Limited", "https://api.limited.example");
        input.billing_config.limit_value = Some(50.0);
        input.billing_config.limit_unit = Some(unit.into());
        input
    }

    /// A store that has synced the Hub, so its rate table is not empty. Written
    /// through a second connection because the cache belongs to the GUI's schema,
    /// which the gateway reads and does not create (see `limits::tests`).
    fn store_with_hub_rates(rates: &str) -> (tempfile::TempDir, Store) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS hub_models_cache (
                 id        INTEGER PRIMARY KEY CHECK (id = 1),
                 version   INTEGER NOT NULL,
                 sha256    TEXT NOT NULL,
                 payload   TEXT NOT NULL,
                 synced_at TEXT NOT NULL
             )",
        )
        .unwrap();
        conn.execute(
            "INSERT INTO hub_models_cache (id, version, sha256, payload, synced_at)
             VALUES (1, 1, 'sha', ?1, '2026-01-01T00:00:00Z')",
            rusqlite::params![format!(
                r#"{{"version":1,"exchange_rates":{rates},"models":[]}}"#
            )],
        )
        .unwrap();
        (dir, store)
    }

    // A limit's currency has to be one this machine can convert. The limit is
    // measured against costs priced in other currencies, and `convert_amount` hands
    // a currency it has no rate for back **unchanged** — added to the others at
    // 1:1 — so a limit denominated in one fires at the wrong time.
    //
    // The rule lives in the normalizer, which is where every writer passes: the
    // dialog and the CLI both build a `NewProviderInput`, and the share importer
    // calls it directly.
    #[test]
    fn a_limit_currency_must_be_one_this_machine_can_convert() {
        // Never synced: no table at all, so the two currencies the Hub publishes
        // rates against. Empty is not "anything goes" — no rate exists for a third
        // one either.
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        assert_eq!(known_limit_currencies(&s), vec!["USD", "CNY"]);
        assert!(add_provider(&s, &aux, &limited_input("USD")).is_ok());
        assert!(
            add_provider(&s, &aux, &limited_input("cny")).is_ok(),
            "and it is not case-sensitive"
        );
        let err = match add_provider(&s, &aux, &limited_input("EUR")) {
            Err(e) => e,
            Ok(_) => panic!("EUR has no rate on this machine"),
        };
        assert!(
            err.contains("EUR") && err.contains("CNY"),
            "the refusal names the currency and what it does know: {err}"
        );

        // Synced: the table's own list, in order.
        let (_dir, synced) = store_with_hub_rates(r#"{"USD":1.0,"CNY":7.1,"EUR":0.9}"#);
        let aux2 = Aux::open_in_memory().unwrap();
        assert_eq!(known_limit_currencies(&synced), vec!["CNY", "EUR", "USD"]);
        assert!(add_provider(&synced, &aux2, &limited_input("eur")).is_ok());
        assert!(add_provider(&synced, &aux2, &limited_input("JPY")).is_err());

        // The counting units are not currencies, and a limit in one is what the
        // vast majority of providers have.
        for unit in ["requests", "wan_tokens"] {
            assert!(
                add_provider(&s, &aux, &limited_input(unit)).is_ok(),
                "{unit} is a counting unit"
            );
        }
    }

    // ── Declared prices (the Custom form's Prices section) ──────────────────

    /// A bundle as the form sends it.
    fn price_row(model_id: &str, input: &str, output: &str) -> ProviderPriceInput {
        ProviderPriceInput {
            model_id: model_id.into(),
            input: input.into(),
            output: output.into(),
            cache_read: None,
            cache_creation: None,
        }
    }

    fn prices(currency: &str, models: Vec<ProviderPriceInput>) -> ProviderPricesInput {
        ProviderPricesInput {
            currency: currency.into(),
            models,
        }
    }

    /// What the prices section hands the backend, both ways: a provider typed in
    /// by hand carries the figures to its row, and the dialog reads them back.
    #[test]
    fn declared_prices_round_trip_through_add_and_edit() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();

        let mut input = catalog_input("Manual", "https://api.manual.example");
        input.billing_config.limit_value = Some(50.0);
        input.billing_config.limit_unit = Some("CNY".into());
        input.prices = Some(prices(
            "cny",
            vec![
                price_row("kimi-k2", "1.5", "6"),
                price_row("glm-4.6", "2", "8"),
            ],
        ));
        let vm = add_provider(&s, &aux, &input).unwrap();

        // The currency is stored uppercase, and the figures come back as typed —
        // the dialog is the only thing that can correct them, so it has to see
        // what it sent.
        let stored = vm.prices.expect("the VM carries the declared prices");
        assert_eq!(stored["currency"], "CNY");
        assert_eq!(stored["models"][0]["model_id"], "kimi-k2");
        assert_eq!(stored["models"][0]["input"], "1.5");
        // …and the gateway's own reader agrees, keyed by the row's id.
        let rows = s.load_declared_prices().unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.provider_id == vm.id));
        assert!(rows.iter().all(|r| r.currency == "CNY"));

        // An edit that says nothing about prices keeps them.
        let mut edit = catalog_input("Manual", "https://api.manual.example");
        edit.prices = None;
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &edit,
            &no_vars(),
        )
        .unwrap();
        assert!(vm.prices.is_some(), "absent means keep");

        // An edit with an empty bundle clears them: that is the form's state when
        // the provider leaves pay-as-you-go.
        let mut cleared = catalog_input("Manual", "https://api.manual.example");
        cleared.prices = Some(prices("CNY", vec![]));
        let vm = update_provider(
            &s,
            &aux,
            std::path::Path::new("/tmp"),
            &vm.id,
            &cleared,
            &no_vars(),
        )
        .unwrap();
        assert!(vm.prices.is_none());
        assert!(s.load_declared_prices().unwrap().is_empty());
    }

    /// What the section refuses, and why each refusal is one rather than a
    /// silent drop: both would leave the provider quietly mispriced.
    #[test]
    fn declared_prices_refuse_a_currency_or_a_rate_that_cannot_be_used() {
        let known = known_limit_currencies(&store());

        // A currency this machine cannot convert: these figures are what the
        // spending limit is measured against.
        let err =
            normalize_declared_prices(Some(&prices("EUR", vec![price_row("m", "1", "2")])), &known)
                .expect_err("EUR has no rate on this machine");
        assert!(err.contains("EUR") && err.contains("CNY"), "{err}");
        assert!(normalize_declared_prices(Some(&prices("YEN", vec![])), &known).is_err());

        // A rate the price table cannot parse as a number would reach the cost
        // arithmetic as NaN — in every request to this provider.
        for bad in ["abc", "-1", "1e999"] {
            let bundle = prices("USD", vec![price_row("m", bad, "2")]);
            assert!(
                normalize_declared_prices(Some(&bundle), &known).is_err(),
                "`{bad}` is not a rate"
            );
        }
    }

    /// The rows the section drops rather than stores: a blank model id (the empty
    /// tail row the form always leaves) and a duplicate.
    #[test]
    fn declared_prices_drop_blank_and_duplicate_rows() {
        let known = known_limit_currencies(&store());
        let bundle = prices(
            "USD",
            vec![
                price_row("", "9", "9"),
                price_row("kimi-k2", "1.5", "6"),
                price_row("Kimi-K2", "99", "99"),
                price_row("glm-4.6", "2", "8"),
            ],
        );
        let blob = normalize_declared_prices(Some(&bundle), &known)
            .unwrap()
            .expect("two rows survive");
        let parsed = kiwano_adapters::model_pricing::DeclaredPrices::parse(&blob).unwrap();
        // Case-insensitively deduplicated, first wins: the table keys a model in
        // lowercase, so keeping both would make one of them win by write order.
        assert_eq!(parsed.models.len(), 2);
        assert_eq!(parsed.models[0].input, "1.5");

        // A rate the user left out is zero, not the input rate.
        let bundle = prices("USD", vec![price_row("m", "1", "2")]);
        let blob = normalize_declared_prices(Some(&bundle), &known)
            .unwrap()
            .unwrap();
        let parsed = kiwano_adapters::model_pricing::DeclaredPrices::parse(&blob).unwrap();
        assert_eq!(parsed.models[0].cache_read, "0");
        assert_eq!(parsed.models[0].cache_creation, "0");

        // Nothing to say, in both of the ways the form says it.
        assert!(normalize_declared_prices(None, &known).unwrap().is_none());
        assert!(
            normalize_declared_prices(Some(&prices("USD", vec![])), &known)
                .unwrap()
                .is_none()
        );
    }

    /// A shared config carries the declared prices with the provider, and they
    /// are validated for the importing machine the way a limit's unit is — same
    /// rule, because the limit is measured against them.
    #[test]
    fn a_shared_provider_keeps_its_declared_prices() {
        let known = known_limit_currencies(&store());
        let blob = normalize_declared_prices(
            Some(&prices("CNY", vec![price_row("kimi-k2", "1.5", "6")])),
            &known,
        )
        .unwrap();

        let carried = import_declared_prices(blob.as_deref(), &known).unwrap();
        assert_eq!(carried, blob, "unchanged where the currency converts");

        // A blob this build cannot read is dropped, not fatal: it says nothing
        // to honour, and failing the import would cost the provider its row.
        assert_eq!(
            import_declared_prices(Some("{not json"), &known).unwrap(),
            None
        );
        assert_eq!(import_declared_prices(None, &known).unwrap(), None);

        // A currency this machine cannot convert is refused, exactly as the same
        // file's limit unit would be: the provider is about to be costed in it.
        let foreign = r#"{"currency":"EUR","models":[{"model_id":"m","input":"1","output":"2"}]}"#;
        assert!(import_declared_prices(Some(foreign), &known).is_err());
    }

    /// The boundary the matching rule turns on. A future "simplify to
    /// `starts_with`" has to fail here rather than silently link a lookalike
    /// host to a vendor's prices.
    #[test]
    fn endpoint_matches_takes_a_path_boundary() {
        // Equal keys, and either direction of the path prefix: a bare host
        // typed by the user, or a path an imported config carries.
        assert!(endpoint_matches("api.deepseek.com", "api.deepseek.com"));
        assert!(endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com/anthropic"
        ));
        assert!(endpoint_matches(
            "api.deepseek.com/anthropic",
            "api.deepseek.com"
        ));
        assert!(endpoint_matches("api.deepseek.com/v1", "api.deepseek.com"));

        // …and what must not match: a neighbour host, another port, and two
        // different sub-services under one host.
        assert!(!endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com.evil.com"
        ));
        assert!(!endpoint_matches(
            "api.deepseek.com",
            "api.deepseek.com:8443"
        ));
        assert!(!endpoint_matches(
            "api.deepseek.com/v1",
            "api.deepseek.com/anthropic"
        ));
        // An empty key would otherwise be a prefix of everything.
        assert!(!endpoint_matches("", "api.deepseek.com"));
        assert!(!endpoint_matches("api.deepseek.com", ""));
    }

    /// Added from the CLI or by hand, the endpoint still names the entry:
    /// that is what makes the provider's own prices reachable.
    #[test]
    fn add_provider_links_the_unique_catalog_entry() {
        let s = store();
        let aux = catalog_aux();

        // A bare host where the entry names a deeper path.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("DeepSeek", "https://api.deepseek.com"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("deepseek"));

        // A path the entry advertises among two — still one entry, one link.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("KFC", "https://api.kimi.com/coding"),
        )
        .unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding")
        );

        // The other direction: the local URL carries the deeper path.
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("Bare", "https://api.bare.example/v1"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id).as_deref(), Some("bare-host"));
    }

    /// The default model the form collects survives the trip to storage and back.
    /// It used to be dropped on the floor — collected, sent, ignored — which is
    /// why reopening a provider showed an empty box however carefully it had been
    /// filled in.
    #[test]
    fn add_provider_stores_the_default_model_and_hands_it_back() {
        let s = store();
        let aux = catalog_aux();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.model_default = "deepseek-v4-pro".into();
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(vm.model_default.as_deref(), Some("deepseek-v4-pro"));
        assert_eq!(
            s.get_provider(&vm.id)
                .unwrap()
                .unwrap()
                .model_default
                .as_deref(),
            Some("deepseek-v4-pro"),
            "on the row, not only in the reply"
        );

        // Whitespace is not a model name, and an empty box is "not set" rather
        // than the empty string.
        let mut blank = catalog_input("Blank", "https://api.blank.example/v1");
        blank.model_default = "   ".into();
        let blank_vm = add_provider(&s, &aux, &blank).unwrap();
        assert_eq!(blank_vm.model_default, None);
        assert_eq!(
            s.get_provider(&blank_vm.id).unwrap().unwrap().model_default,
            None
        );

        // An edit is authoritative: clearing the box clears the column, the way
        // emptying the endpoint list rewrites it.
        let mut edit = catalog_input("DeepSeek", "https://api.deepseek.com");
        edit.model_default = String::new();
        update_provider(&s, &aux, live_home(&[]).path(), &vm.id, &edit, &no_vars()).unwrap();
        assert_eq!(s.get_provider(&vm.id).unwrap().unwrap().model_default, None);
    }

    /// An edit that names no agents leaves the bindings exactly as they are.
    ///
    /// Which is not the same as sending the set that is already bound: that loop
    /// promotes *this* provider to primary for every agent it names and flattens
    /// the agent's strategy to Single. Saving an unrelated field therefore used
    /// to reorder an agent's failover queue and change how it routes.
    #[test]
    fn an_edit_that_names_no_agents_leaves_the_bindings_alone() {
        let s = store();
        let aux = catalog_aux();

        // Two providers serving claude. The second one added is primary.
        let mut first = catalog_input("A", "https://a.example.com/v1");
        first.agents = Some(vec!["claude".into()]);
        let a = add_provider(&s, &aux, &first).unwrap();
        let mut second = catalog_input("B", "https://b.example.com/v1");
        second.agents = Some(vec!["claude".into()]);
        let b = add_provider(&s, &aux, &second).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(b.id.as_str())
        );

        // The agent is set up as a failover queue behind that primary.
        s.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();

        // An edit that says nothing about agents: nothing moves.
        let home = live_home(&["claude"]);
        let mut edit = catalog_input("A", "https://a.example.com/v1");
        edit.agents = None;
        update_provider(&s, &aux, home.path(), &a.id, &edit, &no_vars()).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(b.id.as_str()),
            "the edit must not promote itself over the standing primary"
        );
        assert_eq!(
            s.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Failover,
            "and must not flatten the agent's strategy"
        );

        // Naming agents still rebinds — that is the one path that may, and the
        // reason the field is an Option rather than a Vec.
        edit.agents = Some(vec!["claude".into()]);
        update_provider(&s, &aux, home.path(), &a.id, &edit, &no_vars()).unwrap();
        assert_eq!(
            s.primary_provider_id("claude").unwrap().as_deref(),
            Some(a.id.as_str())
        );
        assert_eq!(
            s.get_strategy("claude").unwrap().unwrap().kind,
            StrategyType::Single
        );
    }

    /// A stored key answers only for the endpoints that provider already answers
    /// on. That guard is the reason the fallback is safe to offer at all: the URL
    /// field is editable, and "use the saved key for whatever is in the box"
    /// would make the Test button a way to post a credential to any host.
    #[test]
    fn a_stored_key_covers_only_the_providers_own_endpoints() {
        let s = store();
        let aux = catalog_aux();
        let mut input = catalog_input("Kimi", "https://api.moonshot.cn");
        input.api_key = "sk-secret".into();
        let vm = add_provider(&s, &aux, &input).unwrap();

        // Its own endpoint, however it is spelled.
        for spelling in [
            "https://api.moonshot.cn",
            "api.moonshot.cn",
            "api.moonshot.cn/",
            "HTTPS://API.MOONSHOT.CN",
        ] {
            assert_eq!(
                stored_key_for(&s, &vm.id, spelling).as_deref(),
                Some("sk-secret"),
                "{spelling} is where this provider answers"
            );
        }

        // Anywhere else: nothing, so the probe goes out anonymous and reports
        // what it finds instead of leaking the key.
        assert_eq!(stored_key_for(&s, &vm.id, "https://evil.example.com"), None);
        assert_eq!(
            stored_key_for(&s, "no-such-provider", "api.moonshot.cn"),
            None
        );
    }

    /// A caller that names an entry is the authority; the inference fills in
    /// what nobody said.
    #[test]
    fn add_provider_keeps_an_explicit_catalog_id() {
        let s = store();
        let aux = catalog_aux();

        let mut input = catalog_input("DeepSeek", "https://api.deepseek.com");
        input.catalog_id = Some("kimi-for-coding".into());
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding"),
            "the caller's answer, not the endpoint's"
        );

        // Even when it names nothing: an id is a statement, not a hint to be
        // corrected into something the endpoint suggests.
        let mut input = catalog_input("Unlisted", "https://api.deepseek.com");
        input.catalog_id = Some("not-a-real-entry".into());
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("not-a-real-entry")
        );
    }

    /// Two entries on one host: the endpoint cannot say which is meant, and a
    /// guess here is a wrong price with nothing to show for it.
    #[test]
    fn add_provider_does_not_guess_between_ambiguous_entries() {
        let s = store();
        let aux = catalog_aux();
        let vm = add_provider(
            &s,
            &aux,
            &catalog_input("Ambiguous", "https://api.example.com"),
        )
        .unwrap();
        assert_eq!(stored_catalog_id(&s, &vm.id), None);
    }

    /// A genuinely custom provider — self-hosted, an aggregator the Hub does
    /// not list — has no entry to link to, and stays that way.
    #[test]
    fn add_provider_leaves_a_custom_endpoint_unlinked() {
        let s = store();
        let aux = catalog_aux();
        for endpoint in [
            "https://my-own.example.com",
            "https://api.deepseek.com.evil.com",
            "https://api.deepseek.com:8443",
        ] {
            let vm = add_provider(&s, &aux, &catalog_input("Custom", endpoint)).unwrap();
            assert_eq!(stored_catalog_id(&s, &vm.id), None, "{endpoint}");
        }
    }

    /// The provider's extra per-protocol endpoints are part of the match, the
    /// same set the shelf's `added` derives from.
    #[test]
    fn add_provider_links_from_an_extra_endpoint() {
        let s = store();
        let aux = catalog_aux();
        let mut input = catalog_input("KFC", "https://my-own.example.com");
        input.endpoints = vec![NewEndpointInput {
            protocol: "anthropic".into(),
            endpoint: "https://api.kimi.com/coding".into(),
        }];
        let vm = add_provider(&s, &aux, &input).unwrap();
        assert_eq!(
            stored_catalog_id(&s, &vm.id).as_deref(),
            Some("kimi-for-coding")
        );
    }

    /// The backfill, on rows that predate the column: only `NULL`s are touched,
    /// an existing link is left alone down to its `updated_at` (a backfill is
    /// not a user edit), and a second run has nothing left to do.
    #[test]
    fn link_providers_fills_only_nulls() {
        let s = store();
        let aux = catalog_aux();

        let mut ds = provider("ds", "DeepSeek", Billing::Metered);
        ds.base_url = "https://api.deepseek.com".into();
        let mut kfc = provider("kfc", "Kimi For Coding", Billing::Metered);
        kfc.base_url = "https://api.kimi.com/coding".into();
        kfc.catalog_id = Some("kimi-for-coding".into());
        let mut ambiguous = provider("amb", "Ambiguous", Billing::Metered);
        ambiguous.base_url = "https://api.example.com".into();
        for p in [&ds, &kfc, &ambiguous] {
            s.insert_provider(p).unwrap();
        }

        assert_eq!(link_providers(&s, &aux).unwrap(), 1);
        assert_eq!(stored_catalog_id(&s, "ds").as_deref(), Some("deepseek"));
        assert_eq!(
            stored_catalog_id(&s, "kfc").as_deref(),
            Some("kimi-for-coding")
        );
        assert_eq!(stored_catalog_id(&s, "amb"), None);
        assert_eq!(
            s.get_provider("kfc").unwrap().unwrap().updated_at,
            kfc.updated_at,
            "a link already made is a fact, not something to re-derive"
        );

        // Idempotent: the second pass has nothing to write.
        assert_eq!(link_providers(&s, &aux).unwrap(), 0);
    }

    /// Never synced: there is nothing to infer from, so nothing is written —
    /// and nothing is cleared either.
    #[test]
    fn link_providers_is_a_noop_without_a_catalog() {
        let s = store();
        let mut p = provider("ds", "DeepSeek", Billing::Metered);
        p.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&p).unwrap();

        assert_eq!(link_providers(&s, &linkless_aux()).unwrap(), 0);
        assert_eq!(stored_catalog_id(&s, "ds"), None);
    }

    /// The shelf's badge asks the same question as the link — "is this entry
    /// one the user already has?" — so it answers the same way. Otherwise a
    /// linked provider would still be offered for adding a second time.
    #[test]
    fn catalog_added_matches_the_link_rule() {
        let aux = catalog_aux();
        let s = store();
        let mut p = provider("ds", "DeepSeek", Billing::Metered);
        p.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&p).unwrap();

        let added: Vec<String> = load_catalog(&s, &aux)
            .entries
            .into_iter()
            .filter(|e| e.added)
            .map(|e| e.id)
            .collect();
        assert_eq!(added, ["deepseek"]);
    }

    #[test]
    fn catalog_added_derives_from_provider_endpoints() {
        let aux = Aux::open_in_memory().unwrap();
        let s = store();
        // Seeded from the Hub cache, which is the only source now — this used to
        // lean on the bundled catalog, and the two entries below are the ones it
        // asserted against: DeepSeek on its primary, Kimi For Coding reachable on
        // a second protocol at a different path.
        let payload = r#"{"total":2,"entries":[
            {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
             "billing":"payg","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com"}]},
            {"id":"kimi-for-coding","name":"Kimi For Coding","tag":"official","rating":4,
             "billing":"plan","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.kimi.com/coding/v1"},
                          {"protocol":"anthropic","endpoint":"https://api.kimi.com/coding"}]}
        ]}"#;
        aux.save_hub_cache(payload, "2026-09-07T00:00:00Z").unwrap();

        // nothing added yet: the flags a payload may carry are ignored
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

        // a provider on the primary endpoint also marks the entry added.
        // Same `aux`, not a fresh one: `added` is derived from the store, and a
        // fresh aux would have no cache — an empty shelf that passes for the
        // wrong reason.
        let mut d = provider("ds", "DeepSeek", Billing::Metered);
        d.base_url = "https://api.deepseek.com".into();
        s.insert_provider(&d).unwrap();
        let list2 = load_catalog(&s, &aux);
        let added2: Vec<&str> = list2
            .entries
            .iter()
            .filter(|e| e.added)
            .map(|e| e.id.as_str())
            .collect();
        // catalog order as published: DeepSeek first
        assert_eq!(added2, ["deepseek", "kimi-for-coding"]);
    }

    /// Disabling takes a provider out of service without taking anything away:
    /// the binding is still there, the key is still there, and the row says so.
    #[test]
    fn a_disabled_provider_keeps_everything_but_its_place_in_the_route() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("a1", "Alpha", Billing::Metered);
        p.api_key = Some("sk-keep".into());
        s.insert_provider(&p).unwrap();
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

        set_provider_enabled(&s, "a1", false).unwrap();
        let stored = s.get_provider("a1").unwrap().unwrap();
        assert!(!stored.enabled);
        assert_eq!(stored.api_key.as_deref(), Some("sk-keep"));
        assert_eq!(
            s.bindings_for_agent("claude").unwrap().len(),
            1,
            "the route is untouched: what changed is whether this row may serve"
        );
        // The row reads as disabled rather than as healthy, and its agents fall
        // through to whoever is next (nobody, here — the route is empty).
        let home = live_home(&["claude"]);
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(vms.len(), 1);
        assert!(!vms[0].enabled);
        assert!(
            vms[0].serving_agents.is_empty(),
            "a parked provider serves nobody"
        );
        assert_eq!(vms[0].health.note.as_deref(), Some("Disabled"));

        // Back on, and the same route applies again.
        set_provider_enabled(&s, "a1", true).unwrap();
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        assert!(vms[0].enabled);
        assert_eq!(vms[0].serving_agents, ["claude"]);

        // Idempotent, and an unknown id is an error rather than a no-op.
        set_provider_enabled(&s, "a1", true).unwrap();
        assert!(set_provider_enabled(&s, "ghost", false).is_err());
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
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
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
            agents: Some(vec!["codex".into()]),
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &linkless_aux(), &input).unwrap();
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
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
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
            agents: Some(vec![]),
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
        add_provider(&s, &linkless_aux(), &input).unwrap();

        let rows = s.list_providers().unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].endpoints.len(), 1);
        assert_eq!(
            rows[0].endpoints[0].base_url,
            "https://qianfan.baidubce.com/anthropic/coding"
        );
        assert_eq!(
            rows[0].endpoints[0].protocol,
            kiwanod::store::Protocol::Anthropic
        );
    }

    #[test]
    fn provider_vm_carries_endpoints_and_note_suffix() {
        let s = store();
        let input = NewProviderInput {
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
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
            agents: Some(vec![]),
            endpoints: vec![NewEndpointInput {
                protocol: "anthropic".into(),
                endpoint: "https://qianfan.baidubce.com/anthropic/coding".into(),
            }],
            advanced: None,
            plan_query: None,
        };
        let vm = add_provider(&s, &linkless_aux(), &input).unwrap();
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
        let vms = build_provider_vms(&s, &aux, live_home(&[]).path(), &no_vars()).unwrap();
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
            catalog_id: None,
            // No declared prices: this fixture is priced by the Hub's table.
            prices: None,
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
            agents: Some(vec!["codex".into()]), // rebind: claude dropped
            endpoints: Vec::new(),
            advanced: None,
            plan_query: None,
        };
        let vm = update_provider(
            &s,
            &aux,
            live_home(&["codex"]).path(),
            "p1",
            &input,
            &no_vars(),
        )
        .unwrap();
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
            s.record_usage(&kiwanod::store::UsageRecord {
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
                cost_off_peak: None,
            })
            .unwrap();
            // The headline count reads request_logs, not usage: seed both, as
            // the gateway does for a forwarded request.
            s.insert_request_log(&kiwanod::store::RequestLogNew {
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
                reasoning_tokens: 0,
                usage_missing: false,
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
                cost_off_peak: None,
            })
            .unwrap();
        }
    }

    #[test]
    fn dashboard_windows_cover_the_right_days() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        // Display conversion uses the Hub's published rates and there is no
        // compiled snapshot behind them, so the fixture publishes the rate the
        // cost assertion below converts with.
        let hub = serde_json::json!({
            "version": 1,
            "exchange_rates": { "USD": 1.0, "CNY": 7.1 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(1, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
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
        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
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

        update_settings(
            &s,
            &aux,
            &serde_json::json!({ "tz_offset_minutes": 480 }),
            &no_vars(),
        )
        .unwrap();
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

    // The app owns the streaming timeouts' UI and the gateway owns their
    // enforcement, and the two halves meet in `gateway_settings`: a patch has to
    // reach the sidecar's copy, not just this side's JSON, or the settings row
    // would look applied and change nothing. The defaults are asserted against
    // the gateway's own so the two cannot drift apart in silence.
    #[test]
    fn streaming_timeouts_round_trip_into_the_gateways_copy() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();

        assert_eq!(
            kiwanod::store::StreamTimeouts {
                first_byte_secs: u64::from(default_stream_first_byte_secs()),
                idle_secs: u64::from(default_stream_idle_secs()),
            },
            kiwanod::store::StreamTimeouts::default(),
            "the app's defaults and the gateway's must be the same numbers"
        );

        let vm = update_settings(
            &s,
            &aux,
            &serde_json::json!({ "stream_idle_secs": 45 }),
            &no_vars(),
        )
        .unwrap();
        assert_eq!(vm.stream_idle_secs, 45);

        let cfg = s.load_stream_timeouts().unwrap();
        assert_eq!(cfg.idle_secs, 45, "the sidecar's copy moved");
        assert_eq!(
            cfg.first_byte_secs,
            u64::from(default_stream_first_byte_secs()),
            "patching one leaves the other alone"
        );

        // And the screen reads back what it wrote.
        let reread = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(reread.stream_idle_secs, 45);
    }

    // The files a takeover would rewrite, as the agent's settings dialog lists
    // them. Read out of `takeover_paths` — the function the takeover itself writes
    // through — so the list cannot drift from what a takeover actually touches,
    // and printed with `~` because that is how the same paths read in the docs.
    #[test]
    fn the_takeover_vm_carries_the_files_a_takeover_rewrites() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path();

        let vm = build_settings_with_home(&s, &aux, home, &no_vars()).unwrap();
        let paths = |agent: &str| {
            vm.takeovers
                .iter()
                .find(|t| t.agent == agent)
                .unwrap_or_else(|| panic!("{agent} is in the registry"))
                .config_paths
                .clone()
        };

        // Spelled with the platform's separator, because that is what the screen
        // should show and what the row's copy button should hand over. A literal
        // `~/.claude/settings.json` here is an assertion only Unix passes — which
        // is how this test first went red, on Windows, against a `display_path`
        // that glued `~/` to a Windows path.
        let sep = std::path::MAIN_SEPARATOR;
        let at_home = |tail: &str| format!("~{sep}{}", tail.replace('/', &sep.to_string()));
        assert_eq!(paths("claude"), vec![at_home(".claude/settings.json")]);
        // Two files: the config Codex reads and the auth it keeps beside it.
        assert_eq!(
            paths("codex"),
            vec![at_home(".codex/config.toml"), at_home(".codex/auth.json")]
        );
        assert_eq!(paths("grokbuild"), vec![at_home(".grok/config.toml")]);

        // Every built-in resolves to at least one file — except the one whose
        // takeover is macOS-only. `claude-desktop` comes back empty elsewhere by
        // design, and the row renders nothing for it rather than a path that does
        // not exist.
        for t in vm.takeovers.iter().filter(|t| t.agent != "claude-desktop") {
            assert!(!t.config_paths.is_empty(), "{} has no files", t.agent);
        }
        let desktop = vm
            .takeovers
            .iter()
            .find(|t| t.agent == "claude-desktop")
            .expect("claude-desktop is in the registry");
        assert_eq!(
            desktop.config_paths.is_empty(),
            cfg!(not(target_os = "macos")),
            "claude-desktop's files are macOS-only"
        );

        // A path outside the home tree keeps its absolute form rather than being
        // rewritten into a `~/…` that names nothing. Built the way the real ones
        // are — a component at a time — because a tail written as one literal with
        // forward slashes keeps them on Windows: `Path` separates on either, so the
        // raw text is what the caller wrote.
        assert_eq!(
            display_path(&home.join(".hermes").join("config.yaml"), home),
            at_home(".hermes/config.yaml")
        );
        assert_eq!(
            display_path(Path::new("/opt/hermes/config.yaml"), home),
            "/opt/hermes/config.yaml"
        );
    }

    // The screen's half of an agent's ceilings: what it writes is what the gateway
    // measures. Two windows are written at once here because that is the shape the
    // change was for — the zero and empty cases matter for the same reason they
    // did when there was one.
    #[test]
    fn agent_limits_round_trip_as_a_set_of_windows() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let agent = "claude";

        let p = provider("ds-1", "DeepSeek", Billing::Metered);
        s.insert_provider(&p).unwrap();
        s.upsert_binding(&Binding {
            agent: agent.into(),
            provider_id: p.id.clone(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        let route = |s: &Store| {
            build_agent_routes(s)
                .unwrap()
                .into_iter()
                .find(|r| r.agent == agent)
                .expect("the agent has a binding")
        };
        assert!(route(&s).limits.is_empty(), "no rows is no ceiling");

        let day = AgentLimitVm {
            period: "day".into(),
            period_limit: 100.0,
            limit_unit: None,
        };
        let month = AgentLimitVm {
            period: "monthly".into(),
            period_limit: 50.0,
            limit_unit: Some("CNY".into()),
        };
        set_agent_limits(&s, agent, vec![day.clone(), month.clone()]).unwrap();
        assert_eq!(
            route(&s).limits,
            vec![day.clone(), month.clone()],
            "both windows come back, in window order"
        );

        // A window of zero is the absence of that window, not a ceiling of
        // nothing: the gateway would ignore it while the screen showed it.
        set_agent_limits(
            &s,
            agent,
            vec![
                day.clone(),
                AgentLimitVm {
                    period: "weekly".into(),
                    period_limit: 0.0,
                    limit_unit: None,
                },
            ],
        )
        .unwrap();
        assert_eq!(
            route(&s).limits,
            vec![day.clone()],
            "the zero window is gone"
        );

        // Replacing the set keeps the surviving window's age — it is the same
        // window, not a new one that happens to look the same.
        let first_set = s.agent_limits_for(agent).unwrap()[0].created_at.clone();
        set_agent_limits(&s, agent, vec![day.clone(), month.clone()]).unwrap();
        let rows = s.agent_limits_for(agent).unwrap();
        assert_eq!(rows.len(), 2);
        assert_eq!(
            rows.iter().find(|l| l.period == "day").unwrap().created_at,
            first_set
        );

        // And an empty set clears them all.
        set_agent_limits(&s, agent, vec![]).unwrap();
        assert!(route(&s).limits.is_empty());

        // A screen that was never asked about limits still builds.
        assert_eq!(
            build_settings_with_home(&s, &aux, tmp.path(), &no_vars())
                .unwrap()
                .language,
            "system"
        );
    }

    #[test]
    fn settings_roundtrip_and_merge() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        // Every build reads this temp home, never the developer's real one:
        // takeover state comes from the live files, so `$HOME` would otherwise
        // decide what these assertions see.
        let tmp = tempfile::tempdir().unwrap();
        let v0 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(v0.language, "system");
        assert!(v0.takeovers.iter().all(|t| !t.enabled));

        let patch =
            serde_json::json!({ "language": "en", "telemetry": true, "takeovers": "ignored" });
        // The patch still carries `telemetry`, a key older blobs hold and
        // nothing reads any more: it must be ignored, not choke the merge.
        let v1 = update_settings(&s, &aux, &patch, &no_vars()).unwrap();
        assert_eq!(v1.language, "en");
        // The stored blob now holds a key the struct no longer declares. It has
        // to parse anyway: a failed parse falls back to every default at once,
        // which would silently reset the reader's whole settings page.
        let reread = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert_eq!(reread.language, "en", "an old key is ignored, not fatal");
        // takeovers untouched by patch (read against the temp home, like every
        // other assertion here — `update_settings` builds its answer against
        // the process home, which this test must not depend on)
        let after_patch = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(after_patch.takeovers.iter().all(|t| !t.enabled));

        // no ~/.claude/settings.json → rejected and no key left behind
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap_err();
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != "claude"));

        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        let v2 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
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
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), "{}");
        let v3 = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(
            !v3.takeovers
                .iter()
                .find(|t| t.agent == "claude")
                .unwrap()
                .enabled
        );
    }

    #[test]
    fn takeover_state_comes_from_the_live_file_not_the_key_table() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();

        // The registration disappears (a rolled-back key row) while the config
        // stays rewritten: the reader must still report the takeover, because
        // that is the state the user has to be able to undo.
        for k in s.list_placeholder_keys().unwrap() {
            if k.agent == "claude" {
                s.delete_placeholder_key(&k.key).unwrap();
            }
        }
        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(
            claude.enabled,
            "the live config still points at the gateway"
        );
        // …and the key is read out of the file rather than out of the table.
        assert!(claude
            .placeholder_key
            .as_deref()
            .unwrap_or_default()
            .starts_with("kw-ag-claude-"));

        // The reverse: a key row with no rewritten file and no backup is not a
        // takeover.
        std::fs::write(&settings, "{}").unwrap();
        aux.delete_takeover_backup("claude").unwrap();
        s.upsert_placeholder_key("kw-ag-claude-orphan", "claude")
            .unwrap();
        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(
            !claude.enabled,
            "a key with no rewritten config (and no backup) is not a takeover"
        );
        assert!(claude.placeholder_key.is_none());
    }

    /// A user-defined agent is a route and nothing else: a row, a key and a
    /// strategy. It needs no config file to route — the empty temp home below is
    /// the proof, since a built-in agent with the same binding reads as dormant
    /// there.
    #[test]
    fn a_custom_agent_routes_without_a_config_file() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", Some("night batch"), Some("gemini")).unwrap();

        assert!(a.id.starts_with("long-tasks-"), "{}", a.id);
        assert_eq!(
            a.protocol.as_deref(),
            Some("gemini"),
            "the protocol it was created with travels back"
        );
        assert!(!is_builtin_agent(&a.id));
        let key = a.placeholder_key.clone().expect("a key is minted with it");
        assert!(key.starts_with(&format!("kw-ag-{}", a.id)), "{key}");

        add_agent_binding(&s, &a.id, "p1").unwrap();
        add_agent_binding(&s, "claude", "p1").unwrap(); // the control: dormant here
        let home = live_home(&[]); // nothing taken over on this machine
        let vms = build_provider_vms(&s, &aux, home.path(), &no_vars()).unwrap();
        let vm = &vms[0];
        assert_eq!(vm.agents, [a.id.as_str()], "the custom agent is bound");
        assert_eq!(
            vm.serving_agents,
            [a.id.as_str()],
            "…and it serves: it has no config for the evidence to be missing from"
        );
        assert!(vm.is_current);

        // The UI gets it, with the key it is configured by, and the takeover
        // list is unmoved: there is no config here to take over.
        let settings = build_settings_with_home(&s, &aux, home.path(), &no_vars()).unwrap();
        assert_eq!(settings.custom_agents.len(), 1);
        assert_eq!(settings.custom_agents[0].label, "Long Tasks");
        assert_eq!(
            settings.custom_agents[0].protocol.as_deref(),
            Some("gemini")
        );
        assert_eq!(
            settings.custom_agents[0].placeholder_key.as_deref(),
            Some(key.as_str())
        );
        assert_eq!(
            settings.custom_agents[0].note.as_deref(),
            Some("night batch")
        );
        assert!(settings.takeovers.iter().all(|t| !t.enabled));
        assert!(
            set_agent_takeover(&s, &aux, &a.id, true, 8317, home.path(), &no_vars()).is_err(),
            "takeover is for agents with a config"
        );
    }

    /// Deleting a custom agent takes its route and its key with it and leaves
    /// the usage history — the same line provider deletion draws. Its traffic
    /// stays on the dashboard, under the id, because the rows that recorded it
    /// are not the route that was deleted.
    #[test]
    fn removing_a_custom_agent_clears_its_route_and_keeps_its_history() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        add_agent_binding(&s, &a.id, "p1").unwrap();
        let mut row = usage_row("p1");
        row.agent = a.id.clone();
        row.cost = Some(2.0);
        row.cost_currency = Some("CNY".into());
        s.record_usage(&row).unwrap();

        remove_custom_agent(&s, &a.id).unwrap();

        assert!(s.bindings_for_agent(&a.id).unwrap().is_empty());
        assert!(s.get_strategy(&a.id).unwrap().is_none());
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .all(|k| k.agent != a.id));
        assert!(s.get_custom_agent(&a.id).unwrap().is_none());
        // The provider is the user's row and stays; the usage is history.
        assert!(s.get_provider("p1").unwrap().is_some());
        assert_eq!(s.usage_totals(Some(&a.id), None, None).unwrap().requests, 1);

        // The dashboard still accounts for that traffic, labelled by its id —
        // the agent's name is gone, its requests are not.
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        let row = d
            .by_agent
            .iter()
            .find(|r| r.agent == a.id)
            .expect("history outlives the route");
        assert_eq!(row.label, a.id);
        assert!(d.filter_agents.iter().any(|f| f.id == a.id));

        // And it is gone as an agent: a second removal has nothing to remove.
        assert!(remove_custom_agent(&s, &a.id).is_err());
    }

    /// A rename moves the label — and the note — and nothing else. The id is
    /// what bindings, keys and usage rows point at, so a rename that touched it
    /// would orphan the agent's route and its history.
    #[test]
    fn renaming_a_custom_agent_keeps_its_id_route_and_history() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Alpha", Billing::Metered))
            .unwrap();
        let a = add_custom_agent(&s, "Long Tasks", Some("at night"), None).unwrap();
        add_agent_binding(&s, &a.id, "p1").unwrap();
        let mut row = usage_row("p1");
        row.agent = a.id.clone();
        s.record_usage(&row).unwrap();

        let renamed = update_custom_agent(&s, &a.id, "Nightly batch", Some("moved"), None).unwrap();

        assert_eq!(renamed.id, a.id, "the id is what everything points at");
        assert_eq!(renamed.label, "Nightly batch");
        assert_eq!(renamed.note.as_deref(), Some("moved"));
        assert_eq!(
            renamed.placeholder_key, a.placeholder_key,
            "the key belongs to the agent, not to the name its user gave it"
        );
        // The route and the history are still this agent's.
        assert_eq!(s.bindings_for_agent(&a.id).unwrap().len(), 1);
        assert!(s.get_strategy(&a.id).unwrap().is_some());
        assert_eq!(s.usage_totals(Some(&a.id), None, None).unwrap().requests, 1);
        // And the new name is what the dashboard reads for it now.
        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        assert_eq!(
            d.by_agent
                .iter()
                .find(|r| r.agent == a.id)
                .expect("still accounted for")
                .label,
            "Nightly batch"
        );
        // A blank note clears it rather than storing whitespace.
        let cleared = update_custom_agent(&s, &a.id, "Nightly batch", Some("  "), None).unwrap();
        assert_eq!(cleared.note, None);

        // Refusals: a blank name, and an id no agent has — and neither changes
        // what is on file.
        assert!(update_custom_agent(&s, &a.id, "   ", None, None).is_err());
        assert!(update_custom_agent(&s, "no-such-agent", "X", None, None).is_err());
        let stored = s.get_custom_agent(&a.id).unwrap().expect("still there");
        assert_eq!(stored.label, "Nightly batch");
        assert_eq!(stored.id, a.id);
    }

    /// The model a latency ping uses: the provider's own default, else the model
    /// its catalog entry prices — the one model we know the vendor serves. With
    /// neither, there is nothing to ping with, and the caller is told that
    /// rather than handed a model id this code invented.
    #[test]
    fn the_latency_ping_uses_the_providers_own_model_or_its_entrys() {
        let s = store();
        let catalog = r#"{"total":1,"entries":[
            {"id":"deepseek","name":"DeepSeek","tag":"official","rating":4.8,
             "billing":"payg","currency":"USD",
             "endpoints":[{"protocol":"openai","endpoint":"https://api.deepseek.com"}],
             "price_ref":{"model_id":"deepseek-chat","display_name":"DeepSeek Chat",
                          "input":"1","output":"2","currency":"USD"}}]}"#;
        let aux = Aux::open_in_memory().unwrap();
        aux.save_hub_cache(catalog, "2026-09-07T00:00:00Z").unwrap();

        let mut p = provider("p1", "DeepSeek", Billing::Metered);
        p.catalog_id = Some("deepseek".into());
        s.insert_provider(&p).unwrap();
        assert_eq!(
            prompt_test_model(&s, &aux, &p).as_deref(),
            Some("deepseek-chat")
        );

        // A default of its own wins: it is the model the user routes with.
        let mut with_default = p.clone();
        with_default.model_default = Some("deepseek-v4-flash".into());
        assert_eq!(
            prompt_test_model(&s, &aux, &with_default).as_deref(),
            Some("deepseek-v4-flash")
        );

        let orphan = provider("p2", "No Catalog", Billing::Metered);
        assert_eq!(prompt_test_model(&s, &aux, &orphan), None);
    }

    /// The id is derived from the name, is unique per agent even when the name
    /// repeats, and has a word for it even when the name has no ASCII in it.
    /// The two tables that describe a built-in agent have to agree about which
    /// agents exist: a registry entry with no protocols would render as "speaks
    /// nothing", and a protocol row for an id nobody knows is dead weight. Five
    /// parallel tables already drift silently in this codebase (the frontend's
    /// `AGENTS`, `AGENT_ICON` and `SEGMENTS`, plus `CLI_AGENTS` here) — this one
    /// is pinned to its neighbour.
    #[test]
    fn every_built_in_agent_has_at_least_one_protocol() {
        for (agent, label) in AGENTS {
            let protocols = agent_protocols(agent);
            assert!(
                !protocols.is_empty(),
                "{agent} ({label}) is in the registry with no protocol"
            );
            for p in protocols {
                assert!(
                    kiwanod::store::Protocol::parse_str(p).is_some(),
                    "{agent} names a protocol nobody knows: {p}"
                );
            }
        }
        assert_eq!(
            AGENT_PROTOCOLS.len(),
            AGENTS.len(),
            "the two tables describe different numbers of agents"
        );
        // And an id that is not a built-in gets nothing, rather than a guess.
        assert!(agent_protocols("long-tasks-3f9a").is_empty());
    }

    /// The protocol is a word three readers recognise, so a typo is refused
    /// rather than stored — and clearing it is how a user says they would
    /// rather not say.
    #[test]
    fn a_protocol_that_is_not_a_protocol_is_refused() {
        let s = store();
        let err = match add_custom_agent(&s, "Typo", None, Some("opemai")) {
            Ok(vm) => panic!("a typo was accepted: {}", vm.id),
            Err(e) => e,
        };
        assert!(err.contains("unknown protocol"), "{err}");

        let a = add_custom_agent(&s, "Fine", None, Some("  anthropic ")).unwrap();
        assert_eq!(a.protocol.as_deref(), Some("anthropic"), "trimmed");

        let cleared = update_custom_agent(&s, &a.id, "Fine", None, Some("")).unwrap();
        assert_eq!(cleared.protocol, None, "an empty choice is no choice");
    }

    #[test]
    fn custom_agent_ids_are_derived_and_unique() {
        let s = store();
        let first = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        let second = add_custom_agent(&s, "Long Tasks", None, None).unwrap();
        assert!(first.id.starts_with("long-tasks-"));
        assert!(second.id.starts_with("long-tasks-"));
        assert_ne!(first.id, second.id, "same name, two agents");
        assert!(!is_builtin_agent(&first.id));

        // A name with nothing slug-able in it still gets a usable id — and not
        // `slug`'s own fallback word, which belongs to providers.
        let cjk = add_custom_agent(&s, "长任务批处理", None, None).unwrap();
        assert!(cjk.id.starts_with("custom-"), "{}", cjk.id);

        // A name is required; whitespace is not one.
        assert!(add_custom_agent(&s, "   ", None, None).is_err());
        // …and a blank note is the same as no note.
        let blank = add_custom_agent(&s, "Bare", Some("  "), None).unwrap();
        assert_eq!(blank.note, None);
    }

    /// Turning a takeover off hands the agent its own config back — and its
    /// route with it. The providers themselves stay: they are the user's rows,
    /// carrying the key, the plan and the usage history, and they are what a
    /// later takeover re-imports and binds again.
    #[test]
    fn disabling_a_takeover_drops_the_route_and_keeps_the_providers() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(&settings, "{}").unwrap();

        // Two providers serving claude, the second standing by in a failover
        // queue: a route with something in it to lose.
        for (id, name) in [("p1", "One"), ("p2", "Two")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }
        s.upsert_strategy("claude", StrategyType::Failover, None)
            .unwrap();
        for (id, priority) in [("p1", 0), ("p2", 1)] {
            s.upsert_binding(&Binding {
                agent: "claude".into(),
                provider_id: id.into(),
                priority,
                weight: 1,
                win_start: None,
                win_end: None,
                enabled: true,
            })
            .unwrap();
        }

        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        assert_eq!(s.bindings_for_agent("claude").unwrap().len(), 2);
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();

        assert!(
            s.bindings_for_agent("claude").unwrap().is_empty(),
            "the route goes with the takeover"
        );
        assert!(
            s.get_strategy("claude").unwrap().is_none(),
            "…including the strategy it routed by"
        );
        // The rows survive, and read as unbound rather than as served.
        let vms = build_provider_vms(&s, &aux, tmp.path(), &no_vars()).unwrap();
        for id in ["p1", "p2"] {
            let vm = vms
                .iter()
                .find(|v| v.id == id)
                .unwrap_or_else(|| panic!("{id} was deleted with its binding"));
            assert!(vm.agents.is_empty(), "{id} still claims an agent");
            assert!(!vm.is_current, "{id} still reads as in use");
        }
    }

    /// The state that prompted this: an agent Kiwano still holds a takeover
    /// backup and a placeholder-key row for, whose config another tool has since
    /// rewritten to point at the provider directly. It reads as *not* taken
    /// over, because nothing of ours is in the file — a toggle that said
    /// otherwise claimed traffic the gateway never sees, and the provider list
    /// (rightly) showed the provider unbound.
    #[test]
    fn a_config_reverted_behind_our_back_is_not_taken_over() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com","ANTHROPIC_AUTH_TOKEN":"sk-real"}}"#,
        )
        .unwrap();
        s.insert_provider(&provider("p1", "Relay", Billing::Metered))
            .unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p1".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        assert!(crate::takeover::live_placeholder_key("claude", tmp.path(), &no_vars()).is_some());

        // Another tool puts the agent's own config back. The backup row and the
        // key row stay — Kiwano has no way to know it was not us, and nothing
        // here pretends it did.
        let reverted = r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.deepseek.com/anthropic","ANTHROPIC_AUTH_TOKEN":"sk-other"}}"#;
        std::fs::write(&settings, reverted).unwrap();
        assert!(aux.load_takeover_backup("claude").is_some());
        assert!(s
            .list_placeholder_keys()
            .unwrap()
            .iter()
            .any(|k| k.agent == "claude"));

        let v = build_settings_with_home(&s, &aux, tmp.path(), &no_vars()).unwrap();
        let claude = v.takeovers.iter().find(|t| t.agent == "claude").unwrap();
        assert!(!claude.enabled, "the file no longer routes through us");
        assert!(claude.placeholder_key.is_none());
        // …and the provider list agrees: the binding is still in the store, but
        // nothing of ours is serving it.
        let vms = build_provider_vms(&s, &aux, tmp.path(), &no_vars()).unwrap();
        assert!(vms.iter().all(|p| p.agents.is_empty() && !p.is_current));

        // Taking it over again captures what is there *now*: the stale backup
        // would otherwise be what restore writes back, over a config this
        // takeover is not replacing.
        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();
        assert_eq!(std::fs::read_to_string(&settings).unwrap(), reverted);
    }

    #[test]
    fn lost_backup_restores_the_agent_onto_its_primary_provider() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let tmp = tempfile::tempdir().unwrap();
        let settings = tmp.path().join(".claude").join("settings.json");
        std::fs::create_dir_all(settings.parent().unwrap()).unwrap();
        std::fs::write(
            &settings,
            r#"{"env":{"ANTHROPIC_BASE_URL":"https://api.anthropic.com"}}"#,
        )
        .unwrap();

        // A provider the gateway serves for claude, with a path prefix.
        let mut p = provider("p-claude", "Relay", Billing::Metered);
        p.base_url = "https://relay.example.com".into();
        p.api_path = Some("/anthropic".into());
        p.api_key = Some("sk-real".into());
        s.insert_provider(&p).unwrap();
        s.upsert_strategy("claude", StrategyType::Single, None)
            .unwrap();
        s.upsert_binding(&Binding {
            agent: "claude".into(),
            provider_id: "p-claude".into(),
            priority: 0,
            weight: 1,
            win_start: None,
            win_end: None,
            enabled: true,
        })
        .unwrap();

        set_agent_takeover(&s, &aux, "claude", true, 8317, tmp.path(), &no_vars()).unwrap();
        // Lose the backup: the escape hatch is gone, so restore has to fall
        // back to the provider instead of reporting a success it did not have.
        aux.delete_takeover_backup("claude").unwrap();
        set_agent_takeover(&s, &aux, "claude", false, 8317, tmp.path(), &no_vars()).unwrap();

        let env: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&settings).unwrap()).unwrap();
        assert_eq!(
            env["env"]["ANTHROPIC_BASE_URL"],
            "https://relay.example.com/anthropic"
        );
        assert_eq!(env["env"]["ANTHROPIC_AUTH_TOKEN"], "sk-real");
    }

    #[test]
    fn dashboard_shape_with_usage() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        s.insert_provider(&provider("p1", "Prov", Billing::Metered))
            .unwrap();
        let now = rfc3339(unix_now());
        s.record_usage(&kiwanod::store::UsageRecord {
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
            cost_off_peak: None,
        })
        .unwrap();
        // Seed the request-log rows the headline counts: the forwarded request
        // above plus a pre-forward failure (usage tables never see the latter).
        let log = |status: i64, tokens: (i64, i64)| kiwanod::store::RequestLogNew {
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
            reasoning_tokens: 0,
            usage_missing: false,
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
            cost_off_peak: None,
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

    /// The peak premium is the difference between two sums over the *same* rows,
    /// which is only meaningful if both come out of one roll-up and are converted
    /// the same way. Two currencies in the window make the conversion part of the
    /// assertion rather than an identity, and a row whose model publishes no
    /// schedule (off-peak == cost) contributes nothing to the premium.
    #[test]
    fn dashboard_prices_the_peak_premium_over_one_row_set() {
        let (_dir, s, aux) = store_and_aux_on_one_file();
        // A Hub rate table — the quote is deliberately not the real one; what is
        // under test is that it is applied, not what it says.
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 6.0 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();
        for (id, name) in [("p1", "Alpha"), ("p2", "Beta")] {
            s.insert_provider(&provider(id, name, Billing::Metered))
                .unwrap();
        }

        // p1: 10 USD at peak against 4 USD off-peak — a 6 USD premium — plus a
        // row in another currency with no schedule at all (one vendor's off-peak
        // halving applies to some models and not others).
        let mut peak = usage_row("p1");
        peak.cost = Some(10.0);
        peak.cost_currency = Some("USD".into());
        peak.cost_off_peak = Some(4.0);
        s.record_usage(&peak).unwrap();
        let mut flat = usage_row("p1");
        flat.cost = Some(3.0);
        flat.cost_currency = Some("CNY".into());
        flat.cost_off_peak = Some(3.0);
        s.record_usage(&flat).unwrap();
        // p2 is billed the same either way: no premium to report.
        let mut even = usage_row("p2");
        even.cost = Some(2.0);
        even.cost_currency = Some("USD".into());
        even.cost_off_peak = Some(2.0);
        s.record_usage(&even).unwrap();

        let d = build_dashboard(&s, &aux, "7d", None, None).unwrap();
        // (10 + 2) USD × 6 + 3 CNY; off-peak (4 + 2) × 6 + 3.
        assert!((d.cost - 75.0).abs() < 1e-6, "{}", d.cost);
        assert!((d.cost_off_peak - 39.0).abs() < 1e-6, "{}", d.cost_off_peak);
        // What the reader subtracts: 36 CNY of peak rates, in the reader's own
        // currency — the same rows priced the other way, not two populations.
        assert!((d.cost - d.cost_off_peak - 36.0).abs() < 1e-6);

        let p1 = d.by_provider.iter().find(|p| p.id == "p1").unwrap();
        let p2 = d.by_provider.iter().find(|p| p.id == "p2").unwrap();
        assert!((p1.cost - 63.0).abs() < 1e-6, "{}", p1.cost);
        assert!(
            (p1.cost_off_peak - 27.0).abs() < 1e-6,
            "{}",
            p1.cost_off_peak
        );
        assert!((p2.cost - 12.0).abs() < 1e-6);
        assert!(
            (p2.cost_off_peak - 12.0).abs() < 1e-6,
            "no schedule, no premium: {}",
            p2.cost_off_peak
        );
        // The slices add up to the headline they were converted from.
        let sum: f64 = d.by_provider.iter().map(|p| p.cost).sum();
        assert!((sum - d.cost).abs() < 1e-6, "{sum} vs {}", d.cost);
        // …and the per-agent row is the same pair for that agent's rows.
        assert!((d.by_agent[0].cost - 75.0).abs() < 1e-6);
        assert!((d.by_agent[0].cost_off_peak - 39.0).abs() < 1e-6);
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

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), false).unwrap();
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
        let empty = export_request_logs_csv(
            &s,
            path,
            RequestLogFilter {
                agent: Some("codex"),
                ..Default::default()
            },
            false,
        )
        .unwrap();
        assert_eq!(empty.rows_written, 0);
        assert_eq!(std::fs::read_to_string(path).unwrap().lines().count(), 1);
    }

    /// The export's body flag is the only place a body can be withheld, so it
    /// has to actually withhold one — and actually produce one.
    #[test]
    fn export_includes_bodies_only_when_asked() {
        let s = store();
        // One row, with markers that need no CSV quoting, so "is it in the
        // file" is a plain substring test.
        s.insert_request_log(&kiwanod::store::RequestLogNew {
            ts: rfc3339(unix_now() - 90),
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
            input_tokens: 10,
            output_tokens: 2,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            usage_missing: false,
            latency_ms: Some(100),
            first_token_ms: None,
            request_headers: None,
            response_headers: None,
            request_body: Some("request-body-marker".into()),
            response_body: Some("response-body-marker".into()),
            request_size: 19,
            response_size: 20,
            truncated: false,
            cost: None,
            cost_currency: None,
            cost_off_peak: None,
        })
        .unwrap();

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("logs.csv");
        let path = path.to_str().unwrap();

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), false).unwrap();
        assert_eq!(out.rows_written, 1);
        let without = std::fs::read_to_string(path).unwrap();
        assert!(!without.contains("request-body-marker"), "{without}");
        assert!(!without.contains("response-body-marker"), "{without}");
        assert!(!without.contains("request_body"), "no body column either");

        let out = export_request_logs_csv(&s, path, RequestLogFilter::default(), true).unwrap();
        assert_eq!(out.rows_written, 1);
        let with = std::fs::read_to_string(path).unwrap();
        assert!(with.contains("request-body-marker"), "{with}");
        assert!(with.contains("response-body-marker"), "{with}");
        assert!(
            with.contains("request_body,response_body"),
            "header matches"
        );
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
        s.record_usage(&kiwanod::store::UsageRecord {
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
            cost_off_peak: None,
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
            s.record_usage(&kiwanod::store::UsageRecord {
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
                cost_off_peak: None,
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
        let home = live_home(&["claude"]);
        // (a1 current, b1 current, b1 fallback) — the quota-over case is where
        // the two badge claims split: the backup is first in line, not in use,
        // so it reads as fallback and neither provider reads as current.
        let badges = |s: &Store| -> (bool, bool, Vec<String>) {
            let vms = build_provider_vms(s, &aux, home.path(), &no_vars()).unwrap();
            let p = |id: &str| vms.iter().find(|x| x.id == id).unwrap();
            (
                p("a1").is_current,
                p("b1").is_current,
                p("b1").fallback_agents.clone(),
            )
        };

        // single: only the head serves
        assert_eq!(badges(&s), (true, false, vec![]));

        // roundrobin: every candidate takes rotation turns
        set_agent_strategy(&s, "claude", "roundrobin", None).unwrap();
        assert_eq!(badges(&s), (true, true, vec![]));

        // timewindow: a window containing now moves the badge off the head.
        // Built from the same clock the view model reads — `Aux` here defaults
        // to UTC — so the case is stated without depending on the host's zone.
        let now = local_minutes_now(tz_offset(&aux));
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
        assert_eq!(badges(&s), (false, true, vec![]));

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
        assert_eq!(badges(&s), (true, false, vec![]));

        // quota: under the threshold the head serves; over it the head stops
        // serving and the first backup is badged fallback, not in use (windows
        // ignored).
        set_agent_strategy(
            &s,
            "claude",
            "quota",
            Some(r#"{"limit":5,"unit":"requests"}"#),
        )
        .unwrap();
        assert_eq!(badges(&s), (true, false, vec![]));
        for _ in 0..5 {
            s.record_usage(&kiwanod::store::UsageRecord {
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
                cost_off_peak: None,
            })
            .unwrap();
        }
        // Over: neither reads as current (the gateway may still serve the
        // primary when every backup is down, which is its runtime call), and
        // b1 — the configured first backup — reads as fallback.
        assert_eq!(badges(&s), (false, false, vec!["claude".to_string()]));
    }
    /// What a latency test's three outcomes become on the row. The middle one is
    /// the whole point of the `error` column: a refusal is the vendor answering,
    /// which reachability alone cannot tell apart from silence.
    #[test]
    fn a_test_verdict_separates_a_refusal_from_silence() {
        use crate::sidecar::PromptProbe;

        // Answered, accepted.
        let ok = Ok(PromptProbe {
            latency_ms: 218,
            status: 200,
            error: None,
        });
        assert_eq!(test_verdict(&ok), ("reachable", 218, None));

        // Answered, refused: reachable, and the reason travels with it.
        let refused = Ok(PromptProbe {
            latency_ms: 60,
            status: 401,
            error: Some("invalid API key".into()),
        });
        let (status, ms, error) = test_verdict(&refused);
        assert_eq!(
            status, "reachable",
            "the vendor answered — that is the fact"
        );
        assert_eq!(ms, 60);
        assert_eq!(error.as_deref(), Some("invalid API key"));

        // Nobody answered.
        let dead = Err("connection failed: dns error".to_string());
        let (status, ms, error) = test_verdict(&dead);
        assert_eq!(status, "down");
        assert_eq!(ms, 0);
        assert_eq!(error.as_deref(), Some("connection failed: dns error"));
    }

    /// The Status column reads two sources and must not confuse them: a
    /// provider's own round trips when it has any, the prober's unsigned GET
    /// when it does not, and neither for a parked one.
    #[test]
    fn the_status_column_shows_a_providers_own_latency_before_a_probe() {
        // The store and the aux have to share one file here: the traffic average
        // comes off the aux's connection, which in production is the same
        // database the gateway writes usage rows into.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let s = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();

        let mut parked = provider("parked", "Parked", Billing::Metered);
        parked.enabled = false;
        for p in [
            provider("busy", "Busy", Billing::Metered),
            provider("idle", "Idle", Billing::Metered),
            provider("dead", "Dead", Billing::Metered),
            provider("fresh", "Fresh", Billing::Metered),
            provider("tested", "Tested", Billing::Metered),
            provider("refused", "Refused", Billing::Metered),
            parked,
        ] {
            s.insert_provider(&p).unwrap();
        }

        // The prober has an opinion about all of them — including `busy`, whose
        // verdict is stale (it had no traffic when the probe ran). That stale row
        // is exactly what the order has to get right.
        s.upsert_provider_health("busy", "reachable", 500, "probe", None)
            .unwrap();
        s.upsert_provider_health("idle", "reachable", 12, "probe", None)
            .unwrap();
        s.upsert_provider_health("dead", "down", 0, "probe", None)
            .unwrap();
        s.upsert_provider_health("parked", "reachable", 3, "probe", None)
            .unwrap();
        // The Apps screen's own test: a real prompt with the key. One answered,
        // one refused — and a refusal is reachability with a reason, not silence.
        s.upsert_provider_health("tested", "reachable", 218, "test", None)
            .unwrap();
        s.upsert_provider_health("refused", "reachable", 60, "test", Some("invalid API key"))
            .unwrap();

        // Two requests of its own inside the window: 200ms on average.
        for ms in [180, 220] {
            let mut row = usage_row("busy");
            row.latency_ms = Some(ms);
            s.record_usage(&row).unwrap();
        }

        let vms = build_provider_vms(&s, &aux, std::path::Path::new("/tmp"), &no_vars()).unwrap();
        let health = |id: &str| &vms.iter().find(|v| v.id == id).unwrap().health;

        let busy = health("busy");
        assert_eq!(
            busy.source.as_deref(),
            Some("traffic"),
            "its own round trips outrank a verdict that predates them"
        );
        assert_eq!(
            busy.latency_ms,
            Some(200),
            "…and it is their average, not the probe's 500"
        );

        let idle = health("idle");
        assert_eq!(idle.source.as_deref(), Some("probe"));
        assert_eq!(idle.latency_ms, Some(12));
        assert!(
            idle.checked_at.is_some(),
            "a probe is dated; a traffic average is a window"
        );

        let dead = health("dead");
        assert_eq!(dead.state, "error", "an endpoint that did not answer");
        assert_eq!(dead.latency_ms, None, "no answer is not a latency");
        assert_eq!(dead.source.as_deref(), Some("probe"));

        let parked_health = health("parked");
        assert_eq!(parked_health.note.as_deref(), Some("Disabled"));
        assert_eq!(
            parked_health.latency_ms, None,
            "out of every route outranks both"
        );

        // A verdict the app took itself. The number and its provenance both
        // travel: the cell says "you tested it", not "the gateway asked".
        let tested = health("tested");
        assert_eq!(tested.source.as_deref(), Some("test"));
        assert_eq!(tested.latency_ms, Some(218));
        assert_eq!(tested.state, "ok");
        assert_eq!(tested.error, None);

        // Answered and refused: reachable, with the vendor's reason. Read as an
        // error state so the cell cannot pass it off as a working provider.
        let refused = health("refused");
        assert_eq!(refused.source.as_deref(), Some("test"));
        assert_eq!(refused.state, "error");
        assert_eq!(refused.error.as_deref(), Some("invalid API key"));
        assert_eq!(refused.latency_ms, Some(60), "it did answer, in 60ms");

        let fresh = health("fresh");
        assert_eq!(
            fresh.source, None,
            "never probed and never used: nothing to say"
        );
        assert_eq!(fresh.latency_ms, None);
        assert_eq!(fresh.state, "idle");
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
            cost_off_peak: None,
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
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // 100/100 → threshold hit
        for _ in 0..60 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].provider_id, "kimi-1");
        assert_eq!(alerts[0].unit, "requests");

        // same-period dedup: the second check returns nothing
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // toggle off → silent
        let patch = serde_json::json!({ "cost_alert": false });
        update_settings(&s, &aux, &patch, &no_vars()).unwrap();
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());
    }

    /// A plan ceiling is the other way a provider goes out of service, and it
    /// used to do so silently: the gateway took it out of the routes, the UI had
    /// the branch and the strings for the notice, and nothing ever produced one.
    #[test]
    fn a_plan_window_ceiling_notifies_once_per_window() {
        use kiwanod::plan_quota::{cache_write, PlanQuotaReport, PlanTierVm};
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("glm-1", "GLM", Billing::Subscription);
        p.plan_limits = Some(serde_json::json!({ "five_hour": 90.0 }).to_string());
        s.insert_provider(&p).unwrap();

        let report = |util: f64, resets: &str| PlanQuotaReport {
            provider_id: "glm-1".into(),
            template: "zhipu".into(),
            success: true,
            error: None,
            note: None,
            tiers: vec![
                PlanTierVm {
                    name: "five_hour".into(),
                    utilization: util,
                    resets_at: Some(resets.into()),
                    used: None,
                    limit: None,
                    unit: None,
                },
                // Present and quiet: only the window that is over should speak.
                PlanTierVm {
                    name: "weekly_limit".into(),
                    utilization: 10.0,
                    resets_at: None,
                    used: None,
                    limit: None,
                    unit: None,
                },
            ],
            queried_at: 0,
            cached: false,
        };

        // Under the ceiling: nothing to say.
        cache_write(&s, "glm-1", &report(80.0, "2026-09-14T10:00:00Z"));
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // Over it: one notice, carrying the window's own percentage against the
        // ceiling the user set.
        cache_write(&s, "glm-1", &report(95.0, "2026-09-14T10:00:00Z"));
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].unit, "plan_pct");
        assert_eq!(alerts[0].used, 95.0);
        assert_eq!(alerts[0].limit, 90.0);

        // Same window, same news: silent, the way the money cap is.
        assert!(check_usage_alerts(&s, &aux, true).unwrap().is_empty());

        // The window rolls over and is over its ceiling again. The reset time is
        // the identity, so this is what re-arms the dedup rather than letting one
        // hit go quiet for good.
        cache_write(&s, "glm-1", &report(97.0, "2026-09-14T15:00:00Z"));
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 1);
        assert_eq!(alerts[0].used, 97.0);
    }

    /// The dedup key is shared with the desktop notification, so a read-only
    /// caller must be able to ask without consuming the alert the app is about
    /// to raise. A cron poll that silenced the user's notification would be a
    /// bug nobody would connect to the poll.
    #[test]
    fn cost_alert_can_be_read_without_consuming_the_dedup() {
        let s = store();
        let aux = Aux::open_in_memory().unwrap();
        let mut p = provider("kimi-1", "Kimi", Billing::Subscription);
        p.period_limit = Some(10.0);
        p.limit_unit = Some("requests".into());
        p.reset_period = Some("monthly".into());
        s.insert_provider(&p).unwrap();
        for _ in 0..10 {
            s.record_usage(&usage_row("kimi-1")).unwrap();
        }

        // Read-only: the alert is reported every time, and the dedup is
        // untouched — which is what `--mark-notified` is for.
        let first = check_usage_alerts(&s, &aux, false).unwrap();
        let second = check_usage_alerts(&s, &aux, false).unwrap();
        assert_eq!(first.len(), 1);
        assert_eq!(second.len(), 1, "a read must not consume the alert");

        // The app's marking call still works and still dedups.
        assert_eq!(check_usage_alerts(&s, &aux, true).unwrap().len(), 1);
        assert!(
            check_usage_alerts(&s, &aux, true).unwrap().is_empty(),
            "marking is what suppresses the repeat"
        );
        assert!(
            check_usage_alerts(&s, &aux, false).unwrap().is_empty(),
            "…for read-only callers too, once it has been marked"
        );
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
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert_eq!(alerts.len(), 2);
        assert_eq!(alerts[0].provider_id, "ds-1");
        assert_eq!(alerts[1].provider_id, "glm-1");
        assert_eq!(alerts[1].unit, "CNY");
        assert!((alerts[1].used - 60.0).abs() < 1e-6);
    }

    /// A store and an aux on **one file**, which is what production has: the
    /// GUI opens both on `kiwano.db`, so a rate cached by a sync is the rate the
    /// gateway reads back when it measures a limit.
    fn store_and_aux_on_one_file() -> (tempfile::TempDir, Store, Aux) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("kiwano.db");
        let store = Store::open(&path).unwrap();
        let aux = Aux::open(&path).unwrap();
        (dir, store, aux)
    }

    #[test]
    fn currency_limits_convert_with_the_hub_rate_table() {
        let (_dir, s, aux) = store_and_aux_on_one_file();
        // A Hub price table whose CNY rate is nothing like the real one (the Hub
        // quotes several CNY per USD).
        let hub = serde_json::json!({
            "version": 99,
            "exchange_rates": { "USD": 1.0, "CNY": 6.0 },
            "models": []
        })
        .to_string();
        aux.save_hub_models_cache(99, &hub, &"a".repeat(64), "2026-01-01T00:00:00Z")
            .unwrap();

        let mut cny = provider("glm-1", "GLM", Billing::Subscription);
        cny.period_limit = Some(50.0);
        cny.limit_unit = Some("CNY".into());
        s.insert_provider(&cny).unwrap();

        // Dollars, against a limit denominated in yuan — which happens whenever
        // a provider serves a model it does not price itself and the general row
        // is in someone else's currency.
        let mut row = usage_row("glm-1");
        row.cost = Some(10.0);
        row.cost_currency = Some("USD".into());
        s.record_usage(&row).unwrap();

        // 10 USD is 60 CNY at the cached rate, over the 50 CNY limit, so the
        // alert fires. Adding the buckets raw — the old behaviour — read 10 and
        // stayed silent, and so did converting at any rate at or below 5, which
        // is why the rate is quoted high enough to decide the outcome.
        let alerts = check_usage_alerts(&s, &aux, true).unwrap();
        assert!(
            alerts.iter().any(|a| a.provider_id == "glm-1"),
            "10 USD at 6.0 CNY/USD is 60 CNY, over the 50 CNY limit"
        );
    }
}
