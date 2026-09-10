//! Provider token-plan quota queries (ported from cc-switch's coding_plan
//! service).
//!
//! A Subscription provider may carry a `plan_query` JSON blob —
//! `{"template": "<id>", "fields": {...}}` — selecting one of the known
//! quota endpoints (Kimi / Zhipu / Zhipu team / MiniMax / ZenMux /
//! OpenCode Go / Volcengine). `get_plan_quota` executes the template's
//! HTTP call with the provider's API key (plus per-template credential
//! fields), parses the usage windows into utilization tiers, and caches
//! the report in the Aux KV for 5 minutes.
//!
//! Error channels (same contract as cc-switch): transient transport
//! failures (network / read interruption) return `Err` so the frontend
//! can retry and keep the last good value; deterministic failures (bad
//! key, business error, unknown shape) return a report with
//! `success: false` carrying the user-facing reason.

use crate::vm::Aux;
use kiwano_gateway::store::Store;
use std::collections::HashMap;

/// Aux KV cache TTL for one provider's plan quota report.
const CACHE_TTL_MS: i64 = 5 * 60 * 1000;

// ── VM types ──

/// One usage window (five_hour / weekly_limit / monthly) of a plan.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PlanTierVm {
    /// five_hour | weekly_limit | monthly
    pub name: String,
    /// Percent of the window already used. Passed through unclamped
    /// (negative / >100 values are upstream's honest numbers).
    pub utilization: f64,
    pub resets_at: Option<String>,
    /// Absolute used / window cap, when the endpoint reports amounts
    /// (ZenMux USD, Volcengine AFP). Percentage-only endpoints leave
    /// both None.
    pub used: Option<f64>,
    pub limit: Option<f64>,
    pub unit: Option<String>,
}

/// Result of one plan quota query (possibly served from cache).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct PlanQuotaReport {
    pub provider_id: String,
    pub template: String,
    /// false = deterministic failure; `error` carries the user-facing reason.
    pub success: bool,
    pub error: Option<String>,
    /// Plan metadata from the endpoint (Zhipu level, ZenMux tier, Volcengine plan).
    pub note: Option<String>,
    pub tiers: Vec<PlanTierVm>,
    /// Epoch millis of the original query.
    pub queried_at: i64,
    /// true when served from the 5-minute cache.
    pub cached: bool,
}

/// Outcome of a template execution, before the provider_id/timestamp
/// wrapper is attached by the caller.
pub enum QuotaOutcome {
    Ok {
        tiers: Vec<PlanTierVm>,
        note: Option<String>,
    },
    /// Deterministic failure; the message is shown as-is in the UI.
    Failed(String),
}

// ── Small shared helpers ──

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// Parse a JSON value as f64, accepting numbers and numeric strings.
fn parse_f64(value: &serde_json::Value) -> Option<f64> {
    value
        .as_f64()
        .or_else(|| value.as_str().and_then(|s| s.parse().ok()))
}

/// Extract a reset time accepting ISO strings and epoch numbers. Numbers are
/// auto-detected as seconds (<1e12) or milliseconds; values <= 0 (e.g.
/// Volcengine's "-1 = no active window") mean "no reset".
fn extract_reset_time(value: &serde_json::Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        return Some(s.to_string());
    }
    if let Some(n) = value.as_i64() {
        if n <= 0 {
            return None;
        }
        let ms = if n < 1_000_000_000_000 { n * 1000 } else { n };
        return millis_to_iso8601(ms);
    }
    None
}

fn millis_to_iso8601(ms: i64) -> Option<String> {
    let secs = ms.div_euclid(1000);
    let nsecs = (ms.rem_euclid(1000) * 1_000_000) as u32;
    chrono::DateTime::from_timestamp(secs, nsecs).map(|dt| dt.to_rfc3339())
}

fn field_str<'a>(fields: &'a HashMap<String, serde_json::Value>, key: &str) -> &'a str {
    fields
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
}

/// Fold a received HTTP response: outer `Err` = transient (network), inner
/// `Err` = deterministic failure with a user-facing message.
fn fold_response(
    resp: reqwest::blocking::Response,
) -> Result<Result<serde_json::Value, String>, String> {
    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return Ok(Err(format!(
            "Auth failed (HTTP {status}): the API key is invalid or expired"
        )));
    }
    if !status.is_success() {
        let body = resp.text().unwrap_or_default();
        return Ok(Err(format!("Endpoint error (HTTP {status}): {body}")));
    }
    // Read the whole body before parsing: a read failure is transient (outer
    // Err), a parse failure of a complete body is deterministic.
    let raw = resp.text().map_err(|e| format!("Network error: {e}"))?;
    match serde_json::from_str(&raw) {
        Ok(v) => Ok(Ok(v)),
        Err(e) => Ok(Err(format!("Failed to parse the response: {e}"))),
    }
}

/// GET a JSON endpoint and fold transport errors.
fn fetch_json(
    req: reqwest::blocking::RequestBuilder,
) -> Result<Result<serde_json::Value, String>, String> {
    let resp = req.send().map_err(|e| format!("Network error: {e}"))?;
    fold_response(resp)
}

fn blocking_client() -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .map_err(|e| e.to_string())
}

// ── Kimi For Coding ──

fn query_kimi(client: &reqwest::blocking::Client, api_key: &str) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let req = client
        .get("https://api.kimi.com/coding/v1/usages")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let body = match fetch_json(req)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(QuotaOutcome::Ok {
        tiers: parse_kimi_tiers(&body),
        note: None,
    })
}

/// Kimi: `limits[].detail` is the rolling 5-hour window; `usage` is the
/// weekly limit. Both are limit/remaining pairs → invert to utilization.
fn parse_kimi_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let push = |tiers: &mut Vec<PlanTierVm>, name: &str, obj: &serde_json::Value| {
        let limit = obj.get("limit").and_then(parse_f64).unwrap_or(1.0);
        let remaining = obj.get("remaining").and_then(parse_f64).unwrap_or(0.0);
        let resets_at = obj.get("resetTime").and_then(extract_reset_time);
        let used = (limit - remaining).max(0.0);
        let utilization = if limit > 0.0 {
            used / limit * 100.0
        } else {
            0.0
        };
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization,
            resets_at,
            used: None,
            limit: None,
            unit: None,
        });
    };
    if let Some(limits) = body.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            if let Some(detail) = item.get("detail") {
                push(&mut tiers, "five_hour", detail);
            }
        }
    }
    if let Some(usage) = body.get("usage") {
        push(&mut tiers, "weekly_limit", usage);
    }
    tiers
}

// ── Zhipu GLM (personal + team) ──

/// Zhipu `TOKENS_LIMIT` entries are classified by the explicit `unit` field.
enum ZhipuWindow {
    FiveHour,
    Weekly,
}

