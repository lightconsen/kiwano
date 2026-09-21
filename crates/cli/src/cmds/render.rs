//! Rendering: view model in, the text a shell prints out.
//!
//! A layer, not a command — every command module calls into it, so the functions
//! that cross a module boundary are `pub(crate)`. The rest are private.

use super::status::render_footer;
use super::usage::UsageReport;
use crate::output::{ellipsize, fmt_amount, render_table};
use kiwano_core::detect;
use kiwano_core::insights;
use kiwano_core::sidecar;
use kiwano_core::vm;

// ── rendering ───────────────────────────────────────────────────────────────

pub(crate) fn render_status(
    v: &serde_json::Value,
    admin: &str,
    footer: Option<&vm::FooterStatsVm>,
) -> String {
    let mut out = format!(
        "gateway: running (v{}, uptime {}s, admin {admin})",
        v["version"].as_str().unwrap_or("?"),
        v["uptime_secs"].as_u64().unwrap_or(0)
    );
    // The token is what turns a liveness answer into the full report; without
    // it these fields are absent rather than zero, and printing "null" would
    // read as a store that has nothing in it.
    if v.get("providers").is_some() {
        out.push_str(&format!(
            "\nstore: providers {} · bindings {} · placeholder_keys {} · usage_rows {}",
            v["providers"], v["bindings"], v["placeholder_keys"], v["usage_rows"]
        ));
    } else {
        out.push_str(
            "\nstore: not reported (no gateway token — point --db at the shared database)",
        );
    }
    // Providers the gateway is refusing to route to right now, with its reason.
    // This is the operational answer to "why is nothing working" — the app shows
    // it as a dimmed row — and it is derived state that exists only while the
    // gateway runs, so `status` is the only place a shell can see it.
    if let Some(blocked) = v["blocked"].as_array() {
        if !blocked.is_empty() {
            out.push_str("\nblocked:");
            for row in blocked {
                out.push_str(&format!(
                    "\n  {:<28} {}",
                    ellipsize(row["provider_id"].as_str().unwrap_or("?"), 28),
                    row["reason"].as_str().unwrap_or("no reason given")
                ));
            }
        }
    }
    if let Some(routes) = v["routes"].as_array() {
        for r in routes {
            let candidates: Vec<&str> = r["candidates"]
                .as_array()
                .map(|cs| cs.iter().filter_map(|c| c["id"].as_str()).collect())
                .unwrap_or_default();
            out.push_str(&format!(
                "\n  {:<8} {:<10} primary={}",
                r["agent"].as_str().unwrap_or("?"),
                r["strategy"].as_str().unwrap_or("?"),
                r["primary_provider"].as_str().unwrap_or("-")
            ));
            if !candidates.is_empty() {
                out.push_str(&format!(" candidates={}", candidates.join(",")));
            }
        }
    }
    if let Some(footer) = footer {
        out.push('\n');
        out.push_str(&render_footer(footer));
    }
    out
}

pub(crate) fn render_providers(vms: &[vm::ProviderVm]) -> String {
    if vms.is_empty() {
        return "(no providers)".to_string();
    }
    let head = ["ID", "NAME", "PROTO", "ENDPOINT", "BILLING", "AGENTS"];
    let rows: Vec<Vec<String>> = vms
        .iter()
        .map(|p| {
            vec![
                ellipsize(&p.id, 30),
                ellipsize(&p.name, 24),
                p.protocol.clone(),
                ellipsize(&p.endpoint, 40),
                p.billing.clone(),
                p.agents
                    .iter()
                    .map(|a| {
                        // `*` marks the agent's primary, matching the app's
                        // "In use" badge.
                        if p.serving_agents.contains(a) {
                            format!("{a}*")
                        } else {
                            a.clone()
                        }
                    })
                    .collect::<Vec<_>>()
                    .join(","),
            ]
        })
        .collect();
    render_table(&head, &rows, &[])
}

