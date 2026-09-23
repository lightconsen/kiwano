//! The gateway daemon as this process sees it: the watchdog that keeps one
//! running, the bridge that carries its usage ticks to the webview, and the
//! status reader behind the UI's gateway indicator.
//!
//! What the watchdog ticks against is `crate::state::AppState`, and the Hub's
//! startup sync — the third spawn `run` makes — is a catalog concern
//! (`crate::hub`) rather than a daemon one.

use tauri::{Emitter, Manager, State};

use crate::state::AppState;
use kiwano_core::{sidecar, vm};

// ── Daemon lifecycle (tech.md §2.4 B / §4.6) ──
//
// The gateway is a resident daemon, not a child of the GUI: the app must NOT
// kill it on exit, must adopt an already-running instance at startup, and a
// watchdog must respawn it after a crash. `/status` pings (below) are the
// reconnect mechanism.
//
// Startup treats a version mismatch as a replace (see `sidecar::startup_action`).
// The watchdog below does not: its adopt paths only ask whether the admin plane
// answers. That is deliberate — it ticks every few seconds, and killing a
// process because one `/status` call came back odd is worse than running a
// gateway a version behind. Skew cannot appear mid-session anyway: the endpoint
// is held by whatever this app spawned or adopted at startup.

/// Watchdog decision for one tick, factored out for testing.
#[derive(Debug, PartialEq, Eq)]
enum WatchdogAction {
    /// Child running (or adopted instance healthy) — nothing to do.
    None,
    /// Child process exited but an instance answers on the admin port
    /// (e.g. started manually) — drop the stale handle, adopt externally.
    ClearChild,
    /// No usable gateway — spawn one.
    Respawn,
}

fn watchdog_decision(child_exited: Option<bool>, admin_alive: bool) -> WatchdogAction {
    match (child_exited, admin_alive) {
        // no handle: healthy iff admin answers
        (None, true) => WatchdogAction::None,
        (None, false) => WatchdogAction::Respawn,
        // handle present: exited status decides
        (Some(false), _) => WatchdogAction::None,
        (Some(true), true) => WatchdogAction::ClearChild,
        (Some(true), false) => WatchdogAction::Respawn,
    }
}

/// How long to wait before the next tick. Five seconds while things are fine;
/// a gateway that cannot start (port taken, database locked, binary missing)
/// would otherwise be respawned every five seconds for as long as the app runs,
/// each attempt printing its own line. A healthy tick clears the streak, so a
/// one-off crash still comes back promptly.
fn watchdog_delay(consecutive_respawns: u32) -> std::time::Duration {
    std::time::Duration::from_secs(match consecutive_respawns {
        0 => 5,
        1 => 15,
        2 => 45,
        _ => 120,
    })
}

/// How often the app may be told to re-read, at most. A burst of requests lands
/// within a few hundred milliseconds of itself, and without this every one of
/// them would have the screen in front of the user re-read the whole provider
/// list — for numbers that are not going to differ. One re-read a second costs
/// less and reads the same.
const USAGE_TICK_INTERVAL: std::time::Duration = std::time::Duration::from_millis(1000);

/// How long to wait before following the event stream again after it ends. It
/// ends because the gateway stopped, because it is not up yet, or because the
/// watchdog replaced it — all of them "shortly", and none of them the app's to
/// fix: bringing the gateway back is the watchdog's job.
const USAGE_RECONNECT_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Bridge the gateway's events to the webview.
///
/// The gateway is a separate process and the window cannot hear it directly, so
/// this thread is the bridge: it holds one subscription to the admin plane's
/// `/events` stream, reconnecting whenever that ends, and re-emits each event
/// as a Tauri event. The frontend's half is `lib/updateEvents.ts`.
///
/// Two kinds ride the stream: usage ticks (`{"kind":"usage"}`, coalesced — the
/// numbers have one source, the store read the frontend makes in response, and
/// a burst of requests is one re-read's worth of news) and typed events
/// (a DLP finding, a limit transition, a refused key), which pass through
/// un-coalesced — each is a fact that happened once, and the webview turns it
/// into a notification and a tray entry.
pub(crate) fn spawn_usage_watch(handle: tauri::AppHandle) {
    std::thread::spawn(move || {
        // Kept across reconnects: a tick arriving moments after the last one is
        // the same news whether or not the stream was re-established in between.
        let mut last: Option<std::time::Instant> = None;
        loop {
            let admin = match handle.try_state::<AppState>() {
                Some(state) => state.admin.clone(),
                // No state yet, or the app is shutting down.
                None => return,
            };
            sidecar::watch_events(&admin, |payload| {
                if payload.contains("\"kind\":\"usage\"") {
                    if let Some(prev) = last {
                        let since = prev.elapsed();
                        if since < USAGE_TICK_INTERVAL {
                            std::thread::sleep(USAGE_TICK_INTERVAL - since);
                        }
                    }
                    last = Some(std::time::Instant::now());
                    let _ = handle.emit("usage-changed", ());
                    return;
                }
                // A typed event: carry the payload as-is — the webview parses
                // it once, and this side does not need to know its shape.
                let _ = handle.emit("gateway-event", payload);
                let _ = crate::tray::refresh_with_event(&handle, payload);
            });
            std::thread::sleep(USAGE_RECONNECT_DELAY);
        }
    });
}

