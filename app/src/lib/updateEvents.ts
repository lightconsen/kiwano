// Event bridges for the tray's update entry point, the startup update check and
// the gateway's live numbers. Desktop notifications cannot be clicked —
// tauri-plugin-notification's action API is mobile-only — so the tray menu
// carries the actionable item and the backend signals the UI over these events.
// The usage tick rides the same bridge because it is the same kind of thing: the
// backend knows something the window cannot see for itself.
//
// Outside the Tauri webview (browser dev, `src/api/dev.ts`) there is no tray
// and no backend, so every subscription here is a no-op.
import { listen } from "@tauri-apps/api/event";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

/** Subscribe to a backend signal; returns an unsubscribe function. */
function subscribe(event: string, cb: () => void): () => void {
  if (!inTauri) return () => {};
  let unlisten: (() => void) | null = null;
  let disposed = false;
  listen(event, () => cb())
    .then((un) => {
      if (disposed) un();
      else unlisten = un;
    })
    .catch(() => {});
  return () => {
    disposed = true;
    unlisten?.();
  };
}

/** The tray's "Update to vX…" item was clicked. */
export function onOpenSettings(cb: () => void): () => void {
  return subscribe("open-settings", cb);
}

/**
 * The startup check just found an update. The event carries no payload on
 * purpose: the handler re-reads the pending update through the API, so the
 * version shown always comes from one place.
 */
export function onUpdateAvailable(cb: () => void): () => void {
  return subscribe("update-available", cb);
}

/**
 * The gateway has just recorded a request, so the numbers on disk have moved.
 *
 * One tick per request, coalesced by the backend to at most one per second. The
 * event carries no payload for the reason the update one does not either: what
 * to show comes from re-reading the API, so there is one source for it and not
 * two that could disagree.
 */
export function onUsageChanged(cb: () => void): () => void {
  return subscribe("usage-changed", cb);
}
