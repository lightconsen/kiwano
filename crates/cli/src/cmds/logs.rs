//! The request log: list, one detail row, and the page filter.

use super::render::{render_log_detail, render_logs};
use super::runtime;
use crate::cli::{LogFilterArgs, LogsCmd};
use crate::{CliError, Ctx};
use kiwano_core::vm;
use kiwanod::store::RequestLogFilter;

// ── logs ────────────────────────────────────────────────────────────────────

pub fn logs(cmd: &LogsCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        LogsCmd::List(args) => {
            if args.page < 1 {
                return Err(CliError::usage("--page is 1-based"));
            }
            if args.page_size < 1 {
                return Err(CliError::usage("--page-size must be positive"));
            }
            let filter = log_filter(&args.filter)?;
            let list = {
                let store = ctx.store()?;
                vm::list_request_logs(store, args.page, args.page_size, filter)?
            };
            let text = render_logs(&list);
            ctx.out.emit(&list, || text);
            Ok(())
        }
        LogsCmd::Show { id } => {
            let detail = {
                let store = ctx.store()?;
                vm::get_request_log(store, *id)?
            };
            match detail {
                Some(detail) => {
                    let text = render_log_detail(&detail);
                    ctx.out.emit(&detail, || text);
                    Ok(())
                }
                // Pruned rather than missing: the retention window drops old
                // rows, so the id genuinely existed once.
                None => Err(runtime(format!(
                    "no request {id} (it may have been pruned by the retention window)"
                ))),
            }
        }
        LogsCmd::Export(args) => {
            let filter = log_filter(&args.filter)?;
            let path = args.out.to_string_lossy();
            let report = {
                let store = ctx.store()?;
                vm::export_request_logs_csv(store, &path, filter, args.include_bodies)?
            };
            if report.truncated {
                ctx.out.note(format!(
                    "note: the result exceeded the export cap; {} rows written and the \
                     file is short of the full set",
                    report.rows_written
                ));
            }
            let text = format!(
                "wrote {} rows to {}",
                report.rows_written,
                args.out.display()
            );
            ctx.out.emit(&report, || text);
            Ok(())
        }
        LogsCmd::Clear { yes } => {
            if !*yes {
                return Err(CliError::usage(
                    "this deletes every logged request; re-run with --yes to confirm",
                ));
            }
            {
                let store = ctx.store()?;
                vm::clear_request_logs(store)?;
            }
            ctx.out.line("cleared the request log");
            Ok(())
        }
        LogsCmd::Dir => {
            let dir = kiwanod::logging::log_dir(&ctx.db);
            ctx.out.line(dir.display().to_string());
            Ok(())
        }
    }
}

fn log_filter(args: &LogFilterArgs) -> Result<RequestLogFilter<'_>, CliError> {
    // The column carries a CHECK constraint, so an unknown value would fail
    // deep in SQLite with a message about a constraint rather than about the
    // flag the user typed.
    let status = match args.status.as_deref() {
        None => None,
        Some(s @ ("ok" | "error")) => Some(s),
        Some(other) => {
            return Err(CliError::usage(format!(
                "invalid --status: {other} (ok|error)"
            )))
        }
    };
    Ok(RequestLogFilter {
        agent: args.agent.as_deref(),
        provider_id: args.provider.as_deref(),
        status,
        from: args.from.as_deref(),
        to: args.to.as_deref(),
    })
}
