//! Kiwano GUI backend: Tauri commands backed by the gateway store + sidecar.
//!
//! Data flow (tech.md §4.1): UI → `invoke` → commands here → gateway Store
//! (same SQLite file the sidecar reads) → `POST :8310/reload` hot-swaps the
//! gateway route table. The gateway process itself is spawned in `setup`.

mod sidecar;
mod vm;

use std::sync::Mutex;

use kiwano_gateway::store::Store;
use tauri::{Manager, RunEvent, State};

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
    std::path::PathBuf::from(home).join(".kiwano").join("kiwano.db")
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
fn add_provider(state: State<AppState>, input: vm::NewProviderInput) -> Result<vm::ProviderVm, String> {
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
fn list_catalog() -> vm::CatalogListVm {
    vm::load_catalog()
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
    state: State<AppState>,
    patch: serde_json::Value,
) -> Result<vm::SettingsVm, String> {
    vm::update_settings(&state.store, &state.aux, &patch)
}

#[tauri::command]
fn set_agent_takeover(state: State<AppState>, agent: String, enabled: bool) -> Result<(), String> {
    vm::set_agent_takeover(&state.store, &agent, enabled)
}

#[tauri::command]
fn get_footer_stats(state: State<AppState>) -> Result<vm::FooterStatsVm, String> {
    vm::build_footer_stats(&state.store)
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
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

            // Sidecar lifecycle (tech.md §4.6): spawn best-effort; the status
            // chip reflects reality via admin pings. Env override supported.
            let child = match sidecar::spawn() {
                Ok(c) => {
                    println!("kiwano: gateway sidecar spawned");
                    Some(c)
                }
                Err(e) => {
                    eprintln!("kiwano: gateway sidecar unavailable: {e}");
                    None
                }
            };

            app.manage(AppState {
                store,
                aux,
                child: Mutex::new(child),
                admin_port,
                data_port,
            });
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
        ])
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app_handle, event| {
            if let RunEvent::Exit = event {
                if let Some(state) = app_handle.try_state::<AppState>() {
                    if let Ok(mut child) = state.child.lock() {
                        if let Some(c) = child.as_mut() {
                            let _ = c.kill();
                        }
                    }
                }
            }
        });
}
