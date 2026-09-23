//! Tray icon, its menu and the OS login item: the pieces that make the app
//! reachable without a window, including close-to-tray and the autostart sync
//! the settings screen writes through.
//!
//! The menu also carries the recent gateway events (a DLP finding, a tripped
//! limit, a refused key): a desktop notification cannot be clicked (tauri's
//! action API is mobile-only), so the tray item is the one click from "told"
//! to "looking" — it opens the app and deep-links where the event lives.

use std::sync::Mutex;

use tauri::{Emitter, Manager};

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
///
/// # The check is on the flag, not on the path
///
/// `is_enabled` answers "is there a login item", not "does it point at *this*
/// executable" — and the plugin records a path when it registers one. So an
/// item that names a binary this app no longer is still reads as enabled, this
/// returns early, and it never gets corrected.
///
/// That is not hypothetical: the 0.1.9 rename made the app's executable
/// `kiwano-app`, and on Windows an install whose *Launch at login* was already
/// on kept a registry entry naming the old `kiwano.exe` — now the name of the
/// command-line client, and a different program entirely. The consequence is an
/// old copy of the app starting at login. There is no self-heal, deliberately:
/// rewriting the item on every launch is the macOS "Background Items Added"
/// spam this early return exists to avoid, and comparing registered paths is
/// platform-specific work the plugin does not expose. The CHANGELOG tells
/// affected users to reinstall once or toggle the setting, which does rewrite
/// it.
pub(crate) fn sync_autostart(app: &tauri::AppHandle, enabled: bool) {
    use tauri_plugin_autostart::ManagerExt;
    let mgr = app.autolaunch();
    if mgr.is_enabled().is_ok_and(|current| current == enabled) {
        return;
    }
    let _ = if enabled { mgr.enable() } else { mgr.disable() };
}

/// One recent gateway event, as the tray shows it: the display line and the
/// deep-link the item carries (`#dashboard/log/7`, `#providers`, …).
#[derive(Debug, Clone)]
pub(crate) struct TrayEvent {
    pub label: String,
    pub link: String,
}

/// The recent events the tray menu shows, newest first, capped — a menu is not
/// a log, and five lines is where "recent" stops being true.
const TRAY_EVENT_CAP: usize = 5;

pub(crate) static TRAY_EVENTS: Mutex<Vec<TrayEvent>> = Mutex::new(Vec::new());

/// Record a gateway event for the tray, and rebuild the menu around it.
///
/// Called from the event bridge on every typed event. The label is the
/// gateway's own note line (rule names, never matched values) or the reason
/// text; `link` is the deep-link the webview navigates to.
pub(crate) fn refresh_with_event<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    payload: &str,
) -> tauri::Result<()> {
    let Ok(v) = serde_json::from_str::<serde_json::Value>(payload) else {
        return Ok(());
    };
    let (label, link) = match v["kind"].as_str() {
        Some("dlp_finding") => (
            format!(
                "Credential: {} — {}",
                v["note"].as_str().unwrap_or("finding"),
                v["agent"].as_str().unwrap_or("agent")
            ),
            format!("#dashboard/log/{}", v["log_id"].as_i64().unwrap_or(0)),
        ),
        Some("limit_hit") => (
            format!("Limit reached: {}", v["reason"].as_str().unwrap_or("")),
            "#dashboard".to_string(),
        ),
        Some("limit_cleared") => (
            format!("Limit cleared: {}", v["provider_id"].as_str().unwrap_or("")),
            "#dashboard".to_string(),
        ),
        Some("auth_failed") => (
            format!(
                "Key invalid: {} ({})",
                v["provider_id"].as_str().unwrap_or(""),
                v["agent"].as_str().unwrap_or("agent")
            ),
            "#providers".to_string(),
        ),
        _ => return Ok(()), // usage ticks and unknown kinds do not surface here
    };

    let link_clone = link.clone();
    let mut events = TRAY_EVENTS.lock().expect("tray events poisoned");
    // The same fact re-delivered after a reconnect replaces its old line
    // rather than stacking a duplicate.
    events.retain(|e| e.link != link_clone);
    events.insert(0, TrayEvent { label, link });
    events.truncate(TRAY_EVENT_CAP);
    let snapshot = events.clone();
    drop(events);

    if let Some(tray) = app.tray_by_id("kiwano-tray") {
        if let Some(state) = app.try_state::<crate::state::AppState>() {
            tray.set_menu(Some(tray_menu(
                app,
                state.data_port,
                None,
                Some(&snapshot),
            )?))?;
        }
    }
    Ok(())
}

