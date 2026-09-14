// The browser-dev mock's *semantics*, not its numbers.
//
// `devApi` is typed as `KiwanoApi`, so its shapes and its vocabulary (which
// protocols exist, which agent ids) are already checked by `tsc` — the gemini
// removal proved that much: the fixtures stopped compiling the moment the union
// lost a member. What types cannot check is an invariant that spans two calls,
// which is exactly the one that bit us: the mock keeps its own route table next
// to its own takeover list, and it is easy to teach one about a change and not
// the other. `pnpm dev` is where a screen's behaviour is looked at by hand, so a
// mock that answers differently from the backend sends that check the wrong way.
import { beforeEach, describe, expect, it, vi } from "vitest";

import type { DashboardWindow, KiwanoApi } from "./types";

/** A pristine mock: `devApi` holds its fixtures in module state, so a test that
    mutates one would otherwise be read by the next test in this file. */
async function freshApi(): Promise<KiwanoApi> {
  vi.resetModules();
  const { devApi } = await import("./dev");
  return devApi;
}

let api: KiwanoApi;

beforeEach(async () => {
  api = await freshApi();
});

describe("the browser-dev mock", () => {
  it("names only agents it reports as taken over", async () => {
    // vm::build_provider_vms's rule, which the screens read as gospel: a binding
    // whose agent has its own config back is not a live route, so the provider
    // must not be listed under it.
    const settings = await api.getSettings();
    const dormant = new Set<string>(
      settings.takeovers.filter((t) => !t.enabled).map((t) => t.agent),
    );
    for (const provider of await api.listProviders()) {
      for (const agent of provider.agents) {
        expect(dormant.has(agent), `${provider.name} is listed under ${agent}`).toBe(false);
      }
      for (const agent of provider.serving_agents) {
        expect(dormant.has(agent), `${provider.name} serves ${agent}`).toBe(false);
      }
    }
  });

  it("drops an agent's route when its takeover is turned off, keeping the provider", async () => {
    const settings = await api.getSettings();
    const taken = settings.takeovers.find((t) => t.enabled);
    expect(taken, "the fixture needs one agent taken over").toBeDefined();
    const { agent } = taken!;
    const before = await api.getAgentRoutes();
    const route = before.find((r) => r.agent === agent);
    expect(route, `${agent} has a route in the fixture`).toBeDefined();
    const providerId = route!.bindings[0].provider_id;
    const providers = await api.listProviders();

    await api.setTakeover(agent, false);

    // vm::set_agent_takeover's two halves: the route goes, the provider row
    // stays (it is the user's, keys and history included).
    expect((await api.getAgentRoutes()).some((r) => r.agent === agent)).toBe(false);
    const after = await api.listProviders();
    expect(after.map((p) => p.id)).toEqual(providers.map((p) => p.id));
    const provider = after.find((p) => p.id === providerId);
    expect(provider?.agents ?? []).not.toContain(agent);
  });
});

/** Every window, because a fixture that is consistent for one and not the others
    is exactly the drift these assertions exist to catch. */
const WINDOWS: DashboardWindow[] = ["today", "7d", "30d"];

const sum = (ns: number[]) => ns.reduce((n, v) => n + v, 0);
const round2 = (n: number) => Math.round(n * 100) / 100;

