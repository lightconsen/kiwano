#!/usr/bin/env node
// Build the site's live demo (site/demo/) out of the app's own source.
//
// What it is: the real `App` — every screen, every component, the app's CSS —
// compiled for the browser over the dev fixtures (`app/src/api/dev`) and entered
// at `app/demo.html`. The landing page frames it at /demo/ and swaps it in when
// a visitor reaches the hero shot, so what they see running is the product, not
// a mock-up of it. Regenerate it whenever anything under `app/src/` changes or
// the shipped demo drifts from the app.
//
// Two things this wrapper does that the vite config cannot:
//   - renames the emitted `demo.html` to `index.html`, so `/demo/` is a page
//     rather than a directory listing (a multi-input build names the file after
//     its input);
//   - reports what it wrote, because the output is git-tracked — the site's
//     deploy uploads `site/` as committed and never builds the app itself.
//
// Usage: node scripts/build-demo.mjs

import { execFileSync } from "node:child_process";
import { cpSync, readdirSync, renameSync, rmSync, statSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");
const OUT = path.join(ROOT, "site", "demo");

execFileSync("pnpm", ["-C", "app", "build:demo"], { cwd: ROOT, stdio: "inherit" });

const emitted = path.join(OUT, "demo.html");
renameSync(emitted, path.join(OUT, "index.html"));

// The demo's own Hub: the four brand marks its shelf rows resolve their logo
// against (see app/src/demo/dataset.ts). Vite only fingerprints what the code
// imports, and these are fetched by URL at runtime, so they come across by hand.
const HUB_LOGS = path.join(OUT, "hub", "logos");
rmSync(path.join(OUT, "hub"), { recursive: true, force: true });
cpSync(path.join(ROOT, "app", "src", "demo", "hub", "logos"), HUB_LOGS, { recursive: true });

let bytes = 0;
let files = 0;
const walk = (dir) => {
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    const full = path.join(dir, entry.name);
    if (entry.isDirectory()) walk(full);
    else {
      bytes += statSync(full).size;
      files += 1;
    }
  }
};
walk(OUT);
console.log(
  `build-demo: site/demo/ — ${files} files, ${(bytes / 1024 / 1024).toFixed(1)} MB ` +
    "(committed; the site deploy serves it as written)",
);
