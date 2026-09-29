// The site's live demo: a second build of the same source, entered at
// `demo.html` instead of the app's `index.html`, emitted into `site/demo/`.
//
// A separate config rather than a second rollup input on the app's, because the
// two builds answer to different rules: the app's output is bundled into the
// Tauri binary and served from the app root, while this one is a static page
// under /demo/ on the marketing site. Keeping them apart means neither can
// accidentally change what ships in the desktop app.
//
// Run with: pnpm -C app build:demo   (scripts/build-demo.mjs wraps it)
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";

export default defineConfig({
  plugins: [react(), tailwindcss()],
  // Absolute, because the page is served at /demo/ and nothing there can know
  // how deep the site is mounted.
  base: "/demo/",
  build: {
    outDir: path.resolve(import.meta.dirname, "../site/demo"),
    emptyOutDir: true,
    // One page, visited after the marketing copy: a single chunk is fewer
    // round trips than a split, and there is no route to split on.
    rollupOptions: {
      input: path.resolve(import.meta.dirname, "demo.html"),
    },
  },
});