pub(crate) fn render_quota(report: &kiwanod::plan_quota::PlanQuotaReport) -> String {
    let mut out = format!("{} · template {}", report.provider_id, report.template);
    if let Some(note) = &report.note {
        out.push_str(&format!("\n{note}"));
    }
    if report.tiers.is_empty() {
        out.push_str("\n(no quota windows reported)");
        return out;
    }
    for tier in &report.tiers {
        out.push_str(&format!(
            "\n  {:<14} {:>6.1}% used",
            tier.name, tier.utilization
        ));
        // Only some endpoints report absolute amounts; the rest are
        // percentage-only and would print a misleading 0/0.
        if let (Some(used), Some(limit)) = (tier.used, tier.limit) {
            out.push_str(&format!(
                "  ({used}/{limit}{})",
                tier.unit
                    .as_deref()
                    .map(|u| format!(" {u}"))
                    .unwrap_or_default()
            ));
        }
        if let Some(resets) = &tier.resets_at {
            out.push_str(&format!("  resets {resets}"));
        }
    }
    if report.cached {
        out.push_str("\n(from the 5-minute cache; --force asks again)");
    }
    out
}

pub(crate) fn render_settings(settings: &vm::SettingsVm) -> String {
    // The shape is the app's; this is the summary a shell wants. `--json`
    // carries the full object.
    let mut out = format!(
        "hub {} · preferred currency {}",
        settings.hub_url, settings.preferred_currency
    );
    out.push_str(&format!(
        "\nrequest logs {} · retention {} days · cost alerts {}",
        if settings.request_logs { "on" } else { "off" },
        settings.log_retention_days,
        if settings.cost_alert { "on" } else { "off" }
    ));
    out.push_str(&format!(
        "\nauto failover {} · auto check update {}",
        if settings.auto_failover { "on" } else { "off" },
        if settings.auto_check_update {
            "on"
        } else {
            "off"
        }
    ));
    if settings.takeovers.is_empty() {
        out.push_str("\nno agent is taken over");
    } else {
        out.push_str("\ntakeovers:");
        for t in &settings.takeovers {
            out.push_str(&format!(
                "\n  {:<16} {}{}",
                t.agent,
                if t.enabled { "routed" } else { "not routed" },
                t.placeholder_key
                    .as_deref()
                    .map(|k| format!("  {k}"))
                    .unwrap_or_default()
            ));
        }
    }
    out
}

pub(crate) fn render_catalog(catalog: &vm::CatalogListVm) -> String {
    if catalog.entries.is_empty() {
        return "(no matching catalog entries)".to_string();
    }
    let head = ["ID", "NAME", "TAG", "PROTOCOL", "ENDPOINT", "ADDED"];
    let rows: Vec<Vec<String>> = catalog
        .entries
        .iter()
        .map(|e| {
            vec![
                ellipsize(&e.id, 24),
                ellipsize(&e.name, 24),
                e.tag.clone(),
                e.protocol.clone(),
                ellipsize(&e.endpoint, 36),
                if e.added { "yes" } else { "no" }.to_string(),
            ]
        })
        .collect();
    let mut out = render_table(&head, &rows, &[]);
    out.push_str(&format!(
        "\n{} of {} entries",
        catalog.entries.len(),
        catalog.total
    ));
    out
}

pub(crate) fn render_logs(list: &vm::RequestLogListVm) -> String {
    if list.rows.is_empty() {
        return "(no matching requests)".to_string();
    }
    let head = [
        "ID", "TIME", "AGENT", "PROVIDER", "MODEL", "ST", "MS", "IN", "OUT",
    ];
    let rows: Vec<Vec<String>> = list
        .rows
        .iter()
        .map(|r| {
            vec![
                r.id.to_string(),
                r.ts.clone(),
                r.agent.clone().unwrap_or_else(|| "-".to_string()),
                r.provider_id.clone().unwrap_or_else(|| "-".to_string()),
                ellipsize(r.model.as_deref().unwrap_or("-"), 24),
                r.status_code.to_string(),
                r.latency_ms
                    .map(|ms| ms.to_string())
                    .unwrap_or_else(|| "-".to_string()),
                r.input_tokens.to_string(),
                r.output_tokens.to_string(),
            ]
        })
        .collect();
    let mut out = render_table(&head, &rows, &[0, 5, 6, 7, 8]);
    // The total, not the page length, is what tells a caller whether to keep
    // paging.
    out.push_str(&format!("\n{} of {} matching", list.rows.len(), list.total));
    out
}

