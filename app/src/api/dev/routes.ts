// The routing fixtures, and the read model derived from them: which provider
// would serve each agent right now, which is first in line behind it, and what
// the Apps screen's notes and badges are built from. Mirrors
// `crates/core/src/vm/routes.rs` and the `vm::build_provider_vms` rules.

import type { AgentId, AgentRoute, Provider, StrategyBinding } from "../types";
import { providers } from "./providers";
import { settings } from "./settings";

// ── Agent routing strategies (tech.md §4.7) ──

/** One candidate of a route. `weight` is a roundrobin share (see the matrix's
    rotating route); every other strategy reads it as 1. */
export function bind(pid: string, priority: number, win?: [string, string]): StrategyBinding {
  const p = providers.find((x) => x.id === pid)!;
  return {
    provider_id: pid,
    provider_name: p.name,
    logo_char: p.logo_char,
    logo_color: p.logo_color,
    priority,
    weight: 1,
    win_start: win?.[0] ?? null,
    win_end: win?.[1] ?? null,
    enabled: p.enabled,
  };
}

export const agentRoutes: AgentRoute[] = [
  {
    agent: "claude",
    strategy: "failover",
    config: null,
    // The matrix row rides the queue's tail, so the failover role is on a row
    // that also demonstrates a limit state.
    bindings: [bind("deepseek", 0), bind("kimi", 1), bind("m-payg-cny", 2)],
    limits: [],
  },
  {
    agent: "codex",
    strategy: "single",
    config: null,
    bindings: [bind("deepseek", 0)],
    limits: [],
  },
  {
    // A user-defined agent: a route like any other, over its own key.
    agent: "long-tasks-3f9a",
    strategy: "failover",
    config: null,
    bindings: [bind("deepseek", 0), bind("kimi", 1), bind("m-plan-quota", 2)],
    limits: [],
  },
  {
    // Kimi heads opencode (globally "In use") while standing by in claude's
    // queue — exercises the agent tabs' local vs global "In use" badge.
    agent: "opencode",
    strategy: "single",
    config: null,
    bindings: [bind("kimi", 0)],
    limits: [],
  },
  {
    // Overnight window on the standby exercises the timewindow wraparound path.
    agent: "grokbuild",
    strategy: "timewindow",
    config: null,
    bindings: [bind("deepseek", 0), bind("ollama", 1, ["22:00", "06:00"])],
    limits: [],
  },
  // The matrix's own routes: the three strategies no built-in agent runs.
  {
    // Every candidate serves, in proportion to its weight. Both rows carry an
    // "In use" badge here — which is what distinguishes roundrobin from the
    // failover queues above, where the standby is a note with no badge.
    agent: "rotating-pool-4a1f",
    strategy: "roundrobin",
    config: null,
    bindings: [{ ...bind("m-payg-req", 0), weight: 3 }, bind("m-payg-wan", 1)],
    limits: [],
  },
  {
    // The head serves whenever the window is not in force; the second candidate
    // owns 22:00–06:00, which wraps midnight.
    agent: "nightly-batch-7c2e",
    strategy: "timewindow",
    config: null,
    bindings: [bind("m-payg-trend", 0), bind("m-unl", 1, ["22:00", "06:00"])],
    limits: [],
  },
  {
    // A ceiling the head is over: the gateway refuses to route to it, and the
    // row reads dimmed with the reason in the Status column. The config's unit
    // is the strategy's own vocabulary (requests | tokens) — the currency
    // ceiling that blocked the row is the *provider's*, a different limit.
    // The tail is what "volumes are over" shows: the first backup reads as
    // "Fallback" — first in line, not in use (vm::build_provider_vms).
    agent: "quota-guard-5b8d",
    strategy: "quota",
    config: JSON.stringify({ limit: 500, unit: "requests" }),
    bindings: [bind("m-payg-over", 0), bind("m-payg-trend", 1)],
    limits: [],
  },
];

export function strategyOf(agent: AgentId): AgentRoute {
  // The backend lazily creates a Single-strategy route on first takeover
  // (vm::set_agent_takeover); the mock does the same so agents enabled from
  // Settings without a pre-seeded route behave identically.
  let route = agentRoutes.find((r) => r.agent === agent);
  if (!route) {
    route = { agent, strategy: "single", config: null, bindings: [], limits: [] };
    agentRoutes.push(route);
  }
  return route;
}

