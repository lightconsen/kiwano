//! The agents' own history, backfilled once on first launch — and on demand
//! from `history_import`, the command a future "scan now" button will call.
//!
//! This is the app's thin half of `kiwano_core::history`: the reader lives there
//! (shared with the CLI), the daemon owns the store and the watermark, and what
//! is left here is *when* to run the scan and how to keep it from mattering.
//!
//! It is a **backfill, not a sync**. It reads what predates the gateway and
//! never re-reads what the gateway already metered — the daemon's per-agent
//! watermark is that boundary, and the row keys make a second run an upsert
//! rather than a duplicate. So this does not "keep the dashboard up to date":
//! it fills in the history that came before Kiwano, once.
//!
//! Everything here is best-effort. The scan is an enhancement — a dashboard is
//! only emptier without it — so no failure reaches startup: the home is checked
//! for before it is asked for (the resolver panics, and this build cannot catch
//! a panic), and every other error is logged and dropped.

use std::path::PathBuf;

use tauri::Manager;

use crate::state::AppState;
use kiwano_core::history;
use kiwano_core::vm;

/// Every agent a reader exists for, named rather than discovered: a reader is
/// code — a file format parsed by hand — so what *can* be read is a fact about
/// this build, not about what happens to be installed. The same pair the CLI's
/// `history import` scans by default.
const READABLE: [&str; 2] = ["claude", "codex"];

/// First-launch history backfill: run **at most once per launch, and only when
/// there is something to do**.
///
/// The gate is the daemon's scan stamp (`history.last_scan_at`, written when
/// rows land and read by [`kiwano_core::daemon_api::DaemonApi::history_scanned_at`]),
/// so a machine that has backfilled before pays exactly one HTTP call and
/// nothing else. The read the stamp guards is seconds of file I/O — a year of
/// transcripts parsed line by line — for rows that cannot have changed since
/// the last scan: the daemon's watermark keeps out everything the gateway has
/// metered, and the older rows are already stored. Running it on every launch
/// would spend that time to reach the same answer.
///
/// Structured like [`crate::hub::spawn_hub_sync`]: a task on Tauri's async
/// runtime, a short retry because launched at login this races the sidecar's own
/// start, `spawn_blocking` for the blocking work, and every failure logged and
/// swallowed — the Hub check is an enhancement, and so is this one.
pub(crate) fn spawn_history_backfill(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        const RETRY_SECS: [u64; 5] = [0, 2, 6, 20, 60];
        let mut scan_needed = false;
        for (attempt, delay) in RETRY_SECS.iter().enumerate() {
            if *delay > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(*delay)).await;
            }
            // The stamp check is a synchronous admin-plane call, so it goes on
            // the blocking pool like every other blocking call here.
            let outcome = tokio::task::spawn_blocking(|| {
                kiwano_core::daemon_api::DaemonApi::connect().history_scanned_at()
            })
            .await;
            match outcome {
                Ok(Ok(Some(scanned_at))) => {
                    tracing::debug!(scanned_at = %scanned_at, "history already backfilled");
                    return;
                }
                Ok(Ok(None)) => {
                    scan_needed = true;
                    break;
                }
                // The daemon has not come up yet (launched at login, this beats
                // the sidecar) — retry across the first minute, then give up
                // quietly rather than scan against a daemon that cannot store
                // the rows.
                Ok(Err(e)) if attempt == RETRY_SECS.len() - 1 => {
                    tracing::warn!(error = %e, "history scan check failed");
                    return;
                }
                Ok(Err(_)) => {} // another attempt is coming
                Err(e) => {
                    tracing::warn!(error = %e, "history scan check did not finish");
                    return;
                }
            }
        }
        if !scan_needed {
            return;
        }
        let app = handle.clone();
        match tokio::task::spawn_blocking(move || {
            scan_once(&app, &kiwano_core::daemon_api::DaemonApi::connect())
        })
        .await
        {
            Ok(Ok(_)) => {}
            Ok(Err(e)) => tracing::warn!(error = %e, "history backfill skipped"),
            Err(e) => tracing::warn!(error = %e, "history backfill did not finish"),
        }
    });
}

/// Read this machine's transcripts once and hand every row to the daemon —
/// `history_import`, the command behind a future "scan now" button.
///
/// It re-runs the same read and import the startup backfill does, but ignores
/// the scan stamp: the user asked for a scan, so it re-reads, and the daemon's
/// upsert re-prices anything that had no price the first time. The command is
/// `async` — and the attribute carries it, not the signature. `#[tauri::command]`
/// on an `async fn` still runs the body on the main thread unless `(async)` is
/// spelled out, which would put a synchronously-blocking daemon round trip (and
/// a login shell, on the first scan) in the way of the window. The
/// `one_writer` test in this crate is what holds that: it names any command that
/// reaches the daemon without the annotation (`migrate.local.md` §10.26).
#[tauri::command(async)]
pub async fn history_import(app: tauri::AppHandle) -> Result<vm::HistoryImportReport, String> {
    tokio::task::spawn_blocking(move || {
        scan_once(&app, &kiwano_core::daemon_api::DaemonApi::connect())
    })
    .await
    .map_err(|e| format!("the history scan did not finish: {e}"))?
}

