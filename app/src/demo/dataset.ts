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

import type { AgentId } from "../api/types";
import { AGENTS } from "../api/types";
import { providers } from "../api/dev/providers";
import { NOT_INSTALLED_AGENTS } from "../api/dev/agent_dirs";

/** How many provider rows the demo keeps — the four the fixture opens with
    (a metered plan, a request-count plan, a disabled row, a local one), which
    is the spread the Apps screen is worth showing. */
const PROVIDERS = 4;

/** The agents a visitor sees taken over, in the order the strip shows them. */
const AGENTS_ON_SHOW: AgentId[] = ["claude", "codex", "gemini", "crush"];

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
}
