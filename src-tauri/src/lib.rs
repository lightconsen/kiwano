//! Kiwano GUI backend: Tauri commands backed by the gateway store + sidecar.
//!
//! Data flow (tech.md §4.1): UI → `invoke` → commands here → gateway Store
//! (same SQLite file the sidecar reads) → `POST :8310/reload` hot-swaps the
//! gateway route table. The gateway process itself is spawned in `setup`.

mod creds;
mod detect;
mod import;
mod share;
mod sidecar;
mod sync;
mod takeover;
mod vm;

use std::sync::Mutex;

use kiwano_gateway::store::Store;
use tauri::{Manager, State};

use detect::{detect_agents, probe_agent_versions};
use vm::Aux;

struct AppState {
    store: Store,
    aux: Aux,
    child: Mutex<Option<std::process::Child>>,
    admin_port: u16,
    data_port: u16,
}

fn env_port(name: &str, default: u16) -> u16 {
    std::env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn default_db_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    std::path::PathBuf::from(home)
        .join(".kiwano")
        .join("kiwano.db")
}

fn db_path() -> std::path::PathBuf {
    match std::env::var("KIWANO_DB_PATH") {
        Ok(v) if !v.is_empty() => v.into(),
        _ => default_db_path(),
    }
}

fn after_mutation(state: &State<AppState>) {
    sidecar::notify_reload(state.admin_port);
}

// ── Daemon lifecycle (tech.md §2.4 B / §4.6) ──
//
// The gateway is a resident daemon, not a child of the GUI: the app must NOT
// kill it on exit, must adopt an already-running instance at startup, and a
// watchdog must respawn it after a crash. `/status` pings (below) are the
// reconnect mechanism.

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

fn spawn_watchdog(handle: tauri::AppHandle) {
    std::thread::spawn(move || loop {
        std::thread::sleep(std::time::Duration::from_secs(5));
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
        let admin_alive = sidecar::ping_admin(state.admin_port);
        match watchdog_decision(child_exited, admin_alive) {
            WatchdogAction::None => {}
            WatchdogAction::ClearChild => {
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
                if sidecar::ping_admin(state.admin_port) {
                    *child = None;
                    continue;
                }
                match sidecar::spawn() {
                    Ok(c) => {
                        println!("kiwano: gateway respawned by watchdog");
                        *child = Some(c);
                    }
                    Err(e) => eprintln!("kiwano: watchdog respawn failed: {e}"),
                }
            }
        }
    });
}

// ── Tray / autostart (tech.md §3 P1: tray + close-to-tray + launch at login) ──

/// Sync the persisted autostart setting to the OS login items (idempotent;
/// failures are silent — realigned on next launch).
fn sync_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    let _ = if enabled { mgr.enable() } else { mgr.disable() };
}

fn setup_tray(app: &tauri::App, data_port: u16) -> tauri::Result<()> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
    use tauri::tray::TrayIconBuilder;

    let open = MenuItem::with_id(app, "open", "Open Kiwano", true, None::<&str>)?;
    let gw = MenuItem::with_id(
        app,
        "gateway",
        format!("Gateway :{data_port}"),
        false,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, "quit", "Quit Kiwano", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &gw, &sep, &quit])?;

    TrayIconBuilder::with_id("kiwano-tray")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip(format!("Kiwano — local gateway :{data_port}"))
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                }
            }
            "quit" => app.exit(0), // the daemon outlives the GUI (tech.md §2.4 B)
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
fn get_gateway_status(state: State<AppState>) -> vm::GatewayStatusVm {
    vm::GatewayStatusVm {
        running: sidecar::ping_admin(state.admin_port),
        port: state.data_port,
    }
}

#[tauri::command]
fn list_providers(state: State<AppState>) -> Result<Vec<vm::ProviderVm>, String> {
    vm::build_provider_vms(&state.store, &state.aux)
}