/// The shared scan: resolve the inputs, read, split under the body limit, send
/// each chunk, sum the reports.
///
/// Blocking throughout — file I/O, JSON parsing, and a synchronous admin-plane
/// client — so both callers run it on the blocking pool. `AppHandle` is taken
/// rather than `State` because the shell variables live on the managed state and
/// this closure has to be `'static` to move onto that pool.
/// The shared scan: resolve the inputs, read this machine's transcripts, split
/// them under the body limit, and hand each chunk to the daemon.
///
/// The client is passed in rather than built here, and that is not a detail: it
/// is what keeps this function's daemon calls inside the caller's
/// `spawn_blocking`, which is the property the `one_writer` test looks for — a
/// synchronous round trip on the main thread is a frozen window
/// (`migrate.local.md` §10.26).
fn scan_once(
    app: &tauri::AppHandle,
    api: &kiwano_core::daemon_api::DaemonApi,
) -> Result<vm::HistoryImportReport, String> {
    let state = app
        .try_state::<AppState>()
        .ok_or_else(|| "the app state is not ready".to_string())?;
    let Some(home) = home_or_none() else {
        return Err("no trustworthy home directory to scan".to_string());
    };
    // Asked for at most once per launch and cached on the state — the same
    // answer the commands use, so the app never spawns a second login shell just
    // to learn where an agent keeps its files.
    let vars = state.shell_vars().clone();

    tracing::info!(agents = ?READABLE, "scanning agent history");
    let agents: Vec<String> = READABLE.iter().map(|a| (*a).to_string()).collect();
    let read = history::read_history(&home, &vars, &agents);
    if !read.skips.is_empty() {
        // A file that could not be opened: worth a line, never a failure — the
        // rest of the scan is every bit as valid without it.
        tracing::warn!(
            skipped = read.skips.len(),
            "some history files could not be read"
        );
    }

    let mut total = vm::HistoryImportReport::default();
    if !read.batch.usage.is_empty() || !read.batch.sessions.is_empty() {
        // One request per chunk: the admin plane's 2 MiB body limit is far below
        // what a first scan of a busy machine produces, so the sender loops.
        for piece in history::chunks(&read.batch, history::MAX_ROWS_PER_REQUEST) {
            total.add(api.import_history(&piece)?);
        }
    }
    tracing::info!(
        rows = total.usage_rows,
        sessions = total.sessions,
        skipped_by_watermark = total.skipped_by_watermark,
        unpriced = total.unpriced, unpriced_models = ?total.unpriced_models,
        "history backfill finished"
    );
    Ok(total)
}

/// The user's home, or `None` when there is none we can trust.
///
/// `paths::home_dir()` fails closed by panicking when `dirs::home_dir()` finds
/// nothing — it refuses to invent a directory for Kiwano's own files. That panic
/// is **not catchable in the build that ships**: the workspace's release profile
/// sets `panic = "abort"`, so a `catch_unwind` would take the process with it.
/// The panic is therefore *avoided* rather than caught — its own condition is
/// read from the environment first, and `home_dir()` is called only once an
/// answer is there.
///
/// On macOS and Linux the check is exact: `dirs` answers from a non-empty
/// `$HOME` (its other input is the `CC_SWITCH_TEST_HOME` override, which is
/// checked the same way `get_home_dir` checks it). On Windows
/// `dirs` reads the profile known folder instead, which no environment variable
/// names — but the app cannot reach here without a home anyway: `run` resolves
/// the database path, and that resolution calls `get_home_dir` eagerly, before
/// the `tauri::Builder` is even built. A running process has already had a home
/// to put its data in, so the call cannot panic.
fn home_or_none() -> Option<PathBuf> {
    if cfg!(windows) {
        return Some(crate::paths::home_dir());
    }
    // Mirrors `get_home_dir`: a blank `CC_SWITCH_TEST_HOME` is not an answer,
    // and `dirs` treats an empty `$HOME` as unset.
    let test_home = std::env::var("CC_SWITCH_TEST_HOME")
        .map(|v| !v.trim().is_empty())
        .unwrap_or(false);
    let home = std::env::var_os("HOME").is_some_and(|h| !h.is_empty());
    (test_home || home).then(crate::paths::home_dir)
}
