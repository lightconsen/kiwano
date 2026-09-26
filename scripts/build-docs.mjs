#!/usr/bin/env node
// Render the user-facing docs (docs/*.md) into site/docs/<slug>/index.html.
//
// The site keeps its no-build-step property at serve time: this script runs
// before the Pages upload (site.yml, right after sync-site.mjs) and writes
// plain HTML that git tracks, so what is served is what was committed. The
// source of truth for content stays docs/*.md on GitHub; the site is only a
// rendering of it.
//
// Publishing is an explicit manifest, not a directory scan: a page goes up
// by being listed in PAGES below. docs/ also holds internal design and
// planning notes (custom-agents, sign, upgrade, request-logs-applications)
// — written for contributors, in Chinese, citing internal commits — and
// those stay GitHub-only because the manifest does not name them.
//
// The sitemap is regenerated here too: docs pages carry the last commit
// date of their source markdown (read from git, so it is accurate by
// construction), while the landing page still carries no lastmod — its only
// changing fact is stamped by sync-site.mjs, and a hand-maintained date
// would go stale the way the hero version line did.
//
// Usage: node scripts/build-docs.mjs

import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DOCS = join(ROOT, "docs");
const SITE_DOCS = join(ROOT, "site", "docs");
const GITHUB_BLOB = "https://github.com/lightconsen/kiwano/blob/main";
const SITE = "https://kiwano.cc";

// The published set, in sidebar and prev/next order. slugs equal the source
// file's basename so relative links between docs (`[x](strategies.md)`)
// rewrite mechanically.
const PAGES = [
  { slug: "getting-started", file: "getting-started.md" },
  { slug: "agent-takeover", file: "agent-takeover.md" },
  { slug: "strategies", file: "strategies.md" },
  { slug: "cli", file: "cli.md" },
];

const require_ = createRequire(import.meta.url);
const _marked = require_(join(ROOT, "scripts", "vendor", "marked.min.js"));
const marked = _marked.marked ?? _marked.default ?? _marked;

// ── link rewriting ──────────────────────────────────────────────────────────
// Runs on the markdown before parsing. Docs cross-link with bare filenames
// (`agent-takeover.md`); those become clean /docs/ URLs. A link escaping the
// docs directory (`../packaging/INSTALL.md`) points at the GitHub blob — the
// target is contributor-facing and has no rendered home. Anything else
// relative fails the build: an unrewritten .md link would 404 on the site.

function rewriteLinks(md, fromFile) {
  return md.replace(/\]\(([^)#\s]+\.md)(#[^)\s]*)?\)/g, (_, target, anchor) => {
    if (target.startsWith("../")) {
      return `](${GITHUB_BLOB}/${target.slice(3)})${anchor ?? ""})`;
    }
    const page = PAGES.find((p) => p.file === target);
    if (!page) {
      console.error(`build-docs: ${fromFile} links to '${target}', which is not in PAGES —` +
        " either publish that page or link its GitHub blob explicitly");
      process.exit(2);
    }
    return `](/docs/${page.slug}/${anchor ?? ""})`;
  });
}

// ── markdown → fragments ────────────────────────────────────────────────────