pub(crate) fn spawn_watchdog(handle: tauri::AppHandle) {
    let mut respawns = 0u32;
    std::thread::spawn(move || loop {
        std::thread::sleep(watchdog_delay(respawns));
        let Some(state) = handle.try_state::<AppState>() else {
            return;
        };
        let child_exited = {
            let Ok(mut child) = state.child.lock() else {
                return;
            };
            child
                .as_mut()
                .map(|c| c.try_wait().map(|st| st.is_some()).unwrap_or(true))
        };
        let admin_alive = sidecar::ping_admin(&state.admin);
        match watchdog_decision(child_exited, admin_alive) {
            // Serving: whatever streak there was is over.
            WatchdogAction::None => respawns = 0,
            WatchdogAction::ClearChild => {
                respawns = 0;
                if let Ok(mut child) = state.child.lock() {
                    *child = None;
                }
            }
            WatchdogAction::Respawn => {
                // Re-check under the lock to avoid double-spawn races; the
                // ping is repeated because the admin may have come up since.
                let Ok(mut child) = state.child.lock() else {
                    return;
                };
                if sidecar::ping_admin(&state.admin) {
                    *child = None;
                    respawns = 0;
                    continue;
                }
                respawns += 1;
                match sidecar::spawn() {
                    Ok(c) => {
                        if respawns == 1 {
                            tracing::info!("gateway respawned by watchdog");
                        } else {
                            tracing::warn!(
                                respawns,
                                backoff_secs = watchdog_delay(respawns).as_secs(),
                                "gateway respawned again; backing off"
                            );
                        }
                        *child = Some(c);
                    }
                    Err(e) => tracing::error!(error = %e, "watchdog respawn failed"),
                }
            }
        }
    });
}

#[tauri::command]
pub fn get_gateway_status(state: State<AppState>) -> vm::GatewayStatusVm {
    let report = sidecar::gateway_status(&state.admin);
    let blocked = report
        .as_ref()
        .and_then(|r| r.get("blocked"))
        .and_then(|b| b.as_array())
        .map(|rows| {
            rows.iter()
                .filter_map(|row| {
                    Some(vm::BlockedProviderVm {
                        id: row.get("provider_id")?.as_str()?.to_string(),
                        reason: row.get("reason")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default();
    vm::GatewayStatusVm {
        running: report.is_some(),
        port: state.data_port,
        blocked,
    }
}

#[cfg(test)]
mod tests {
    use super::{watchdog_decision, watchdog_delay, WatchdogAction};

    #[test]
    fn watchdog_matrix() {
        // healthy child → nothing
        assert_eq!(watchdog_decision(Some(false), true), WatchdogAction::None);
        assert_eq!(watchdog_decision(Some(false), false), WatchdogAction::None);
        // crashed child
        assert_eq!(
            watchdog_decision(Some(true), false),
            WatchdogAction::Respawn
        );
        // crashed child but external instance answers → adopt, drop handle
        assert_eq!(
            watchdog_decision(Some(true), true),
            WatchdogAction::ClearChild
        );
        // adopted instance (no handle)
        assert_eq!(watchdog_decision(None, true), WatchdogAction::None);
        // spawn failed at startup → retry
        assert_eq!(watchdog_decision(None, false), WatchdogAction::Respawn);
    }

    #[test]
    fn watchdog_backs_off_only_while_respawns_keep_failing() {
        use std::time::Duration;
        // The common case — a tick after a healthy one — stays at the base.
        assert_eq!(watchdog_delay(0), Duration::from_secs(5));
        // And each respawn without a healthy tick between them waits longer,
        // up to a ceiling rather than growing without bound.
        assert!(watchdog_delay(1) > watchdog_delay(0));
        assert!(watchdog_delay(2) > watchdog_delay(1));
        assert_eq!(watchdog_delay(3), watchdog_delay(50));
        assert_eq!(watchdog_delay(50), Duration::from_secs(120));
    }
}