/// `unit: 3, number: 5` → 5-hour rolling window; `unit: 6` (number 7 or 1,
/// both observed) → weekly window. Unknown/missing units return None and the
/// caller falls back to the reset-time heuristic.
fn classify_zhipu_window(item: &serde_json::Value) -> Option<ZhipuWindow> {
    match item.get("unit").and_then(|v| v.as_i64()) {
        Some(3) => Some(ZhipuWindow::FiveHour),
        Some(6) => Some(ZhipuWindow::Weekly),
        _ => None,
    }
}

/// Parse `data.limits[]` into the two known windows. Explicit `unit`
/// classification wins; entries without a recognized unit fall back to
/// reset-time order (earliest first fills five_hour, then weekly). The
/// reset-time fallback must not override explicit classification: at the end
/// of a weekly cycle the weekly window can reset sooner than the 5-hour one.
fn parse_zhipu_token_tiers(data: &serde_json::Value) -> Vec<PlanTierVm> {
    type Entry = (Option<i64>, f64, Option<String>);
    let mut five_hour: Option<Entry> = None;
    let mut weekly: Option<Entry> = None;
    let mut unclassified: Vec<Entry> = Vec::new();

    if let Some(limits) = data.get("limits").and_then(|v| v.as_array()) {
        for item in limits {
            let limit_type = item.get("type").and_then(|v| v.as_str()).unwrap_or("");
            // Case-insensitive: upstream casing drift must not break detection.
            if !(limit_type.eq_ignore_ascii_case("TOKENS_LIMIT")
                || limit_type.eq_ignore_ascii_case("CREDIT_LIMIT"))
            {
                continue;
            }
            let percentage = item
                .get("percentage")
                .and_then(|v| v.as_f64())
                .unwrap_or(0.0);
            let reset_ms = item.get("nextResetTime").and_then(|v| v.as_i64());
            let reset_iso = reset_ms.and_then(millis_to_iso8601);
            let entry = (reset_ms, percentage, reset_iso);
            match classify_zhipu_window(item) {
                Some(ZhipuWindow::FiveHour) if five_hour.is_none() => five_hour = Some(entry),
                Some(ZhipuWindow::Weekly) if weekly.is_none() => weekly = Some(entry),
                _ => unclassified.push(entry),
            }
        }
    }

    unclassified.sort_by_key(|(reset, _, _)| (reset.is_some(), reset.unwrap_or(i64::MIN)));
    for entry in unclassified {
        if five_hour.is_none() {
            five_hour = Some(entry);
        } else if weekly.is_none() {
            weekly = Some(entry);
        }
        // Zhipu currently sends at most two TOKENS_LIMIT rows; extras are dropped.
    }

    let mut tiers = Vec::new();
    for (name, slot) in [("five_hour", five_hour), ("weekly_limit", weekly)] {
        if let Some((_, percentage, resets_at)) = slot {
            tiers.push(PlanTierVm {
                name: name.to_string(),
                utilization: percentage,
                resets_at,
                used: None,
                limit: None,
                unit: None,
            });
        }
    }
    tiers
}

/// Resolve the Zhipu quota host from the configured base_url: bigmodel.cn
/// (cn) and api.z.ai (international) share the same quota path and shape.
fn zhipu_quota_base(base_url: &str) -> &'static str {
    if base_url.to_lowercase().contains("bigmodel.cn") {
        "https://open.bigmodel.cn"
    } else {
        "https://api.z.ai"
    }
}

/// Parse the Zhipu quota body (personal and team share the same shape).
/// No network IO here, so every failure is deterministic.
fn zhipu_outcome(body: &serde_json::Value) -> QuotaOutcome {
    if body.get("success").and_then(|v| v.as_bool()) == Some(false) {
        let msg = body
            .get("msg")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return QuotaOutcome::Failed(format!("Endpoint error: {msg}"));
    }
    let Some(data) = body.get("data") else {
        return QuotaOutcome::Failed("Response is missing the data field".to_string());
    };
    let note = data
        .get("level")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    QuotaOutcome::Ok {
        tiers: parse_zhipu_token_tiers(data),
        note,
    }
}

fn query_zhipu(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let url = format!(
        "{}/api/monitor/usage/quota/limit",
        zhipu_quota_base(base_url)
    );
    // Zhipu does NOT use a Bearer prefix.
    let req = client
        .get(&url)
        .header("Authorization", api_key)
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");
    let body = match fetch_json(req)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(zhipu_outcome(&body))
}

/// Zhipu team plan: personal-plan path + `?type=2` plus the
/// bigmodel-organization / bigmodel-project headers (all three credentials
/// required). Team plans only exist on the cn site.
fn query_zhipu_team(
    client: &reqwest::blocking::Client,
    api_key: &str,
    organization_id: &str,
    project_id: &str,
) -> Result<QuotaOutcome, String> {
    let url = "https://open.bigmodel.cn/api/monitor/usage/quota/limit?type=2";
    let req = client
        .get(url)
        .header("Authorization", api_key)
        .header("bigmodel-organization", organization_id)
        .header("bigmodel-project", project_id)
        .header("Content-Type", "application/json")
        .header("Accept-Language", "en-US,en");
    let body = match fetch_json(req)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    Ok(zhipu_outcome(&body))
}

// ── MiniMax ──

fn query_minimax(
    client: &reqwest::blocking::Client,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let domain = if base_url.to_lowercase().contains("minimaxi.com") {
        "api.minimaxi.com"
    } else {
        "api.minimax.io"
    };
    let url = format!("https://{domain}/v1/api/openplatform/coding_plan/remains");
    let req = client
        .get(&url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Content-Type", "application/json");
    let body = match fetch_json(req)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    // Business-level error envelope.
    if let Some(base_resp) = body.get("base_resp") {
        let code = base_resp
            .get("status_code")
            .and_then(|v| v.as_i64())
            .unwrap_or(-1);
        if code != 0 {
            let msg = base_resp
                .get("status_msg")
                .and_then(|v| v.as_str())
                .unwrap_or("Unknown error");
            return Ok(QuotaOutcome::Failed(format!(
                "Endpoint error (code {code}): {msg}"
            )));
        }
    }
    Ok(QuotaOutcome::Ok {
        tiers: parse_minimax_tiers(&body),
        note: None,
    })
}

/// The `coding_plan/remains` endpoint reports remaining percent for the
/// `general` (coding plan) model bucket; video and friends are skipped. The
/// 5h bucket is always present; the weekly bucket only exists when
/// `current_weekly_status == 1` (status 3 = plan has no weekly cap and the
/// percent is pinned at 100 — not a real bucket).
fn parse_minimax_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let Some(model_remains) = body.get("model_remains").and_then(|v| v.as_array()) else {
        return tiers;
    };
    let Some(item) = model_remains.iter().find(|item| {
        item.get("model_name")
            .and_then(|v| v.as_str())
            .map(|s| s == "general")
            .unwrap_or(false)
    }) else {
        return tiers;
    };

    if let Some(remain_pct) = item
        .get("current_interval_remaining_percent")
        .and_then(|v| v.as_f64())
    {
        tiers.push(PlanTierVm {
            name: "five_hour".to_string(),
            utilization: 100.0 - remain_pct,
            resets_at: item
                .get("end_time")
                .and_then(|v| v.as_i64())
                .and_then(millis_to_iso8601),
            used: None,
            limit: None,
            unit: None,
        });
    }

    if item.get("current_weekly_status").and_then(|v| v.as_i64()) == Some(1) {
        if let Some(remain_pct) = item
            .get("current_weekly_remaining_percent")
            .and_then(|v| v.as_f64())
        {
            tiers.push(PlanTierVm {
                name: "weekly_limit".to_string(),
                utilization: 100.0 - remain_pct,
                resets_at: item
                    .get("weekly_end_time")
                    .and_then(|v| v.as_i64())
                    .and_then(millis_to_iso8601),
                used: None,
                limit: None,
                unit: None,
            });
        }
    }
    tiers
}

