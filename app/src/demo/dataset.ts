// The demo's dataset: a handful of rows, not the whole fixture.
//
// The dev fixtures exist to exercise every corner of every screen — a couple of
// dozen providers, a row per status, every agent installed. That is the right
// dataset for developing against and the wrong one for a page a visitor scrolls
// past: the embedded demo re-reads the screen each time a request lands, so the
// work per tick scales with the number of rows, and the landing page pays for
// it in CPU. What a visitor needs to see is that the product is alive and what
// it looks like; four providers and a few agents say that as well as forty.
//
// Trimming happens *before* the app mounts, here, rather than by branching in a
// screen: the screens keep reading exactly what they read in the app, and this
// module is the only thing that knows the demo is smaller than the fixture.

import type { AgentId, CatalogEntry } from "../api/types";
import { AGENTS } from "../api/types";
import { providers } from "../api/dev/providers";
import { catalog } from "../api/dev/catalog";
import { NOT_INSTALLED_AGENTS } from "../api/dev/agent_dirs";
import { settings } from "../api/dev/settings";

/** How many provider rows the demo keeps — the four the fixture opens with
    (a metered plan, a request-count plan, a disabled row, a local one), which
    is the spread the Apps screen is worth showing. */
const PROVIDERS = 4;

/** The agents a visitor sees taken over, in the order the strip shows them. */
const AGENTS_ON_SHOW: AgentId[] = ["claude", "codex", "gemini", "crush"];

/**
 * The Models shelf, trimmed to the providers the demo actually has: a catalogue
 * entry the demo never adds is a row about somebody else's product, and the
 * shelf reads better as the short list of what is on the machine.
 */
const CATALOG_ON_SHOW = ["deepseek", "kimi", "zhipu-glm", "ollama"] as const;

/**
 * The demo's own Hub, for the one asset the shelf asks it for.
 *
 * A catalogue row's logo is a *hub-relative* path — `Shelf/Row.tsx` resolves it
 * against the configured `hub_url` — so a row offline would fall back to a
 * letter in a coloured square. The demo ships the four marks it needs under
 * /demo/hub/logos/ and points hub_url at that folder, which keeps the resolution
 * path in the screens identical to the app's instead of special-casing them.
 */
const DEMO_HUB = "hub/catalog.json";

/**
 * Cut the fixture down to size. Exported as a function rather than done at
 * import time, so the order — trim, then mount — is visible where it matters.
 */
export function useDemoDataset(): void {
  // The matrix rows are pushed after the four base ones (see dev/providers.ts);
  // truncating in place keeps the same array the screens already imported.
  providers.splice(PROVIDERS);

  // The strip shows only what detection reports as installed, so "which agents
  // the demo runs" is decided here. The list is *set*, not extended: the fixture
  // already lists four agents as missing, and a demo that only added to it would
  // call Gemini CLI absent while claiming to show it.
  const shown = new Set<string>(AGENTS_ON_SHOW);
  const hidden = AGENTS.map((a) => a.id as AgentId).filter((id) => !shown.has(id));
  NOT_INSTALLED_AGENTS.splice(0, NOT_INSTALLED_AGENTS.length, ...hidden);

  // Point the shelf's logo resolution at the demo's own folder (see DEMO_HUB).
  settings.hub_url = new URL(`${import.meta.env.BASE_URL}${DEMO_HUB}`, window.location.origin).toString();

  const shownCatalogs = new Set<string>(CATALOG_ON_SHOW);
  const kept: CatalogEntry[] = [];
  for (const entry of catalog) {
    if (!shownCatalogs.has(entry.id)) continue;
    // `logos/<entry id>` — the filenames the demo's hub folder carries, with
    // GLM's mark filed under the name the catalogue gives that entry.
    entry.logo = `logos/${entry.id === "zhipu-glm" ? "zhipu" : entry.id}.${entry.id === "kimi" ? "webp" : "svg"}`;
    kept.push(entry);
  }
  catalog.splice(0, catalog.length, ...kept);
}