#[tauri::command]
fn add_provider(
    state: State<AppState>,
    input: vm::NewProviderInput,
) -> Result<vm::ProviderVm, String> {
    let vm = vm::add_provider(&state.store, &input)?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
fn enable_provider(state: State<AppState>, id: String) -> Result<(), String> {
    vm::enable_provider(&state.store, &id)?;
    after_mutation(&state);
    Ok(())
}

#[tauri::command]
fn update_provider(
    state: State<AppState>,
    id: String,
    input: vm::NewProviderInput,
) -> Result<vm::ProviderVm, String> {
    let vm = vm::update_provider(&state.store, &state.aux, &id, &input)?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
fn delete_provider(state: State<AppState>, id: String) -> Result<bool, String> {
    let ok = vm::delete_provider(&state.store, &id)?;
    after_mutation(&state);
    Ok(ok)
}

#[tauri::command(async)]
fn test_latency(endpoint: String) -> Result<u64, String> {
    sidecar::measure_latency(&endpoint)
}

#[tauri::command]
fn list_catalog(state: State<AppState>) -> vm::CatalogListVm {
    vm::load_catalog(&state.aux)
}

#[tauri::command]
fn get_dashboard(state: State<AppState>, window: String) -> Result<vm::DashboardVm, String> {
    vm::build_dashboard(&state.store, &state.aux, &window)
}

#[tauri::command]
fn get_settings(state: State<AppState>) -> Result<vm::SettingsVm, String> {
    vm::build_settings(&state.store, &state.aux)
}

#[tauri::command]
fn update_settings(
    app: tauri::AppHandle,
    state: State<AppState>,
    patch: serde_json::Value,
) -> Result<vm::SettingsVm, String> {
    let vm = vm::update_settings(&state.store, &state.aux, &patch)?;
    // apply autostart changes to the OS login items immediately
    if let Some(v) = patch.get("autostart").and_then(|v| v.as_bool()) {
        sync_autostart(&app, v);
    }
    Ok(vm)
}

#[tauri::command]
fn set_agent_takeover(state: State<AppState>, agent: String, enabled: bool) -> Result<(), String> {
    let data_port = state.data_port;
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    vm::set_agent_takeover(
        &state.store,
        &state.aux,
        &agent,
        enabled,
        data_port,
        &std::path::PathBuf::from(home),
    )?;
    // enabling may import a provider and bind it → the route table changed
    after_mutation(&state);
    Ok(())
}

#[tauri::command]
fn get_footer_stats(state: State<AppState>) -> Result<vm::FooterStatsVm, String> {
    vm::build_footer_stats(&state.store, &state.aux)
}

// ── Agent strategies (tech.md §4.7: strategy types / candidate ordering) ──

#[tauri::command]
fn get_agent_routes(state: State<AppState>) -> Result<Vec<vm::AgentRouteVm>, String> {
    vm::build_agent_routes(&state.store)
}

#[tauri::command]
fn update_agent_strategy(
    state: State<AppState>,
    agent: String,
    strategy: String,
    config: Option<String>,
) -> Result<(), String> {
    vm::set_agent_strategy(&state.store, &agent, &strategy, config.as_deref())?;
    after_mutation(&state);
    Ok(())
}

#[tauri::command]
fn reorder_agent_bindings(
    state: State<AppState>,
    agent: String,
    provider_ids: Vec<String>,
) -> Result<(), String> {
    vm::reorder_agent_bindings(&state.store, &agent, &provider_ids)?;
    after_mutation(&state);
    Ok(())
}

/// Hub catalog sync (network IO → runs on the async command thread, no UI blocking).
#[tauri::command(async)]
fn sync_hub(state: State<AppState>) -> Result<vm::SyncReportVm, String> {
    let hub_url = vm::ui_settings(&state.aux).hub_url;
    sync::sync_from_hub(&state.aux, &hub_url)
}

// ── Multi-key rotation (spec §4.1 P1) ──

/// A provider's rotation keys (the ones besides the primary key).
#[tauri::command]
fn list_api_keys(state: State<AppState>, provider_id: String) -> Result<Vec<vm::ApiKeyVm>, String> {
    vm::list_api_keys(&state.store, &provider_id)
}

/// Append a rotation key; triggers /reload so the gateway key pool picks it up immediately.
#[tauri::command]
fn add_api_key(
    state: State<AppState>,
    provider_id: String,
    api_key: String,
    label: Option<String>,
) -> Result<vm::ApiKeyVm, String> {
    let vm = vm::add_api_key(&state.store, &provider_id, &api_key, label.as_deref())?;
    after_mutation(&state);
    Ok(vm)
}

#[tauri::command]
fn delete_api_key(state: State<AppState>, id: i64) -> Result<bool, String> {
    let ok = vm::delete_api_key(&state.store, id)?;
    after_mutation(&state);
    Ok(ok)
}

// ── Cost alerts (spec §4.1 P1: notify when usage hits the per-period limit) ──

/// Polled periodically by the frontend; hits not yet notified are returned so
/// the frontend can raise a system notification (KV dedup prevents repeats).
#[tauri::command]
fn check_usage_alerts(state: State<AppState>) -> Result<Vec<vm::UsageAlertVm>, String> {
    vm::check_usage_alerts(&state.store, &state.aux)
}

// ── Config sharing (spec §4.1 P1: export/import of one-click scheme JSON) ──

/// The frontend picks the target path via the dialog plugin first; this writes the file (file IO → async).
#[tauri::command(async)]
fn export_config(state: State<AppState>, path: String) -> Result<usize, String> {
    let json = share::export_config(&state.store)?;
    std::fs::write(&path, &json).map_err(|e| e.to_string())?;
    Ok(state
        .store
        .list_providers()
        .map_err(|e| e.to_string())?
        .len())
}

#[tauri::command(async)]
fn import_config(state: State<AppState>, path: String) -> Result<share::ImportReport, String> {
    let json = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let report = share::import_config(&state.store, &json)?;
    after_mutation(&state);
    Ok(report)
}

#[tauri::command]
fn import_cc_switch(state: State<AppState>) -> import::ImportReportVm {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let home = std::path::PathBuf::from(home).join(".cc-switch");
    let report = import::run_import(
        &state.store,
        Some(&home.join("cc-switch.db")),
        Some(&home.join("config.json")),
    );
    if report.imported > 0 {
        after_mutation(&state);
    }
    report
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let path = db_path();
            if let Some(dir) = path.parent() {
                std::fs::create_dir_all(dir)
                    .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
            }
            let store = Store::open(&path).map_err(|e| e.to_string())?;
            let aux = Aux::open(&path).map_err(|e| e.to_string())?;
            let data_port = env_port("KIWANO_DATA_PORT", 8317);
            let admin_port = env_port("KIWANO_ADMIN_PORT", 8310);

            // Sidecar lifecycle (tech.md §4.6): adopt an already-running
            // daemon first; only spawn when the admin port is silent. The
            // watchdog then keeps it alive for the GUI's lifetime.
            let already_running = sidecar::ping_admin(admin_port);
            let child = if already_running {
                println!("kiwano: adopting running gateway on admin :{admin_port}");
                None
            } else {
                match sidecar::spawn() {
                    Ok(c) => {
                        println!("kiwano: gateway sidecar spawned");
                        Some(c)
                    }
                    Err(e) => {
                        eprintln!("kiwano: gateway sidecar unavailable: {e}");
                        None
                    }
                }
            };

            let ui = vm::ui_settings(&aux);
            app.manage(AppState {
                store,
                aux,
                child: Mutex::new(child),
                admin_port,
                data_port,
            });
            spawn_watchdog(app.handle().clone());

            // Tray + login items (the autostart/close_to_tray settings become real from here on)
            app.handle().plugin(tauri_plugin_autostart::init(
                tauri_plugin_autostart::MacosLauncher::LaunchAgent,
                None,
            ))?;
            setup_tray(app, data_port)?;
            sync_autostart(app.handle(), ui.autostart);

            // Hub catalog uses local JSON for now (bundled catalog.json +
            // hub_cache); network sync protocol v0 is implemented (sync.rs /
            // sync_hub command) but disabled — startup sync and the
            // settings-page entry return once a real Hub is live.
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            get_gateway_status,
            list_providers,
            add_provider,
            update_provider,
            delete_provider,
            enable_provider,
            test_latency,
            list_catalog,
            get_dashboard,
            get_settings,
            update_settings,
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
            reorder_agent_bindings,
            export_config,
            import_config,
            detect_agents,
            probe_agent_versions,
        ])
        .on_window_event(|window, event| {
            // Close-to-tray: intercept CloseRequested and hide the window
            // instead of quitting (toggleable in settings)
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let state = window.app_handle().state::<AppState>();
                if vm::ui_settings(&state.aux).close_to_tray {
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

#[cfg(test)]
mod tests {
    use super::{watchdog_decision, WatchdogAction};

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
}