// ── ZenMux ──

/// ZenMux's quota URL is user-supplied (the endpoint is not derivable from
/// the inference base_url); the fields carry it verbatim.
fn query_zenmux(
    client: &reqwest::blocking::Client,
    quota_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    let req = client
        .get(quota_url)
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let body = match fetch_json(req)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    if body.get("success").and_then(|v| v.as_bool()) != Some(true) {
        let msg = body
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("Unknown error");
        return Ok(QuotaOutcome::Failed(format!("Endpoint error: {msg}")));
    }
    let Some(data) = body.get("data") else {
        return Ok(QuotaOutcome::Failed(
            "Response is missing the data field".to_string(),
        ));
    };

    let mut tiers = Vec::new();
    for (key, name) in [
        ("quota_5_hour", "five_hour"),
        ("quota_7_day", "weekly_limit"),
    ] {
        let Some(q) = data.get(key) else { continue };
        let usage_pct = q.get("usage_percentage").and_then(parse_f64).unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization: usage_pct * 100.0,
            resets_at: q
                .get("resets_at")
                .and_then(|v| v.as_str())
                .map(String::from),
            used: q.get("used_value_usd").and_then(parse_f64),
            limit: q.get("max_value_usd").and_then(parse_f64),
            unit: Some("USD".to_string()),
        });
    }

    // Plan tier + account status as the note.
    let plan_tier = data
        .get("plan")
        .and_then(|p| p.get("tier"))
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let account_status = data
        .get("account_status")
        .and_then(|v| v.as_str())
        .unwrap_or("");
    let note = (!plan_tier.is_empty()).then(|| format!("{plan_tier} ({account_status})"));

    Ok(QuotaOutcome::Ok { tiers, note })
}

// ── OpenCode Go ──

/// Response shape (undocumented first-party route):
/// `{"usage":{"rolling"|"weekly"|"monthly":{"status","percent","resetsAt"}}}`.
/// Each window is parsed defensively — the endpoint changed shape once on
/// launch day, so malformed windows are skipped rather than failing all.
/// A percent of 0 makes the upstream `resetsAt` a placeholder (now+window),
/// which is dropped.
fn parse_opencode_go_tiers(body: &serde_json::Value) -> Vec<PlanTierVm> {
    const WINDOWS: [(&str, &str); 3] = [
        ("rolling", "five_hour"),
        ("weekly", "weekly_limit"),
        ("monthly", "monthly"),
    ];
    let Some(usage) = body.get("usage") else {
        return Vec::new();
    };
    let mut tiers = Vec::new();
    for (key, tier_name) in WINDOWS {
        let Some(window) = usage.get(key) else {
            continue;
        };
        let Some(percent) = window.get("percent").and_then(parse_f64) else {
            continue;
        };
        let resets_at = if percent > 0.0 {
            window.get("resetsAt").and_then(extract_reset_time)
        } else {
            None
        };
        tiers.push(PlanTierVm {
            name: tier_name.to_string(),
            utilization: percent,
            resets_at,
            used: None,
            limit: None,
            unit: None,
        });
    }
    tiers
}

fn query_opencode_go(
    client: &reqwest::blocking::Client,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    if api_key.trim().is_empty() {
        return Ok(QuotaOutcome::Failed("API key is empty".to_string()));
    }
    // The usage endpoint only accepts `Authorization: Bearer` — the inverse
    // of the inference side, which wants x-api-key.
    let req = client
        .get("https://opencode.ai/zen/go/v1/usage")
        .header("Authorization", format!("Bearer {api_key}"))
        .header("Accept", "application/json");
    let resp = match req.send() {
        Ok(r) => r,
        Err(e) => return Err(format!("Network error: {e}")),
    };
    let status = resp.status();
    // 403 EntitlementError: the key is valid (Zen and Go share the workspace
    // key) but the workspace has no Go subscription — a distinct message.
    if status == reqwest::StatusCode::FORBIDDEN {
        return Ok(QuotaOutcome::Failed(
            "API key is valid but not subscribed to OpenCode Go (HTTP 403)".to_string(),
        ));
    }
    let body = match fold_response(resp)? {
        Ok(b) => b,
        Err(msg) => return Ok(QuotaOutcome::Failed(msg)),
    };
    let tiers = parse_opencode_go_tiers(&body);
    // No window parsed = shape unrecognized (the endpoint changed shape once
    // on launch day) — fail loudly instead of rendering an empty card.
    if tiers.is_empty() {
        return Ok(QuotaOutcome::Failed(
            "Unrecognized response shape".to_string(),
        ));
    }
    Ok(QuotaOutcome::Ok { tiers, note: None })
}

// ── Volcengine Agent Plan / Coding Plan ──
//
// Unlike the Bearer data-plane endpoints above, Volcengine usage lives on the
// control-plane OpenAPI gateway (`open.volcengineapi.com`) behind Signature
// V4 (AK/SK) — the inference API key is rejected with 400 InvalidAuthorization.
// Users configure the account AccessKey ID + Secret in the plan-query fields.
// Detection is automatic: GetAFPUsage (Agent Plan, absolute quotas) first,
// then GetCodingPlanUsage (Coding Plan, percentages).

const VOLCENGINE_OPENAPI_HOST: &str = "open.volcengineapi.com";
const VOLCENGINE_API_VERSION: &str = "2024-01-01";
const VOLCENGINE_DEFAULT_REGION: &str = "cn-beijing";
const VOLCENGINE_SERVICE: &str = "ark";
const VOLCENGINE_CONTENT_TYPE: &str = "application/json; charset=utf-8";
const VOLCENGINE_SIGNED_HEADERS: &str = "host;x-date;x-content-sha256;content-type";
const VOLCENGINE_AKSK_HINT: &str =
    "Check that the AccessKey ID / Secret are correct and the account has Ark usage query (OpenAPI) permission";

enum VolcCall {
    Body(serde_json::Value),
    Auth(String),
    Soft(String),
    Transient(String),
}

