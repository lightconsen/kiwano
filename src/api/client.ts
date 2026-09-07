// UI 数据访问唯一入口。
// - Tauri WebView 内 → tauriApi（真实 SQLite + gateway sidecar）
// - 纯浏览器 dev（pnpm dev 无 Tauri）→ mockApi（与原型逐字一致的数据）
// 组件层不感知差异；集成后组件零改动。
import type { KiwanoApi } from "./types";
import { mockApi } from "./mock";
import { tauriApi } from "./tauri";

const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;

export const api: KiwanoApi = inTauri ? tauriApi : mockApi;
