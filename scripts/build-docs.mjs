#!/usr/bin/env node
// Render the user-facing docs (docs/*.md) into site/docs/<slug>/index.html,
// with the zh-Hans sources alongside at site/docs/zh-Hans/<slug>/.
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
// A page with a zh-Hans source gets both renders and reciprocal hreflang
// links (plus x-default → English). A page without one — cli.md, for now —
// publishes English-only with no alternates, which is what Google wants to
// see rather than alternates pointing at the same URL. The zh sidebar walks
// the same page order and falls back to the English page where no zh
// source exists, so a zh reader never dead-ends.
//
// The sitemap is regenerated here too: docs pages carry the last commit
// date of their source markdown (read from git, so it is accurate by
// construction), while the landing page still carries no lastmod — its only
// changing fact is stamped by sync-site.mjs, and a hand-maintained date
// would go stale the way the hero version line did.
//
// Usage: node scripts/build-docs.mjs

import { readFileSync, writeFileSync, mkdirSync, readdirSync, rmSync, existsSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { execSync } from "node:child_process";

const ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const DOCS = join(ROOT, "docs");
const SITE_DOCS = join(ROOT, "site", "docs");
const GITHUB_BLOB = "https://github.com/lightconsen/kiwano/blob/main";
const SITE = "https://kiwano.cc";

// The languages the docs are published in, English first — it is the source,
// the fallback for a page nobody has translated, and the x-default target.
// Adding a language means adding it here and dropping `<page>.<locale>.md`
// files beside the English ones: a page translated into one language renders
// in that language alone, and the others fall back to English for it.
const LOCALES = ["en", "zh-Hans", "zh-Hant", "ja"];

/** Each language's name, in its own language — the hub and the language
    switch show these, so a reader looking for 日本語 recognises it whatever
    the page is currently in. */
const LOCALE_LABEL = { en: "English", "zh-Hans": "简体中文", "zh-Hant": "繁體中文", ja: "日本語" };

/** `<html lang>` gets the locale id; OG wants a region, so Traditional is
    `zh_TW` and Japanese `ja_JP`. */
const LOCALE_TAG = { en: "en", "zh-Hans": "zh-Hans", "zh-Hant": "zh-Hant", ja: "ja" };
const LOCALE_OG = { en: "en_US", "zh-Hans": "zh_CN", "zh-Hant": "zh_TW", ja: "ja_JP" };

/** `/docs/` for English, `/docs/<locale>/` otherwise. No trailing slash: the
    caller adds one when it means a directory, and the paths are also used as
    file paths and canonical URLs, where a doubled slash shows up in the page. */
const docsPath = (locale, slug = "") =>
  `/docs${locale === "en" ? "" : `/${locale}`}${slug ? `/${slug}` : ""}`;

// The published set, in sidebar and prev/next order. slugs equal the source
// file's basename so relative links between docs (`[x](strategies.md)`)
// rewrite mechanically. `translated` names the localized sources: a language
// absent from it simply has no page here, and its readers get the English one.
const PAGES = [
  {
    slug: "getting-started",
    file: "getting-started.md",
    translated: {
      "zh-Hans": "getting-started.zh-Hans.md",
      "zh-Hant": "getting-started.zh-Hant.md",
      ja: "getting-started.ja.md",
    },
  },
  {
    slug: "agent-takeover",
    file: "agent-takeover.md",
    translated: {
      "zh-Hans": "agent-takeover.zh-Hans.md",
      "zh-Hant": "agent-takeover.zh-Hant.md",
      ja: "agent-takeover.ja.md",
    },
  },
  {
    slug: "strategies",
    file: "strategies.md",
    translated: {
      "zh-Hans": "strategies.zh-Hans.md",
      "zh-Hant": "strategies.zh-Hant.md",
      ja: "strategies.ja.md",
    },
  },
  {
    slug: "cli",
    file: "cli.md",
    // The command reference: the commands themselves stay English (the CLI
    // is English-only), but the prose around them translates like any other
    // page. A language whose file is not on disk simply does not appear here
    // — the loader checks, so a gap publishes as the English page.
    translated: {
      "zh-Hans": "cli.zh-Hans.md",
      "zh-Hant": "cli.zh-Hant.md",
      ja: "cli.ja.md",
    },
  },
];

const require_ = createRequire(import.meta.url);
const _marked = require_(join(ROOT, "scripts", "vendor", "marked.min.cjs"));
const marked = _marked.marked ?? _marked.default ?? _marked;

// ── link rewriting ──────────────────────────────────────────────────────────
// Runs on the markdown before parsing. Docs cross-link with bare filenames
// (`agent-takeover.md`); those become clean /docs/ URLs — a zh source links
// to its own language where it exists, to the English page where it does
// not. A link escaping the docs directory (`../packaging/INSTALL.md`)
// points at the GitHub blob — the target is contributor-facing and has no
// rendered home. Anything else relative fails the build: an unrewritten
// .md link would 404 on the site.

function rewriteLinks(md, fromFile, lang) {
  let out = md.replace(/\]\(([^)#\s]+\.md)(#[^)\s]*)?\)/g, (_, target, anchor) => {
    if (target.startsWith("../")) {
      return `](${GITHUB_BLOB}/${target.slice(3)})${anchor ?? ""})`;
    }
    // A translated source naming its sibling in the same language
    // (`agent-takeover.ja.md`) goes to that page outright; a bare English
    // filename resolves to the reader's language when the page exists in it,
    // and to English otherwise — the same fallback the sidebar uses, so a
    // link and the menu beside it never disagree.
    const named = PAGES.find((p) => p.translated[lang] === target);
    if (named) return `](${docsPath(lang, named.slug)}/${anchor ?? ""})`;
    const other = LOCALES.filter((l) => l !== "en").find((l) =>
      PAGES.some((p) => p.translated[l] === target));
    if (other) {
      const page = PAGES.find((p) => p.translated[other] === target);
      return `](${docsPath(other, page.slug)}/${anchor ?? ""})`;
    }
    const page = PAGES.find((p) => p.file === target);
    if (!page) {
      console.error(`build-docs: ${fromFile} links to '${target}', which is not in PAGES —` +
        " either publish that page or link its GitHub blob explicitly");
      process.exit(2);
    }
    return `](${docsPath(page.docs[lang] ? lang : "en", page.slug)}/${anchor ?? ""})`;
  });
  // Relative assets (the screenshots under docs/screenshots/) serve from the
  // rendered tree, which build-docs copies below.
  out = out.replace(/\]\(([^)#\s]+\.(?:png|jpe?g|webp|gif|svg|mp4))\)/g, (_, asset) =>
    asset.startsWith("/") ? `](${asset})` : `](/docs/${asset})`);
  // Fail-closed: any remaining relative target would 404 in production.
  for (const m of out.matchAll(/\]\(([^)\s]+)\)/g)) {
    if (!/^(https?:|\/|#|mailto:)/.test(m[1])) {
      console.error(`build-docs: ${fromFile} has unresolvable relative link '${m[1]}'`);
      process.exit(2);
    }
  }
  return out;
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
// and wrong as visible copy under the title. CJK packs ~3× the characters
// per visual width; the cap holds anyway — a 158-char zh snippet is long,
// not short.
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

function renderBody(md, fromFile, lang) {
  const html = addHeadingIds(marked.parse(rewriteLinks(md, fromFile, lang), { gfm: true }));
  // Docs images all sit below the fold; load them lazily.
  return html.replace(/<img /g, '<img loading="lazy" decoding="async" ');
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
.content img {
  display: block; max-width: 100%; height: auto;
  border: 1px solid var(--line-strong); border-radius: 10px;
  margin: 22px auto 10px; background: oklch(0.09 0.006 260);
}
.content p:has(> img) { margin: 0; }
.content p:has(> img) + p:has(> em) {
  margin: 0 0 22px; font-size: 12.5px; color: var(--mut); text-align: center;
}

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

function esc(s) {
  return s.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;").replace(/"/g, "&quot;");
}

// Reciprocal hreflang across every language a path is published in;
// x-default points at the English URL. Hubs and doc pages call it alike.
// One language alone is not an alternate set — it is just the page.
function hreflangBlock(paths) {
  const langs = LOCALES.filter((l) => paths[l]);
  if (langs.length < 2) return "";
  const lines = langs.map((l) => `<link rel="alternate" hreflang="${LOCALE_TAG[l]}" href="${SITE}${paths[l]}/">`);
  lines.push(`<link rel="alternate" hreflang="x-default" href="${SITE}${paths.en}/">`);
  return lines.join("\n");
}

function head(doc, alternates, jsonLd) {
  const locale = LOCALE_OG[doc.lang];
  const alternate = LOCALES.filter((l) => l !== doc.lang && doc.published.includes(l))
    .map((l) => `\n<meta property="og:locale:alternate" content="${LOCALE_OG[l]}">`)
    .join("");
  return `<meta charset="UTF-8">
<meta name="viewport" content="width=device-width, initial-scale=1.0">
<title>${esc(doc.title)} — Kiwano Docs</title>
<meta name="description" content="${esc(doc.description)}">
<link rel="canonical" href="${SITE}${doc.path}/">
${alternates}
<meta name="theme-color" content="#000000">
<meta property="og:type" content="article">
<meta property="og:url" content="${SITE}${doc.path}/">
<meta property="og:title" content="${esc(doc.title)}">
<meta property="og:description" content="${esc(doc.description)}">
<meta property="og:image" content="${SITE}/assets/app-providers.png">
<meta property="og:site_name" content="Kiwano">
<meta property="og:locale" content="${locale}">${alternate}
<meta name="twitter:card" content="summary_large_image">
<link rel="icon" type="image/svg+xml" href="/assets/kiwano-logo.svg">
${HEAD_FONT}
<style>${CSS}</style>
<script type="application/ld+json">
${JSON.stringify(jsonLd, null, 2)}
</script>`;
}

// The sidebar walks the page order in the reader's language: that language's
// render of each page where one exists, the English page otherwise — so a
// reader sees translated titles for what is translated and is never
// dead-ended by a page that is not.
function sidebarSequence(lang) {
  return PAGES.map((p) => p.docs[lang] ?? p.docs.en);
}

function chrome(bodyInner, doc) {
  const items = sidebarSequence(doc.lang)
    .map((d) => `<a href="${d.path}/"${d.path === doc.path ? ' class="on"' : ""}>${esc(d.navTitle)}</a>`)
    .join("\n");
  // The crumb doubles as the way back to this language's hub.
  const hub = `${docsPath(doc.lang)}/`;
  return `<div class="topbar"><div class="topbar-in">
  <a class="brand" href="/"><img src="/assets/kiwano-logo.svg" alt="">Kiwano</a>
  <div class="top-links">
    <a href="/">kiwano.cc</a>
    <a href="https://github.com/lightconsen/kiwano" target="_blank" rel="noopener">GitHub</a>
    <a href="https://x.com/kiwano_cc" target="_blank" rel="noopener">X</a>
  </div>
</div></div>
<div class="doc">
  <aside class="side">
    <a class="crumb" href="${hub}">Docs</a>
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

function breadcrumb(doc) {
  return {
    "@context": "https://schema.org",
    "@type": "BreadcrumbList",
    itemListElement: [
      { "@type": "ListItem", position: 1, name: "Kiwano", item: `${SITE}/` },
      { "@type": "ListItem", position: 2, name: "Docs", item: `${SITE}/docs/` },
      { "@type": "ListItem", position: 3, name: doc.title, item: `${SITE}${doc.path}/` },
    ],
  };
}

// ── page build ──────────────────────────────────────────────────────────────
// Two passes: metadata first (the sidebar and prev/next of every page name
// every other page's title), then rendering.

function loadDoc(file, path, lang, published) {
  const md = readFileSync(join(DOCS, file), "utf8");
  const { title, para, description, bodyMd } = extractMeta(md);
  return {
    file, path, lang, published,
    title,
    // Sidebar, prev/next and the hub show plain text — an inline-code title
    // (`kiwano`) reads with stray backticks once escaped.
    navTitle: stripMd(title),
    description,
    lede: para,
    body: bodyMd,
  };
}

for (const page of PAGES) {
  page.docs = {};
  // Which languages this page is actually published in: English always, plus
  // every translation whose file is on disk. Read from the filesystem rather
  // than assumed from the manifest, so a source that has not landed yet (or
  // one that was deleted) cannot produce a page with no content behind it.
  const available = ["en", ...LOCALES.filter((l) => l !== "en" && page.translated[l] && existsSync(join(DOCS, page.translated[l])))];
  for (const locale of available) {
    page.docs[locale] = loadDoc(
      locale === "en" ? page.file : page.translated[locale],
      docsPath(locale, page.slug),
      locale,
      available,
    );
  }
}

for (const page of PAGES) {
  for (const doc of Object.values(page.docs)) {
    if (!doc) continue;

    // Prev/next follow the sidebar sequence (the reader's language), so a
    // zh page chains through the zh renders, with the English cli page as a
    // first-class stop rather than a gap.
    const seq = sidebarSequence(doc.lang);
    const idx = seq.findIndex((d) => d.path === doc.path);
    const prev = seq[idx - 1];
    const next = seq[idx + 1];
    const pn = [
      prev ? `<a class="prev" href="${prev.path}/"><div class="dir">← Previous</div><div class="t">${esc(prev.navTitle)}</div></a>` : "<span></span>",
      next ? `<a class="next" href="${next.path}/"><div class="dir">Next →</div><div class="t">${esc(next.navTitle)}</div></a>` : "",
    ].join("\n");

    const html = `<!DOCTYPE html>
<html lang="${doc.lang}">
<head>
${head(doc, hreflangBlock(Object.fromEntries(Object.entries(page.docs).map(([l, d]) => [l, d.path]))), breadcrumb(doc))}
</head>
<body>
${chrome(`<h1>${marked.parse(doc.title).replace(/<p>|<\/p>\n?$/g, "")}</h1>
<p class="lede">${esc(doc.lede)}</p>
${renderBody(doc.body, doc.file, doc.lang)}
<div class="pn-nav">
${pn}
</div>`, doc)}
</body>
</html>
`;
    const dir = join(SITE_DOCS, doc.path.replace("/docs/", ""));
    mkdirSync(dir, { recursive: true });
    writeFileSync(join(dir, "index.html"), html);
    console.log(`build-docs: ${doc.path}/ — ${doc.title}`);
  }
}

// ── docs hubs (/docs/ and /docs/zh-Hans/) ─────────────────────────────────────
// One hub per language. The zh hub lists the zh renders, with the English
// cli page as a card in place (same fallback rule the sidebar uses), so
// both hubs catalog the same pages and the landing page's Docs link has a
// same-language destination in either state.

const HUB_CSS = `${CSS}
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

/** Hub copy per language. The title and description are the language's own —
    a hub is a page a reader is meant to read, not a list of links. */
const HUB_COPY = {
  en: {
    title: "Kiwano Docs",
    description:
      "Setup, agent takeover, routing strategies and the full command reference for the Kiwano local gateway.",
    contributor:
      'Design and planning notes (contributor-facing) live in <a href="' + GITHUB_BLOB + '/docs" target="_blank" rel="noopener">docs/ on GitHub</a>.',
  },
  "zh-Hans": {
    title: "Kiwano 文档",
    description: "安装、Agent 接管、路由策略与完整命令参考——Kiwano 本地网关的中文文档。",
    contributor:
      '面向贡献者的设计与规划笔记见 <a href="' + GITHUB_BLOB + '/docs" target="_blank" rel="noopener">GitHub 的 docs/ 目录</a>。',
  },
  "zh-Hant": {
    title: "Kiwano 文件",
    description: "安裝、Agent 接管、路由策略與完整指令參考——Kiwano 本機閘道的繁體中文文件。",
    contributor:
      '面向貢獻者的設計與規劃筆記見 <a href="' + GITHUB_BLOB + '/docs" target="_blank" rel="noopener">GitHub 的 docs/ 目錄</a>。',
  },
  ja: {
    title: "Kiwano ドキュメント",
    description:
      "インストール、Agent の引き継ぎ、ルーティングストラテジー、コマンドリファレンス——Kiwano ローカルゲートウェイの日本語ドキュメント。",
    contributor:
      '設計・計画のメモ（コントリビューター向け）は <a href="' + GITHUB_BLOB + '/docs" target="_blank" rel="noopener">GitHub の docs/</a> にあります。',
  },
};

// One hub per language that has at least one translated page, plus English.
// A hub lists that language's renders, falling back to the English page where
// a page is not translated — the same rule the sidebar follows, so the two
// never disagree about what exists.
const HUBS = ["en", ...LOCALES.filter((l) => l !== "en" && PAGES.some((p) => p.docs[l]))].map((locale) => {
  const copy = HUB_COPY[locale];
  const others = LOCALES.filter((l) => l !== locale && PAGES.some((p) => p.docs[l]));
  return {
    lang: locale,
    path: docsPath(locale),
    title: copy.title,
    description: copy.description,
    locale,
    cards: PAGES.map((p) => {
      const d = p.docs[locale] ?? p.docs.en;
      return `  <a class="card" href="${d.path}/"><h2>${esc(d.navTitle)}</h2><p>${esc(d.description)}</p></a>`;
    }),
    notes: [
      // Discovery for a reader who landed on the wrong language's hub: the
      // other languages, by their own names, linking to their hubs.
      others.length
        ? `  <p class="gh-note">${others.map((l) => `<a href="${docsPath(l)}/">${LOCALE_LABEL[l]}</a>`).join(" · ")}</p>`
        : "",
      `  <p class="gh-note">${copy.contributor}</p>`,
    ].filter(Boolean),
  };
});

for (const hub of HUBS) {
  const html = `<!DOCTYPE html>
<html lang="${hub.lang}">
<head>
${head(
  { title: hub.title, description: hub.description, path: hub.path, lang: hub.lang,
    published: LOCALES.filter((l) => HUBS.some((h) => h.locale === l)) },
  hreflangBlock(Object.fromEntries(HUBS.map((h) => [h.locale, h.path]))),
  {
  "@context": "https://schema.org",
  "@type": "BreadcrumbList",
  itemListElement: [
    { "@type": "ListItem", position: 1, name: "Kiwano", item: `${SITE}/` },
    { "@type": "ListItem", position: 2, name: "Docs", item: `${SITE}/docs/` },
  ],
})}
<style>${HUB_CSS}</style>
</head>
<body>
<div class="topbar"><div class="topbar-in">
  <a class="brand" href="/"><img src="/assets/kiwano-logo.svg" alt="">Kiwano</a>
  <div class="top-links">
    <a href="/">kiwano.cc</a>
    <a href="https://github.com/lightconsen/kiwano" target="_blank" rel="noopener">GitHub</a>
    <a href="https://x.com/kiwano_cc" target="_blank" rel="noopener">X</a>
  </div>
</div></div>
<div class="wrap">
  <div class="crumb">Docs</div>
  <h1>${hub.title}</h1>
  <p>${hub.description}</p>
  <div class="grid">
${hub.cards.join("\n")}
  </div>
${hub.notes.join("\n")}
</div>
<div class="foot"><div class="foot-in">
  <span>© 2026 Kiwano · GPL-3.0-or-later</span>
  <a href="${GITHUB_BLOB}/docs" target="_blank" rel="noopener">These docs on GitHub</a>
</div></div>
</body>
</html>
`;
  const dir = join(SITE_DOCS, hub.path.replace("/docs", "").replace(/^\//, ""));
  mkdirSync(dir, { recursive: true });
  writeFileSync(join(dir, "index.html"), html);
  console.log(`build-docs: ${hub.path}/ — ${hub.title}`);
}

// Screenshots the docs reference copy into the rendered tree; only media
// travels (the source dir also carries working files), and stale outputs are
// removed so a deleted screenshot cannot linger on the site.
const SHOTS = join(DOCS, "screenshots");
const OUT_SHOTS = join(SITE_DOCS, "screenshots");
mkdirSync(OUT_SHOTS, { recursive: true });
const MEDIA = /\.(png|jpe?g|webp|gif|svg|mp4)$/;
for (const f of readdirSync(SHOTS)) {
  if (MEDIA.test(f)) writeFileSync(join(OUT_SHOTS, f), readFileSync(join(SHOTS, f)));
}
for (const f of readdirSync(OUT_SHOTS)) {
  if (!existsSync(join(SHOTS, f))) {
    rmSync(join(OUT_SHOTS, f));
    console.log(`build-docs: removed stale site/docs/screenshots/${f}`);
  }
}

// Generated directories no longer in the manifest would still deploy (the
// whole site/ tree uploads), so remove them rather than warn. Same sweep
// inside zh-Hans/, which mirrors the manifest's translated subset.
const topSlugs = [...PAGES.map((p) => p.slug), ...LOCALES.filter((l) => l !== "en"), "screenshots"];
for (const entry of readdirSync(SITE_DOCS, { withFileTypes: true })) {
  if (entry.isDirectory() && !topSlugs.includes(entry.name)) {
    rmSync(join(SITE_DOCS, entry.name), { recursive: true });
    console.log(`build-docs: removed stale site/docs/${entry.name}/`);
  }
}
for (const locale of LOCALES.filter((l) => l !== "en")) {
  const dir = join(SITE_DOCS, locale);
  if (!existsSync(dir)) continue;
  const live = PAGES.filter((p) => p.docs[locale]).map((p) => p.slug);
  for (const entry of readdirSync(dir, { withFileTypes: true })) {
    if (entry.isDirectory() && !live.includes(entry.name)) {
      rmSync(join(dir, entry.name), { recursive: true });
      console.log(`build-docs: removed stale site/docs/${locale}/${entry.name}/`);
    }
  }
}

// ── 旧中文路径的别名 ────────────────────────────────────────────────────────
// 简体中文的 id 从 `zh-CN` 改成 `zh-Hans` 时(2026-10-02),文档 URL 跟着改了。
// /docs/zh-CN/ 在改之前是线上且被收录过的,所以那条路径继续给一个跳转页——
// 页面级 meta refresh 加上 canonical,指向新地址,并被排除在 sitemap 之外
// (sitemap 由 PAGES 生成,这个目录不在其中,天然不列)。
//
// 写在清扫之后:`zh-CN` 不在 topSlugs 里,清扫会把它当残留删掉——每次构建
// 都由这里重建,所以顺序反了就等于没写。
const LEGACY_ZH_PATH = "zh-CN";
const legacyDir = join(SITE_DOCS, LEGACY_ZH_PATH);
rmSync(legacyDir, { recursive: true, force: true });
for (const page of PAGES.filter((p) => p.docs["zh-Hans"])) {
  const dir = join(legacyDir, page.slug);
  mkdirSync(dir, { recursive: true });
  writeFileSync(
    join(dir, "index.html"),
    `<!doctype html>
<html lang="zh-Hans">
  <head>
    <meta charset="utf-8">
    <title>跳转中 — Kiwano 文档</title>
    <link rel="canonical" href="${SITE}/docs/zh-Hans/${page.slug}/">
    <meta http-equiv="refresh" content="0; url=/docs/zh-Hans/${page.slug}/">
    <meta name="robots" content="noindex">
  </head>
  <body>
    <p>这个页面的地址已改为 <a href="/docs/zh-Hans/${page.slug}/">/docs/zh-Hans/${page.slug}/</a>。</p>
  </body>
</html>
`,
  );
}
writeFileSync(
  join(legacyDir, "index.html"),
  `<!doctype html>
<html lang="zh-Hans">
  <head>
    <meta charset="utf-8">
    <title>跳转中 — Kiwano 文档</title>
    <link rel="canonical" href="${SITE}/docs/zh-Hans/">
    <meta http-equiv="refresh" content="0; url=/docs/zh-Hans/">
    <meta name="robots" content="noindex">
  </head>
  <body>
    <p>文档的地址已改为 <a href="/docs/zh-Hans/">/docs/zh-Hans/</a>。</p>
  </body>
</html>
`,
);
console.log(`build-docs: site/docs/${LEGACY_ZH_PATH}/ — ${PAGES.filter((p) => p.docs["zh-Hans"]).length + 1} redirect page(s)`);

// ── sitemap ─────────────────────────────────────────────────────────────────

function lastCommitDate(file) {
  try {
    return execSync(`git log -1 --format=%cs -- ${file}`, { cwd: ROOT }).toString().trim();
  } catch {
    return "";
  }
}

const docUrls = PAGES.flatMap((p) =>
  Object.entries(p.docs).map(([locale, d]) => `  <url>
    <loc>${SITE}${d.path}/</loc>
    <lastmod>${lastCommitDate(join("docs", locale === "en" ? p.file : p.translated[locale]))}</lastmod>
  </url>`),
);

const urls = [
  `  <url>
    <loc>${SITE}/</loc>
  </url>`,
  `  <url>
    <loc>${SITE}/docs/</loc>
    <lastmod>${lastCommitDate("docs")}</lastmod>
  </url>`,
  `  <url>
    <loc>${SITE}/docs/zh-Hans/</loc>
    <lastmod>${lastCommitDate("docs")}</lastmod>
  </url>`,
  ...docUrls,
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
console.log(`build-docs: sitemap.xml — ${urls.length} urls`);