/// Extract the control-plane region from the data-plane base_url
/// (`ark.cn-beijing.volces.com` → `cn-beijing`), falling back to cn-beijing.
fn volcengine_region(base_url: &str) -> String {
    let host = base_url
        .split_once("://")
        .map(|(_, rest)| rest)
        .unwrap_or(base_url)
        .split('/')
        .next()
        .unwrap_or("");
    host.split('.')
        .find(|p| p.starts_with("cn-") || p.starts_with("ap-"))
        .map(|p| p.to_string())
        .unwrap_or_else(|| VOLCENGINE_DEFAULT_REGION.to_string())
}

/// Auth-class OpenAPI error codes stop the two-plan probe immediately (both
/// plans share the same AK/SK credential).
fn volcengine_is_auth_error_code(code: &str) -> bool {
    let c = code.to_lowercase();
    c.contains("auth")
        || c.contains("signature")
        || c.contains("accessdenied")
        || c.contains("denied")
        || c.contains("unauthorized")
        || c.contains("forbidden")
        || c.contains("credential")
        || c.contains("token")
}

/// Pull `ResponseMetadata.Error` (or top-level `Error`) out of an OpenAPI body.
fn volcengine_response_error(body: &serde_json::Value) -> Option<(String, String)> {
    let err = body
        .get("ResponseMetadata")
        .and_then(|m| m.get("Error"))
        .or_else(|| body.get("Error"))?;
    let code = err.get("Code").and_then(|v| v.as_str()).unwrap_or("");
    let msg = err.get("Message").and_then(|v| v.as_str()).unwrap_or("");
    if code.is_empty() && msg.is_empty() {
        None
    } else {
        Some((code.to_string(), msg.to_string()))
    }
}

// ── Volcengine Signature V4 ──
//
// A Volcengine variant of AWS SigV4 with two fatal differences from the
// standard algorithm (per volc-openapi-demos/signature/java/Sign.java):
//   1. canonical headers / SignedHeaders use the FIXED order
//      `host;x-date;x-content-sha256;content-type` (NOT alphabetical);
//   2. the algorithm string is `HMAC-SHA256` (no `AWS4` prefix), the
//      credential scope ends in `request` (not `aws4_request`), and the key
//      derivation is `kDate = HMAC(SK, date)` with no prefix either.
// The canonical query is still alphabetically ordered (as in standard SigV4);
// service = `ark`, POST, empty body.

fn volc_sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    format!("{:x}", Sha256::digest(data))
}

/// HMAC-SHA256 (RFC 2104), implemented directly on sha2 to avoid pulling in
/// the hmac crate for this single use.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    const BLOCK: usize = 64;
    let mut norm = [0u8; BLOCK];
    if key.len() > BLOCK {
        norm[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        norm[..key.len()].copy_from_slice(key);
    }
    let mut inner = Sha256::new();
    for b in &norm {
        inner.update([b ^ 0x36]);
    }
    inner.update(data);
    let inner = inner.finalize();
    let mut outer = Sha256::new();
    for b in &norm {
        outer.update([b ^ 0x5c]);
    }
    outer.update(inner);
    outer.finalize().into()
}

/// RFC3986-unreserved-escape for the canonical query string.
fn volc_uri_encode(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    for byte in input.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char)
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

/// Canonical query sorted by key (`Action` < `Region` < `Version`), shared
/// verbatim between the signature and the request URL.
fn volcengine_canonical_query(action: &str, region: &str) -> String {
    let mut pairs = [
        ("Action", action),
        ("Region", region),
        ("Version", VOLCENGINE_API_VERSION),
    ];
    pairs.sort_by(|a, b| a.0.cmp(b.0));
    pairs
        .iter()
        .map(|(k, v)| format!("{}={}", volc_uri_encode(k), volc_uri_encode(v)))
        .collect::<Vec<_>>()
        .join("&")
}

/// Build the Volcengine Signature V4 `Authorization` header. Returns
/// `(Authorization, X-Date, X-Content-Sha256)` — all three go into the
/// request headers. `now` is a parameter for deterministic tests.
fn volcengine_sign(
    access_key_id: &str,
    secret_access_key: &str,
    region: &str,
    canonical_query: &str,
    body: &[u8],
    now: chrono::DateTime<chrono::Utc>,
) -> (String, String, String) {
    let x_date = now.format("%Y%m%dT%H%M%SZ").to_string();
    let short_date = now.format("%Y%m%d").to_string();
    let x_content_sha256 = volc_sha256_hex(body);

    // Fixed-order canonical headers (Volcengine-specific, NOT sorted).
    let canonical_headers = format!(
        "host:{VOLCENGINE_OPENAPI_HOST}\nx-date:{x_date}\nx-content-sha256:{x_content_sha256}\ncontent-type:{VOLCENGINE_CONTENT_TYPE}\n"
    );
    let canonical_request = format!(
        "POST\n/\n{canonical_query}\n{canonical_headers}\n{VOLCENGINE_SIGNED_HEADERS}\n{x_content_sha256}"
    );

    let credential_scope = format!("{short_date}/{region}/{VOLCENGINE_SERVICE}/request");
    let string_to_sign = format!(
        "HMAC-SHA256\n{x_date}\n{credential_scope}\n{}",
        volc_sha256_hex(canonical_request.as_bytes())
    );

    // Key derivation: kDate = HMAC(SK, date) (no AWS4 prefix), suffix `request`.
    let k_date = hmac_sha256(secret_access_key.as_bytes(), short_date.as_bytes());
    let k_region = hmac_sha256(&k_date, region.as_bytes());
    let k_service = hmac_sha256(&k_region, VOLCENGINE_SERVICE.as_bytes());
    let k_signing = hmac_sha256(&k_service, b"request");
    let signature: String = hmac_sha256(&k_signing, string_to_sign.as_bytes())
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let authorization = format!(
        "HMAC-SHA256 Credential={access_key_id}/{credential_scope}, SignedHeaders={VOLCENGINE_SIGNED_HEADERS}, Signature={signature}"
    );
    (authorization, x_date, x_content_sha256)
}

