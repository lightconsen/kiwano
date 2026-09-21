//! The insights report: findings, evidence, tuning advice.
//!
//! `build_insights_report` is the one entry point `mcp` and `rules` share, so it is
//! `pub(crate)` rather than private.

use super::render::render_insights;
use super::runtime;
use crate::cli::InsightsArgs;
use crate::{CliError, Ctx};
use kiwano_core::insights;
use kiwano_core::vm;
use kiwanod::store::{RequestLogEntry, RequestLogFilter, Store};
use std::collections::BTreeMap;

// ── insights ────────────────────────────────────────────────────────────────

pub fn insights(args: &InsightsArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    if args.days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let tuning = vm::ui_settings(ctx.aux()?).feat_tuning_advice;
    let report = build_insights_report(ctx.store()?, args.days, args.agent.clone(), tuning)?;
    let text = render_insights(&report);
    ctx.out.emit(&report, || text);
    Ok(())
}

/// The report both `insights` and the MCP server answer from. The export read
/// is the one uncapped-by-page path over request_logs; insights wants every
/// row in the window, newest-first order included (the core builder sorts for
/// itself). At the cap the report covers the newest EXPORT_ROW_CAP rows — the
/// same honest ceiling the CSV export has.
pub(crate) fn build_insights_report(
    store: &Store,
    days: i64,
    agent: Option<String>,
    tuning_advice: bool,
) -> Result<insights::InsightsReport, CliError> {
    let now = vm::unix_now();
    let window = insights::Window {
        from: vm::rfc3339(now - days * 86_400),
        to: vm::rfc3339(now),
    };
    let entries = store
        .export_request_logs(
            RequestLogFilter {
                agent: agent.as_deref(),
                from: Some(&window.from),
                to: Some(&window.to),
                ..Default::default()
            },
            kiwanod::store::EXPORT_ROW_CAP,
        )
        .map_err(runtime)?;
    let bodies = sample_insight_bodies(store, &entries)?;
    let rows: Vec<insights::InsightRow> = entries.iter().map(insight_row_of).collect();
    Ok(insights::build_insights(
        days,
        agent,
        window,
        &rows,
        &bodies,
        tuning_advice,
    ))
}

fn insight_row_of(e: &RequestLogEntry) -> insights::InsightRow {
    insights::InsightRow {
        id: e.id,
        ts: e.ts.clone(),
        agent: e.agent.clone(),
        provider_id: e.provider_id.clone(),
        model: e.model.clone(),
        session_id: e.session_id.clone(),
        status_code: e.status_code,
        error_kind: e.error_kind.clone(),
        input_tokens: e.input_tokens,
        output_tokens: e.output_tokens,
        cache_read_tokens: e.cache_read_tokens,
        cache_creation_tokens: e.cache_creation_tokens,
        reasoning_tokens: e.reasoning_tokens,
        request_size: e.request_size,
    }
}

/// The newest untruncated body per agent, for the busiest four agents. The
/// overhead rule parses JSON, so a truncated capture is skipped: it would not
/// parse, and its ratios would be meaningless even if it did. Bodies stay
/// inside this process — only the measurement reaches the report.
fn sample_insight_bodies(
    store: &Store,
    entries: &[RequestLogEntry],
) -> Result<Vec<insights::BodySample>, CliError> {
    // `entries` arrive newest-first, so the first sighting of an agent is
    // already its most recent row.
    let mut per_agent: BTreeMap<Option<String>, (i64, i64)> = BTreeMap::new();
    for e in entries {
        let slot = per_agent.entry(e.agent.clone()).or_insert((0, e.id));
        slot.0 += 1;
    }
    let mut agents: Vec<(Option<String>, (i64, i64))> = per_agent.into_iter().collect();
    agents.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));

    let mut samples = Vec::new();
    for (agent, (_, latest_id)) in agents.into_iter().take(4) {
        let Some(detail) = store.get_request_log(latest_id).map_err(runtime)? else {
            continue;
        };
        if detail.entry.truncated {
            continue;
        }
        let Some(request_body) = detail.request_body else {
            continue;
        };
        samples.push(insights::BodySample {
            log_id: latest_id,
            agent,
            request_body,
        });
    }
    Ok(samples)
}
