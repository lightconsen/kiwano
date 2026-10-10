//! `kiwano sessions` — one list of sessions, from both ledgers.
//!
//! The gateway has two notions of "session" and this is the command that shows
//! both. The **traffic** side is what the gateway itself routed
//! (`request_logs.session_id`): the exact requests, tokens and cost. The
//! **imported** side is what the agents' own files say, read by
//! `kiwano history import`: the project, the turns and the tools, which the
//! gateway never sees. A row says which side it came from, and the two are never
//! added (they describe overlapping-but-not-equal work — a retry is one file
//! turn and several traffic rows); the merged shape and its rules live in
//! `kiwano_api::sessions::SessionVm`.
//!
//! It replaces the old `history sessions`, which showed the imported half alone:
//! a session that ran entirely through the gateway was invisible there, and one
//! that predated the takeover showed no cost.

use super::render::render_sessions;
use crate::cli::SessionsArgs;
use crate::{CliError, Ctx};

pub fn sessions(args: &SessionsArgs, ctx: &mut Ctx) -> Result<(), CliError> {
    let rows = ctx
        .api
        .sessions(args.agent.as_deref(), args.project.as_deref(), args.days)?;
    let text = render_sessions(&rows);
    // The scan stamps are text-only, on purpose: they are a note about the
    // *imported* half, and under `--json` stdout has to be the rows and nothing
    // else. Listed per agent, because "which agents have been read" is the
    // question — one that a single "last scan" line answered for the wrong
    // subject. The leading newline is not decoration: `render_table` returns no
    // trailing one, so a footer appended without it lands on the table's own
    // bottom border.
    let scans = ctx.api.history_scans().unwrap_or_default();
    let mut scanned: Vec<(&String, &String)> = scans.iter().collect();
    scanned.sort();
    let text = if scanned.is_empty() {
        format!("{text}\nnever scanned: run `kiwano history import`\n")
    } else {
        let listed = scanned
            .iter()
            .map(|(agent, at)| format!("{agent} {at}"))
            .collect::<Vec<_>>()
            .join(" · ");
        format!("{text}\nscanned: {listed}\n")
    };
    ctx.out.emit(&rows, || text);
    Ok(())
}
