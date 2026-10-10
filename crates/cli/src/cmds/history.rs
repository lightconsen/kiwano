//! The agents' own history: read their session files, hand the rows over.
//!
//! The reading is the client's — the files are in the user's home, and the daemon
//! is not allowed to touch them (`migrate.local.md` §5 #1/#2). What this command
//! adds is the *shape* of the transaction: read, split into request-sized
//! batches, send each one, and add the reports up. It is also the way to see what
//! a scan found without importing anything (`--dry-run`), which is the only
//! honest answer to "why is my dashboard still empty?" — the files may simply not
//! be where this machine expects them.

use super::render::{render_history_import, render_sessions};
use super::runtime;
use crate::cli::HistoryCmd;
use crate::{CliError, Ctx};
use kiwano_core::history::{self, HistoryRead};

/// Every agent a reader exists for.
///
/// Named here rather than discovered: a reader is code — a file format parsed by
/// hand — so the list of what can be read is a fact about this build, not about
/// what happens to be installed.
const READABLE: [&str; 2] = ["claude", "codex"];

pub fn history(cmd: &HistoryCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        HistoryCmd::Import { agent, dry_run } => import(agent, *dry_run, ctx),
        HistoryCmd::Sessions {
            project,
            agent,
            days,
        } => {
            let since = days.map(|d| {
                kiwano_core::vm::time::rfc3339(kiwano_core::vm::unix_now() - d.max(0) * 86_400)
            });
            let rows =
                ctx.api
                    .list_sessions(agent.as_deref(), project.as_deref(), since.as_deref())?;
            let scanned = ctx.api.history_scanned_at().unwrap_or(None);
            let text = render_sessions(&rows);
            let text = match scanned {
                Some(at) => format!("{text}last scan: {at}\n"),
                None => format!("{text}never scanned: run `kiwano history import`\n"),
            };
            ctx.out.emit(&rows, || text);
            Ok(())
        }
    }
}

fn import(agents: &[String], dry_run: bool, ctx: &mut Ctx) -> Result<(), CliError> {
    let wanted: Vec<String> = if agents.is_empty() {
        READABLE.iter().map(|a| (*a).to_string()).collect()
    } else {
        for a in agents {
            if !READABLE.contains(&a.as_str()) {
                return Err(runtime(format!(
                    "no reader for `{a}`: this build reads {}",
                    READABLE.join(", ")
                )));
            }
        }
        agents.to_vec()
    };

    let HistoryRead {
        batch,
        detail,
        skips,
    } = history::read_history(&ctx.home, ctx.config_vars(), &wanted);
    // Detail and skips are the client's own account of *this machine* — which
    // roots were looked at, which files answered, which could not be opened —
    // and they are diagnostics, so they go to stderr. Under `--json` stdout has
    // to be one document and nothing else, and a scan is exactly the command a
    // caller pipes into `jq`.
    for line in detail.iter().chain(skips.iter()) {
        ctx.out.note(line.clone());
    }

    if dry_run {
        // "read", not "would import": this side counts what the *files* hold, and
        // the daemon then subtracts everything it already metered (the per-agent
        // watermark). On a machine that has been routing through the gateway for
        // months, most of what is read here is skipped there — so a number that
        // promised an import would be wrong in the direction that matters.
        let text = format!(
            "read {} row(s) and {} session(s); the gateway skips whatever it already metered",
            batch.usage.len(),
            batch.sessions.len()
        );
        ctx.out.emit(&batch, || text);
        return Ok(());
    }

    if batch.usage.is_empty() && batch.sessions.is_empty() {
        // A payload line, not a note: "there was nothing to read" is the answer
        // to the question this command was asked.
        ctx.out.emit(&batch, || "nothing to import".to_string());
        return Ok(());
    }

    let mut total = kiwano_core::vm::HistoryImportReport::default();
    let batches = history::chunks(&batch, history::MAX_ROWS_PER_REQUEST);
    for piece in batches {
        let report = ctx.api.import_history(&piece)?;
        total.add(report);
    }
    let text = render_history_import(&total, batch.usage.len());
    ctx.out.emit(&total, || text);
    Ok(())
}
