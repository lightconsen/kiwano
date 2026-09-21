//! Aggregate usage over a window, per provider.
//!
//! `UsageReport` is the `--json` shape. It is `pub(crate)` only because `render`
//! prints it.

use super::render::render_usage;
use super::runtime;
use crate::cli::UsageArgs;
use crate::{CliError, Ctx};
use kiwano_core::vm;
use kiwanod::store::UsageTotals;
use std::collections::BTreeMap;

// ── usage ───────────────────────────────────────────────────────────────────

pub fn usage(args: &UsageArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    if args.days <= 0 {
        return Err(CliError::usage("--days must be a positive number"));
    }
    let since = vm::rfc3339(vm::unix_now() - args.days * 86_400);
    let (totals, by_provider, names) = {
        let store = ctx.store()?;
        let totals = store
            .usage_totals(args.agent.as_deref(), None, Some(&since))
            .map_err(runtime)?;
        let by_provider = store
            .usage_by_provider(args.agent.as_deref(), None, Some(&since))
            .map_err(runtime)?;
        let names: BTreeMap<String, String> = store
            .list_providers()
            .map_err(runtime)?
            .into_iter()
            .map(|p| (p.id, p.name))
            .collect();
        (totals, by_provider, names)
    };

    let report = UsageReport {
        days: args.days,
        agent: args.agent.clone(),
        totals,
        by_provider: by_provider
            .into_iter()
            .map(|u| {
                let name = names
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
