// The demo's stand-in for the backend's signals.
//
// In the app the gateway pushes events over Tauri; a browser build has none, so
// lib/updateEvents.ts subscribes to window events named `kiwano:<event>`
// instead. This is the other end of that wire: the simulator says "the numbers
// moved" and the screens re-read the fixtures, exactly as they re-read the real
// API when the gateway says it.

/** Tell the app the numbers moved; no payload, because what to show comes from
    re-reading — one source for the figures, not two that could disagree. */
export function publishUsageTick(): void {
  window.dispatchEvent(new CustomEvent("kiwano:usage-changed"));
}
