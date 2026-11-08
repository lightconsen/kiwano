//! `kiwano db` — what the store holds, and reclaiming the space it no longer uses.
//!
//! One writer: the CLI never opens the daemon's database (`tests/one_writer.rs`),
//! so what it can do here is ask. `stats` is an ordinary admin-plane read.
//!
//! `vacuum` is the exception the rule allows, because it is the operation that
//! *requires* the daemon to be gone: compacting a SQLite file needs an exclusive
//! lock. With no daemon there is no admin plane to ask, so this process does the
//! work — through `kiwano_core::db_file`, which owns the SQL and refuses cleanly
//! when the file turns out to be held anyway.

use crate::cli::DbCmd;
use crate::CliError;
use crate::Ctx;
use kiwano_core::db_file::{self, VacuumError};

pub fn db(cmd: &DbCmd, ctx: &mut Ctx) -> Result<(), CliError> {
    match cmd {
        DbCmd::Stats => stats(ctx),
        DbCmd::Vacuum { dry_run } => vacuum(*dry_run, ctx),
    }
}

fn stats(ctx: &mut Ctx) -> Result<(), CliError> {
    let stats: serde_json::Value = ctx
        .api
        .db_stats()
        .map_err(|e| CliError::runtime(format!("{e} — is the gateway running?")))?;

    // The file's footprint is a fact about this disk, so the client can read it
    // without touching the database: `metadata` opens no connection.
    let file_bytes = db_file::footprint(&ctx.db);
    let n = |k: &str| stats.get(k).and_then(|v| v.as_i64()).unwrap_or(0);
    let pending = n("pending_backfill");

    let text = format!(
        "database  {}\n\
         file      {}\n\
         in use    {}\n\
         free      {}  (deleted rows waiting to be reused)\n\
         \n\
         bodies    {} chunks, {}\n\
         backfill  {}\n",
        ctx.db.display(),
        human(file_bytes),
        human(n("used_bytes") as u64),
        human(n("free_bytes") as u64),
        n("body_chunks"),
        human(n("body_chunk_bytes") as u64),
        if pending == 0 {
            "done".to_string()
        } else {
            format!("{pending} rows still stored as text")
        },
    );
    ctx.out.emit(&stats, || text);
    Ok(())
}

fn vacuum(dry_run: bool, ctx: &mut Ctx) -> Result<(), CliError> {
    if !ctx.db.exists() {
        return Err(CliError::usage(format!(
            "no database at {}",
            ctx.db.display()
        )));
    }

    // Ask the daemon first: if it answers, it is running, and a live daemon must
    // never have its file rewritten underneath it. This is the check that
    // actually holds — the file lock is only a fallback, because WAL lets a
    // second process take the write lock and VACUUM "succeed" against a daemon
    // that is still serving.
    if ctx.api.db_stats().is_ok() {
        return Err(CliError {
            message: "the gateway is running and is using the database — stop it first \
                      (`kiwano gateway stop`), then run this again"
                .to_string(),
            code: crate::EXIT_NEGATIVE,
        });
    }

    let map = |e: VacuumError| match e {
        // "Stop the gateway" is an instruction, not a failure of the command:
        // the exit code says to the caller that nothing was attempted.
        VacuumError::Busy(msg) => CliError {
            message: msg,
            code: crate::EXIT_NEGATIVE,
        },
        other => CliError::runtime(other.to_string()),
    };

    if dry_run {
        let size = db_file::reclaimable(&ctx.db).map_err(map)?;
        let text = format!(
            "file now     {}\n\
             reclaimable  {}\n\
             after        ~{}\n",
            human(size.file_bytes),
            human(size.reclaimable_bytes),
            human(size.file_bytes.saturating_sub(size.reclaimable_bytes)),
        );
        ctx.out.emit(&size, || text);
        return Ok(());
    }

    // Reading the reclaimable size first is what stops a no-op compaction from
    // rewriting a database that has nothing to give back.
    let before = db_file::reclaimable(&ctx.db).map_err(map)?;
    if before.reclaimable_bytes == 0 {
        ctx.out
            .note("nothing to reclaim: no page has been freed since the last compaction");
        return Ok(());
    }

    ctx.out.note(format!(
        "compacting {} — this rewrites the file and needs room for a second copy",
        ctx.db.display()
    ));
    let after = db_file::compact(&ctx.db).map_err(map)?;
    let text = format!(
        "before  {}\n\
         after   {}\n\
         freed   {}\n",
        human(before.file_bytes),
        human(after.file_bytes),
        human(before.file_bytes.saturating_sub(after.file_bytes)),
    );
    ctx.out.emit(
        &serde_json::json!({
            "before_bytes": before.file_bytes,
            "after_bytes": after.file_bytes,
            "freed_bytes": before.file_bytes.saturating_sub(after.file_bytes),
        }),
        || text,
    );
    Ok(())
}

fn human(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{v:.1} {}", UNITS[unit])
    }
}
