#!/usr/bin/env node
// `pnpm tauri <args>` — build the gateway sidecar for the profile the CLI is
// about to use, then hand over to it.
//
// The sidecar has to sit beside the GUI binary, and the profile decides which
// directory that is: `tauri dev` runs `target/debug/kiwano`, `tauri build`
// `target/release/kiwano`. A release sidecar next to a debug GUI is in a
// directory the app never looks in — and because this machine usually has a
// debug gateway lying around from earlier builds, that mistake hides itself
// exactly the way the missing-sidecar bug did.
//
// One entry point covers both commands because the CLI is what knows which
// subcommand it got. release.yml reaches this too: with `tauriScript` unset,
// tauri-action resolves the build to `pnpm tauri build`.

import { execFileSync, spawnSync } from "node:child_process";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const args = process.argv.slice(2);
const profile = args.includes("build") ? "release" : "debug";

execFileSync(
  process.execPath,
  [join(ROOT, "scripts", "build-sidecar.mjs"), "--profile", profile],
  { cwd: ROOT, stdio: "inherit" },
);

// `shell` on Windows only: `tauri` there is a .cmd shim, which execFile cannot
// launch directly.
const cli = spawnSync("tauri", args, {
  cwd: ROOT,
  stdio: "inherit",
  shell: process.platform === "win32",
});
process.exit(cli.status ?? 1);
