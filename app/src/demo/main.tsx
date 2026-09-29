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
import { startSimulation } from "./simulate";

// The window the desktop app opens at, and the size the screens are laid out
// for — the demo frame is that window, not a responsive impression of it.
document.documentElement.dataset.demo = "true";

ReactDOM.createRoot(document.getElementById("root") as HTMLElement).render(
  <React.StrictMode>
    <App />
  </React.StrictMode>,
);

startSimulation();
