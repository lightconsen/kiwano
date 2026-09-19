//! `kiwano insights`: how the agents actually spend their tokens, computed
//! from the gateway's own request log.
//!
//! The report is one page: a per-agent scorecard of five metrics, a short
//! list of rule findings with the log ids that back them, and the sessions
//! whose context grew the most. Everything is derived locally from
//! `request_logs` (plus a couple of sampled `request_bodies` for the fixed
//! overhead rule), so the privacy promise does not bend: nothing is sent
//! anywhere and no model is asked to summarise anything.
//!
//! The log holds requests but not outcomes — whether the task succeeded is
//! invisible to the gateway — so every metric here is a proxy. That is also
//! why findings carry evidence ids: a human can reopen the row
//! (`kiwano logs show <id>`) and judge.
//!
//! Metric definitions (the renderer prints them under the scorecard):
//!
//! - **Cache hit** = `cache_read / (input + cache_read + cache_creation)`.
//!   Cache writes sit in the denominator because they are paid at full input
//!   price; leaving them out inflates every rate (the same choice the Apps
//!   screen's cache column makes).
//! - **Ctx growth** = per session, last turn's context ÷ first turn's, where
//!   a turn's context is its input-side tokens (`input + cache_read +
//!   cache_creation` — with caching on, `input_tokens` alone is just the
//!   uncached sliver and would misread a healthy session as a shrinking one).
//!   The scorecard shows the median across an agent's multi-turn sessions.
//! - **Reasoning** = `reasoning / output` tokens; `–` when the provider
//!   reports no reasoning at all, which is not the same as 0%.
//! - **Retries** = a request that follows an errored one within 60s, same
//!   model, inside one session — or, for a log whose rows predate session
//!   ids, inside one agent+provider pair.

use std::collections::{BTreeMap, BTreeSet};

use serde::Serialize;

/// How close behind an errored request a same-model resend must be to count
/// as a retry of it.
const RETRY_WINDOW_SECS: i64 = 60;
/// A burst becomes a finding at this many retries (the anchor excluded).
const MIN_RETRY_BURST: usize = 3;
/// Below this hit rate the cache rule fires …
const CACHE_LOW_PCT: i64 = 20;
/// … provided the agent moved at least this many input-side tokens, so one
/// small session cannot earn a finding.
const CACHE_MIN_DENOM: i64 = 10_000;
/// A session that grew at least this much, over at least `BLOAT_MIN_TURNS`
/// turns, without ever falling back, never compacted.
const BLOAT_MIN_GROWTH: f64 = 8.0;
const BLOAT_MIN_TURNS: i64 = 5;
/// Share of a request body the fixed tools/system payload must reach to be
/// worth a finding, and its absolute floor — below that, the schema is not
/// the problem even if it is half of a tiny body.
const OVERHEAD_MIN_PCT: i64 = 40;
const OVERHEAD_MIN_BYTES: usize = 8 * 1024;
/// Evidence ids printed per finding before the ellipsis.
const MAX_EVIDENCE: usize = 6;

/// The slice of a `request_logs` row the report reads. The CLI narrows the
/// store's `RequestLogEntry` to this so the rules never depend on the full
/// 30-column shape.
#[derive(Debug, Clone, PartialEq)]
pub struct InsightRow {
    pub id: i64,
    /// RFC3339 UTC — the lexicographically ordered text the store filters on.
    pub ts: String,
    pub agent: Option<String>,
    pub provider_id: Option<String>,
    pub model: Option<String>,
    pub session_id: Option<String>,
    pub status_code: i64,
    pub error_kind: Option<String>,
    pub input_tokens: i64,
    pub output_tokens: i64,
    pub cache_read_tokens: i64,
    pub cache_creation_tokens: i64,
    pub reasoning_tokens: i64,
    pub request_size: i64,
}

impl InsightRow {
    /// A turn's context size: everything the model has to re-read, cached or
    /// not. See the module docs for why `input_tokens` alone would mislead.
    fn context(&self) -> i64 {
        self.input_tokens + self.cache_read_tokens + self.cache_creation_tokens
    }
}

/// One request body the CLI sampled for the overhead rule: the newest
/// untruncated body per agent, a handful at most. The body never leaves this
/// module — only the measurement of its fixed part does.
#[derive(Debug, Clone, PartialEq)]
pub struct BodySample {
    pub log_id: i64,
    pub agent: Option<String>,
    pub request_body: String,
}