fn volcengine_openapi_call(
    client: &reqwest::blocking::Client,
    region: &str,
    access_key_id: &str,
    secret_access_key: &str,
    action: &str,
) -> VolcCall {
    let canonical_query = volcengine_canonical_query(action, region);
    let url = format!("https://{VOLCENGINE_OPENAPI_HOST}/?{canonical_query}");
    let body: &[u8] = b"";
    let (authorization, x_date, x_content_sha256) = volcengine_sign(
        access_key_id,
        secret_access_key,
        region,
        &canonical_query,
        body,
        chrono::Utc::now(),
    );

    let resp = client
        .post(&url)
        .header("X-Date", x_date)
        .header("X-Content-Sha256", x_content_sha256)
        .header("Content-Type", VOLCENGINE_CONTENT_TYPE)
        .header("Authorization", authorization)
        .timeout(std::time::Duration::from_secs(15))
        .send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => return VolcCall::Transient(format!("Network error: {e}")),
    };

    let status = resp.status();
    if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
        return VolcCall::Auth(format!(
            "Auth failed (HTTP {status}). {VOLCENGINE_AKSK_HINT}"
        ));
    }
    if !status.is_success() {
        // The gateway returns 4xx (often 400) with the same
        // ResponseMetadata.Error envelope as the 200 path for signature /
        // credential errors — parse it so a rejected key surfaces as an
        // auth error with the AK/SK hint, not a generic API error.
        let raw = resp.text().unwrap_or_default();
        if let Ok(body) = serde_json::from_str::<serde_json::Value>(&raw) {
            if let Some((code, msg)) = volcengine_response_error(&body) {
                if volcengine_is_auth_error_code(&code) {
                    return VolcCall::Auth(format!(
                        "Auth failed (HTTP {status}, {code}): {msg}. {VOLCENGINE_AKSK_HINT}"
                    ));
                }
                return VolcCall::Soft(format!("Endpoint error (HTTP {status}, {code}): {msg}"));
            }
        }
        return VolcCall::Soft(format!("Endpoint error (HTTP {status}): {raw}"));
    }

    let raw = match resp.text() {
        Ok(b) => b,
        Err(e) => return VolcCall::Transient(format!("Network error: {e}")),
    };
    let body: serde_json::Value = match serde_json::from_str(&raw) {
        Ok(v) => v,
        Err(e) => return VolcCall::Soft(format!("Failed to parse the response: {e}")),
    };

    // Business errors arrive as 200 + ResponseMetadata.Error.
    if let Some((code, msg)) = volcengine_response_error(&body) {
        if volcengine_is_auth_error_code(&code) {
            return VolcCall::Auth(format!(
                "Auth failed ({code}): {msg}. {VOLCENGINE_AKSK_HINT}"
            ));
        }
        return VolcCall::Soft(format!("Endpoint error ({code}): {msg}"));
    }
    VolcCall::Body(body)
}

/// Parse `GetAFPUsage`'s `Result` into tiers. Shows the 5h / weekly / monthly
/// windows (matching the console); `AFPDaily` is hidden upstream (its quota
/// often exceeds the weekly cap — a historical default, not a real limit).
/// `Quota <= 0` means the window is not subscribed → skipped, which also
/// lets an authenticated-but-no-Agent-Plan result fall through to the
/// Coding Plan probe.
fn parse_afp_tiers(result: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    for (key, name) in [
        ("AFPFiveHour", "five_hour"),
        ("AFPWeekly", "weekly_limit"),
        ("AFPMonthly", "monthly"),
    ] {
        let Some(win) = result.get(key) else { continue };
        let quota = win.get("Quota").and_then(parse_f64).unwrap_or(0.0);
        if quota <= 0.0 {
            continue;
        }
        let used = win.get("Used").and_then(parse_f64).unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization: used / quota * 100.0,
            resets_at: win.get("ResetTime").and_then(extract_reset_time),
            used: Some(used),
            limit: Some(quota),
            unit: None,
        });
    }
    tiers
}

/// Normalize a `GetCodingPlanUsage` window label to a tier name.
fn volcengine_coding_window(label: &str) -> Option<&'static str> {
    match label.to_lowercase().as_str() {
        "session" | "5h" | "fivehour" | "five_hour" | "rolling_5h" => Some("five_hour"),
        "weekly" | "week" | "7d" => Some("weekly_limit"),
        "monthly" | "month" => Some("monthly"),
        _ => None,
    }
}

/// Parse `GetCodingPlanUsage`'s `Result` defensively: the API is not fully
/// documented; fields are matched loosely (`Level` is the real label as of
/// 2026-06, with fallbacks), percent-only values, second-level resets.
fn parse_coding_plan_tiers(result: &serde_json::Value) -> Vec<PlanTierVm> {
    let mut tiers = Vec::new();
    let arr = result
        .get("QuotaUsage")
        .and_then(|v| v.as_array())
        .or_else(|| result.get("Usages").and_then(|v| v.as_array()))
        .or_else(|| result.get("Details").and_then(|v| v.as_array()));
    let Some(arr) = arr else { return tiers };

    for item in arr {
        let label = item
            .get("Level")
            .and_then(|v| v.as_str())
            .or_else(|| item.get("Type").and_then(|v| v.as_str()))
            .or_else(|| item.get("Period").and_then(|v| v.as_str()))
            .or_else(|| item.get("Label").and_then(|v| v.as_str()))
            .or_else(|| item.get("Window").and_then(|v| v.as_str()))
            .unwrap_or("");
        let Some(name) = volcengine_coding_window(label) else {
            continue;
        };
        let utilization = item
            .get("Percent")
            .and_then(parse_f64)
            .or_else(|| item.get("UsedPercent").and_then(parse_f64))
            .or_else(|| item.get("UsagePercent").and_then(parse_f64))
            .unwrap_or(0.0);
        tiers.push(PlanTierVm {
            name: name.to_string(),
            utilization,
            resets_at: item
                .get("ResetTime")
                .or_else(|| item.get("ResetTimestamp"))
                .and_then(extract_reset_time),
            used: None,
            limit: None,
            unit: None,
        });
    }
    tiers
}

/// Agent Plan first (GetAFPUsage), then Coding Plan. Auth failures stop
/// immediately (shared credential); transient failures propagate as Err.
fn query_volcengine(
    client: &reqwest::blocking::Client,
    base_url: &str,
    access_key_id: &str,
    secret_access_key: &str,
) -> Result<QuotaOutcome, String> {
    let region = volcengine_region(base_url);
    let mut soft_errors: Vec<String> = Vec::new();
    let mut empty_responses: Vec<String> = Vec::new();
    let summarize = |action: &str, body: &serde_json::Value| -> String {
        let raw: String = body.to_string().chars().take(700).collect();
        format!("{action}={raw}")
    };

    // 1) Agent Plan: GetAFPUsage
    match volcengine_openapi_call(
        client,
        &region,
        access_key_id,
        secret_access_key,
        "GetAFPUsage",
    ) {
        VolcCall::Auth(detail) => return Ok(QuotaOutcome::Failed(detail)),
        VolcCall::Transient(detail) => return Err(format!("GetAFPUsage: {detail}")),
        VolcCall::Soft(detail) => soft_errors.push(format!("GetAFPUsage: {detail}")),
        VolcCall::Body(body) => {
            let result = body.get("Result").unwrap_or(&body);
            let tiers = parse_afp_tiers(result);
            if !tiers.is_empty() {
                let plan = result
                    .get("PlanType")
                    .and_then(|v| v.as_str())
                    .map(str::trim)
                    .filter(|s| !s.is_empty())
                    .map(|s| format!("Agent Plan {s}"));
                return Ok(QuotaOutcome::Ok { tiers, note: plan });
            }
            empty_responses.push(summarize("GetAFPUsage", &body));
        }
    }

    // 2) Coding Plan: GetCodingPlanUsage
    match volcengine_openapi_call(
        client,
        &region,
        access_key_id,
        secret_access_key,
        "GetCodingPlanUsage",
    ) {
        VolcCall::Auth(detail) => return Ok(QuotaOutcome::Failed(detail)),
        VolcCall::Transient(detail) => return Err(format!("GetCodingPlanUsage: {detail}")),
        VolcCall::Soft(detail) => soft_errors.push(format!("GetCodingPlanUsage: {detail}")),
        VolcCall::Body(body) => {
            let result = body.get("Result").unwrap_or(&body);
            let tiers = parse_coding_plan_tiers(result);
            if !tiers.is_empty() {
                return Ok(QuotaOutcome::Ok {
                    tiers,
                    note: Some("Coding Plan".to_string()),
                });
            }
            empty_responses.push(summarize("GetCodingPlanUsage", &body));
        }
    }

    if !soft_errors.is_empty() {
        Ok(QuotaOutcome::Failed(soft_errors.join("; ")))
    } else if !empty_responses.is_empty() {
        // Signature passed and the request reached the business layer, but no
        // quota could be parsed. Include the raw payloads for diagnosis.
        Ok(QuotaOutcome::Failed(format!(
            "No active plan subscription found (signature passed). Raw response: {}",
            empty_responses.join(" || ")
        )))
    } else {
        Ok(QuotaOutcome::Failed(
            "No active Agent Plan or Coding Plan subscription under these credentials".to_string(),
        ))
    }
}

