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
    // The scan stamp is text-only, on purpose: it is a note about the *imported*
    // half, and under `--json` stdout has to be the rows and nothing else.
    let text = match ctx.api.history_scanned_at().unwrap_or(None) {
        Some(at) => format!("{text}last scan: {at}\n"),
        None => format!("{text}never scanned: run `kiwano history import`\n"),
    };
    ctx.out.emit(&rows, || text);
    Ok(())
}