/// The window the report covers, RFC3339 UTC, both ends inclusive.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Window {
    pub from: String,
    pub to: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Totals {
    pub requests: i64,
    /// Distinct session ids; rows without one simply do not count.
    pub sessions: i64,
    /// Distinct agents, the unattributed rows counting as one.
    pub agents: i64,
    pub errors: i64,
    /// `errors / requests`, per cent to one decimal; 0 when there are none.
    pub error_pct: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct AgentScore {
    /// `None` for rows the gateway could not attribute; rendered "–".
    pub agent: Option<String>,
    pub requests: i64,
    pub sessions: i64,
    /// Cache hit per cent, or `None` when the window holds no input-side
    /// tokens — "nothing cached yet" is not "the cache never hit".
    pub cache_hit_pct: Option<i64>,
    /// The hit rate's denominator, so a consumer can tell a solid rate from
    /// a lucky one.
    pub input_side_tokens: i64,
    /// Median context growth across this agent's multi-turn sessions.
    pub ctx_growth: Option<f64>,
    /// `None` when no output tokens, or the provider reports no reasoning.
    pub reasoning_pct: Option<i64>,
    pub retries: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Finding {
    /// One of `cache` / `bloat` / `retry` / `overhead`.
    pub tag: String,
    pub agent: Option<String>,
    /// The conclusion, one sentence, no agent prefix (the renderer adds it).
    pub summary: String,
    /// The quantified comparison; may be empty.
    pub detail: String,
    /// `request_logs` ids a human can reopen with `kiwano logs show <id>`.
    pub evidence: Vec<i64>,
    /// True when the evidence names the ends of a range, or was capped,
    /// rather than listing every row.
    pub evidence_span: bool,
    /// The rule this finding teaches, when it teaches one — the text
    /// `kiwano rules apply` writes into the agent's instruction file (Features:
    /// rule injection). Kept separate from `detail`: the detail quotes the
    /// numbers, the rule is what to *do* and must read as an instruction.
    pub rule: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SessionGrowth {
    pub session_id: String,
    pub agent: Option<String>,
    pub turns: i64,
    pub first_context: i64,
    pub last_context: i64,
    pub growth: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct InsightsReport {
    /// The window actually requested, echoed for the JSON consumer.
    pub days: i64,
    /// The `--agent` filter, if any.
    pub agent_filter: Option<String>,
    pub window: Window,
    pub totals: Totals,
    /// Busiest agent first.
    pub scorecard: Vec<AgentScore>,
    /// Ordered by rule (cache, bloat, retry, overhead), each capped.
    pub findings: Vec<Finding>,
    /// The five sessions whose context grew the most.
    pub top_sessions: Vec<SessionGrowth>,
}

/// Compute the whole report. `rows` may arrive in any order (the store reads
/// newest-first; the scans here sort for themselves), and `bodies` are the
/// caller's samples for the overhead rule — empty is fine, the rule just
/// stays quiet.
pub fn build_insights(
    days: i64,
    agent_filter: Option<String>,
    window: Window,
    rows: &[InsightRow],
    bodies: &[BodySample],
) -> InsightsReport {
    // Every scan below wants chronological order, whatever the store returned.
    let mut sorted: Vec<&InsightRow> = rows.iter().collect();
    sorted.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.id.cmp(&b.id)));

    let totals = build_totals(&sorted);
    let stats = session_stats(&sorted);
    let (retry_counts, retry_findings) = retry_scan(&sorted);
    let scorecard = build_scorecard(&sorted, &stats, &retry_counts);

    let mut findings = cache_findings(&sorted, &scorecard);
    findings.extend(bloat_findings(&stats, &scorecard));
    findings.extend(retry_findings);
    findings.extend(overhead_findings(bodies));

    let mut top_sessions: Vec<SessionGrowth> = stats
        .iter()
        .filter(|s| s.turns >= 2 && s.growth.is_some())
        .map(|s| SessionGrowth {
            session_id: s.session_id.clone(),
            agent: s.agent.clone(),
            turns: s.turns,
            first_context: s.first_context,
            last_context: s.last_context,
            growth: s.growth.expect("filtered above"),
        })
        .collect();
    top_sessions.sort_by(|a, b| {
        b.growth
            .partial_cmp(&a.growth)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.session_id.cmp(&b.session_id))
    });
    top_sessions.truncate(5);

    InsightsReport {
        days,
        agent_filter,
        window,
        totals,
        scorecard,
        findings,
        top_sessions,
    }
}

// ── totals and scorecard ────────────────────────────────────────────────────

fn build_totals(rows: &[&InsightRow]) -> Totals {
    let requests = rows.len() as i64;
    let sessions: BTreeSet<&str> = rows
        .iter()
        .filter_map(|r| r.session_id.as_deref())
        .collect();
    // The unattributed rows are one "agent" of their own: they happened, and
    // hiding them would make the request count disagree with the scorecard.
    let agents: BTreeSet<Option<&str>> = rows.iter().map(|r| r.agent.as_deref()).collect();
    let errors = rows.iter().filter(|r| r.status_code >= 400).count() as i64;
    let error_pct = if requests == 0 {
        0.0
    } else {
        round1(errors as f64 * 100.0 / requests as f64)
    };
    Totals {
        requests,
        sessions: sessions.len() as i64,
        agents: agents.len() as i64,
        errors,
        error_pct,
    }
}

/// Per-agent accumulator behind the scorecard row.
#[derive(Default)]
struct Agg {
    requests: i64,
    sessions: BTreeSet<String>,
    cache_read: i64,
    input_side: i64,
    output: i64,
    reasoning: i64,
}

fn build_scorecard(
    rows: &[&InsightRow],
    stats: &[SessionStat],
    retry_counts: &BTreeMap<Option<String>, i64>,
) -> Vec<AgentScore> {
    let mut aggs: BTreeMap<Option<String>, Agg> = BTreeMap::new();
    for r in rows {
        let a = aggs.entry(r.agent.clone()).or_default();
        a.requests += 1;
        if let Some(sid) = &r.session_id {
            a.sessions.insert(sid.clone());
        }
        a.cache_read += r.cache_read_tokens;
        a.input_side += r.context();
        a.output += r.output_tokens;
        a.reasoning += r.reasoning_tokens;
    }

    let mut scores: Vec<AgentScore> = aggs
        .into_iter()
        .map(|(agent, a)| {
            let cache_hit_pct = if a.input_side > 0 {
                Some(pct(a.cache_read, a.input_side))
            } else {
                None
            };
            // Reasoning `–` rather than 0%: most providers do not break
            // thinking out at all, and a zero there means "not reported".
            let reasoning_pct = if a.reasoning > 0 && a.output > 0 {
                Some(pct(a.reasoning, a.output))
            } else {
                None
            };
            let growths: Vec<f64> = stats
                .iter()
                .filter(|s| s.agent == agent && s.growth.is_some())
                .map(|s| s.growth.expect("filtered above"))
                .collect();
            AgentScore {
                ctx_growth: median(growths).map(round1),
                retries: retry_counts.get(&agent).copied().unwrap_or(0),
                agent,
                requests: a.requests,
                sessions: a.sessions.len() as i64,
                cache_hit_pct,
                input_side_tokens: a.input_side,
                reasoning_pct,
            }
        })
        .collect();
    scores.sort_by(|a, b| {
        b.requests
            .cmp(&a.requests)
            .then_with(|| a.agent.cmp(&b.agent))
    });
    scores
}

// ── sessions and the bloat rule ─────────────────────────────────────────────

/// One session's shape, in chronological order of its rows.
struct SessionStat {
    session_id: String,
    agent: Option<String>,
    turns: i64,
    first_context: i64,
    last_context: i64,
    /// `None` when the first turn carries no context to compare against.
    growth: Option<f64>,
    /// Context never fell back — the signature of a session that grew to its
    /// limit without a single compaction.
    monotonic: bool,
    first_id: i64,
    last_id: i64,
}

fn session_stats(rows: &[&InsightRow]) -> Vec<SessionStat> {
    let mut by_session: BTreeMap<&str, Vec<&InsightRow>> = BTreeMap::new();
    for r in rows {
        if let Some(sid) = r.session_id.as_deref() {
            by_session.entry(sid).or_default().push(r);
        }
    }
    by_session
        .into_iter()
        .map(|(sid, mut rows)| {
            rows.sort_by(|a, b| a.ts.cmp(&b.ts).then(a.id.cmp(&b.id)));
            let first = rows.first().expect("group is non-empty");
            let last = rows.last().expect("group is non-empty");
            let first_context = first.context();
            let last_context = last.context();
            let monotonic = rows.windows(2).all(|w| w[1].context() >= w[0].context());
            SessionStat {
                growth: (first_context > 0).then(|| last_context as f64 / first_context as f64),
                session_id: sid.to_string(),
                agent: first.agent.clone(),
                turns: rows.len() as i64,
                first_context,
                last_context,
                monotonic,
                first_id: first.id,
                last_id: last.id,
            }
        })
        .collect()
}

fn bloat_findings(stats: &[SessionStat], scorecard: &[AgentScore]) -> Vec<Finding> {
    let mut candidates: Vec<&SessionStat> = stats
        .iter()
        .filter(|s| {
            s.turns >= BLOAT_MIN_TURNS
                && s.monotonic
                && s.growth.is_some_and(|g| g >= BLOAT_MIN_GROWTH)
        })
        .collect();
    candidates.sort_by(|a, b| {
        b.growth
            .partial_cmp(&a.growth)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
    candidates
        .into_iter()
        .take(2)
        .map(|s| {
            let growth = s.growth.expect("filtered above");
            // The agent's own median puts the outlier in perspective: 16× is
            // alarming next to a 3× median and merely consistent next to 15×.
            let median_note = scorecard
                .iter()
                .find(|a| a.agent == s.agent)
                .and_then(|a| a.ctx_growth)
                .map(|m| format!("; this agent's median session grows {m:.1}×"))
                .unwrap_or_default();
            Finding {
                summary: format!(
                    "session {}: {} turns, context {} → {} tokens ({:.1}×), never compacted",
                    ellipsize_session(&s.session_id),
                    s.turns,
                    crate::vm::fmt_tokens(s.first_context),
                    crate::vm::fmt_tokens(s.last_context),
                    growth,
                ),
                detail: format!(
                    "the context grew every single turn without one fallback{}",
                    median_note
                ),
                tag: "bloat".to_string(),
                agent: s.agent.clone(),
                evidence: vec![s.first_id, s.last_id],
                evidence_span: true,
                rule: Some(
                    "Compact early: when a task's context has grown several-fold, summarize and start a fresh session instead of pushing every turn into one window."
                        .to_string(),
                ),
            }
        })
        .collect()
}

/// Session ids are the client's own and can be long; the report shows enough
/// of one to recognise it.
fn ellipsize_session(sid: &str) -> String {
    if sid.chars().count() <= 24 {
        sid.to_string()
    } else {
        let head: String = sid.chars().take(12).collect();
        let tail: String = sid.chars().skip(sid.chars().count() - 4).collect();
        format!("{head}…{tail}")
    }
}

// ── retries ─────────────────────────────────────────────────────────────────

/// Rows without a session id fall back to agent+provider: a log from before
/// session capture would otherwise show zero retries no matter how hard the
/// client hammered an endpoint.
fn retry_group_key(row: &InsightRow) -> String {
    match (&row.session_id, &row.agent, &row.provider_id) {
        (Some(sid), ..) => format!("s:{sid}"),
        (None, agent, provider) => format!(
            "a:{}:{}",
            agent.as_deref().unwrap_or(""),
            provider.as_deref().unwrap_or("")
        ),
    }
}

fn parse_ts(ts: &str) -> Option<i64> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .ok()
        .map(|dt| dt.timestamp())
}

/// Whether `row` is a retry of `prev`: the previous attempt errored, the
/// model is the same, and the resend landed within the window.
fn is_retry(prev: &InsightRow, row: &InsightRow) -> bool {
    if prev.error_kind.is_none() || prev.model != row.model {
        return false;
    }
    match (parse_ts(&prev.ts), parse_ts(&row.ts)) {
        (Some(a), Some(b)) => (0..=RETRY_WINDOW_SECS).contains(&(b - a)),
        _ => false,
    }
}

/// One error and the resends chained behind it.
struct Burst<'a> {
    anchor: &'a InsightRow,
    /// The anchor followed by its retries, chronologically.
    chain: Vec<&'a InsightRow>,
}

/// Counts every agent's retries and collects the bursts big enough to report.
fn retry_scan(rows: &[&InsightRow]) -> (BTreeMap<Option<String>, i64>, Vec<Finding>) {
    let mut by_group: BTreeMap<String, Vec<&InsightRow>> = BTreeMap::new();
    for r in rows {
        by_group.entry(retry_group_key(r)).or_default().push(r);
    }

    let mut counts: BTreeMap<Option<String>, i64> = BTreeMap::new();
    let mut bursts: Vec<Burst<'_>> = Vec::new();

    for group in by_group.values() {
        // `rows` arrived chronological, and the group preserves that order.
        let mut current: Option<Burst<'_>> = None;
        let mut prev: Option<&InsightRow> = None;
        for row in group {
            let continues = current.is_some() && prev.is_some_and(|p| is_retry(p, row));
            if continues {
                *counts.entry(row.agent.clone()).or_default() += 1;
                current.as_mut().expect("checked above").chain.push(row);
            } else {
                if let Some(b) = current.take() {
                    bursts.push(b);
                }
                if row.error_kind.is_some() {
                    current = Some(Burst {
                        anchor: row,
                        chain: vec![row],
                    });
                }
            }
            prev = Some(row);
        }
        if let Some(b) = current.take() {
            bursts.push(b);
        }
    }

    // Worst waste first, and only the three worst: a log full of retry
    // storms should still produce a readable page.
    bursts.sort_by_key(|b| std::cmp::Reverse(b.chain.len()));
    let findings = bursts
        .iter()
        .filter(|b| b.chain.len() > MIN_RETRY_BURST)
        .take(3)
        .map(burst_finding)
        .collect();
    (counts, findings)
}

fn burst_finding(b: &Burst<'_>) -> Finding {
    let anchor = b.anchor;
    let last = b.chain.last().expect("chain holds the anchor");
    let retries = b.chain.len() - 1;
    let span = match (parse_ts(&anchor.ts), parse_ts(&last.ts)) {
        (Some(a), Some(l)) => l - a,
        _ => 0,
    };
    let out_tokens: i64 = b.chain.iter().map(|r| r.output_tokens).sum();
    let all_failed = b.chain.iter().all(|r| r.status_code >= 400);
    let kind = anchor.error_kind.as_deref().unwrap_or("error");
    let model = anchor
        .model
        .as_deref()
        .map(|m| format!(" on {m}"))
        .unwrap_or_default();
    let clock = |ts: &str| ts.get(11..19).unwrap_or(ts).to_string();

    let evidence: Vec<i64> = b.chain.iter().map(|r| r.id).take(MAX_EVIDENCE).collect();
    let truncated = b.chain.len() > evidence.len();

    Finding {
        summary: format!(
            "{retries} retries in {span}s ({}→{}), all `{kind}`{model}",
            clock(&anchor.ts),
            clock(&last.ts),
        ),
        detail: format!(
            "{} output tokens across the whole burst{}",
            crate::vm::fmt_tokens(out_tokens),
            if all_failed {
                "; every attempt in it failed"
            } else {
                ""
            }
        ),
        tag: "retry".to_string(),
        agent: anchor.agent.clone(),
        // The ellipsis only means "capped" here; a burst lists its rows.
        evidence,
        evidence_span: truncated,
        rule: Some(
            "Never retry a failing request in a tight loop: back off, and on a protocol or configuration error stop and report instead of retrying."
                .to_string(),
        ),
    }
}

// ── the cache rule ──────────────────────────────────────────────────────────

fn cache_findings(rows: &[&InsightRow], scorecard: &[AgentScore]) -> Vec<Finding> {
    // Provider sets per agent, so the finding can compare like with like:
    // a low rate next to a high one on the same provider is evidence, while
    // a low rate on a provider nobody else uses is just a number.
    let mut providers: BTreeMap<Option<String>, BTreeSet<String>> = BTreeMap::new();
    for r in rows {
        if let Some(pid) = &r.provider_id {
            providers
                .entry(r.agent.clone())
                .or_default()
                .insert(pid.clone());
        }
    }

    // Error rates per agent: a window dominated by failures has no
    // steady-state caching behaviour to judge — those requests never got far
    // enough to hit or miss a cache. The retry rule tells that story instead.
    let mut err_rate: BTreeMap<Option<String>, (i64, i64)> = BTreeMap::new();
    for r in rows {
        let slot = err_rate.entry(r.agent.clone()).or_default();
        slot.0 += 1;
        slot.1 += (r.status_code >= 400) as i64;
    }

    let mut flagged: Vec<&AgentScore> = scorecard
        .iter()
        .filter(|s| {
            let mostly_failing = err_rate
                .get(&s.agent)
                .is_some_and(|(total, errs)| *errs * 2 > *total);
            !mostly_failing
                && s.input_side_tokens >= CACHE_MIN_DENOM
                && s.cache_hit_pct.is_some_and(|p| p < CACHE_LOW_PCT)
        })
        .collect();
    flagged.sort_by_key(|s| s.cache_hit_pct.unwrap_or(0));
    flagged
        .into_iter()
        .take(2)
        .map(|s| {
            let own = providers.get(&s.agent).cloned().unwrap_or_default();
            let comparison = scorecard
                .iter()
                .filter(|o| {
                    o.agent != s.agent
                        && o.input_side_tokens >= CACHE_MIN_DENOM
                        && o.cache_hit_pct.unwrap_or(0) >= CACHE_LOW_PCT * 2
                        && providers
                            .get(&o.agent)
                            .is_some_and(|p| !p.is_disjoint(&own))
                })
                .max_by_key(|o| o.cache_hit_pct.unwrap_or(0));
            let detail = match comparison {
                Some(o) => {
                    let shared = providers
                        .get(&o.agent)
                        .and_then(|p| p.intersection(&own).next())
                        .cloned()
                        .unwrap_or_default();
                    format!(
                        "on provider `{shared}`, {} hits {}% — the difference is a stable prompt prefix",
                        o.agent.as_deref().unwrap_or("another agent"),
                        o.cache_hit_pct.unwrap_or(0),
                    )
                }
                None => "the prefix it sends must be changing between turns; system/tools churn defeats the cache".to_string(),
            };
            // Evidence: the agent's biggest input-side rows in the window —
            // those are where the missed cache actually cost.
            let mut by_context: Vec<(i64, i64)> = rows
                .iter()
                .filter(|r| r.agent == s.agent)
                .map(|r| (r.context(), r.id))
                .collect();
            by_context.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
            let evidence: Vec<i64> = by_context.into_iter().take(4).map(|(_, id)| id).collect();
            Finding {
                summary: format!(
                    "cache hit rate is {}% over {} input-side tokens",
                    s.cache_hit_pct.unwrap_or(0),
                    crate::vm::fmt_tokens(s.input_side_tokens),
                ),
                detail,
                tag: "cache".to_string(),
                agent: s.agent.clone(),
                evidence,
                evidence_span: false,
                rule: Some(
                    "Keep the prompt prefix stable across turns: do not reorder tool definitions, edit the system prompt, or inject timestamps into it."
                        .to_string(),
                ),
            }
        })
        .collect()
}

// ── the fixed-overhead rule ─────────────────────────────────────────────────

fn overhead_findings(bodies: &[BodySample]) -> Vec<Finding> {
    bodies
        .iter()
        .filter_map(|s| {
            // A truncated capture never parses, and its ratios would be
            // meaningless anyway — the CLI only samples complete bodies.
            let body: serde_json::Value = serde_json::from_str(&s.request_body).ok()?;
            // The part of the body that is the same on every turn: the tool
            // schemas and the system prompt, under whichever key the dialect
            // uses for them.
            let fixed: usize = ["tools", "system", "instructions"]
                .iter()
                .filter_map(|k| body.get(*k))
                .map(|part| serde_json::to_string(part).map(|s| s.len()).unwrap_or(0))
                .sum();
            let len = s.request_body.len().max(1);
            let share = fixed as i64 * 100 / len as i64;
            if share < OVERHEAD_MIN_PCT || fixed < OVERHEAD_MIN_BYTES {
                return None;
            }
            Some(Finding {
                summary: format!(
                    "every request carries ~{} kB of fixed tools/system payload — {share}% of the body",
                    fixed / 1024,
                ),
                detail: "it is re-sent on every turn of every session; trimming unused tools is the cheapest token win there is".to_string(),
                tag: "overhead".to_string(),
                agent: s.agent.clone(),
                evidence: vec![s.log_id],
                evidence_span: false,
                rule: Some(
                    "Trim the fixed payload: drop tools the current task does not use — every request re-sends the tool schemas."
                        .to_string(),
                ),
            })
        })
        .take(2)
        .collect()
}

// ── small math ──────────────────────────────────────────────────────────────

fn pct(part: i64, whole: i64) -> i64 {
    if whole <= 0 {
        0
    } else {
        (part as f64 * 100.0 / whole as f64).round() as i64
    }
}

fn round1(v: f64) -> f64 {
    (v * 10.0).round() / 10.0
}

fn median(mut v: Vec<f64>) -> Option<f64> {
    if v.is_empty() {
        return None;
    }
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    let m = v.len() / 2;
    if v.len() % 2 == 1 {
        Some(v[m])
    } else {
        Some((v[m - 1] + v[m]) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(id: i64, ts: &str, agent: &str) -> InsightRow {
        InsightRow {
            id,
            ts: format!("2026-09-17T{ts}Z"),
            agent: Some(agent.to_string()),
            provider_id: Some("p1".to_string()),
            model: Some("m1".to_string()),
            session_id: None,
            status_code: 200,
            error_kind: None,
            input_tokens: 0,
            output_tokens: 0,
            cache_read_tokens: 0,
            cache_creation_tokens: 0,
            reasoning_tokens: 0,
            request_size: 0,
        }
    }

    fn window() -> Window {
        Window {
            from: "2026-09-10T00:00:00Z".to_string(),
            to: "2026-09-17T00:00:00Z".to_string(),
        }
    }

    fn build(rows: &[InsightRow], bodies: &[BodySample]) -> InsightsReport {
        build_insights(7, None, window(), rows, bodies)
    }

    #[test]
    fn empty_window_is_an_honest_zero() {
        let r = build(&[], &[]);
        assert_eq!(r.totals.requests, 0);
        assert_eq!(r.totals.error_pct, 0.0);
        assert!(r.findings.is_empty());
        assert!(r.scorecard.is_empty());
    }

    #[test]
    fn retry_burst_counts_and_reports_with_evidence() {
        // One errored request, then three resends inside the minute. Without
        // a session id they group by agent+provider — the pre-session log
        // must still see the storm.
        let mut rows = vec![];
        for i in 0..4 {
            let mut r = row(i + 1, &format!("12:00:{:02}", i * 10), "codex");
            r.status_code = 500;
            r.error_kind = Some("protocol_mismatch".to_string());
            r.output_tokens = 25;
            rows.push(r);
        }
        let rep = build(&rows, &[]);
        assert_eq!(rep.scorecard[0].retries, 3);
        assert_eq!(rep.totals.errors, 4);
        let f = &rep.findings[0];
        assert_eq!(f.tag, "retry");
        assert!(f.summary.contains("3 retries in 30s"), "{f:?}");
        assert!(f.summary.contains("protocol_mismatch"), "{f:?}");
        assert!(f.detail.contains("100 output tokens"), "{f:?}");
        assert_eq!(f.evidence, vec![1, 2, 3, 4]);
        assert!(!f.evidence_span);
    }

    #[test]
    fn retries_respect_the_window_and_the_model() {
        // Same shape, but the resend lands 61s later, and the one after it
        // asks for a different model: neither is a retry.
        let mut a = row(1, "12:00:00", "codex");
        a.error_kind = Some("upstream_error".to_string());
        let mut b = row(2, "12:01:01", "codex");
        b.error_kind = Some("upstream_error".to_string());
        let mut c = row(3, "12:01:05", "codex");
        c.model = Some("m2".to_string());
        let rep = build(&[a, b, c], &[]);
        assert_eq!(rep.scorecard[0].retries, 0);
        assert!(rep.findings.is_empty(), "{:?}", rep.findings);
    }

    #[test]
    fn a_success_ends_the_burst_and_softens_the_detail() {
        let mut rows = vec![];
        for i in 0..3 {
            let mut r = row(i + 1, &format!("12:00:{:02}", i * 5), "claude");
            r.status_code = 503;
            r.error_kind = Some("upstream_error".to_string());
            rows.push(r);
        }
        let ok = row(4, "12:00:20", "claude"); // succeeded — the burst's last resend
        rows.push(ok);
        let rep = build(&rows, &[]);
        assert_eq!(rep.scorecard[0].retries, 3);
        let f = &rep.findings[0];
        // One attempt got through, so the "every attempt failed" clause —
        // which the all-error burst above does carry — must not appear.
        assert!(!f.detail.contains("every attempt"), "{f:?}");
    }

    #[test]
    fn cache_hit_rate_puts_writes_in_the_denominator() {
        // 4.3M reads against 5.4M fresh input and 0.6M writes: 42%, not the
        // 44% a writes-free denominator would claim (the Apps column's test
        // pins the same arithmetic).
        let mut rows = vec![];
        for i in 0..10 {
            let mut r = row(i + 1, &format!("12:{i:02}:00"), "claude");
            r.input_tokens = 540_000;
            r.cache_read_tokens = 430_000;
            r.cache_creation_tokens = 60_000;
            rows.push(r);
        }
        let rep = build(&rows, &[]);
        assert_eq!(rep.scorecard[0].cache_hit_pct, Some(42));
        // 42% is above the 20% floor, so the cache rule stays quiet.
        assert!(rep.findings.is_empty(), "{:?}", rep.findings);
    }

    #[test]
    fn low_hit_rate_finds_a_same_provider_comparison() {
        let mk = |agent: &str, id_base: i64, read: i64| -> Vec<InsightRow> {
            (0..10)
                .map(|i| {
                    let mut r = row(id_base + i, &format!("12:{i:02}:00"), agent);
                    r.input_tokens = 1_000;
                    r.cache_read_tokens = read;
                    r.output_tokens = 10;
                    r
                })
                .collect()
        };
        let mut rows = mk("codex", 1, 100); // 100/11_000 ≈ 9%
        rows.extend(mk("claude", 101, 9_000)); // 9000/10_000 = 90%
        let rep = build(&rows, &[]);
        let f = rep
            .findings
            .iter()
            .find(|f| f.tag == "cache")
            .expect("a cache finding");
        assert_eq!(f.agent.as_deref(), Some("codex"));
        assert!(f.summary.contains("9%"), "{f:?}");
        assert!(f.detail.contains("claude"), "{f:?}");
        assert!(f.detail.contains("90%"), "{f:?}");
        assert_eq!(f.evidence.len(), 4);
    }

    #[test]
    fn an_error_storm_earns_no_cache_finding() {
        // codex pointed at the wrong port: every request errors out, so its
        // 0% hit rate says nothing about prefix stability — those requests
        // never reached a cache. The retry rule owns that story.
        let rows: Vec<InsightRow> = (0..10)
            .map(|i| {
                let mut r = row(i + 1, &format!("12:{i:02}:00"), "codex");
                r.status_code = 500;
                r.error_kind = Some("protocol_mismatch".to_string());
                r.input_tokens = 2_000;
                r
            })
            .collect();
        let rep = build(&rows, &[]);
        assert_eq!(rep.scorecard[0].cache_hit_pct, Some(0));
        assert!(
            rep.findings.iter().all(|f| f.tag != "cache"),
            "{:?}",
            rep.findings
        );
        assert!(
            rep.findings.iter().any(|f| f.tag == "retry"),
            "{:?}",
            rep.findings
        );
    }

    #[test]
    fn a_tiny_denominator_earns_no_cache_finding() {
        // 0% hit rate, but only a few hundred tokens moved: too thin to call.
        let mut r = row(1, "12:00:00", "gemini");
        r.input_tokens = 500;
        let rep = build(&[r], &[]);
        assert_eq!(rep.scorecard[0].cache_hit_pct, Some(0));
        assert!(rep.findings.is_empty(), "{:?}", rep.findings);
    }

    #[test]
    fn a_session_that_never_falls_back_is_bloat() {
        // Five turns, doubling each time, monotonic: 16× and never compacted.
        let rows: Vec<InsightRow> = (0..5)
            .map(|i| {
                let mut r = row(i + 1, &format!("12:{i:02}:00"), "cline");
                r.session_id = Some("s-77".to_string());
                r.input_tokens = 10_000 * (1 << i);
                r
            })
            .collect();
        let rep = build(&rows, &[]);
        assert_eq!(rep.totals.sessions, 1);
        assert_eq!(rep.scorecard[0].ctx_growth, Some(16.0));
        assert_eq!(rep.top_sessions.len(), 1);
        assert_eq!(rep.top_sessions[0].growth, 16.0);
        let f = rep
            .findings
            .iter()
            .find(|f| f.tag == "bloat")
            .expect("a bloat finding");
        assert!(f.summary.contains("s-77"), "{f:?}");
        assert!(f.summary.contains("16.0×"), "{f:?}");
        assert_eq!(f.evidence, vec![1, 5]);
        assert!(f.evidence_span);
    }

    #[test]
    fn one_compaction_excuses_the_session() {
        // Same growth overall, but the context fell back in the middle:
        // that is a compact, not bloat.
        let sizes = [10_000, 40_000, 160_000, 20_000, 80_000];
        let rows: Vec<InsightRow> = sizes
            .iter()
            .enumerate()
            .map(|(i, &s)| {
                let mut r = row(i as i64 + 1, &format!("12:{i:02}:00"), "cline");
                r.session_id = Some("s-78".to_string());
                r.input_tokens = s;
                r
            })
            .collect();
        let rep = build(&rows, &[]);
        assert!(
            rep.findings.iter().all(|f| f.tag != "bloat"),
            "{:?}",
            rep.findings
        );
    }

    #[test]
    fn context_counts_cached_tokens_as_context() {
        // With caching on, `input_tokens` alone shrinks turn over turn; the
        // session's context (input-side total) is what must not fall back.
        let rows: Vec<InsightRow> = (0..5)
            .map(|i| {
                let mut r = row(i + 1, &format!("12:{i:02}:00"), "claude");
                r.session_id = Some("s-1".to_string());
                r.input_tokens = 1_000;
                r.cache_read_tokens = 9_000 * (1 << i);
                r
            })
            .collect();
        let rep = build(&rows, &[]);
        let s = &rep.top_sessions[0];
        assert_eq!(s.first_context, 10_000);
        assert_eq!(s.last_context, 145_000);
        assert_eq!(s.growth, 14.5);
    }

    #[test]
    fn reasoning_share_distinguishes_zero_from_unreported() {
        let mut with = row(1, "12:00:00", "claude");
        with.output_tokens = 1_000;
        with.reasoning_tokens = 180;
        let without = row(2, "12:01:00", "gemini"); // provider reports none
        let rep = build(&[with, without], &[]);
        let claude = rep
            .scorecard
            .iter()
            .find(|s| s.agent.as_deref() == Some("claude"))
            .unwrap();
        let gemini = rep
            .scorecard
            .iter()
            .find(|s| s.agent.as_deref() == Some("gemini"))
            .unwrap();
        assert_eq!(claude.reasoning_pct, Some(18));
        assert_eq!(gemini.reasoning_pct, None);
    }

    #[test]
    fn overhead_measures_the_fixed_part_of_a_real_body() {
        // A 40 kB tools array inside a 64 kB body: 62%, well over the floor.
        let tools = serde_json::json!([{ "name": "x", "description": "y".repeat(40_000) }]);
        let body = serde_json::json!({
            "model": "m1",
            "tools": tools,
            "messages": [{"role": "user", "content": "z".repeat(20_000)}],
        })
        .to_string();
        let sample = BodySample {
            log_id: 9,
            agent: Some("gemini".to_string()),
            request_body: body,
        };
        let r = row(9, "12:00:00", "gemini");
        let rep = build(&[r], &[sample]);
        let f = rep
            .findings
            .iter()
            .find(|f| f.tag == "overhead")
            .expect("an overhead finding");
        assert!(f.summary.contains("% of the body"), "{f:?}");
        assert_eq!(f.evidence, vec![9]);
    }

    #[test]
    fn overhead_ignores_unparseable_and_small_bodies() {
        let broken = BodySample {
            log_id: 1,
            agent: Some("codex".to_string()),
            request_body: "{\"tools\": [truncated".to_string(),
        };
        let small = BodySample {
            log_id: 2,
            agent: Some("codex".to_string()),
            // 100% fixed, but the whole body is a rounding error.
            request_body: r#"{"tools":[{"name":"x"}]}"#.to_string(),
        };
        let rep = build(&[], &[broken, small]);
        assert!(rep.findings.is_empty(), "{:?}", rep.findings);
    }

    #[test]
    fn unattributed_rows_are_an_agent_of_their_own() {
        let mut anon = row(1, "12:00:00", "x");
        anon.agent = None;
        anon.status_code = 401;
        anon.error_kind = Some("unauthorized".to_string());
        let named = row(2, "12:01:00", "codex");
        let rep = build(&[anon, named], &[]);
        assert_eq!(rep.totals.agents, 2);
        assert_eq!(rep.scorecard.len(), 2);
        assert!(rep.scorecard.iter().any(|s| s.agent.is_none()));
    }
}
