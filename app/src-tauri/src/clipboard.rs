//! The one action behind every Copy button: put a string on the system
//! clipboard.
//!
//! Native rather than the webview's own `execCommand("copy")`, and the reason
//! is a freeze, not taste. WKWebView has no Clipboard API, so the web route is
//! "select the text in a `<textarea>` and let WebKit's editor serialize it" —
//! and that serialization runs on the main thread, over the whole selection.
//! A request log's Copy carries its bodies, which the gateway stores in full by
//! default (no `max_body_bytes`), and a coding agent's request is routinely
//! several megabytes. Measured on macOS in a WKWebView: 2 MB took 0.5 s, 7 MB
//! 6.8 s, 14 MB 37 s — each of them a window that answers nothing until it is
//! over. The OS pasteboard takes the string in one call; no text has to be laid
//! out, and the cost does not grow with the size of the log.
//!
//! The plugin keeps a live `arboard::Clipboard` in its state for exactly this
//! reason: on X11 the process owns the selection, and a clipboard dropped at the
//! end of the call would take the copied text with it.

use tauri::AppHandle;
use tauri_plugin_clipboard_manager::ClipboardExt;

#[tauri::command(async)]
pub fn copy_text(app: AppHandle, text: String) -> Result<(), String> {
    app.clipboard().write_text(text).map_err(|e| e.to_string())
}
