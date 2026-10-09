//! Kiwano GUI backend: Tauri commands backed by the gateway store + sidecar.
//!
//! Data flow (tech.md §4.1): UI → `invoke` → commands here → the daemon's API
//! over the admin plane (a unix socket / named pipe, or another machine — see
//! `sidecar`). The daemon owns the database and re-reads its own route table
//! after a write, so nothing pings it. The gateway process itself is spawned in
//! `setup`.
//!
//! The module is split by domain. Every command keeps the name it had when this
//! was one file: `generate_handler!` below names all 53 exactly as it did, and
//! the facade re-exports each one, so the 53 `invoke` names in
//! `app/src/api/tauri.ts` are untouched by the move.
//!
//! `state` and `paths` are the substrate the rest shares — the managed state,
//! and where the database, the home directory and the data port come from. `daemon`, `tray`, `hub` and `updater`
//! are the pieces the window does not call — the watchdog, the tray, the Hub
//! sync and the update check — and the remaining modules are the command
//! surface, one per screen or domain it serves.
//!
//! `run` stays in the facade because it is the entry point `main.rs` names and
//! the order it starts things in is the contract: `logging::init` before
//! anything that can fail, `app.manage` before the `spawn_*` calls that
//! `try_state` past it, and the autostart plugin registered late rather than
//! with the other four.

mod update;

pub mod agents;
pub mod alerts;
pub mod catalog;
pub mod daemon;
pub mod dashboard;
pub mod hub;
pub mod keys;
pub mod logs;
pub mod paths;
pub mod probe;
pub mod providers;
pub mod quota;
pub mod settings;
pub mod sharing;
pub mod state;
pub mod strategies;
pub mod takeover;
pub mod tray;
pub mod updater;

use std::sync::{Mutex, OnceLock};

// The application layer lives in `kiwano-core`, shared with the CLI. Importing
// the module by name keeps every `vm::…` / `pricing::…` call site unchanged —
// `use` puts the same name in scope that `mod` did. The command modules repeat
// the aliases they use, for the same reason.
use kiwanod::store::Store;
use tauri::Manager;
use tauri_plugin_notification::NotificationExt;

use kiwano_core::{sidecar, vm};
use vm::Aux;

use crate::daemon::{spawn_usage_watch, spawn_watchdog};
use crate::hub::spawn_hub_sync;
use crate::paths::{db_path, env_port, local_db_path, resolved_db};
use crate::state::AppState;
use crate::tray::{setup_tray, sync_autostart};
use crate::updater::record_update_check;

// ── the command surface, one arm per module ──
//
// `generate_handler!` at the bottom names every command, and each one is
// re-exported here exactly once, so a name that goes missing from a module is a
// compile error rather than a command the frontend calls into the void.

pub use agents::{
    agent_search_dirs, clear_agent_dir, detect_agents, get_currency_meta, probe_agent_versions,
    set_agent_dir, verify_agent_dir,
};
pub use alerts::check_usage_alerts;
pub use catalog::{list_catalog, list_model_prices, open_url};
pub use daemon::get_gateway_status;
pub use dashboard::get_dashboard;
pub use hub::sync_hub;
pub use keys::{add_api_key, delete_api_key, list_api_keys};
pub use logs::{
    ack_credential_finding, check_credential_finding, clear_request_logs, export_request_logs,
    get_request_log, list_request_logs, open_log_folder,
};
pub use probe::{list_models, test_endpoint, test_latency, test_provider_latency};
pub use providers::{
    add_provider, delete_provider, list_providers, set_provider_enabled, update_provider,
};
pub use quota::get_plan_quota;
pub use settings::{get_settings, update_settings};
pub use sharing::{export_config, import_cc_switch, import_config};
pub use strategies::{
    add_agent_binding, add_custom_agent, apply_agent_route, get_agent_routes, remove_agent_binding,
    remove_custom_agent, reorder_agent_bindings, set_agent_limits, update_agent_binding,
    update_agent_strategy, update_custom_agent,
};
pub use takeover::set_agent_takeover;
pub use updater::{check_app_update, get_footer_stats, get_pending_update};

