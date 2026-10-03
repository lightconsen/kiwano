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
// Comments do not ship. They are how the source explains itself — why the
// drawer closes on resize, why the diagram has two layers — and the source is
// public at site/src/, so nothing is hidden by dropping them from the artifact.
// What they cost is the page: 28 HTML and 50 CSS comments are 6.2 KB of the
// 103 KB page, and because most of them are Chinese (which gzip barely helps)
// that is ~6 KB of the ~39 KB a visitor downloads — 16%, for notes nobody
// reads in view-source.
//
// Two strippers, each run over its own source and never over the assembled
// page. That is not tidiness: the CSS and JS *markers* are themselves
// `/* … */` comments, so a `/* */` pass over the template deletes the very
// placeholders the substitution looks for — and the build then ships a page
// with no stylesheet and no script while reporting success, because the guard
// below only ever checked for markers *left over*, never for markers used.
// Strip the template's HTML comments and the stylesheet's CSS comments, leave
// the markers alone, and inlining stays a plain substitution.
//
// Strip rather than minify the CSS: removing comments leaves every other byte
// of the stylesheet alone, where a CSS minifier is a rewriting step with its
// own opinions about a file this size. The JS bundle needs neither — esbuild
// already drops its comments with `minify: true`.
const stripHtmlComments = (text) => text.replace(/<!--[\s\S]*?-->/g, "");
const stripCssComments = (text) => text.replace(/\/\*[\s\S]*?\*\//g, "");
const css = stripCssComments(readFileSync(join(SRC, "styles.css"), "utf8"));

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
//
// The template's own comments go first, before anything is inlined into it:
// the CSS and the JS contain `<!--`-shaped strings of their own (the
// dictionaries hold HTML fragments), and a regex run over the assembled page
// would be free to eat them. Comments come out of the template, then the code
// goes in, and the code is never scanned.
const next = stripHtmlComments(template)
  .replace(CSS_MARK, () => css)
  .replace(JS_MARK, () => bundle);
// Two directions, and the second one asserts on content rather than on length:
// a page missing its stylesheet and its script is still ~45 KB of template, so
// "did the result get big enough" passes for exactly the failure this is here
// to catch. Looking for the tail of each payload does not — both markers can be
// gone and the page still be empty, which is what shipping a stripped template
// looked like.
const inlined = next.includes(css.slice(-120)) && next.includes(bundle.slice(-120));
if (next.includes(CSS_MARK) || next.includes(JS_MARK) || !inlined) {
  console.error(
    "build-site: the stylesheet or the bundle is not in the page — a marker was" +
      " replaced, or eaten before it could be. The markers are `/* … */` comments" +
      " themselves; strip comments from the sources, never from the template.",
  );
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
