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

import type { KiwanoApi } from "./types";

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
    const dormant = new Set(
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
