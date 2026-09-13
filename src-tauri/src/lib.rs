//! Kiwano GUI backend: Tauri commands backed by the gateway store + sidecar.
//!
//! Data flow (tech.md §4.1): UI → `invoke` → commands here → gateway Store
//! (same SQLite file the sidecar reads) → `POST /reload` on the admin plane
//! (a unix socket / named pipe, see `sidecar`) hot-swaps the gateway route
//! table. The gateway process itself is spawned in `setup`.

mod update;

use std::sync::Mutex;

use kiwano_gateway::store::{RequestLogFilter, Store};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

// The application layer lives in `kiwano-core`, shared with the CLI. Importing
// the module by name keeps every `vm::…` / `pricing::…` call site below
// unchanged — `use` puts the same name in scope that `mod` did.
use kiwano_core::{detect, import, pricing, share, sidecar, sync, vm};
use sidecar::AdminEndpoint;
use vm::Aux;

struct AppState {
    store: Store,
    aux: Aux,
    child: Mutex<Option<std::process::Child>>,
    /// Where the admin plane is, not a port: a socket beside the database, or a
    /// per-user pipe on Windows. Resolved once at startup and shared by every
    /// caller, so nothing re-reads the environment mid-session.
    admin: AdminEndpoint,
    data_port: u16,
    /// Update found by the silent startup check, so the UI can show it without
    /// the user running a check by hand (see `get_pending_update`).
    pending_update: Mutex<Option<update::UpdateInfoVm>>,
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
    sidecar::notify_reload(&state.admin);
}

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

fn spawn_watchdog(handle: tauri::AppHandle) {
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

/// Hub sync at startup: catalog + pricing, retried briefly so a login that
/// beats the network does not settle for the cache. Runs on a plain thread for
/// the same reason `sync_hub` is not `async` (blocking reqwest). Exhausting the
/// retries is logged and nothing else — the Hub is an enhancement, and the
/// cached or bundled data always works offline.
fn spawn_hub_sync(handle: tauri::AppHandle) {
    std::thread::spawn(move || {
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
                std::thread::sleep(std::time::Duration::from_secs(*delay));
            }
            match sync::sync_from_hub(&state.aux, &hub_url) {
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
        // The Hub may have brought a newer price table. Re-run the seed (a
        // no-op when the version and content are unchanged) and reload the
        // daemon so its in-memory table is rebuilt from the mirror.
        match pricing::seed_model_pricing(&state.aux) {
            Ok(r) if r.seeded > 0 => {
                tracing::info!(
                    version = r.version,
                    rows = r.seeded,
                    "hub model pricing seeded"
                );
                sidecar::notify_reload(&state.admin);
            }
            Err(e) => tracing::warn!(error = %e, "model pricing seed failed"),
            _ => {}
        }
    });
}

// ── Tray / autostart (tech.md §3 P1: tray + close-to-tray + launch at login) ──

/// Sync the persisted autostart setting to the OS login items.
///
/// Only writes when it actually differs. Registering a login item rewrites its
/// plist, and macOS announces every rewrite with a "Background Items Added"
/// notice — so calling `enable()` on a launch that is *already* launched by
/// that item, and again on every settings save (the whole object round-trips,
/// the field with it), turns one registration into a stream of them.
///
/// Failures stay silent; the next launch realigns.
fn sync_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if mgr.is_enabled().is_ok_and(|current| current == enabled) {
        return;
    }
    let _ = if enabled { mgr.enable() } else { mgr.disable() };
}

/// Tray menu. `update` adds an entry naming the version the silent check found:
/// tauri-plugin-notification exposes no click callback on desktop (its action
/// API is mobile-only), so the notification can only inform — the tray item is
/// what turns "an update exists" into one click.
fn tray_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_port: u16,
    update: Option<&str>,
) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};

    let open = MenuItem::with_id(app, "open", "Open Kiwano", true, None::<&str>)?;
    let gw = MenuItem::with_id(
        app,
        "gateway",
        format!("Gateway :{data_port}"),
        false,
        None::<&str>,
    )?;
    let sep = PredefinedMenuItem::separator(app)?;
    let upd = match update {
        Some(v) => Some(MenuItem::with_id(
            app,
            "update",
            format!("Update to {v}…"),
            true,
            None::<&str>,
        )?),
        None => None,
    };
    let quit = MenuItem::with_id(app, "quit", "Quit Kiwano", true, None::<&str>)?;

    let mut items: Vec<&dyn tauri::menu::IsMenuItem<R>> = vec![&open, &gw, &sep];
    if let Some(u) = upd.as_ref() {
        items.push(u);
    }
    items.push(&quit);
    Menu::with_items(app, &items)
}

