// An agent's route: its candidates and their order, weights and windows, the
// strategy it runs, and the ceilings it carries. Mirrors `vm::routes`.

import type { AgentId, AgentLimit, AgentRoute, KiwanoApi, StrategyKind } from "../../types";
import { delay } from "../delay";
import { providers } from "../providers";
import { agentRoutes, bind, strategyOf } from "../routes";

export const routesApi: Pick<
  KiwanoApi,
  | "getAgentRoutes"
  | "setAgentLimits"
  | "updateAgentStrategy"
  | "reorderAgentBindings"
  | "updateAgentBinding"
  | "addAgentBinding"
  | "removeAgentBinding"
  | "applyAgentRoute"
> = {
  async getAgentRoutes(): Promise<AgentRoute[]> {
    await delay();
    // Mirror the backend: only agents with at least one binding get a route
    return agentRoutes
      .filter((r) => r.bindings.length > 0)
      .map((r) => ({ ...r, bindings: r.bindings.map((b) => ({ ...b })) }));
  },

  async setAgentLimits(agent: AgentId, limits: AgentLimit[]): Promise<void> {
    await delay();
    // A window of zero is the absence of that window rather than a ceiling of
    // nothing, mirroring vm::set_agent_limits — a fixture that stored it would
    // show a limit the gateway would not enforce.
    strategyOf(agent).limits = limits.filter((l) => l.period && l.period_limit > 0);
  },

  async updateAgentStrategy(agent: AgentId, strategy: StrategyKind, config?: string | null): Promise<void> {
    await delay();
    strategyOf(agent).strategy = strategy;
    strategyOf(agent).config = config ?? null;
    // Entering roundrobin: reseed the weights as an even split of 100
    // (remainder to the head of the queue), mirroring vm::set_agent_strategy
    if (strategy === "roundrobin") {
      const bs = strategyOf(agent).bindings;
      bs.forEach((b, i) => {
        b.weight = Math.max(1, Math.floor(100 / bs.length) + (i < 100 % bs.length ? 1 : 0));
      });
    }
  },

  async reorderAgentBindings(agent: AgentId, providerIds: string[]): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    route.bindings = providerIds
      .map((pid, i) => {
        const b = route.bindings.find((x) => x.provider_id === pid)!;
        return { ...b, priority: i };
      })
      .sort((a, b) => a.priority - b.priority);
  },

  async updateAgentBinding(
    agent: AgentId,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ): Promise<void> {
    await delay();
    const b = strategyOf(agent).bindings.find((x) => x.provider_id === providerId);
    if (!b) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    if (patch.weight != null) b.weight = Math.max(1, patch.weight);
    // Both bounds are set/cleared together: a half window would never match.
    if (patch.win_start !== undefined || patch.win_end !== undefined) {
      const s = patch.win_start || null;
      const e = patch.win_end || null;
      if (s && e) {
        b.win_start = s;
        b.win_end = e;
      } else {
        b.win_start = null;
        b.win_end = null;
      }
    }
  },

  async addAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    if (route.bindings.some((b) => b.provider_id === providerId)) return;
    route.bindings.push(bind(providerId, route.bindings.length));
    // Mirror the backend: a provider's agents derive from its bindings
    const p = providers.find((x) => x.id === providerId)!;
    if (!p.agents.includes(agent)) p.agents.push(agent);
  },

  async removeAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    const i = route.bindings.findIndex((b) => b.provider_id === providerId);
    if (i < 0) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    route.bindings.splice(i, 1);
    const p = providers.find((x) => x.id === providerId)!;
    p.agents = p.agents.filter((a) => a !== agent);
  },

  async applyAgentRoute(target: AgentId, source: AgentId): Promise<void> {
    await delay();
    if (target === source) throw new Error("cannot copy an agent's route onto itself");
    const src = agentRoutes.find((r) => r.agent === source);
    if (!src || src.bindings.length === 0) throw new Error(`${source} has no route to copy`);
    const existing = agentRoutes.find((r) => r.agent === target);
    const before = new Set(existing?.bindings.map((b) => b.provider_id) ?? []);
    const copy: AgentRoute = {
      agent: target,
      strategy: src.strategy,
      config: src.config,
      bindings: src.bindings.map((b) => ({ ...b })),
      // The ceilings are the agent's own, not part of the route being copied:
      // inheriting someone else's budget is not what "copy this route" means.
      limits: existing?.limits ?? [],
    };
    const i = agentRoutes.findIndex((r) => r.agent === target);
    if (i >= 0) agentRoutes[i] = copy;
    else agentRoutes.push(copy);
    // Mirror the backend: a provider's agents derive from its bindings
    for (const pid of before) {
      if (copy.bindings.some((b) => b.provider_id === pid)) continue;
      const p = providers.find((x) => x.id === pid);
      if (p) p.agents = p.agents.filter((a) => a !== target);
    }
    for (const b of copy.bindings) {
      const p = providers.find((x) => x.id === b.provider_id);
      if (p && !p.agents.includes(target)) p.agents.push(target);
    }
  },
};
