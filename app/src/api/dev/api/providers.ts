// The provider surface: the gateway's own status, the Apps screen's list, and
// the add / edit / park / delete paths behind it.

import type { AgentId, GatewayStatus, KiwanoApi, NewProviderInput, PromptLatency, Provider } from "../../types";
import { nextId } from "../ids";
import { delay } from "../delay";
import { endpointNote } from "../endpoints";
import { MATRIX } from "../matrix";
import { providers } from "../providers";
import { agentRoutes, agentsOf, applyServingFlags, quotaFallbackNow, servingNow, standbyFlags } from "../routes";

export const providersApi: Pick<
  KiwanoApi,
  | "getGatewayStatus"
  | "listProviders"
  | "addProvider"
  | "updateProvider"
  | "deleteProvider"
  | "setProviderEnabled"
  | "testProviderLatency"
> = {
  async getGatewayStatus(): Promise<GatewayStatus> {
    await delay();
    // The rows the matrix marks as over a limit. The gateway is what decides
    // this, and both the dimmed Usage cell and the Status column come from here
    // — a mock that always answers `[]` leaves the state unreachable in `pnpm
    // dev`, which is the only place it can be looked at.
    return {
      running: true,
      port: 8317,
      blocked: MATRIX.filter((m) => m.blocked).map((m) => ({ id: m.id, reason: m.blocked! })),
    };
  },

  async listProviders(filter: AgentId | "all" = "all"): Promise<Provider[]> {
    await delay();
    const serving = servingNow();
    const fallback = quotaFallbackNow();
    const { backups } = standbyFlags();
    return providers
      .filter((p) => filter === "all" || p.agents.includes(filter))
      .map((p) => {
        const agents = agentsOf(p.id);
        return {
          ...p,
          agents: agents as Provider["agents"],
          serving_agents: agents.filter((a) => serving.has(`${a}/${p.id}`)) as Provider["serving_agents"],
          is_current: agents.some((a) => serving.has(`${a}/${p.id}`)),
          // First in line behind an over-threshold primary — a separate claim
          // from "serving", so it gets its own slot on the wire.
          ...(agents.some((a) => fallback.has(`${a}/${p.id}`))
            ? { fallback_agents: agents.filter((a) => fallback.has(`${a}/${p.id}`)) }
            : {}),
          agents_note: backups.has(p.id)
            ? "Failover queue"
            : agents.length
              ? `${agents.length} agent(s)`
              : undefined,
        };
      });
  },

  async addProvider(input: NewProviderInput): Promise<Provider> {
    await delay();
    const endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    const p: Provider = {
      id: input.name.toLowerCase().replace(/\s+/g, "-") + "-" + nextId(),
      name: input.name,
      // Which catalog entry it came from: `vm::add_provider` stores it, and the
      // edit dialog reaches the entry through it for the endpoints, the currency
      // and the quota query the stored row may be missing.
      catalog_id: input.catalog_id ?? null,
      // What the entry bills in, or what the user declared — the fixture has no
      // catalog to ask, so a hand-added provider takes the demo's own currency.
      currency: input.prices?.currency ?? "CNY",
      logo_char: input.name.charAt(0).toUpperCase(),
      logo_color: "#555555",
      endpoint: input.endpoint.replace(/^https?:\/\//, ""),
      endpoint_note: endpointNote(input.protocol, endpoints),
      protocol: input.protocol,
      endpoints,
      billing: input.billing,
      limit_unit: input.billing_config.limit_unit,
      enabled: true,
      agents: input.agents ?? [],
      // Optimistic, mirrors vm::add_provider; listProviders recomputes
      serving_agents: [],
      is_current: (input.agents ?? []).length > 0,
      agents_note: (input.agents ?? []).length
        ? `${input.agents!.length} agent(s)`
        : undefined,
      health: { state: "idle", latency_ms: null },
      usage: null,
      advanced: input.advanced,
      model_default: input.model_default.trim() || null,
      plan_query: input.plan_query ?? null,
      plan_limits: input.billing_config.plan_limits ?? null,
      // What the user declared this provider charges; absent = none declared,
      // which is how a provider priced by the Hub's table reads.
      prices: input.prices ?? null,
    };
    providers.unshift(p);
    return p;
  },

  async updateProvider(id: string, input: NewProviderInput): Promise<Provider> {
    await delay();
    const t = providers.find((p) => p.id === id);
    if (!t) throw new Error(`provider ${id} not found`);
    t.name = input.name.trim();
    t.model_default = input.model_default.trim() || null;
    t.endpoint = input.endpoint.replace(/^https?:\/\//, "");
    t.protocol = input.protocol;
    t.endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    t.endpoint_note = endpointNote(input.protocol, t.endpoints);
    t.billing = input.billing;
    // Absent `agents` keeps the existing bindings (mirrors vm::update_provider):
    // which providers serve an agent is edited on the Apps screen, not here.
    if (input.agents !== undefined) t.agents = [...input.agents];
    t.serving_agents = t.agents.filter((a) => servingNow().has(`${a}/${t.id}`));
    t.is_current = t.serving_agents.length > 0;
    const fb = t.agents.filter((a) => quotaFallbackNow().has(`${a}/${t.id}`));
    if (fb.length > 0) (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents = fb;
    else delete (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents;
    t.agents_note = t.agents.length ? `${t.agents.length} agent(s)` : undefined;
    // Absent `advanced` keeps existing values (mirrors vm::update_provider).
    if (input.advanced !== undefined) t.advanced = input.advanced;
    if (input.plan_query !== undefined) t.plan_query = input.plan_query;
    t.limit_unit = input.billing_config.limit_unit;
    t.plan_limits = input.billing_config.plan_limits ?? null;
    // Absent `prices` keeps what is stored (mirrors vm::update_provider): the
    // form omits the field whenever its Prices section is not on screen.
    if (input.prices !== undefined) {
      t.prices = input.prices.models.length ? input.prices : null;
    }
    return t;
  },

  async deleteProvider(id: string): Promise<void> {
    await delay();
    const i = providers.findIndex((p) => p.id === id);
    if (i >= 0) providers.splice(i, 1);
    // vm::delete_provider in the mock's smaller world: the provider's bindings
    // go with it, and every agent whose head it was promotes the next
    // candidate, renumbering what is left. Without this the route kept a
    // pointer to a row that is gone — which `pnpm dev` renders as a table with
    // a row missing rather than as the promotion the app actually performs.
    for (const r of agentRoutes) {
      if (!r.bindings.some((b) => b.provider_id === id)) continue;
      const wasHead = r.bindings[0].provider_id === id;
      r.bindings = r.bindings.filter((b) => b.provider_id !== id);
      if (wasHead) r.bindings = r.bindings.map((b, i) => ({ ...b, priority: i }));
    }
  },

  async setProviderEnabled(id: string, enabled: boolean): Promise<void> {
    await delay();
    // vm::set_provider_enabled: the row's own flag, no route touched. What
    // changes is whether it may serve — which `servingNow` reads, the same way
    // the gateway's route table reads `providers.enabled`.
    const target = providers.find((p) => p.id === id);
    if (!target) throw new Error(`provider not found: ${id}`);
    target.enabled = enabled;
    applyServingFlags();
  },

  async testProviderLatency(id: string): Promise<PromptLatency> {
    // A real round trip cannot happen in a browser mock, so this answers the
    // shape the app reads: a model, a latency, and a failure when the provider
    // is one the fixture says is broken.
    await delay(600);
    const p = providers.find((x) => x.id === id);
    if (!p) throw new Error(`provider not found: ${id}`);
    const model = p.model_default ?? "ping-model";
    const checked_at = new Date().toISOString();
    // …and it records the verdict, because that is what the backend does
    // (`vm::test_provider_latency` writes it as the provider's health) and it is
    // what makes the Status cell catch up after a click. A mock that only moved
    // the number on the button would send `pnpm dev` looking at a cell that
    // stays stale — the disagreement this file's header warns about.
    if (p.enabled === false) {
      // Answered and refused: a 401 is the vendor talking, so reachability is
      // not what failed — the key is.
      p.health = {
        state: "error",
        latency_ms: 180,
        source: "test",
        checked_at,
        error: "invalid API key",
      };
      return { provider_id: id, model, latency_ms: 180, status: 401, error: "invalid API key" };
    }
    const base = 220 + (id.length % 7) * 130;
    p.health = { state: "ok", latency_ms: base, source: "test", checked_at };
    return { provider_id: id, model, latency_ms: base, status: 200, error: null };
  },
};