/// Rebuild the tray menu, e.g. after the startup check found an update.
fn refresh_tray<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_port: u16,
    update: Option<&str>,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("kiwano-tray") {
        tray.set_menu(Some(tray_menu(app, data_port, update)?))?;
    }
    Ok(())
}

fn show_main<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

fn setup_tray(app: &tauri::App, data_port: u16) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;

    let menu = tray_menu(app.handle(), data_port, None)?;

    TrayIconBuilder::with_id("kiwano-tray")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip(format!("Kiwano — local gateway :{data_port}"))
        .menu(&menu)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => show_main(app),
            "update" => {
                // The About block reads the pending update on mount, so landing
                // on Settings is enough to show it.
                show_main(app);
                let _ = app.emit("open-settings", ());
            }
            "quit" => app.exit(0), // the daemon outlives the GUI (tech.md §2.4 B)
            _ => {}
        })
        .build(app)?;
    Ok(())
}

#[tauri::command]
fn get_gateway_status(state: State<AppState>) -> vm::GatewayStatusVm {
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

/// Protocol-aware probe: GET the protocol's models route (auth headers only
/// when a key is given) and classify the answer. A missing key is fine —
/// 401/403 still proves the protocol route exists. Async: the probe uses an
/// async HTTP client (a blocking one panics when dropped on the runtime).
#[tauri::command]
async fn test_endpoint(
    protocol: String,
    endpoint: String,
    api_key: Option<String>,
) -> Result<sidecar::ProbeReport, String> {
    sidecar::probe_endpoint(&protocol, &endpoint, api_key.as_deref()).await
}

/// Live model-name list for the Default model picker (requires the API key:
/// cloud providers reject anonymous /models calls).
#[tauri::command]
async fn list_models(
    protocol: String,
    endpoint: String,
    api_key: String,
) -> Result<Vec<String>, String> {
    sidecar::fetch_model_names(&protocol, &endpoint, &api_key).await
}

#[tauri::command]
fn list_catalog(state: State<AppState>) -> vm::CatalogListVm {
    vm::load_catalog(&state.store, &state.aux)
}

#[tauri::command]
fn get_dashboard(
    state: State<AppState>,
    window: String,
    provider_id: Option<String>,
    agent: Option<String>,
) -> Result<vm::DashboardVm, String> {
    vm::build_dashboard(
        &state.store,
        &state.aux,
        &window,
        provider_id.as_deref(),
        agent.as_deref(),
    )
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
    // The gateway re-reads the log config on /reload; ping it when the
    // request-log settings changed so the toggle applies without a restart.
    if patch.get("request_logs").is_some() || patch.get("log_retention_days").is_some() {
        after_mutation(&state);
    }
    Ok(vm)
}

// ── Agent detection and currency metadata ──
//
// Both readers are parameterless or take only the auxiliary connection, so they
// carry no Tauri state themselves; these wrappers exist to give the frontend
// the command names it invokes.

#[tauri::command(async)]
fn detect_agents() -> Vec<detect::AgentDetectVm> {
    detect::detect_agents()
}

#[tauri::command(async)]
fn probe_agent_versions() -> Vec<detect::AgentVersionVm> {
    detect::probe_agent_versions()
}

#[tauri::command]
fn get_currency_meta(state: State<AppState>) -> Result<pricing::CurrencyMetaVm, String> {
    pricing::currency_meta(&state.aux)
}

// ── Plan quota (the reader itself lives in the gateway crate: the same code
//    enforces the ceiling, so the display and the block cannot disagree) ──

#[tauri::command(async)]
fn get_plan_quota(
    state: State<AppState>,
    provider_id: String,
    force: Option<bool>,
) -> Result<kiwano_gateway::plan_quota::PlanQuotaReport, String> {
    kiwano_gateway::plan_quota::get_plan_quota_report(
        &state.store,
        &provider_id,
        force.unwrap_or(false),
    )
}

// ── Request logs (full data-plane audit trail, migration V5) ──

#[tauri::command]
#[allow(clippy::too_many_arguments)]
fn list_request_logs(
    state: State<AppState>,
    page: i64,
    page_size: i64,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
) -> Result<vm::RequestLogListVm, String> {
    vm::list_request_logs(
        &state.store,
        page,
        page_size,
        RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
        },
    )
}

