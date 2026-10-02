// The site's live demo: the app itself, in a browser, over the dev fixtures.
//
// Nothing here re-implements a screen. It mounts the same `App` the desktop
// build mounts — same components, same screens, same styles — with the same
// client the browser dev server already uses (`src/api/client.ts` picks devApi
// outside Tauri), and starts the simulator that gives it traffic. That is the
// whole point of building it this way: the demo cannot drift from the product,
// because it *is* the product with sample data.
//
// What it is not: the gateway. Numbers move because `simulate.ts` moves them,
// and the one thing the demo has to fake — the backend's "something changed"
// signal — is the browser branch in `lib/updateEvents.ts`.

import React from "react";
import ReactDOM from "react-dom/client";
import "@fontsource-variable/inter";
import "@fontsource/jetbrains-mono/400.css";
import "@fontsource/jetbrains-mono/500.css";
import "@fontsource/jetbrains-mono/600.css";
import "../index.css";
import App from "../App";
import { DICTIONARIES, resolveLocale, setLocale } from "../i18n";
import { settings } from "../api/dev/settings";
import { useDemoDataset } from "./dataset";
import { setPaused, startSimulation } from "./simulate";

// The window the desktop app opens at, and the size the screens are laid out
// for — the demo frame is that window, not a responsive impression of it.
document.documentElement.dataset.demo = "true";

/** Adopt the framing page's language.
 *
 * The real app decides its own — a settings preference that defaults to the
 * browser's — which is right for someone who installed it and wrong for a
 * demo framed by a page that has its own language switch: an English page
 * could be showing a Chinese demo, or the reverse.
 *
 * Two ways in, because the visitor can switch at two different times: the
 * query string, for a demo created after they chose (the usual case — the
 * header is at the top, the frame loads further down), and a message, for a
 * switch made while the demo is already running. Both land on the same two
 * things: the fixture, so the app's Settings screen agrees with its own UI,
 * and the i18n runtime, which is what repaints. */
function adoptLanguage(pref: unknown) {
  // 页面发来的是它自己的语言 id,只认应用真的 ship 了的那几门(`DICTIONARIES`
  // 的键):页面先加了新语言而应用还没有时,这道闸就是"页面是日文、里面还是英
  // 文"不会发生的原因。别的值一律不理,演示保持自己的语言。
  if (typeof pref !== "string" || !(pref in DICTIONARIES)) return;
  const locale = resolveLocale(pref);
  settings.language = locale;
  setLocale(locale);
}

// The dataset first: the screens must mount against the trimmed fixture, not
// re-render into it a frame later — and before the first paint, so nothing
// flashes in the browser's language on its way to the page's.
useDemoDataset();
adoptLanguage(new URLSearchParams(window.location.search).get("lang"));

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

startSimulation();

// The landing page frames this page, so it is also the one that knows whether
// anyone is looking at it: it posts as the frame enters and leaves the viewport.
// Each field is applied only when present — the two are sent independently.
window.addEventListener("message", (e) => {
  if (e.origin !== window.location.origin) return;
  if (e.data?.type !== "kiwano-demo") return;
  if (typeof e.data.visible === "boolean") setPaused(!e.data.visible);
  if (typeof e.data.lang === "string") adoptLanguage(e.data.lang);
});