// Mirror vm::build_provider_vms: the "In use" badge marks the provider(s) that
// would serve a request issued right now under each agent's strategy. Quota
// mirrors vm::quota_over_threshold too: over the ceiling, the head stops
// serving and the first backup reads as fallback, not in use — which backup
// (if any) actually takes a request is the gateway's breakers' runtime call.
// The mock fixtures carry window totals, not per-day ones, so the fixture's
// request count stands in for the day's in `quotaOver`.
export function servingNow(): Set<string> {
  const out = new Set<string>();
  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const inWin = (n: number, s: string, e: string) => {
    const [sh, sm] = s.split(":").map(Number);
    const [eh, em] = e.split(":").map(Number);
    const a = sh * 60 + sm;
    const b = eh * 60 + em;
    return a <= b ? n >= a && n <= b : n >= a || n <= b;
  };
  for (const r of routedRoutes()) {
    // Disabled bindings and parked providers both drop out, as they do in the
    // gateway's route table.
    const enabled = r.bindings.filter(
      (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
    );
    if (enabled.length === 0) continue;
    const head = enabled[0].provider_id;
    if (r.strategy === "roundrobin") {
      enabled.forEach((b) => out.add(`${r.agent}/${b.provider_id}`));
    } else if (r.strategy === "timewindow") {
      const hit = enabled.find(
        (b) => b.win_start && b.win_end && inWin(nowMin, b.win_start, b.win_end),
      );
      out.add(`${r.agent}/${hit?.provider_id ?? head}`);
    } else if (r.strategy === "quota" && quotaOver(r)) {
      // over the ceiling: nothing serves (the backup is first in line, below)
    } else {
      // single / failover / quota-under: the head (breaker state is gateway runtime)
      out.add(`${r.agent}/${head}`);
    }
  }
  return out;
}

/** The quota strategy's configured first backup, while the head is over its
    threshold — vm::build_provider_vms's `fallback` map, mirrored. */
export function quotaFallbackNow(): Set<string> {
  const out = new Set<string>();
  for (const r of routedRoutes()) {
    if (r.strategy !== "quota") continue;
    const enabled = r.bindings.filter(
      (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
    );
    if (enabled.length < 2 || !quotaOver(r)) continue;
    out.add(`${r.agent}/${enabled[1].provider_id}`);
  }
  return out;
}

/** Whether a quota route's head has consumed its config's limit — the mock's
    stand-in for `vm::quota_over_threshold` (which reads the same-day totals). */
function quotaOver(r: AgentRoute): boolean {
  const cfg = r.config ? (JSON.parse(r.config) as { limit: number; unit?: string }) : null;
  if (!cfg || !Number.isFinite(cfg.limit)) return false;
  const head = r.bindings.find(
    (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
  );
  if (!head) return false;
  const p = providers.find((x) => x.id === head.provider_id);
  const consumed =
    cfg.unit === "tokens"
      ? ((p?.usage?.input_tokens ?? 0) + (p?.usage?.output_tokens ?? 0))
      : (p?.usage?.requests ?? 0);
  return consumed >= cfg.limit;
}

/** Recompute every provider's serving/fallback flags from the current routes —
    what `listProviders` (and the mutation paths that re-derive them) show.
    Agents come from the routes (`agentsOf`), not the fixtures' static arrays,
    which drift from `agentRoutes` by design. */
export function applyServingFlags() {
  const serving = servingNow();
  const fallback = quotaFallbackNow();
  for (const p of providers) {
    const agents = agentsOf(p.id);
    p.serving_agents = agents.filter((a) => serving.has(`${a}/${p.id}`)) as Provider["serving_agents"];
    p.is_current = p.serving_agents.length > 0;
    const fb = agents.filter((a) => fallback.has(`${a}/${p.id}`));
    if (fb.length > 0) p.fallback_agents = fb;
    else delete p.fallback_agents;
  }
}

// vm::build_provider_vms derives ProviderVm.agents from the bindings; the mock
// fixtures' static arrays drift from agentRoutes, so derive them the same way.
/** Routes whose agent currently points at the gateway — the mock's stand-in
    for the real VM's live-file check. A route the store kept for an agent that
    has been handed its own config back is a plan, not traffic, so it must not
    put the provider in that agent's column or under "In use". */
function routedRoutes(): AgentRoute[] {
  return agentRoutes.filter(
    (r) =>
      // A user-defined agent routes as long as it exists: it has no config
      // file for the takeover list to be reporting on (vm::live_bound_agents).
      settings.custom_agents.some((a) => a.id === r.agent) ||
      settings.takeovers.find((t) => t.agent === r.agent)?.enabled,
  );
}

export function agentsOf(pid: string): string[] {
  return routedRoutes()
    .filter((r) => r.bindings.some((b) => b.provider_id === pid))
    .map((r) => r.agent);
}

// Mirror vm::build_provider_vms's failover-queue classification: a non-head
// candidate is a queue member ("Failover queue" note — no badge; standby
// badges read as a contradiction next to "In use") unless its strategy
// rotates through everyone (roundrobin) or it serves its own time window.
export function standbyFlags(): { backups: Set<string> } {
  const backups = new Set<string>();
  for (const r of routedRoutes()) {
    const head = r.bindings.filter((b) => b.enabled)[0]?.provider_id;
    if (!head) continue;
    for (const b of r.bindings) {
      if (b.provider_id === head) continue;
      const standby =
        r.strategy === "roundrobin"
          ? false
          : r.strategy === "timewindow"
            ? !(b.win_start && b.win_end)
            : true;
      if (standby) backups.add(b.provider_id);
    }
  }
  return { backups };
}
