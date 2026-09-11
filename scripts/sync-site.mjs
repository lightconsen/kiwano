#!/usr/bin/env node
// Stamp the version and licence from `package.json` into `site/index.html`.
//
// The landing page is a single static file with no build step, which is worth
// keeping — but two values in it are copies of facts the project already
// records elsewhere, and both had gone stale: the release line advertised
// v0.1.3 while the project was on 0.1.7, and the footer still offered
// Apache-2.0 two releases after the licence moved to GPL-3.0-or-later. A
// licence that names the wrong licence is worse than a stale version: it is a
// factual claim about how the code may be used.
//
// So neither is written by hand. `package.json` carries both (`version` and
// `license`), and this rewrites the page from it. It runs before the Pages
// upload, so the deployed site cannot be stale whatever the committed file
// says; running it locally keeps the committed file honest too.
//
// Usage: node scripts/sync-site.mjs [--check]
//   --check  report what would change and exit 1 if anything would; rewrite
//            nothing. For a gate that should fail rather than fix.

import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SITE = join(ROOT, "site", "index.html");
const check = process.argv.includes("--check");

const pkg = JSON.parse(readFileSync(join(ROOT, "package.json"), "utf8"));
const { version, license } = pkg;
if (!version || !license) {
  console.error("sync-site: package.json has no `version` or no `license`");
  process.exit(2);
}

const html = readFileSync(SITE, "utf8");
const edits = [];

// 1. The release line. Anchored on the platform list that follows it rather
//    than on `v<semver>` alone, so a version mentioned in prose elsewhere is
//    not caught by accident. Appears twice: the page's own default text (which
//    is what a reader without JS sees) and the English dictionary entry.
const RELEASE_LINE = /v\d+\.\d+\.\d+(?= · <b>macOS 12\+<\/b>)/g;
const releaseHits = [...html.matchAll(RELEASE_LINE)];
if (releaseHits.length === 0) {
  console.error(
    "sync-site: no release line matched — the hero note was reworded, so the\n" +
      "           anchor in this script no longer describes the page. Fix the\n" +
      "           pattern rather than deleting the check: silence here means the\n" +
      "           version silently stops being stamped.",
  );
  process.exit(2);
}

// 2. The licence link's text, found by its target: the anchor that points at
//    the LICENSE file. Its text is the SPDX identifier, verbatim.
const LICENSE_LINK = /(<a href="[^"]*\/LICENSE"[^>]*>)([^<]*)(<\/a>)/g;
const licenseHits = [...html.matchAll(LICENSE_LINK)];
if (licenseHits.length === 0) {
  console.error("sync-site: no link to the LICENSE file found in the footer");
  process.exit(2);
}

let next = html.replace(RELEASE_LINE, `v${version}`);
next = next.replace(LICENSE_LINK, (_, open, text, close) => {
  if (text !== license) edits.push(`licence link: ${text} -> ${license}`);
  return `${open}${license}${close}`;
});
for (const hit of releaseHits) {
  if (hit[0] !== `v${version}`) {
    edits.push(`release line: ${hit[0]} -> v${version}`);
  }
}

if (edits.length === 0) {
  console.log(`sync-site: already current (v${version}, ${license})`);
  process.exit(0);
}
if (check) {
  console.error(`sync-site: site/index.html is stale:\n  ${edits.join("\n  ")}`);
  process.exit(1);
}
writeFileSync(SITE, next);
console.log(`sync-site: ${edits.join("\n           ")}`);
