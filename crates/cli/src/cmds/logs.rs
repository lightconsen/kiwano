//! The request log: list, one detail row, and the page filter.

use super::render::{render_log_detail, render_logs};
use super::runtime;
use crate::cli::{LogFilterArgs, LogsCmd, LogsRedactArgs};
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
            let list = ctx
                .api
                .list_request_logs(args.page, args.page_size, filter)?;
            let text = render_logs(&list);
            ctx.out.emit(&list, || text);
            Ok(())
        }
        LogsCmd::Show { id } => {
            let detail = ctx.api.get_request_log(*id)?;
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
            // The rows come over the wire; the **file** is written here, because
            // the path is this machine's (a `--out` the user typed).
            let (csv, written, truncated) = ctx.api.export_request_logs(filter)?;
            let report = vm::write_request_log_export(&path, &csv, written, truncated)?;
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
            ctx.api.clear_request_logs()?;
            ctx.out.line("cleared the request log");
            Ok(())
        }
        LogsCmd::Redact(args) => logs_redact(args, ctx),
        // Stays local on purpose: this is the log directory on **this** machine,
        // derived from where this invocation keeps its database. A remote daemon
        // writes its logs where *it* lives, and there is no endpoint for a path
        // (§5 #1: the interface does not carry paths).
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
        session_id: args.session.as_deref(),
    })
}

/// `logs redact` — drop captured bodies that carry a secret, keep the rows.
fn logs_redact(args: &LogsRedactArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    // The literal comes from a file, and the file is **not** echoed anywhere:
    // the whole purpose is that the string stops appearing in new places. What
    // the user gets back is a count.
    let literal = match &args.literal_file {
        Some(path) => Some(
            std::fs::read_to_string(path)
                .map_err(|e| runtime(format!("cannot read {}: {e}", path.display())))?
                .trim()
                .to_string(),
        ),
        None => None,
    };
    if args.ids.is_empty() && literal.is_none() {
        return Err(CliError::usage(
            "name what to redact: --id <N> (repeatable) or --literal-file <PATH>",
        ));
    }
    if !args.yes && !args.dry_run {
        return Err(CliError::usage(
            "this removes captured bodies for good; re-run with --yes to confirm \
             (or --dry-run to see what it would touch)",
        ));
    }
    let report = ctx
        .api
        .redact_log_bodies(&args.ids, literal.as_deref(), args.dry_run)?;
    let verb = if args.dry_run {
        "would redact"
    } else {
        "redacted"
    };
    ctx.out.line(format!(
        "{verb} the bodies of {} of {} matching rows",
        report.redacted, report.matched
    ));
    Ok(())
}