/// Tray menu. `update` adds an entry naming the version the silent check found:
/// tauri-plugin-notification exposes no click callback on desktop (its action
/// API is mobile-only), so the notification can only inform — the tray item is
/// what turns "an update exists" into one click. The `events` entries work the
/// same way for gateway events, which the same plugin limitation keeps
/// clickless as notifications.
fn tray_menu<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_port: u16,
    update: Option<&str>,
    events: Option<&[TrayEvent]>,
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
    let sep2 = PredefinedMenuItem::separator(app)?;

    let mut items: Vec<&dyn tauri::menu::IsMenuItem<R>> = vec![&open, &gw, &sep];

    // Recent events, each its own item and its own deep-link. Enabled because
    // clicking is the whole point; a stale fact stays visible until replaced —
    // the tray is the "what have I missed", not a live status.
    let event_items: Vec<MenuItem<R>> = match events {
        Some(list) if !list.is_empty() => list
            .iter()
            .map(|e| {
                MenuItem::with_id(
                    app,
                    format!("goto:{}", e.link),
                    e.label.clone(),
                    true,
                    None::<&str>,
                )
            })
            .collect::<Result<Vec<_>, _>>()?,
        _ => Vec::new(),
    };
    if !event_items.is_empty() {
        items.push(&event_items[0]);
        for item in event_items.iter().skip(1) {
            items.push(item);
        }
        items.push(&sep2);
    }

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
    if let Some(u) = upd.as_ref() {
        items.push(u);
    }
    let quit = MenuItem::with_id(app, "quit", "Quit Kiwano", true, None::<&str>)?;
    items.push(&quit);
    Menu::with_items(app, &items)
}

/// Rebuild the tray menu, e.g. after the startup check found an update.
pub(crate) fn refresh_tray<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    data_port: u16,
    update: Option<&str>,
) -> tauri::Result<()> {
    if let Some(tray) = app.tray_by_id("kiwano-tray") {
        let events = TRAY_EVENTS.lock().expect("tray events poisoned").clone();
        tray.set_menu(Some(tray_menu(app, data_port, update, Some(&events))?))?;
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

pub(crate) fn setup_tray(app: &tauri::App, data_port: u16) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;

    let menu = tray_menu(app.handle(), data_port, None, None)?;

    TrayIconBuilder::with_id("kiwano-tray")
        .icon(app.default_window_icon().expect("bundle icon").clone())
        .tooltip(format!("Kiwano — local gateway :{data_port}"))
        .menu(&menu)
        .on_menu_event(|app, event| {
            let id = event.id.as_ref().to_string();
            if let Some(link) = id.strip_prefix("goto:") {
                // Deep-link: open the window and hand the webview the link to
                // navigate — the same route the in-app banners deep-link to.
                show_main(app);
                let _ = app.emit("open-gateway-event", link.to_string());
                return;
            }
            match id.as_str() {
                "open" => show_main(app),
                "update" => {
                    // The About block reads the pending update on mount, so landing
                    // on Settings is enough to show it.
                    show_main(app);
                    let _ = app.emit("open-settings", ());
                }
                "quit" => app.exit(0), // the daemon outlives the GUI (tech.md §2.4 B)
                _ => {}
            }
        })
        .build(app)?;
    Ok(())
}
