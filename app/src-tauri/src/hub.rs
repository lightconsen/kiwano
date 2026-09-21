//! The Hub catalog: the sync the user triggers from Settings, and the retried
//! one startup runs. Both end the same way — rebuild what the documents imply,
//! then reload the daemon at most once — so the button and the startup path
//! cannot drift apart.

use tauri::{Manager, State};

use crate::state::AppState;
use kiwano_core::{sidecar, sync, vm};

/// Hub sync at startup: catalog + pricing, retried briefly so a login that
/// beats the network does not settle for the cache. A task on Tauri's async
/// runtime, like the sync behind the button — the HTTP is async now, so this
/// needs neither a thread of its own nor a blocking client on one. Exhausting
/// the retries is logged and nothing else — the Hub is an enhancement, and the
/// cached data always works offline.
pub(crate) fn spawn_hub_sync(handle: tauri::AppHandle) {
    tauri::async_runtime::spawn(async move {
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        let hub_url = vm::ui_settings(&state.aux).hub_url;
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
            match sync::sync_from_hub(&state.aux, &hub_url).await {
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
        // The Hub may have brought a newer price table or made a provider
        // linkable. Rebuild both from the cache it just wrote and reload the
        // daemon once — the same call the Sync button makes, so the two cannot
        // drift apart again.
        if sync::apply_hub_documents(&state.store, &state.aux) {
            sidecar::notify_reload(&state.admin);
        }
    });
}

/// Hub catalog sync — an `async` command, because the fetch is async. It used to
/// be a synchronous one `block_on`-ing a blocking reqwest client on Tauri's
/// blocking pool: the same work, on a thread of its own, to satisfy a client that
/// panics if it is dropped inside a runtime.
#[tauri::command]
pub async fn sync_hub(state: State<'_, AppState>) -> Result<vm::SyncReportVm, String> {
    let hub_url = vm::ui_settings(&state.aux).hub_url;
    let report = sync::sync_from_hub(&state.aux, &hub_url).await?;
    // Rebuild everything derived from the documents just cached — the price
    // mirror and the provider↔catalog links — and reload the daemon once if
    // either wrote. A manual sync does not otherwise touch the daemon, so this
    // is the whole of its effect on what gets billed.
    if sync::apply_hub_documents(&state.store, &state.aux) {
        sidecar::notify_reload(&state.admin);
    }
    Ok(report)
}