// ── Template dispatch ──

/// Curated monthly price hints per template (endpoints don't report the
/// subscription price). None = unknown.
pub fn plan_monthly_price(plan_query: Option<&str>) -> Option<String> {
    let raw = plan_query?;
    let v: serde_json::Value = serde_json::from_str(raw).ok()?;
    match v.get("template")?.as_str()? {
        "opencode_go" => Some("$10/mo".to_string()),
        _ => None,
    }
}

/// Execute a template against its credentials. Outer `Err` = transient
/// network failure (frontend retries); inner failure becomes a
/// success:false report.
fn run_template(
    template: &str,
    fields: &HashMap<String, serde_json::Value>,
    base_url: &str,
    api_key: &str,
) -> Result<QuotaOutcome, String> {
    let client = blocking_client()?;
    match template {
        "kimi" => query_kimi(&client, api_key),
        "zhipu" => query_zhipu(&client, base_url, api_key),
        "zhipu_team" => {
            let org = field_str(fields, "organization_id");
            let project = field_str(fields, "project_id");
            if api_key.trim().is_empty() || org.is_empty() || project.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "The Zhipu team plan needs the API key + org ID + project ID".to_string(),
                ))
            } else {
                query_zhipu_team(&client, api_key, org, project)
            }
        }
        "minimax" => query_minimax(&client, base_url, api_key),
        "zenmux" => {
            let quota_url = field_str(fields, "quota_url");
            if quota_url.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "Fill in the ZenMux usage endpoint URL".to_string(),
                ))
            } else {
                query_zenmux(&client, quota_url, api_key)
            }
        }
        "opencode_go" => query_opencode_go(&client, api_key),
        "volcengine" => {
            let ak = field_str(fields, "access_key_id");
            let sk = field_str(fields, "secret_access_key");
            if ak.is_empty() || sk.is_empty() {
                Ok(QuotaOutcome::Failed(
                    "Volcengine Ark usage queries need the account AccessKey ID + Secret (not the inference API key)".to_string(),
                ))
            } else {
                query_volcengine(&client, base_url, ak, sk)
            }
        }
        // Grok reports quota over an undocumented gRPC-web API — stubbed.
        "grok" => Ok(QuotaOutcome::Failed(
            "Grok plan queries are not supported yet (the gRPC-web API is not adapted)".to_string(),
        )),
        other => Ok(QuotaOutcome::Failed(format!(
            "Unknown plan query template: {other}"
        ))),
    }
}

// ── Cache (Aux KV, 5-minute TTL) ──

fn cache_key(provider_id: &str) -> String {
    format!("plan_quota_cache:{provider_id}")
}

fn cache_read(aux: &Aux, provider_id: &str) -> Option<PlanQuotaReport> {
    let raw = aux.get_setting(&cache_key(provider_id))?;
    let v: serde_json::Value = serde_json::from_str(&raw).ok()?;
    let ts = v.get("ts")?.as_i64()?;
    if now_millis() - ts > CACHE_TTL_MS {
        return None;
    }
    let mut report: PlanQuotaReport = serde_json::from_value(v.get("report")?.clone()).ok()?;
    report.cached = true;
    Some(report)
}

fn cache_write(aux: &Aux, provider_id: &str, report: &PlanQuotaReport) {
    let v = serde_json::json!({ "ts": now_millis(), "report": report });
    let _ = aux.set_setting(&cache_key(provider_id), &v.to_string());
}

/// Query one provider's plan quota. Cached for 5 minutes unless `force`.
/// Outer `Err` = transient network failure; deterministic failures come back
/// as a `success: false` report.
pub fn get_plan_quota_report(
    store: &Store,
    aux: &Aux,
    provider_id: &str,
    force: bool,
) -> Result<PlanQuotaReport, String> {
    if !force {
        if let Some(hit) = cache_read(aux, provider_id) {
            return Ok(hit);
        }
    }
    let p = store
        .get_provider(provider_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("Provider not found: {provider_id}"))?;
    let query: serde_json::Value = p
        .plan_query
        .as_deref()
        .and_then(|s| serde_json::from_str(s).ok())
        .ok_or_else(|| "This provider has no plan query configured".to_string())?;
    let template = query
        .get("template")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let fields: HashMap<String, serde_json::Value> = query
        .get("fields")
        .and_then(|v| v.as_object())
        .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
        .unwrap_or_default();
    let api_key = p.api_key.clone().unwrap_or_default();

    let mut report = match run_template(&template, &fields, &p.base_url, &api_key)? {
        QuotaOutcome::Ok { tiers, note } => PlanQuotaReport {
            provider_id: provider_id.to_string(),
            template,
            success: true,
            error: None,
            note,
            tiers,
            queried_at: now_millis(),
            cached: false,
        },
        QuotaOutcome::Failed(error) => PlanQuotaReport {
            provider_id: provider_id.to_string(),
            template,
            success: false,
            error: Some(error),
            note: None,
            tiers: Vec::new(),
            queried_at: now_millis(),
            cached: false,
        },
    };
    if report.success {
        cache_write(aux, provider_id, &report);
    }
    report.provider_id = provider_id.to_string();
    Ok(report)
}

