//! Tray icon, its menu and the OS login item: the pieces that make the app
//! reachable without a window, including close-to-tray and the autostart sync
//! the settings screen writes through.

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
pub(crate) fn refresh_tray<R: tauri::Runtime>(
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

pub(crate) fn setup_tray(app: &tauri::App, data_port: u16) -> tauri::Result<()> {
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
