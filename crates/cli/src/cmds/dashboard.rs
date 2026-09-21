//! The dashboard and the usage alerts.
//!
//! `alerts` reads what `dashboard` computes, so they share a module.

use super::render::{render_alerts, render_dashboard};
use crate::cli::DashboardArgs;
use crate::{CliError, Ctx};
use kiwano_core::vm;

// ── dashboard / alerts ──────────────────────────────────────────────────────

pub fn dashboard(args: &DashboardArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let window = match args.window.as_str() {
        w @ ("today" | "7d" | "30d" | "all") => w,
        other => {
            return Err(CliError::usage(format!(
                "invalid --window: {other} (today|7d|30d|all)"
            )))
        }
    };
    let data = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::build_dashboard(
            store,
            aux,
            window,
            args.provider.as_deref(),
            args.agent.as_deref(),
        )?
    };
    let text = render_dashboard(&data, window);
    ctx.out.emit(&data, || text);
    Ok(())
}

/// Alerts are read-only unless `--mark-notified`, because the dedup key they
/// consult is the same one the app uses to decide whether to raise a desktop
/// notification. A cron poll consuming it would silence the alert the user was
/// waiting for.
pub fn alerts(mark_notified: bool, ctx: &mut Ctx) -> Result<(), CliError> {
    let alerts = {
        let (store, aux) = (ctx.store()?, ctx.aux()?);
        vm::check_usage_alerts(store, aux, mark_notified)?
    };
    if alerts.is_empty() {
        // Not an error: "nothing is over budget" is the answer to the question.
        ctx.out.note("no provider is over its allowance");
    }
    let text = render_alerts(&alerts);
    ctx.out.emit(&alerts, || text);
    Ok(())
}
