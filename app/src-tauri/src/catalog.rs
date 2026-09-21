//! The Hub catalog as the UI reads it: the entries, the price mirror they price
//! against, and the one action that leaves the app — opening a vendor page in
//! the user's browser.

use tauri::{AppHandle, State};

use crate::state::AppState;
use kiwano_core::{pricing, vm};

#[tauri::command]
pub fn list_catalog(state: State<AppState>) -> vm::CatalogListVm {
    vm::load_catalog(&state.store, &state.aux)
}

/// The price mirror — the rows the gateway charges with — so the Models page can
/// price every model a provider serves, not just the entry's flagship.
#[tauri::command]
pub fn list_model_prices(state: State<AppState>) -> Result<Vec<pricing::ModelPriceEntry>, String> {
    pricing::list_model_prices(&state.store)
}

/// Open a vendor's site in the user's browser.
///
/// The URL comes from the Hub's catalog, which the app does not control, so it
/// is fenced here rather than trusted: this hands a string to the OS opener, and
/// anything but http(s) would let a catalog reach a local file or a registered
/// scheme handler. A rejected URL is an error the caller shows, not a silent
/// no-op.
#[tauri::command]
pub fn open_url(app: AppHandle, url: String) -> Result<(), String> {
    use tauri_plugin_opener::OpenerExt;
    let trimmed = url.trim();
    if !(trimmed.starts_with("https://") || trimmed.starts_with("http://")) {
        return Err(format!("refusing to open a non-http(s) url: {trimmed}"));
    }
    app.opener()
        .open_url(trimmed, None::<&str>)
        .map_err(|e| e.to_string())
}
