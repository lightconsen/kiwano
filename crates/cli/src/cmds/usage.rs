//! Aggregate usage over a window, per provider.
//!
//! `UsageReport` is the `--json` shape. It is `pub(crate)` only because `render`
//! prints it.

use super::render::render_usage;
use crate::cli::UsageArgs;
use crate::{CliError, Ctx};
use kiwanod::store::UsageTotals;

// ── usage ───────────────────────────────────────────────────────────────────

pub fn usage(args: &UsageArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    if args.days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    // The whole window in one answer: the daemon draws it from its own clock and
    // aggregates it, and this side only decides how to lay it out
    // (`migrate.local.md` §10.45).
    let view = ctx.api.usage_report(args.days, args.agent.as_deref())?;
    let report = UsageReport {
        days: args.days,
        agent: args.agent.clone(),
        totals: view.totals,
        by_provider: view
            .by_provider
            .into_iter()
            .map(|u| {
                let name = view
                    .names
                    .get(&u.provider_id)
                    .cloned()
                    .unwrap_or_else(|| u.provider_id.clone());
                (u.provider_id, name, u.totals)
            })
            .collect(),
    };
    let text = render_usage(&report);
    ctx.out.emit(&report, || text);
    Ok(())
}

/// The `--json` shape of `usage`. Field names are the CLI's own; the app has no
/// equivalent object because its usage surface is the dashboard.
#[derive(serde::Serialize)]
pub(crate) struct UsageReport {
    pub(crate) days: i64,
    pub(crate) agent: Option<String>,
    pub(crate) totals: UsageTotals,
    pub(crate) by_provider: Vec<(String, String, UsageTotals)>,
}
