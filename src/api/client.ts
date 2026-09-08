// Single entry point for UI data access.
// - Inside the Tauri WebView → tauriApi (real SQLite + gateway sidecar)
// - Pure browser dev (pnpm dev without Tauri) → mockApi (data matching the prototype verbatim)
// The component layer is unaware of the difference; zero component changes after integration.
import type { KiwanoApi } from "./types";
import { mockApi } from "./mock";
import { tauriApi } from "./tauri";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: KiwanoApi = inTauri ? tauriApi : mockApi;