/// Writes the filtered log slice to `path` as CSV. The frontend picks the path
/// from the dialog plugin first — the same split as `export_config` — and this
/// is `async` because a full export is a lot of formatting to block a thread on.
/// `include_bodies` is the export dialog's choice; bodies are always captured,
/// so this is where the file decides whether to carry them. Defaults to off for
/// a caller that does not send it — the safe direction for a file that may be
/// shared.
#[tauri::command(async)]
#[allow(clippy::too_many_arguments)]
fn export_request_logs(
    state: State<AppState>,
    path: String,
    agent: Option<String>,
    provider_id: Option<String>,
    status: Option<String>,
    from: Option<String>,
    to: Option<String>,
    include_bodies: Option<bool>,
) -> Result<vm::RequestLogExportVm, String> {
    vm::export_request_logs_csv(
        &state.store,
        &path,
        RequestLogFilter {
            agent: agent.as_deref(),
            provider_id: provider_id.as_deref(),
            status: status.as_deref(),
            from: from.as_deref(),
            to: to.as_deref(),
        },
        include_bodies.unwrap_or(false),
    )
}

#[tauri::command]
fn get_request_log(
    state: State<AppState>,
    id: i64,
) -> Result<Option<vm::RequestLogDetailVm>, String> {
    vm::get_request_log(&state.store, id)
}

