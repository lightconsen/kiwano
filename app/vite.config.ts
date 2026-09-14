// `vitest/config`'s defineConfig is Vite's plus the `test` block, so the test
// runner and the app share one file: a second config would mean a second copy
// of the alias below, and the copy that drifts is the one nobody runs.
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
import tailwindcss from "@tailwindcss/vite";
import path from "node:path";
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  plugins: [react(), tailwindcss()],

  test: {
    // The screens are DOM components: `document` has to exist before they render.
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    // Screens and their imports only — `src/api/dev.ts` is a fixture module, not
    // a unit to be discovered as one.
    include: ["src/**/*.test.{ts,tsx}"],
  },

  resolve: {
    alias: {
      // `import.meta.dirname`, not `__dirname`: vitest loads this config through
      // Vite's native loader, which has no `__dirname` and says so on every run.
      "@": path.resolve(import.meta.dirname, "./src"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