describe("the dashboard fixtures", () => {
  it("are one table seen several ways", async () => {
    for (const window of WINDOWS) {
      const d = await api.getDashboard(window);
      const trend = sum(d.trend.map((t) => t.requests));

      // The backend's arithmetic, which the tiles only look right against: the
      // trend, the provider split and the agent split are the same `usage` rows
      // grouped three ways, and the headline is the request log — those rows
      // plus the ones that never reached a provider.
      expect(sum(d.by_provider.map((p) => p.requests)), `${window} by_provider`).toBe(trend);
      expect(sum(d.by_agent.map((a) => a.requests)), `${window} by_agent`).toBe(trend);
      expect(d.requests, `${window} headline`).toBeGreaterThanOrEqual(trend);

      for (const p of d.by_provider) {
        // The share is of the headline, not of the breakdown — so the slices do
        // not have to reach 100%, they just have to be that number.
        expect(p.pct, `${window}/${p.id} pct`).toBe(Math.round((p.requests * 100) / d.requests));
        expect(p.requests, `${window}/${p.id} requests`).toBeGreaterThan(0);
        expect(p.cost, `${window}/${p.id} premium`).toBeGreaterThanOrEqual(p.cost_off_peak);
      }

      // Money is the same money from both sides, and the premium is never
      // negative (the off-peak rate is a discount or nothing at all).
      expect(round2(sum(d.by_provider.map((p) => p.cost))), `${window} cost`).toBeCloseTo(d.cost, 2);
      expect(round2(sum(d.by_agent.map((a) => a.cost))), `${window} agent cost`).toBeCloseTo(d.cost, 2);
      expect(round2(sum(d.by_provider.map((p) => p.cost_off_peak))), `${window} off-peak`).toBeCloseTo(
        d.cost_off_peak,
        2,
      );
      expect(round2(sum(d.by_agent.map((a) => a.cost_off_peak))), `${window} agent off-peak`).toBeCloseTo(
        d.cost_off_peak,
        2,
      );
      expect(d.cost, `${window} premium`).toBeGreaterThanOrEqual(d.cost_off_peak);

      // The trend's tokens are what the headline splits into input and output.
      expect(d.input_tokens + d.output_tokens, `${window} tokens`).toBe(sum(d.trend.map((t) => t.tokens)));

      // The option lists are the rows that had traffic, in the same order the
      // breakdowns print them.
      expect(d.filter_providers.map((p) => p.id)).toEqual(d.by_provider.map((p) => p.id));
      expect(d.filter_agents.map((a) => a.id)).toEqual(d.by_agent.map((a) => a.agent));

      // Buckets are distinct: the axis prints one label per bucket, and two rows
      // sharing a date would collide as a React key and as a tick.
      const dates = d.trend.map((t) => t.date);
      expect(new Set(dates).size, `${window} trend dates`).toBe(dates.length);
    }
  });

  it("route a user-defined agent as long as it exists", async () => {
    // The mock's equivalent of `vm::live_bound_agents`' exception: a custom
    // agent has no config file, so the takeover list has nothing to say about
    // it — asking it whether the agent is routed would answer "no" and hide its
    // providers from the All tab.
    const settings = await api.getSettings();
    const custom = settings.custom_agents[0];
    expect(custom, "the fixture has a user-defined agent").toBeDefined();

    const route = (await api.getAgentRoutes()).find((r) => r.agent === custom.id);
    expect(route, "it has a route").toBeDefined();
    expect(route!.bindings.length).toBeGreaterThan(0);

    const provider = (await api.listProviders()).find((p) =>
      route!.bindings.some((b) => b.provider_id === p.id),
    );
    expect(provider?.agents, "its provider names it").toContain(custom.id);

    // Deleting it takes the route with it, and the provider stays.
    await api.removeCustomAgent(custom.id);
    expect((await api.getAgentRoutes()).some((r) => r.agent === custom.id)).toBe(false);
    expect((await api.listProviders()).some((p) => p.id === provider!.id)).toBe(true);
    expect((await api.getSettings()).custom_agents.some((a) => a.id === custom.id)).toBe(false);
  });

  it("promote the next candidate when a bound provider is deleted", async () => {
    // vm::delete_provider: a provider queued for an agent can always be
    // deleted, and the agent it led promotes its next candidate rather than
    // being left pointing at a row that is gone.
    const before = await api.getAgentRoutes();
    const route = before.find((r) => r.bindings.length > 1)!;
    const [head, next] = route.bindings;

    await api.deleteProvider(head.provider_id);

    const after = (await api.getAgentRoutes()).find((r) => r.agent === route.agent)!;
    expect(after.bindings.map((b) => b.provider_id)).not.toContain(head.provider_id);
    expect(after.bindings[0].provider_id).toBe(next.provider_id);
    expect(after.bindings.map((b) => b.priority)).toEqual(
      after.bindings.map((_, i) => i),
    );
  });

  it("narrow a window to one provider or one agent", async () => {
    // The screen sets one filter and every panel answers for it, so the mock
    // has to narrow by the id it was handed — the lookup it used to do by
    // display name resolved nothing at all once a vendor was spelled two ways.
    const one = await api.getDashboard("7d", "kimi");
    expect(one.by_provider.map((p) => p.id)).toEqual(["kimi"]);
    expect(one.by_provider[0].pct).toBe(100);
    expect(one.filter_providers.map((p) => p.id)).toEqual(["deepseek", "kimi", "glm", "ollama"]);

    const byAgent = await api.getDashboard("7d", undefined, "codex");
    expect(byAgent.by_agent.map((a) => a.agent)).toEqual(["codex"]);
  });

  it("filter by ids the provider list actually has", async () => {
    // A filter that names an id no provider answers to is a selection that
    // narrows to nothing — the name-matching lookup this used to do stopped
    // resolving the moment a vendor was spelled differently in the two lists.
    const known = new Set((await api.listProviders()).map((p) => p.id));
    for (const window of WINDOWS) {
      for (const p of (await api.getDashboard(window)).filter_providers) {
        expect(known.has(p.id), `${p.label} (${p.id}) is not a provider`).toBe(true);
      }
    }
  });
});