#[tauri::command]
fn clear_request_logs(state: State<AppState>) -> Result<(), String> {
    vm::clear_request_logs(&state.store)
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
fn get_footer_stats(app: AppHandle, state: State<AppState>) -> Result<vm::FooterStatsVm, String> {
    // tauri.conf.json is the single source of truth for the app version —
    // the same one the updater compares against.
    let version = format!("v{}", app.package_info().version);
    vm::build_footer_stats(&state.store, &state.aux, &version)
}

/// The update the silent startup check found, if any. The About block reads
/// this on mount so a user who just saw the "update available" notification
/// finds the same version waiting in Settings.
#[tauri::command]
fn get_pending_update(state: State<AppState>) -> Option<update::UpdateInfoVm> {
    state
        .pending_update
        .lock()
        .ok()
        .and_then(|slot| slot.clone())
}

/// Everything that must happen when a check returns. Kept in one place so the
/// startup check and the manual one cannot drift: both record the result (the
/// update banner and About read it), refresh the tray entry, and tell an open
/// UI.
fn record_update_check(app: &AppHandle, info: Option<&update::UpdateInfoVm>) {
    if let Some(state) = app.try_state::<AppState>() {
        if let Ok(mut slot) = state.pending_update.lock() {
            // A check that finds nothing clears a previous hit: the pending
            // value always describes the latest answer, not the best one.
            *slot = info.cloned();
        }
        let port = state.data_port;
        let _ = refresh_tray(app, port, info.map(|i| i.version.as_str()));
    }
    if info.is_some() {
        let _ = app.emit("update-available", ());
    }
}

/// Settings' "Check for updates". A thin wrapper over the module function so
/// every check — startup or manual — goes through `record_update_check`.
#[tauri::command]
async fn check_app_update(app: AppHandle) -> Result<Option<update::UpdateInfoVm>, String> {
    let info = update::check(app.clone()).await?;
    record_update_check(&app, info.as_ref());
    Ok(info)
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

/// Patch one binding's strategy parameters (weight / local time window).
#[tauri::command]
fn update_agent_binding(
    state: State<AppState>,
    agent: String,
    provider_id: String,
    weight: Option<i64>,
    win_start: Option<String>,
    win_end: Option<String>,
) -> Result<(), String> {
    vm::update_agent_binding(
        &state.store,
        &agent,
        &provider_id,
        weight,
        win_start,
        win_end,
    )?;
    after_mutation(&state);
    Ok(())
}

/// Bind a provider to an agent (appended at the queue tail) and unbind it.
#[tauri::command]
fn add_agent_binding(
    state: State<AppState>,
    agent: String,
    provider_id: String,
) -> Result<(), String> {
    vm::add_agent_binding(&state.store, &agent, &provider_id)?;
    after_mutation(&state);
    Ok(())
}

#[tauri::command]
fn remove_agent_binding(
    state: State<AppState>,
    agent: String,
    provider_id: String,
) -> Result<(), String> {
    vm::remove_agent_binding(&state.store, &agent, &provider_id)?;
    after_mutation(&state);
    Ok(())
}

/// Copy another agent's whole route (strategy + ordered candidates) onto
/// this one, replacing whatever it had.
#[tauri::command]
fn apply_agent_route(state: State<AppState>, target: String, source: String) -> Result<(), String> {
    vm::apply_agent_route(&state.store, &target, &source)?;
    after_mutation(&state);
    Ok(())
}

/// Hub catalog sync. Deliberately a *synchronous* command: Tauri dispatches
/// those onto its blocking thread pool, which is where a blocking reqwest
/// client belongs — an `async` command would run it on the tokio runtime, and
/// dropping a blocking client there panics (see sidecar.rs on the same trap).
#[tauri::command]
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
///
/// Notification only: the gateway enforces the limits, from its own timer, so
/// they hold whether or not this app is running. Nothing here disables a
/// provider any more.
#[tauri::command]
fn check_usage_alerts(state: State<AppState>) -> Result<Vec<vm::UsageAlertVm>, String> {
    vm::check_usage_alerts(&state.store, &state.aux, true)
}

// ── Config sharing (spec §4.1 P1: export/import of one-click scheme JSON) ──

/// The frontend picks the target path via the dialog plugin first; this writes the file (file IO → async).
/// Credentials are omitted unless `include_keys` is set (local backup only).
#[tauri::command(async)]
fn export_config(
    state: State<AppState>,
    path: String,
    include_keys: Option<bool>,
) -> Result<usize, String> {
    share::export_config_to_file(&state.store, &path, include_keys.unwrap_or(false))
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
// ── Logs ──

/// Reveal the log directory in the OS file manager. The path follows the
/// database, which only this side knows, so the UI asks rather than guesses.
#[tauri::command]
fn open_log_folder(app: AppHandle) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let dir = kiwano_gateway::logging::log_dir(&db_path());
    // It may not exist yet — nothing has been logged before the directory is
    // made, and an empty folder explains itself better than an error does.
    let _ = std::fs::create_dir_all(&dir);
    app.opener()
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}

pub fn run() {
    // Before anything that can fail, and before the webview: a packaged app is
    // started by launchd, where stdout goes to /dev/null, so without this the
    // only way to read what happened is to run it from a terminal.
    kiwano_gateway::logging::init(&db_path());

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
            let aux = Aux::open(&path).map_err(|e| e.to_string())?;
            let data_port = env_port("KIWANO_DATA_PORT", 8317);
            // Not a port: the admin plane is a socket beside the database, or a
            // per-user named pipe.
            let admin = sidecar::admin_endpoint();

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

            let ui = vm::ui_settings(&aux);
            // Seed the bundled model price table into model_pricing
            // (version-gated no-op after the first run).
            match pricing::seed_model_pricing(&aux) {
                Ok(r) if !r.skipped => {
                    tracing::info!(version = r.version, rows = r.seeded, "model pricing seeded")
                }
                Err(e) => tracing::warn!(error = %e, "model pricing seed failed"),
                _ => {}
            }
            app.manage(AppState {
                store,
                aux,
                child: Mutex::new(child),
                admin,
                data_port,
                pending_update: Mutex::new(None),
            });
            spawn_watchdog(app.handle().clone());
            // Hub catalog: one-shot conditional sync (skips the download when
            // the manifest sha matches the cache). Silent, opt-out-free, and
            // failure-tolerant — the bundled catalog is the offline fallback.
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
            list_providers,
            add_provider,
            update_provider,
            delete_provider,
            enable_provider,
            test_latency,
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
            update_agent_binding,
            add_agent_binding,
            remove_agent_binding,
            apply_agent_route,
            export_config,
            import_config,
            detect_agents,
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