#[tauri::command(async)]
pub fn get_plan_quota(
    state: tauri::State<'_, crate::AppState>,
    provider_id: String,
    force: Option<bool>,
) -> Result<PlanQuotaReport, String> {
    get_plan_quota_report(
        &state.store,
        &state.aux,
        &provider_id,
        force.unwrap_or(false),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── shared helpers ──

    #[test]
    fn hmac_sha256_matches_rfc4231_vector() {
        // RFC 4231 test case 2: key "Jefe", data "what do ya want for nothing?"
        let mac = hmac_sha256(b"Jefe", b"what do ya want for nothing?");
        let hex: String = mac.iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }

    #[test]
    fn reset_time_accepts_strings_seconds_and_millis() {
        assert_eq!(
            extract_reset_time(&json!("2026-08-26T14:12:03.000Z")).as_deref(),
            Some("2026-08-26T14:12:03.000Z")
        );
        // Seconds < 1e12 are scaled to millis.
        assert!(extract_reset_time(&json!(1_782_057_600_i64)).is_some());
        // Millis pass through.
        assert!(extract_reset_time(&json!(1_782_057_600_000_i64)).is_some());
        // Non-positive means "no active window".
        assert!(extract_reset_time(&json!(0)).is_none());
        assert!(extract_reset_time(&json!(-1)).is_none());
    }

    // ── Kimi ──

    #[test]
    fn kimi_tiers_from_limits_and_usage() {
        let body = json!({
            "limits": [
                { "detail": { "limit": 100.0, "remaining": 25.0, "resetTime": "2026-09-10T15:00:00Z" } }
            ],
            "usage": { "limit": 200.0, "remaining": 80.0, "resetTime": "2026-09-14T00:00:00Z" }
        });
        let tiers = parse_kimi_tiers(&body);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 75.0).abs() < 1e-9);
        assert!(tiers[0].resets_at.is_some());
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 60.0).abs() < 1e-9);
    }

    // ── Zhipu ──

    #[test]
    fn zhipu_unit_field_overrides_reset_order() {
        // issue #3036 case: at the end of a weekly cycle the weekly window
        // resets sooner than the 5-hour one; the unit field must win.
        let data = json!({
            "limits": [
                { "type": "TOKENS_LIMIT", "unit": 6, "number": 7, "percentage": 42.0, "nextResetTime": 1_000_003_600_000_i64 },
                { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 1.0,  "nextResetTime": 1_000_018_000_000_i64 }
            ]
        });
        let tiers = parse_zhipu_token_tiers(&data);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 1.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 42.0).abs() < 1e-9);
    }

    #[test]
    fn zhipu_zero_percent_five_hour_has_no_reset_but_stays_five_hour() {
        // The 5-hour bucket at 0% may lack nextResetTime; it must not be
        // re-slotted as the weekly window.
        let data = json!({
            "limits": [
                { "type": "TOKENS_LIMIT", "percentage": 25.0, "nextResetTime": 2_000_000_000_000_i64 },
                { "type": "TOKENS_LIMIT", "percentage": 0.0 }
            ]
        });
        let tiers = parse_zhipu_token_tiers(&data);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!(tiers[0].resets_at.is_none());
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!(tiers[1].resets_at.is_some());
    }

    #[test]
    fn zhipu_quota_base_routes_by_host() {
        assert_eq!(
            zhipu_quota_base("https://open.bigmodel.cn/api/paas/v4"),
            "https://open.bigmodel.cn"
        );
        assert_eq!(
            zhipu_quota_base("https://api.z.ai/api/paas/v4"),
            "https://api.z.ai"
        );
        // Case-insensitive, matching the preset URL handling.
        assert_eq!(
            zhipu_quota_base("HTTPS://OPEN.BIGMODEL.CN/api/paas/v4"),
            "https://open.bigmodel.cn"
        );
        // Unknown hosts default to the international endpoint.
        assert_eq!(
            zhipu_quota_base("https://example.com/zhipu"),
            "https://api.z.ai"
        );
    }

    #[test]
    fn zhipu_business_error_and_missing_data() {
        let failed = zhipu_outcome(&json!({ "success": false, "msg": "bad key" }));
        assert!(matches!(failed, QuotaOutcome::Failed(m) if m.contains("bad key")));
        let missing = zhipu_outcome(&json!({ "success": true }));
        assert!(matches!(missing, QuotaOutcome::Failed(_)));
        let ok = zhipu_outcome(&json!({
            "success": true,
            "data": {
                "level": "max",
                "limits": [
                    { "type": "TOKENS_LIMIT", "unit": 3, "number": 5, "percentage": 26.0 },
                    { "type": "TOKENS_LIMIT", "unit": 6, "number": 1, "percentage": 5.0 }
                ]
            }
        }));
        match ok {
            QuotaOutcome::Ok { tiers, note } => {
                assert_eq!(tiers.len(), 2);
                assert_eq!(note.as_deref(), Some("max"));
            }
            _ => panic!("expected ok outcome"),
        }
    }

    // ── MiniMax ──

    #[test]
    fn minimax_general_tiers_invert_remaining_percent() {
        let body = json!({
            "model_remains": [
                { "model_name": "general",
                  "current_interval_remaining_percent": 98.0,
                  "current_weekly_remaining_percent": 95.0,
                  "current_weekly_status": 1,
                  "end_time": 1_780_329_600_000_i64,
                  "weekly_end_time": 1_780_848_000_000_i64 },
                { "model_name": "video", "current_interval_remaining_percent": 50.0 }
            ]
        });
        let tiers = parse_minimax_tiers(&body);
        assert_eq!(tiers.len(), 2);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 2.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 5.0).abs() < 1e-9);
    }

    #[test]
    fn minimax_weekly_status_3_skips_weekly_tier() {
        // status=3 = plan has no weekly cap; the pinned 100% must not render
        // as a fake "0% used" bucket.
        let body = json!({
            "model_remains": [{
                "model_name": "general",
                "current_interval_remaining_percent": 99.0,
                "current_weekly_status": 3,
                "current_weekly_remaining_percent": 100.0
            }]
        });
        let tiers = parse_minimax_tiers(&body);
        assert_eq!(tiers.len(), 1);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 1.0).abs() < 1e-9);
    }

    // ── OpenCode Go ──

    #[test]
    fn opencode_go_three_windows_parse_defensively() {
        let body = json!({
            "usage": {
                "rolling": { "status": "ok", "percent": 37, "resetsAt": "2026-08-26T14:12:03.000Z" },
                "weekly":  { "status": "ok", "percent": "62", "resetsAt": "2026-08-31T00:00:00.000Z" },
                "monthly": { "status": "rate-limited", "percent": 100, "resetsAt": "2026-09-11T00:00:00.000Z" }
            }
        });
        let tiers = parse_opencode_go_tiers(&body);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 37.0).abs() < 1e-9);
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 62.0).abs() < 1e-9);
        assert_eq!(tiers[2].name, "monthly");
        assert!((tiers[2].utilization - 100.0).abs() < 1e-9);
    }

    #[test]
    fn opencode_go_zero_percent_drops_placeholder_reset() {
        let body = json!({
            "usage": { "rolling": { "status": "ok", "percent": 0, "resetsAt": "2026-08-26T15:00:00.000Z" } }
        });
        let tiers = parse_opencode_go_tiers(&body);
        assert_eq!(tiers.len(), 1);
        assert!(tiers[0].resets_at.is_none());
    }

    #[test]
    fn opencode_go_legacy_flat_shape_returns_empty() {
        let body = json!({
            "rollingUsage": { "status": "ok", "usagePercent": 37, "resetInSec": 3600 }
        });
        assert!(parse_opencode_go_tiers(&body).is_empty());
    }

    // ── Volcengine ──

    #[test]
    fn volcengine_afp_three_windows_skip_daily_and_zero_quota() {
        let result = json!({
            "PlanType": "Large",
            "AFPFiveHour": { "Quota": 50.0,  "Used": 12.5,  "ResetTime": 1778806800000_i64 },
            "AFPDaily":    { "Quota": 100.0, "Used": 22.5,  "ResetTime": 1778803200000_i64 },
            "AFPWeekly":   { "Quota": 500.0, "Used": 150.0, "ResetTime": 1779062400000_i64 },
            "AFPMonthly":  { "Quota": 0.0,   "Used": 0.0 }
        });
        let tiers = parse_afp_tiers(&result);
        assert_eq!(tiers.len(), 2, "daily hidden, zero-quota monthly skipped");
        assert_eq!(tiers[0].name, "five_hour");
        assert!((tiers[0].utilization - 25.0).abs() < 1e-9);
        assert_eq!(tiers[0].used, Some(12.5));
        assert_eq!(tiers[0].limit, Some(50.0));
        assert_eq!(tiers[1].name, "weekly_limit");
        assert!((tiers[1].utilization - 30.0).abs() < 1e-9);
    }

    #[test]
    fn volcengine_coding_plan_real_shape() {
        let result = json!({
            "Status": "Running",
            "QuotaUsage": [
                { "Level": "session", "Percent": 0.0,      "ResetTimestamp": -1_i64 },
                { "Level": "weekly",  "Percent": 1.672568, "ResetTimestamp": 1782057600_i64 },
                { "Level": "monthly", "Percent": 0.836284, "ResetTimestamp": 1784303999_i64 }
            ]
        });
        let tiers = parse_coding_plan_tiers(&result);
        assert_eq!(tiers.len(), 3);
        assert_eq!(tiers[0].name, "five_hour");
        assert!(tiers[0].resets_at.is_none(), "ResetTimestamp=-1 → no reset");
        assert_eq!(tiers[1].name, "weekly_limit");
        assert_eq!(tiers[2].name, "monthly");
    }

    #[test]
    fn volcengine_region_and_query_and_sign_contract() {
        assert_eq!(
            volcengine_region("https://ark.cn-beijing.volces.com/api/coding"),
            "cn-beijing"
        );
        assert_eq!(
            volcengine_region("https://example.com/api/coding"),
            "cn-beijing"
        );
        assert_eq!(
            volcengine_canonical_query("GetAFPUsage", "cn-beijing"),
            "Action=GetAFPUsage&Region=cn-beijing&Version=2024-01-01"
        );

        let now = chrono::DateTime::parse_from_rfc3339("2024-06-21T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        let region = "cn-beijing";
        let query = volcengine_canonical_query("GetAFPUsage", region);
        let (auth, x_date, x_content) =
            volcengine_sign("AKLTtest", "secretkey", region, &query, b"", now);
        // Empty-body SHA-256.
        assert_eq!(
            x_content,
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
        assert_eq!(x_date, "20240621T000000Z");
        // No AWS4 prefix, scope suffix `request`, fixed SignedHeaders order.
        assert!(
            auth.starts_with("HMAC-SHA256 Credential=AKLTtest/20240621/cn-beijing/ark/request,")
        );
        assert!(auth.contains("SignedHeaders=host;x-date;x-content-sha256;content-type,"));
        let sig = auth.rsplit("Signature=").next().unwrap();
        assert_eq!(sig.len(), 64);
        assert!(sig.bytes().all(|b| b.is_ascii_hexdigit()));
        // Deterministic.
        let (auth2, _, _) = volcengine_sign("AKLTtest", "secretkey", region, &query, b"", now);
        assert_eq!(auth, auth2);
    }

    #[test]
    fn volcengine_auth_error_classification() {
        assert!(volcengine_is_auth_error_code("AccessDenied"));
        assert!(volcengine_is_auth_error_code("SignatureDoesNotMatch"));
        assert!(volcengine_is_auth_error_code("InvalidAuthorization"));
        assert!(!volcengine_is_auth_error_code("InvalidParameter.Action"));
        assert!(!volcengine_is_auth_error_code("InternalError"));

        let body = json!({
            "ResponseMetadata": { "Error": { "Code": "AccessDenied", "Message": "no permission" } }
        });
        let (code, msg) = volcengine_response_error(&body).expect("extracts error");
        assert_eq!(code, "AccessDenied");
        assert_eq!(msg, "no permission");
        assert!(volcengine_response_error(&json!({ "ResponseMetadata": {} })).is_none());
    }

    // ── template dispatch + price hints ──

    #[test]
    fn unknown_template_and_grok_fail_deterministically() {
        let fields = HashMap::new();
        match run_template("grok", &fields, "https://x.grok.com", "k").unwrap() {
            QuotaOutcome::Failed(m) => assert!(m.contains("not supported yet")),
            _ => panic!("grok must fail deterministically"),
        }
        match run_template("whatever", &fields, "https://x", "k").unwrap() {
            QuotaOutcome::Failed(m) => assert!(m.contains("Unknown plan query template")),
            _ => panic!("unknown template must fail"),
        }
    }

    #[test]
    fn zenmux_and_volcengine_require_fields() {
        let empty = HashMap::new();
        match run_template("zenmux", &empty, "https://x", "k").unwrap() {
            QuotaOutcome::Failed(m) => assert!(m.contains("ZenMux")),
            _ => panic!("zenmux needs quota_url"),
        }
        match run_template(
            "volcengine",
            &empty,
            "https://ark.cn-beijing.volces.com/api/coding",
            "",
        )
        .unwrap()
        {
            QuotaOutcome::Failed(m) => assert!(m.contains("AccessKey")),
            _ => panic!("volcengine needs AK/SK"),
        }
    }

    #[test]
    fn price_hint_only_for_known_templates() {
        let pq = json!({ "template": "opencode_go", "fields": {} }).to_string();
        assert_eq!(plan_monthly_price(Some(&pq)).as_deref(), Some("$10/mo"));
        let pq2 = json!({ "template": "kimi", "fields": {} }).to_string();
        assert_eq!(plan_monthly_price(Some(&pq2)), None);
        assert_eq!(plan_monthly_price(None), None);
        assert_eq!(plan_monthly_price(Some("not json")), None);
    }
}