pub(crate) fn render_log_detail(detail: &kiwanod::store::RequestLogDetail) -> String {
    let e = &detail.entry;
    let mut out = format!(
        "#{} {} {} {}{}",
        e.id,
        e.ts,
        e.method,
        e.path,
        e.query
            .as_deref()
            .map(|q| format!("?{q}"))
            .unwrap_or_default()
    );
    out.push_str(&format!(
        "\nstatus {} · {} ms{}",
        e.status_code,
        e.latency_ms.unwrap_or(0),
        if e.is_streaming { " · streaming" } else { "" }
    ));
    out.push_str(&format!(
        "\nagent {} · provider {} · model {}",
        e.agent.as_deref().unwrap_or("-"),
        e.provider_id.as_deref().unwrap_or("-"),
        e.model.as_deref().unwrap_or("-")
    ));
    out.push_str(&format!(
        "\ntokens in {} · out {} · cache_read {} · size {}→{}",
        e.input_tokens, e.output_tokens, e.cache_read_tokens, e.request_size, e.response_size
    ));
    if let Some(kind) = &e.error_kind {
        out.push_str(&format!(
            "\nerror {kind}: {}",
            e.error_message.as_deref().unwrap_or("(no message)")
        ));
    }
    if let Some(notes) = &e.request_notes {
        // What the compat shim changed on the way through — diagnostic text
        // the raw request body alone does not explain.
        out.push_str(&format!("\nsanitizer {notes}"));
    }
    // Bodies are the reason to open one of these; they are already redacted by
    // the capture layer.
    for (label, body) in [
        ("request", &detail.request_body),
        ("response", &detail.response_body),
    ] {
        if let Some(body) = body {
            out.push_str(&format!("\n{label} body:\n{body}"));
        }
    }
    if e.truncated {
        out.push_str("\n(one or more captured bodies were truncated)");
    }
    out
}

pub(crate) fn render_dashboard(data: &vm::DashboardVm, window: &str) -> String {
    // `—` where there is nothing to compare against — the "all" window, or an
    // earlier window with no traffic. Not "0%", which would read as "unchanged".
    let delta = |pct: Option<i64>| match pct {
        Some(p) => format!("{p}%"),
        None => "—".to_string(),
    };
    let mut out = format!(
        "{} · {} requests ({}) · cost {}",
        window,
        data.requests,
        delta(data.requests_delta_pct),
        fmt_amount(data.cost, 4)
    );
    out.push_str(&format!(
        "\ntokens in {} · out {} · cache_read {}",
        vm::fmt_tokens(data.input_tokens),
        vm::fmt_tokens(data.output_tokens),
        vm::fmt_tokens(data.cache_read_tokens)
    ));
    // The peak premium, only when there is one: most models publish a single
    // price, and a permanent "off-peak 0.0000" would read as a bug.
    let premium = data.cost - data.cost_off_peak;
    if premium > 0.0 {
        out.push_str(&format!(
            "\nof which {} was the peak premium (off-peak: {})",
            fmt_amount(premium, 4),
            fmt_amount(data.cost_off_peak, 4)
        ));
    }
    out.push_str(&format!(
        "\navg latency {} ms ({})",
        data.latency_ms,
        delta(data.latency_delta_pct)
    ));
    // The two splits, as tables: they are columns of numbers with a name in
    // front, which is what a table is for. The headline lines above stay prose —
    // one value per line has no columns to line up.
    if !data.by_provider.is_empty() {
        let rows: Vec<Vec<String>> = data
            .by_provider
            .iter()
            .map(|p| {
                vec![
                    p.name.clone(),
                    p.requests.to_string(),
                    format!("{}%", p.pct),
                    fmt_amount(p.cost, 4),
                ]
            })
            .collect();
        out.push('\n');
        out.push_str(&render_table(
            &["PROVIDER", "REQUESTS", "SHARE", "COST"],
            &rows,
            // Requests, share and cost are numbers; the name is not.
            &[1, 2, 3],
        ));
    }
    if !data.by_agent.is_empty() {
        let rows: Vec<Vec<String>> = data
            .by_agent
            .iter()
            .map(|a| {
                vec![
                    a.label.clone(),
                    a.requests.to_string(),
                    // Already formatted by the view model, unlike the provider
                    // split's raw counts.
                    a.tokens.clone(),
                    fmt_amount(a.cost, 4),
                ]
            })
            .collect();
        out.push('\n');
        out.push_str(&render_table(
            &["AGENT", "REQUESTS", "TOKENS", "COST"],
            &rows,
            &[1, 2, 3],
        ));
    }
    out
}

