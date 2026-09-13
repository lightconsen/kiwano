#!/usr/bin/env node
// Build kiwano-gateway and stage it for Tauri's `externalBin`.
//
// Tauri resolves the config entry `binaries/kiwano-gateway` to
// app/src-tauri/binaries/kiwano-gateway-<target-triple>[.exe] — the triple the
// bundle is being built *for*, which is not always the host's (release.yml
// builds x86_64-apple-darwin on an arm64 runner). The hook environment carries
// no target triple — TAURI_ENV_TARGET_TRIPLE does not exist — so it has to be
// handed in: `--target`, or KIWANO_SIDECAR_TARGET.
//
// The `cargo build` below also leaves the binary beside the GUI binary the same
// invocation produces (target/<profile>/ in dev, target/<triple>/<profile>/
// under --target), which is where src-tauri/src/sidecar.rs looks for it at
// runtime. That is why dev does not need a second copy: cargo's own output is
// already the sibling. Tauri copies externalBin into the bundle only, and
// tauri dev copies nothing at all.
//
// Usage: node scripts/build-sidecar.mjs [--target <triple>] [--profile debug|release]
// Env:   KIWANO_SIDECAR_TARGET  triple to use when --target is absent
//
// Node rather than bash because this runs on windows-latest in CI and on dev
// machines whose shell is not necessarily POSIX.

import { execFileSync } from "node:child_process";
import { copyFileSync, mkdirSync, rmSync, statSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const STAGE = join(ROOT, "app", "src-tauri", "binaries");

const argv = process.argv.slice(2);
const flag = (name, fallback) => {
  const i = argv.indexOf(name);
  return i === -1 ? fallback : argv[i + 1];
};

const profile = flag("--profile", "debug");
if (profile !== "debug" && profile !== "release") {
  console.error(`build-sidecar: --profile must be debug or release, got ${profile}`);
  process.exit(2);
}

const requested = flag("--target", process.env.KIWANO_SIDECAR_TARGET || "");
// The `host:` line is the triple this toolchain compiles for when --target is
// absent. (`rustc --print host-tuple` would be tidier but needs Rust 1.84+.)
const host = /^host: (.+)$/m.exec(execFileSync("rustc", ["-vV"], { encoding: "utf8" }))?.[1]?.trim();
if (!requested && !host) {
  console.error("build-sidecar: cannot read the host triple from `rustc -vV`; pass --target");
  process.exit(2);
}
const triple = requested || host;

const exe = triple.includes("windows") ? ".exe" : "";

// No --locked: this runs on every dev iteration, and a lockfile in flux would
// fail here for a reason that has nothing to do with the sidecar. CI's
// `cargo test --locked` is what guards the lock.
const args = ["build", "-p", "kiwano-gateway"];
if (profile === "release") args.push("--release");
if (requested) args.push("--target", triple);

console.log(`build-sidecar: cargo ${args.join(" ")}`);
execFileSync("cargo", args, { cwd: ROOT, stdio: "inherit" });

const built = join(ROOT, "target", ...(requested ? [triple] : []), profile, `kiwano-gateway${exe}`);
const staged = join(STAGE, `kiwano-gateway-${triple}${exe}`);

mkdirSync(STAGE, { recursive: true });
copyFileSync(built, staged);
// Tauri takes the triple-suffixed name. A bare one left here would be picked up
// by a hand-written invocation and silently bundled as the wrong arch.
rmSync(join(STAGE, `kiwano-gateway${exe}`), { force: true });

console.log(`build-sidecar: staged ${staged} (${statSync(staged).size} bytes)`);
console.log(`build-sidecar: runtime sibling ${built}`);
