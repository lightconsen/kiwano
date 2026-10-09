//! The Hub catalog: the sync the user triggers from Settings, and the retried
//! one startup runs.
//!
//! Both are the daemon's now (`migrate.local.md` §10.14): it fetches, caches,
//! applies what changed, and re-reads its own route table — which is why this
//! module no longer reads the cache or sends a reload ping. The startup loop
//! keeps only what is the app's business: *when* to retry.

use tauri::Manager;

use crate::state::AppState;
use kiwano_core::vm;

/// Hub sync at startup: catalog + pricing, retried briefly so a login that
/// beats the network does not settle for the cache. A task on Tauri's async
/// runtime, like the sync behind the button — the HTTP is async now, so this
/// needs neither a thread of its own nor a blocking client on one. Exhausting
/// the retries is logged and nothing else — the Hub is an enhancement, and the
/// cached data always works offline.
pub(crate) fn spawn_hub_sync(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(_state) = handle.try_state::<AppState>() else {
            return;
        };
        // The daemon resolves the hub_url itself — from the same ui settings row
        // the app used to read, through its own store.
        // Launched at login, this runs while Wi-Fi is often still associating:
        // one attempt then leaves the catalog on whatever was cached for the
        // rest of the session, which is indistinguishable from the Hub being
        // down. Retry across the first minute or so instead — long enough for
        // the network to arrive, short enough to give up quietly if it does not.
        const RETRY_SECS: [u64; 5] = [0, 2, 6, 20, 60];
        for (attempt, delay) in RETRY_SECS.iter().enumerate() {
            if *delay > 0 {
                tokio::time::sleep(std::time::Duration::from_secs(*delay)).await;
            }
            let outcome = tokio::task::spawn_blocking(|| {
                kiwano_core::daemon_api::DaemonApi::connect().sync_hub()
            })
            .await
            .expect("the sync task did not panic");
            match outcome {
                // Already current: the manifest sha matched the cache.
                Ok(r) => {
                    if !r.unchanged {
                        tracing::info!(providers = r.fetched, "hub catalog synced");
                    }
                    break;
                }
                Err(e) if attempt == RETRY_SECS.len() - 1 => {
                    tracing::warn!(attempts = attempt + 1, error = %e, "hub sync failed")
                }
                Err(_) => {} // another attempt is coming
            }
        }
        // The daemon applies what the sync brought and re-reads its own route
        // table — the same work the Sync button does, and nothing for this path
        // to repeat.
    });
}

/// Hub catalog sync — an `async` command, because the fetch is async. It used to
/// be a synchronous one `block_on`-ing a blocking reqwest client on Tauri's
/// blocking pool: the same work, on a thread of its own, to satisfy a client that
/// panics if it is dropped inside a runtime.
#[tauri::command]
pub fn sync_hub() -> Result<vm::SyncReportVm, String> {
    // Served by the daemon (`migrate.local.md` §10.14): it fetches, caches,
    // applies what changed, and re-reads its own route table — the whole of
    // what this command used to orchestrate by hand.
    kiwano_core::daemon_api::DaemonApi::connect().sync_hub()
}
