#!/usr/bin/env node
// Build the marketing site's single-file page (site/index.html) out of the
// sources in site/src/.
//
// What it does: takes site/src/index.html as the template and swaps two
// placeholder markers — /*__SITE_CSS__*/ becomes styles.css verbatim,
// /*__SITE_JS__*/ becomes site/src/main.ts bundled by esbuild (i18n.ts
// inlined, minified, IIFE so the output needs no module support). The result
// overwrites site/index.html,
// which stays git-tracked: the Pages deploy uploads `site/` as committed and
// never builds anything, exactly as before — this script runs locally (and in
// the site workflow) before that upload.
//
// The page deliberately remains ONE file: crawlers read it without executing
// JS, and the OG/JSON-LD metadata must be static. Do not switch the bundle to
// an external .js file without revisiting that decision.
//
// Usage: node scripts/build-site.mjs
//   --check  verify site/index.html matches what the sources build to
//            (used by CI and by sync-site.mjs --check); exits 1 on drift.

import { build } from "esbuild";
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const SRC = join(ROOT, "site", "src");
const OUT = join(ROOT, "site", "index.html");
const check = process.argv.includes("--check");

const CSS_MARK = "/*__SITE_CSS__*/";
const JS_MARK = "/*__SITE_JS__*/";

const template = readFileSync(join(SRC, "index.html"), "utf8");
if (!template.includes(CSS_MARK) || !template.includes(JS_MARK)) {
  console.error("build-site: site/src/index.html lost its markers — nothing to inline into");
  process.exit(2);
}
const css = readFileSync(join(SRC, "styles.css"), "utf8");

const js = await build({
  entryPoints: [join(SRC, "main.ts")],
  bundle: true,
  format: "iife",
  target: "es2020",
  minify: true,
  charset: "utf8",
  write: false,
});
const bundle = js.outputFiles[0].text;

// Replacement via callbacks on purpose: the bundle contains `$` sequences
// (template literals), which a string replacement would mangle.
const next = template
  .replace(CSS_MARK, () => css)
  .replace(JS_MARK, () => bundle);
if (next.includes(CSS_MARK) || next.includes(JS_MARK)) {
  console.error("build-site: placeholder substitution failed — a marker survived replacement");
  process.exit(2);
}

if (check) {
  const current = readFileSync(OUT, "utf8");
  if (current !== next) {
    console.error("build-site: site/index.html is stale — run `node scripts/build-site.mjs`");
    process.exit(1);
  }
  console.log("build-site: site/index.html is current");
  process.exit(0);
}

writeFileSync(OUT, next);
console.log(
  `build-site: site/index.html — template ${(template.length / 1024).toFixed(0)} KB ` +
    `+ css ${(css.length / 1024).toFixed(0)} KB + js ${(bundle.length / 1024).toFixed(0)} KB ` +
    `= ${(next.length / 1024).toFixed(0)} KB`,
);
