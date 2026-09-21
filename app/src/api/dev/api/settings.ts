// Settings, and everything that lives in them: the custom agents, the
// takeovers, the cc-switch import and the Hub sync.

import type { AgentId, AppSettings, CustomAgent, HubSyncReport, ImportReport, KiwanoApi, Protocol } from "../../types";
import { delay } from "../delay";
import { agentRoutes } from "../routes";
import { settings } from "../settings";

/** Dev-only: flips after the first syncHub() so the conditional path is visible. */
let devHubSynced = false;

export const settingsApi: Pick<
  KiwanoApi,
  | "getSettings"
  | "updateSettings"
  | "updateCustomAgent"
  | "addCustomAgent"
  | "removeCustomAgent"
  | "setTakeover"
  | "importCcSwitch"
  | "syncHub"
> = {
  async getSettings(): Promise<AppSettings> {
    await delay();
    // Fresh copy every call: the caller mutates nothing, but React needs a new
    // reference to re-render after setTakeover mutated the fixture in place.
    return structuredClone(settings);
  },

  async updateSettings(patch: Partial<AppSettings>): Promise<AppSettings> {
    await delay();
    Object.assign(settings, patch);
    return settings;
  },

  async updateCustomAgent(
    id: string,
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const list = settings.custom_agents;
    const index = list.findIndex((a) => a.id === id);
    if (index < 0) throw new Error(`no such custom agent: ${id}`);
    // The label, the note and the protocol move; the id is what routes and keys
    // point at. All three are replaced rather than patched — `null` here means
    // "not said", which is a value the field can hold — so a caller sends the
    // ones it is not changing back unchanged, the same line the backend draws.
    const updated: CustomAgent = {
      ...list[index],
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
    };
    settings.custom_agents = [
      ...list.slice(0, index),
      updated,
      ...list.slice(index + 1),
    ];
    return structuredClone(updated);
  },

  async addCustomAgent(
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const created: CustomAgent = {
      // The same shape vm::add_custom_agent derives, so a copied id reads the
      // same in either mode.
      id: `${label.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "custom"}-${Math.random().toString(16).slice(2, 8)}`,
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
      placeholder_key: "",
    };
    created.placeholder_key = `kw-ag-${created.id}-${Math.random().toString(16).slice(2, 6)}`;
    settings.custom_agents = [...settings.custom_agents, created];
    // A route with the same default as the backend's: single, no candidates yet.
    agentRoutes.push({
      agent: created.id,
      strategy: "single",
      config: null,
      bindings: [],
      limits: [],
    });
    return structuredClone(created);
  },

  async removeCustomAgent(id: string): Promise<void> {
    await delay();
    settings.custom_agents = settings.custom_agents.filter((a) => a.id !== id);
    // The route and the key go with it; usage rows are history and stay (the
    // mock has none for these, but the dashboard fixtures are keyed by agent
    // name, so a removed agent's traffic would survive here too).
    const i = agentRoutes.findIndex((r) => r.agent === id);
    if (i >= 0) agentRoutes.splice(i, 1);
  },

  async setTakeover(agent: AgentId, enabled: boolean): Promise<void> {
    await delay();
    const t = settings.takeovers.find((x) => x.agent === agent);
    if (!t) return;
    if (enabled) {
      const suffix = Math.random().toString(16).slice(2, 6);
      t.placeholder_key = `kw-ag-${agent}-${suffix}`;
      t.enabled = true;
    } else {
      // The route goes with the takeover (vm::set_agent_takeover): the agent
      // has its own config back, so it is nobody's candidate — and the
      // providers it named stay in the list, unbound.
      const i = agentRoutes.findIndex((r) => r.agent === agent);
      if (i >= 0) agentRoutes.splice(i, 1);
      t.placeholder_key = null;
      t.enabled = false;
    }
  },

  async importCcSwitch(): Promise<ImportReport> {
    await delay(600);
    return {
      imported: 2,
      skipped: 1,
      detail: [
        "source: ~/.cc-switch/config.json (3 rows)",
        "import ccs-claude-default: DeepSeek (anthropic)",
        "import ccs-codex-default: Groq (openai)",
        "skip claude/claude-official: missing endpoint or key",
      ],
    };
  },

  async syncHub(): Promise<HubSyncReport> {
    await delay(500);
    // Mirror the real conditional sync: the first call downloads, later ones
    // report "unchanged" (manifest sha matched) so both UI branches are
    // reachable in dev.
    const unchanged = devHubSynced;
    devHubSynced = true;
    return {
      fetched: 42,
      synced_at: new Date().toISOString(),
      hub_url: settings.hub_url,
      unchanged,
      pricing_version: 1,
      pricing_unchanged: unchanged,
    };
  },
};