pub(crate) fn render_alerts(alerts: &[vm::UsageAlertVm]) -> String {
    if alerts.is_empty() {
        return "(nothing over its allowance)".to_string();
    }
    let head = ["PROVIDER", "USED", "LIMIT", "UNIT"];
    let rows: Vec<Vec<String>> = alerts
        .iter()
        .map(|a| {
            vec![
                ellipsize(&a.provider_name, 24),
                fmt_amount(a.used, 2),
                fmt_amount(a.limit, 2),
                a.unit.clone(),
            ]
        })
        .collect();
    render_table(&head, &rows, &[1, 2])
}

pub(crate) fn render_probe(report: &sidecar::ProbeReport) -> String {
    let mut out = format!("{} ({} ms)", report.verdict, report.latency_ms);
    if let Some(status) = report.status {
        out.push_str(&format!(" · HTTP {status}"));
    }
    if !report.detail.is_empty() {
        out.push_str(&format!("\n{}", report.detail));
    }
    out
}

pub(crate) fn render_routes(routes: &[vm::AgentRouteVm]) -> String {
    if routes.is_empty() {
        return "(no agent has a route; take one over first)".to_string();
    }
    let mut out = String::new();
    for (i, route) in routes.iter().enumerate() {
        if i > 0 {
            out.push('\n');
        }
        out.push_str(&format!("{} · {}", route.agent, route.strategy));
        if let Some(config) = &route.config {
            out.push_str(&format!(" {config}"));
        }
        for (n, b) in route.bindings.iter().enumerate() {
            let marker = if n == 0 { "→" } else { " " };
            out.push_str(&format!(
                "\n  {marker} {:<28} {}",
                ellipsize(&b.provider_id, 28),
                b.provider_name
            ));
            // Only surfaced when they mean something: a weight on a strategy
            // that ignores it, or a window that is not set, is noise.
            if route.strategy == "roundrobin" {
                out.push_str(&format!("  weight={}", b.weight));
            }
            if let (Some(start), Some(end)) = (&b.win_start, &b.win_end) {
                out.push_str(&format!("  {start}-{end}"));
            }
        }
    }
    out
}