pub fn run() {
    // Before anything that can fail, and before the webview: a packaged app is
    // started by launchd, where stdout goes to /dev/null, so without this the
    // only way to read what happened is to run it from a terminal.
    let db = resolved_db();
    kiwanod::logging::init(&db.path);
    // After logging, not before: the resolver runs to find the log directory, so
    // anything it had to ignore would have been said too early to be written.
    if let Some(note) = db.ignored_note() {
        tracing::warn!("{note}");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .setup(|app| {
            let path = db_path();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            let store = Store::open(&path).map_err(|e| e.to_string())?;
            // The client's own file, beside the daemon's rather than the same as
            // it: everything in here describes *this machine*, and a client on
            // another one would have none of it (`migrate.local.md` §9.5 step 2).
            let local = local_db_path(&path);
            if let Some(dir) = local.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            let aux = Aux::open(&local).map_err(|e| e.to_string())?;
            // Bring across what an older build wrote into the shared file, once.
            // Best-effort: a client that cannot do it keeps working from the
            // daemon's rows for one more run rather than refusing to start.
            if let Err(e) = aux.adopt_from(&path) {
                tracing::warn!(
                    "could not adopt the client rows from {}: {e}",
                    path.display()
                );
            }
            let data_port = env_port("KIWANO_DATA_PORT", 8317);
            // Not a port: the admin plane is a socket beside the database, or a
            // per-user named pipe.
            let admin = sidecar::admin_endpoint_resolved().unwrap_or_else(|e| {
                // A named-but-malformed remote daemon: say so once, then use the
                // local plane so the app still starts and shows the problem.
                tracing::error!("{e}");
                sidecar::admin_endpoint()
            });

            // Sidecar lifecycle (tech.md §4.6): adopt an already-running daemon
            // — but only one of our own version. The daemon deliberately
            // outlives the GUI, so an upgrade meets its predecessor still
            // holding the data port, and the two share a SQLite file whose
            // schema only one of them may understand. `gateway_status` doubles
            // as the liveness probe here — None means nothing is answering on
            // the endpoint.
            let ours = env!("CARGO_PKG_VERSION");
            let status = sidecar::gateway_status(&admin);
            let child = match sidecar::startup_action(status.as_ref(), ours) {
                sidecar::StartupAction::Adopt => {
                    tracing::info!(
                        admin = %admin.describe(),
                        version = ours,
                        "adopting running gateway"
                    );
                    None
                }
                sidecar::StartupAction::Restart => {
                    let running = status
                        .as_ref()
                        .and_then(|s| s.get("version"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("unknown");
                    tracing::warn!(
                        running,
                        ours,
                        admin = %admin.describe(),
                        "the running gateway is a different version; replacing it"
                    );
                    match sidecar::restart(&admin) {
                        Ok(c) => Some(c),
                        Err(e) => {
                            tracing::error!(error = %e, "could not replace the running gateway");
                            None
                        }
                    }
                }
                sidecar::StartupAction::Spawn => match sidecar::spawn() {
                    Ok(c) => {
                        tracing::info!("gateway sidecar spawned");
                        Some(c)
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "gateway sidecar unavailable");
                        None
                    }
                },
            };

            // The settings blob the rest of startup reads (autostart, the
            // update check) comes from the daemon, which owns the row.
            let ui = kiwanod::api::settings::ui_settings(&store).unwrap_or_default();
            // Seeding the price mirror and linking providers to their catalog
            // entries are the **daemon's** work now (`migrate.local.md` §10.14):
            // it does both inside its own sync, and the startup sync below asks
            // it to. Doing them here too would be a second writer of the same
            // tables — which is the arrangement this migration has been removing.

            app.manage(AppState {
                store,
                aux,
                child: Mutex::new(child),
                admin,
                data_port,
                pending_update: Mutex::new(None),
                shell_vars: OnceLock::new(),
            });
            spawn_watchdog(app.handle().clone());
            // Live numbers: the gateway says when it has recorded traffic, and
            // whatever screen is showing re-reads (see `spawn_usage_watch`).
            spawn_usage_watch(app.handle().clone());
            // Hub catalog: one-shot conditional sync (skips the download when
            // the manifest sha matches the cache). Silent, opt-out-free, and
            // failure-tolerant — a failed sync just leaves the cache as it was.
            spawn_hub_sync(app.handle().clone());

            // Tray + login items (the autostart/close_to_tray settings become real from here on)
            app.handle().plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))?;
            setup_tray(app, data_port)?;
            sync_autostart(app.handle(), ui.autostart);

            // Self-update: silent startup check (opt-out in Settings). Failures
            // are ignored — a check that cannot reach the release channel looks
            // exactly like "you are current", so it must not nag about it.
            // A hit is both stored for the UI and announced: the notification
            // alone left Settings offering "Check for updates" as if nothing
            // had been found, contradicting the notification the user just saw.
            if ui.auto_check_update {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn(async move {
                    if let Ok(Some(info)) = update::check(handle.clone()).await {
                        // Records it for the banner and About, refreshes the tray
                        // entry (a desktop notification cannot be clicked), and
                        // nudges an open UI.
                        record_update_check(&handle, Some(&info));
                        let _ = handle
                            .notification()
                            .builder()
                            .title("Kiwano update available")
                            .body(format!(
                                "Version {} is ready to install — open it from the tray menu",
                                info.version
                            ))
                            .show();
                    }
                });
            }

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            list_model_prices,
            open_url,
            list_providers,
            add_provider,
            update_provider,
            delete_provider,
            set_provider_enabled,
            test_latency,
            test_provider_latency,
            test_endpoint,
            list_models,
            list_catalog,
            get_dashboard,
            get_settings,
            update_settings,
            list_request_logs,
            export_request_logs,
            get_request_log,
            clear_request_logs,
            check_credential_finding,
            ack_credential_finding,
            set_agent_takeover,
            get_footer_stats,
            sync_hub,
            check_usage_alerts,
            list_api_keys,
            add_api_key,
            delete_api_key,
            import_cc_switch,
            get_agent_routes,
            update_agent_strategy,
            set_agent_limits,
            reorder_agent_bindings,
            update_agent_binding,
            add_agent_binding,
            remove_agent_binding,
            add_custom_agent,
            update_custom_agent,
            remove_custom_agent,
            apply_agent_route,
            export_config,
            import_config,
            detect_agents,
            set_agent_dir,
            verify_agent_dir,
            agent_search_dirs,
            clear_agent_dir,
            probe_agent_versions,
            check_app_update,
            update::download_and_install_app_update,
            get_pending_update,
            get_currency_meta,
            get_plan_quota,
            open_log_folder,
        ])
        .on_window_event(|window, event| {
            // Close-to-tray: intercept CloseRequested and hide the window
            // instead of quitting (toggleable in settings)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<AppState>();
                // Read from the store: the daemon patches that row, and the aux blob
                // nothing writes any more would answer with a stale value.
                if vm::ui_settings(&state.store).close_to_tray {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|_app_handle, _event| {
            // Deliberately no kill-on-exit: the gateway daemon outlives the
            // GUI (tech.md §2.4 B). The next launch adopts it via admin ping.
        });
}