function stripMd(s) {
  return s
    .replace(/<[^>]+>/g, " ")
    .replace(/!\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/\[([^\]]*)\]\([^)]*\)/g, "$1")
    .replace(/[*_`]/g, "")
    .replace(/\s+/g, " ")
    .trim();
}

// First `# heading` is the page title; the first paragraph after it becomes
// the page lede and — truncated — the meta description. No frontmatter
// anywhere in docs/*.md, so this stays the zero-ceremony contract: write a
// doc the usual way, the metadata follows. Returns the full paragraph too,
// because a description truncated at ~158 chars reads fine in a SERP snippet
// and wrong as visible copy under the title.
function extractMeta(md) {
  const titleMatch = md.match(/^# (.+)$/m);
  if (!titleMatch) {
    console.error("build-docs: page has no `# title` line");
    process.exit(2);
  }
  const title = titleMatch[1].trim();
  const rest = md.slice(md.indexOf(titleMatch[0]) + titleMatch[0].length);
  const blocks = rest.split(/\n\n+/);
  const first = blocks.findIndex((b) => b.trim());
  const para = first === -1 ? "" : stripMd(blocks[first]);
  let description = para;
  if (description.length > 158) {
    description = description.slice(0, 158);
    description = description.slice(0, description.lastIndexOf(" ")) + "…";
  }
  // The body starts after the consumed lede block, so the page does not open
  // by saying the same thing twice.
  const bodyMd = first === -1 ? rest : rest.slice(rest.indexOf(blocks[first]) + blocks[first].length);
  return { title, para, description, bodyMd };
}

function slugify(heading) {
  return stripMd(heading)
    .toLowerCase()
    .replace(/[^a-z0-9一-鿿]+/g, "-")
    .replace(/^-+|-+$/g, "");
}

// marked alone emits no heading ids (that lives in an extension); anchor
// links inside a page need them, so ids are stamped after the parse — with
// the same slug rule GitHub uses, close enough for hand-written anchors.
function addHeadingIds(html) {
  const seen = new Map();
  return html.replace(/<(h[23])>([\s\S]*?)<\/\1>/g, (_, tag, inner) => {
    let id = slugify(inner) || "section";
    const n = (seen.get(id) ?? 0) + 1;
    seen.set(id, n);
    if (n > 1) id = `${id}-${n}`;
    return `<${tag} id="${id}">${inner}</${tag}>`;
  });
}

function renderBody(md, fromFile) {
  return addHeadingIds(marked.parse(rewriteLinks(md, fromFile), { gfm: true }));
}

// ── template ────────────────────────────────────────────────────────────────
// Design tokens are copied from site/index.html so docs read as the same
// product; keep the two token blocks in sync when one changes.

const HEAD_FONT = `<link rel="preconnect" href="https://fonts.googleapis.com">
<link rel="preconnect" href="https://fonts.gstatic.com" crossorigin>
<link href="https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Noto+Sans+SC:wght@400;500;700&family=JetBrains+Mono:wght@400;500;600&display=swap" rel="stylesheet">`;

const CSS = `
:root {
  --bg: #000;
  --surface: oklch(0.145 0.008 260 / 0.55);
  --surface2: oklch(0.205 0.01 260 / 0.6);
  --line: oklch(0.27 0.012 260 / 0.7);
  --line-strong: oklch(0.32 0.014 260 / 0.8);
  --ink: oklch(0.93 0.005 260);
  --mut: oklch(0.58 0.01 260);
  --kiwi: oklch(0.80 0.19 132);
  --kiwi-soft: oklch(0.80 0.19 132 / 0.13);
  --font-sans: Inter, "Noto Sans SC", system-ui, sans-serif;
  --font-mono: "JetBrains Mono", ui-monospace, monospace;
}
* { box-sizing: border-box; margin: 0; padding: 0; }
body {
  background: var(--bg); color: var(--ink);
  font-family: var(--font-sans); font-size: 15px; line-height: 1.7;
  -webkit-font-smoothing: antialiased;
}
a { color: inherit; text-decoration: none; }
code, pre { font-family: var(--font-mono); }

.topbar { border-bottom: 1px solid var(--line); background: oklch(0 0 0 / 0.72); }
.topbar-in { max-width: 1100px; margin: 0 auto; padding: 0 28px; height: 56px; display: flex; align-items: center; gap: 20px; }
.brand { display: flex; align-items: center; gap: 9px; font-weight: 600; font-size: 15px; }
.brand img { width: 22px; height: 22px; border-radius: 6px; }
.top-links { margin-left: auto; display: flex; gap: 6px; }
.top-links a { padding: 6px 12px; border-radius: 8px; font-size: 13px; color: var(--mut); }
.top-links a:hover { color: var(--ink); background: var(--surface2); }

.doc { max-width: 1100px; margin: 0 auto; padding: 36px 28px 90px; display: grid; grid-template-columns: 216px 1fr; gap: 44px; }
.side { align-self: start; position: sticky; top: 24px; }
.side .crumb { font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--kiwi); margin-bottom: 14px; }
.side a { display: block; padding: 6px 12px; border-radius: 8px; font-size: 13.5px; color: var(--mut); line-height: 1.45; }
.side a:hover { color: var(--ink); background: var(--surface2); }
.side a.on { color: var(--kiwi); background: var(--kiwi-soft); font-weight: 500; }

.content { min-width: 0; }
.content h1 { font-size: 28px; font-weight: 700; letter-spacing: -0.015em; line-height: 1.25; }
.content .lede { margin-top: 10px; color: var(--mut); font-size: 14.5px; }
.content h2 { margin: 44px 0 14px; font-size: 20px; font-weight: 650; letter-spacing: -0.01em; padding-top: 22px; border-top: 1px solid var(--line); }
.content h3 { margin: 28px 0 10px; font-size: 16px; font-weight: 600; }
.content p { margin: 12px 0; color: oklch(0.78 0.01 260); }
.content ul, .content ol { margin: 12px 0; padding-left: 24px; color: oklch(0.78 0.01 260); }
.content li { margin: 5px 0; }
.content a { color: var(--kiwi); }
.content a:hover { text-decoration: underline; }
.content code { font-size: 12.5px; color: oklch(0.72 0.02 260); background: var(--surface2); padding: 1px 6px; border-radius: 5px; }
.content pre { margin: 16px 0; padding: 14px 16px; border: 1px solid var(--line); border-radius: 10px; background: oklch(0.09 0.006 260 / 0.85); overflow-x: auto; font-size: 12.5px; line-height: 1.6; }
.content pre code { background: none; padding: 0; color: oklch(0.85 0.01 260); }
.content table { margin: 16px 0; border-collapse: collapse; width: 100%; font-size: 13.5px; }
.content th, .content td { border: 1px solid var(--line); padding: 8px 12px; text-align: left; vertical-align: top; }
.content th { background: var(--surface2); font-weight: 600; }
.content blockquote { margin: 16px 0; padding: 10px 18px; border-left: 3px solid oklch(0.55 0.14 132 / 0.55); background: var(--kiwi-soft); border-radius: 0 10px 10px 0; color: var(--mut); }
.content blockquote p { margin: 4px 0; }
.content hr { border: 0; border-top: 1px solid var(--line); margin: 32px 0; }

.pn-nav { display: grid; grid-template-columns: 1fr 1fr; gap: 12px; margin-top: 56px; }
.pn-nav a { border: 1px solid var(--line); border-radius: 12px; background: var(--surface); padding: 14px 16px; transition: border-color .15s; }
.pn-nav a:hover { border-color: oklch(0.40 0.03 132 / 0.65); }
.pn-nav .dir { font-family: var(--font-mono); font-size: 10.5px; color: var(--mut); letter-spacing: 0.08em; text-transform: uppercase; }
.pn-nav .t { margin-top: 4px; font-size: 13.5px; font-weight: 500; }
.pn-nav a.next { text-align: right; }

.foot { border-top: 1px solid var(--line); }
.foot-in { max-width: 1100px; margin: 0 auto; padding: 20px 28px; display: flex; gap: 18px; font-size: 12px; color: oklch(0.48 0.008 260); flex-wrap: wrap; }
.foot-in a:hover { color: var(--ink); }

@media (max-width: 860px) {
  .doc { grid-template-columns: 1fr; gap: 24px; padding-top: 24px; }
  .side { position: static; }
  .side nav { display: flex; overflow-x: auto; gap: 6px; scrollbar-width: none; padding-bottom: 6px; }
  .side nav::-webkit-scrollbar { display: none; }
  .side a { flex: none; border: 1px solid var(--line); }
  .pn-nav { grid-template-columns: 1fr; }
}
`;

function head(title, description, path, jsonLd) {
  return `<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>${esc(title)}</title>
<meta name="description" content="${esc(description)}">
<link rel="canonical" href="${SITE}${path}/">
<meta name="theme-color" content="#000000">
<meta property="og:type" content="article">
<meta property="og:url" content="${SITE}${path}/">
<meta property="og:title" content="${esc(title)}">
<meta property="og:description" content="${esc(description)}">
<meta property="og:image" content="${SITE}/assets/app-providers.png">
<meta property="og:site_name" content="Kiwano">
<meta property="og:locale" content="en_US">
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" type="image/svg+xml" href="/assets/kiwano-logo.svg">
${HEAD_FONT}
<style>${CSS}</style>
<script type="application/ld+json">
${JSON.stringify(jsonLd, null, 2)}
</script>`;
}

function chrome(bodyInner, activeSlug) {
  const items = PAGES.map(
    (p) => `<a href="/docs/${p.slug}/"${p.slug === activeSlug ? ' class="on"' : ""}>${esc(p.navTitle)}</a>`,
  ).join("\n");
  return `<div class="topbar"><div class="topbar-in">
  <a class="brand" href="/"><img src="/assets/kiwano-logo.svg" alt="">Kiwano</a>
  <div class="top-links">
    <a href="/">kiwano.cc</a>
    <a href="https://github.com/lightconsen/kiwano" target="_blank" rel="noopener">GitHub</a>
  </div>
</div></div>
<div class="doc">
  <aside class="side">
    <div class="crumb">Docs</div>
    <nav>${items}</nav>
  </aside>
  <main class="content">
${bodyInner}
  </main>
</div>
<div class="foot"><div class="foot-in">
  <span>© 2026 Kiwano · GPL-3.0-or-later</span>
  <a href="https://github.com/lightconsen/kiwano/tree/main/docs" target="_blank" rel="noopener">These docs on GitHub</a>
</div></div>`;
}

function esc(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

function breadcrumb(title, slug) {
  return {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: "Kiwano", item: `${SITE}/` },
      { "@type": "ListItem", position: 2, name: "Docs", item: `${SITE}/docs/` },
      { "@type": "ListItem", position: 3, name: title, item: `${SITE}/docs/${slug}/` },
    ],
  };
}

// ── page build ──────────────────────────────────────────────────────────────
// Two passes: metadata first (the sidebar and prev/next of every page name
// every other page's title), then rendering.

for (const p of PAGES) {
  const md = readFileSync(join(DOCS, p.file), "utf8");
  const { title, para, description, bodyMd } = extractMeta(md);
  p.title = title;
  // Sidebar, prev/next and the hub show plain text — an inline-code title
  // (`kiwano`) reads with stray backticks once escaped.
  p.navTitle = stripMd(title);
  p.description = description;
  p.lede = para;
  p.md = bodyMd;
}

for (const p of PAGES) {
  // Drop the source H1 — the template carries the title (with breadcrumb and
  // description above the body), so the page does not say its name twice.
  const body = renderBody(p.md.replace(/^# .+$/m, ""), p.file);

  const idx = PAGES.indexOf(p);
  const prev = PAGES[idx - 1];
  const next = PAGES[idx + 1];
  const pn = [
    prev ? `<a class="prev" href="/docs/${prev.slug}/"><div class="dir">← Previous</div><div class="t">${esc(prev.navTitle)}</div></a>` : "<span></span>",
    next ? `<a class="next" href="/docs/${next.slug}/"><div class="dir">Next →</div><div class="t">${esc(next.navTitle)}</div></a>` : "",
  ].join("\n");

  const html = `<!DOCTYPE html>
<html lang="en">
<head>
${head(`${p.title} — Kiwano Docs`, p.description, `/docs/${p.slug}`, breadcrumb(p.title, p.slug))}
</head>
<body>
${chrome(`<h1>${marked.parse(p.title).replace(/<p>|<\/p>\n?$/g, "")}</h1>
<p class="lede">${esc(p.lede)}</p>
${body}
<div class="pn-nav">
${pn}
</div>`, p.slug)}
</body>
</html>
`;
  const dir = join(SITE_DOCS, p.slug);
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "index.html"), html);
  console.log(`build-docs: /docs/${p.slug}/ — ${p.title}`);
}

// ── docs index (the /docs/ hub) ─────────────────────────────────────────────

const hubCards = PAGES.map(
  (p) => `  <a class="card" href="/docs/${p.slug}/"><h2>${esc(p.navTitle)}</h2><p>${esc(p.description)}</p></a>`,
).join("\n");

const hubTitle = "Kiwano Docs";
const hubDescription = "Setup, agent takeover, routing strategies and the full command reference for the Kiwano local gateway.";
const hubCss = `${CSS}
.wrap { max-width: 760px; margin: 0 auto; padding: 48px 28px 90px; }
.wrap .crumb { font-family: var(--font-mono); font-size: 11px; letter-spacing: 0.14em; text-transform: uppercase; color: var(--kiwi); }
.wrap h1 { margin-top: 12px; font-size: 32px; font-weight: 700; letter-spacing: -0.015em; }
.wrap > p { margin-top: 10px; color: var(--mut); font-size: 15px; }
.grid { margin-top: 34px; display: grid; gap: 12px; }
.card { border: 1px solid var(--line); border-radius: 12px; background: var(--surface); padding: 18px 20px; transition: border-color .15s; }
.card:hover { border-color: oklch(0.40 0.03 132 / 0.65); }
.card h2 { font-size: 16px; font-weight: 600; }
.card p { margin-top: 6px; font-size: 13.5px; color: var(--mut); }
.gh-note { margin-top: 26px; font-size: 12.5px; color: oklch(0.48 0.008 260); }
.gh-note a { color: var(--kiwi); }`;

const hubHtml = `<!DOCTYPE html>
<html lang="en">
<head>
${head(hubTitle, hubDescription, "/docs", {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  itemListElement: [
    { "@type": "ListItem", position: 1, name: "Kiwano", item: `${SITE}/` },
    { "@type": "ListItem", position: 2, name: "Docs", item: `${SITE}/docs/` },
  ],
})}
<style>${hubCss}</style>
</head>
<body>
<div class="topbar"><div class="topbar-in">
  <a class="brand" href="/"><img src="/assets/kiwano-logo.svg" alt="">Kiwano</a>
  <div class="top-links">
    <a href="/">kiwano.cc</a>
    <a href="https://github.com/lightconsen/kiwano" target="_blank" rel="noopener">GitHub</a>
  </div>
</div></div>
<div class="wrap">
  <div class="crumb">Docs</div>
  <h1>${hubTitle}</h1>
  <p>${hubDescription}</p>
  <div class="grid">
${hubCards}
  </div>
  <p class="gh-note">Design and planning notes (contributor-facing) live in
  <a href="${GITHUB_BLOB}/docs" target="_blank" rel="noopener">docs/ on GitHub</a>.</p>
</div>
<div class="foot"><div class="foot-in">
  <span>© 2026 Kiwano · GPL-3.0-or-later</span>
  <a href="${GITHUB_BLOB}/docs" target="_blank" rel="noopener">These docs on GitHub</a>
</div></div>
</body>
</html>
`;
mkdirSync(SITE_DOCS, { recursive: true });
writeFileSync(join(SITE_DOCS, "index.html"), hubHtml);
console.log("build-docs: /docs/ — hub");

// Generated directories no longer in the manifest would still deploy (the
// whole site/ tree uploads), so remove them rather than warn.
for (const entry of readdirSync(SITE_DOCS, { withFileTypes: true })) {
  if (entry.isDirectory() && !PAGES.some((p) => p.slug === entry.name)) {
    rmSync(join(SITE_DOCS, entry.name), { recursive: true });
    console.log(`build-docs: removed stale site/docs/${entry.name}/`);
  }
}

// ── sitemap ─────────────────────────────────────────────────────────────────

function lastCommitDate(file) {
  try {
    return execSync(`git log -1 --format=%cs -- ${file}`, { cwd: ROOT }).toString().trim();
  } catch {
    return "";
  }
}

const urls = [
  `  <url>
    <loc>${SITE}/</loc>
  </url>`,
  `  <url>
    <loc>${SITE}/docs/</loc>
    <lastmod>${lastCommitDate("docs")}</lastmod>
  </url>`,
  ...PAGES.map(
    (p) => `  <url>
    <loc>${SITE}/docs/${p.slug}/</loc>
    <lastmod>${lastCommitDate(join("docs", p.file))}</lastmod>
  </url>`,
  ),
];

const sitemap = `<?xml version="1.0" encoding="UTF-8"?>
<!-- Generated by scripts/build-docs.mjs — do not edit by hand; the page list
     is the PAGES manifest there. The landing page carries no lastmod (its
     only changing fact is stamped by sync-site.mjs; a hand-written date
     would go stale). Docs pages carry the last commit date of their source
     markdown, read from git. -->
<urlset xmlns="http://www.sitemaps.org/schemas/sitemap/0.9">
${urls.join("\n")}
</urlset>
`;
writeFileSync(join(ROOT, "site", "sitemap.xml"), sitemap);
console.log(`build-docs: sitemap.xml — ${PAGES.length + 2} urls`);