pub(crate) fn render_agents(found: &[detect::AgentDetectVm]) -> String {
    if found.is_empty() {
        return "(no agents known)".to_string();
    }
    let head = ["AGENT", "INSTALLED", "PATH"];
    let rows: Vec<Vec<String>> = found
        .iter()
        .map(|a| {
            vec![
                a.agent.clone(),
                if a.installed { "yes" } else { "no" }.to_string(),
                a.path.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&head, &rows, &[])
}

pub(crate) fn render_versions(versions: &[detect::AgentVersionVm]) -> String {
    let head = ["AGENT", "VERSION"];
    let rows: Vec<Vec<String>> = versions
        .iter()
        .map(|v| {
            vec![
                v.agent.clone(),
                v.version.clone().unwrap_or_else(|| "-".to_string()),
            ]
        })
        .collect();
    render_table(&head, &rows, &[])
}

pub(crate) fn render_keys(keys: &[vm::ApiKeyVm], provider_id: &str) -> String {
    if keys.is_empty() {
        return format!("(no rotation keys for {provider_id})");
    }
    let head = ["ID", "KEY", "LABEL", "CREATED"];
    let rows: Vec<Vec<String>> = keys
        .iter()
        .map(|k| {
            vec![
                k.id.to_string(),
                k.masked.clone(),
                k.label.clone().unwrap_or_else(|| "-".to_string()),
                k.created_at.clone(),
            ]
        })
        .collect();
    render_table(&head, &rows, &[])
}

pub(crate) fn render_usage(report: &UsageReport) -> String {
    let mut out = format!(
        "usage ({}d{})",
        report.days,
        report
            .agent
            .as_deref()
            .map(|a| format!(", {a}"))
            .unwrap_or_default()
    );
    out.push_str(&format!(
        "\nrequests {} · input {} · output {} · cache_read {}",
        report.totals.requests,
        vm::fmt_tokens(report.totals.input_tokens),
        vm::fmt_tokens(report.totals.output_tokens),
        vm::fmt_tokens(report.totals.cache_read_tokens)
    ));
    if report.by_provider.is_empty() {
        out.push_str("\n(no usage in window)");
        return out;
    }
    // A table, like the dashboard's splits: same shape of data, same rendering.
    // Token counts are formatted (`1.9M`), which is still a number column — it
    // is read by its right edge.
    let rows: Vec<Vec<String>> = report
        .by_provider
        .iter()
        .map(|(provider_id, name, totals)| {
            vec![
                if name.is_empty() {
                    provider_id.clone()
                } else {
                    name.clone()
                },
                totals.requests.to_string(),
                vm::fmt_tokens(totals.input_tokens),
                vm::fmt_tokens(totals.output_tokens),
            ]
        })
        .collect();
    out.push('\n');
    out.push_str(&render_table(
        &["PROVIDER", "REQUESTS", "IN", "OUT"],
        &rows,
        &[1, 2, 3],
    ));
    out
}

/// The insights report: title line, totals, scorecard with the metric
/// definitions under it, findings with evidence ids, the sessions whose
/// context grew the most, and the privacy footer. No amounts anywhere —
/// money is the dashboard's business, this page is about tokens.
pub(crate) fn render_insights(report: &insights::InsightsReport) -> String {
    fn date(ts: &str) -> &str {
        ts.get(..10).unwrap_or(ts)
    }
    let t = &report.totals;
    let mut out = format!(
        "Kiwano insights · {} → {} · local only",
        date(&report.window.from),
        date(&report.window.to)
    );
    out.push_str(&format!(
        "\n{} requests · {} sessions · {} agents · {} errors ({:.1}%)",
        t.requests, t.sessions, t.agents, t.errors, t.error_pct
    ));
    if t.requests == 0 {
        out.push_str("\n(no requests in window)");
        return out;
    }
    out.push_str(&scorecard_section(report));
    out.push_str(&findings_section(report));
    out.push_str(&top_sessions_section(report));
    out.push_str(
        "\n\nAll statistics aggregate request_logs locally; bodies and keys never leave this machine.",
    );
    out
}

/// `–` where a rate was never reported: not `0%`, which would read as a
/// measurement rather than an absence.
fn dash() -> String {
    "–".to_string()
}

/// The scorecard table and the four denominators behind its rates. The
/// denominators are printed rather than left to guesswork: a rate whose formula
/// the reader has to assume is an invitation to misread it.
fn scorecard_section(report: &insights::InsightsReport) -> String {
    let rows: Vec<Vec<String>> = report
        .scorecard
        .iter()
        .map(|s| {
            vec![
                s.agent.clone().unwrap_or_else(dash),
                s.requests.to_string(),
                s.sessions.to_string(),
                s.cache_hit_pct
                    .map(|p| format!("{p}%"))
                    .unwrap_or_else(dash),
                s.ctx_growth
                    .map(|g| format!("{g:.1}×"))
                    .unwrap_or_else(dash),
                s.reasoning_pct
                    .map(|p| format!("{p}%"))
                    .unwrap_or_else(dash),
                s.retries.to_string(),
            ]
        })
        .collect();
    let mut out = String::from("\n\nScorecard");
    out.push('\n');
    out.push_str(&render_table(
        &[
            "AGENT",
            "REQS",
            "SESS",
            "CACHE HIT",
            "CTX GROWTH",
            "REASONING",
            "RETRIES",
        ],
        &rows,
        &[1, 2, 3, 4, 5, 6],
    ));
    out.push_str("\n  cache hit  = cache_read / (input + cache_read + cache_creation)");
    out.push_str("\n  ctx growth = median last-turn / first-turn context across sessions");
    out.push_str("\n  reasoning  = reasoning / output tokens; – means never reported");
    out.push_str("\n  retries    = resends within 60s of an errored request, same session + model");
    out
}

/// The findings, numbered. Continuation lines align under the sentence, not
/// under the number: each finding reads as one block per rule.
fn findings_section(report: &insights::InsightsReport) -> String {
    let mut out = String::from("\n\nFindings");
    if report.findings.is_empty() {
        out.push_str("\n(no findings in this window)");
        return out;
    }
    for (i, f) in report.findings.iter().enumerate() {
        let prefix = format!("{}. [{}] ", i + 1, f.tag);
        let agent = f
            .agent
            .as_deref()
            .map(|a| format!("{a}: "))
            .unwrap_or_default();
        out.push_str(&format!("\n{prefix}{agent}{}", f.summary));
        let pad = " ".repeat(prefix.len());
        if !f.detail.is_empty() {
            out.push_str(&format!("\n{pad}{}", f.detail));
        }
        if !f.evidence.is_empty() {
            out.push_str(&format!("\n{pad}evidence {}", render_evidence(f)));
        }
    }
    out
}

/// The sessions whose context grew the most — nothing at all when the window
/// held none, so the caller can append it unconditionally.
fn top_sessions_section(report: &insights::InsightsReport) -> String {
    if report.top_sessions.is_empty() {
        return String::new();
    }
    let rows: Vec<Vec<String>> = report
        .top_sessions
        .iter()
        .map(|s| {
            vec![
                s.session_id.clone(),
                s.agent.clone().unwrap_or_else(dash),
                s.turns.to_string(),
                format!(
                    "{} → {}",
                    vm::fmt_tokens(s.first_context),
                    vm::fmt_tokens(s.last_context)
                ),
                format!("{:.1}×", s.growth),
            ]
        })
        .collect();
    let mut out = String::from("\n\nTop sessions by context growth");
    out.push('\n');
    out.push_str(&render_table(
        &[
            "SESSION",
            "AGENT",
            "TURNS",
            "FIRST → LAST CONTEXT",
            "GROWTH",
        ],
        &rows,
        &[2, 4],
    ));
    out
}

/// `#a #b #c` — or `#a … #z` when the ids name the ends of a range, or a
/// trailing `…` when the list was capped. The id is the way back to the row:
/// `kiwano logs show <id>`.
fn render_evidence(f: &insights::Finding) -> String {
    let ids: Vec<String> = f.evidence.iter().map(|id| format!("#{id}")).collect();
    if f.evidence_span && ids.len() == 2 {
        format!("{} … {}", ids[0], ids[1])
    } else if f.evidence_span {
        format!("{} …", ids.join(" "))
    } else {
        ids.join(" ")
    }
}

#[cfg(test)]
mod tests {
    use super::render_status;

    /// `status` is the only place a shell can see the gateway's blocked list:
    /// it is derived state that exists only while the gateway is running, so
    /// there is nothing in SQLite to read it from.
    #[test]
    fn status_text_lists_blocked_providers() {
        let report = serde_json::json!({
            "version": "0.1.8",
            "uptime_secs": 12,
            "providers": 2,
            "bindings": 1,
            "placeholder_keys": 1,
            "usage_rows": 5,
            "blocked": [
                { "provider_id": "capped-1", "reason": "1.00 of 1.00 requests this period" }
            ],
            "routes": []
        });
        let text = render_status(&report, "/tmp/admin.sock", None);
        assert!(text.contains("blocked:"), "{text}");
        assert!(text.contains("capped-1"), "{text}");
        assert!(
            text.contains("1.00 of 1.00 requests this period"),
            "the gateway's reason is the whole point: {text}"
        );
    }

    /// A healthy gateway must not carry a heading with no rows under it, and a
    /// liveness-only answer (no token) has no such key at all.
    #[test]
    fn status_text_stays_quiet_when_nothing_is_blocked() {
        let empty = serde_json::json!({
            "version": "0.1.8", "uptime_secs": 1, "blocked": []
        });
        assert!(!render_status(&empty, "s", None).contains("blocked"));

        let liveness = serde_json::json!({ "version": "0.1.8", "uptime_secs": 1 });
        assert!(!render_status(&liveness, "s", None).contains("blocked"));
    }
}
