// Agent routing strategy panel (tech.md §4.7): strategy type + candidate ordering.
// Rendered inside a single agent's tab: shows that agent's route only (and
// nothing when it has no bindings yet); changes take effect immediately via
// the gateway's /reload.
import { useState } from "react";

import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";

import { api } from "../api/client";
import { AGENTS, type AgentId, type AgentRoute, type StrategyKind } from "../api/types";
import StrategyIcon from "./StrategyIcon";
import { Button } from "@/components/ui/button";

const STRATEGIES: { id: StrategyKind; label: string; hint: string }[] = [
  { id: "single", label: "Single primary", hint: "Always use the primary provider" },
  { id: "failover", label: "Failover", hint: "Fall through standbys in order on failure, switch back on recovery" },
  { id: "roundrobin", label: "Weighted round-robin", hint: "New sessions rotate by weight; sticky per session to keep the upstream prompt cache" },
  { id: "timewindow", label: "Time window", hint: "Pick by each candidate's local time window; fall back to the primary when no window matches" },
  { id: "quota", label: "Quota fallback", hint: "Switch to standbys once the primary exceeds its daily threshold" },
];

function parseQuota(config: string | null): { limit: number; unit: "requests" | "tokens" } {
  try {
    const c = JSON.parse(config ?? "{}") as { limit?: number; unit?: string };
    return {
      limit: Number(c.limit) || 0,
      unit: c.unit === "tokens" ? "tokens" : "requests",
    };
  } catch {
    return { limit: 0, unit: "requests" };
  }
}

// Sensible per-unit thresholds: 100 requests vs 100 tokens a day are not
// meaningful swaps, so a unit switch replaces a default/empty limit with the
// new unit's default and keeps user-tuned values
const DEFAULT_LIMIT: Record<"requests" | "tokens", number> = { requests: 100, tokens: 1_000_000 };

function RouteRow({ route, onChanged }: { route: AgentRoute; onChanged: () => void }) {
  const meta = AGENTS.find((m) => m.id === route.agent)!;
  const hint = STRATEGIES.find((s) => s.id === route.strategy)?.hint ?? "";
  const quota = parseQuota(route.config);

  const setStrategy = async (next: StrategyKind) => {
    if (next === "quota") {
      const config = JSON.stringify({ limit: quota.limit || DEFAULT_LIMIT.requests, unit: quota.unit });
      await api.updateAgentStrategy(route.agent, next, config);
    } else {
      await api.updateAgentStrategy(route.agent, next, null);
    }
    onChanged();
  };

  const setQuota = async (patch: Partial<ReturnType<typeof parseQuota>>) => {
    const next = { ...quota, ...patch };
    // Crossing units on a default (or empty) limit: seed the new unit's default
    if (patch.unit && patch.unit !== quota.unit && (quota.limit === 0 || quota.limit === DEFAULT_LIMIT[quota.unit])) {
      next.limit = DEFAULT_LIMIT[patch.unit];
    }
    await api.updateAgentStrategy(route.agent, "quota", JSON.stringify(next));
    onChanged();
  };

  return (
    <div className="flex items-center gap-3 border-b border-line px-4 py-2">
      <div className="flex w-[200px] flex-none items-center">
        <Select
          value={route.strategy}
          onValueChange={(v) => setStrategy(v as StrategyKind)}
        >
          <SelectTrigger
            size="sm"
            aria-label={`${meta.label} strategy`}
            className="h-7 min-w-[92px] bg-transparent px-1.5 text-[11.5px] dark:bg-transparent"
          >
            {/* Static children replace the selected-item label: icon + name */}
            <SelectValue>
              <StrategyIcon id={route.strategy} className="size-3.5" />
              <span>{STRATEGIES.find((s) => s.id === route.strategy)?.label}</span>
            </SelectValue>
          </SelectTrigger>
          {/* wider than the trigger: the longest label ("Weighted round-robin") must fit */}
          <SelectContent className="min-w-[230px]">
            {STRATEGIES.map((s) => (
              <SelectItem key={s.id} value={s.id}>
                <StrategyIcon id={s.id} className="size-3.5" />
                {s.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>

      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2 text-[11px] text-mut">
        <span className="truncate">{hint}</span>
        {route.strategy === "quota" && (
          <span className="flex flex-none items-center gap-1">
            <Input
              type="number"
              min={1}
              className="h-6 w-20 rounded-md bg-transparent px-1.5 text-right font-mono text-[11px] dark:bg-transparent"
              value={quota.limit || ""}
              placeholder={String(DEFAULT_LIMIT[quota.unit])}
              onChange={(e) => setQuota({ limit: Number(e.target.value) || 0 })}
            />
            <Select
              value={quota.unit}
              onValueChange={(v) => setQuota({ unit: v as "requests" | "tokens" })}
            >
              <SelectTrigger
                size="sm"
                className="h-6 gap-1 bg-transparent px-1 text-[11px] dark:bg-transparent [&_svg:not([class*='size-'])]:size-3"
              >
                <SelectValue />
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="requests">requests/day</SelectItem>
                <SelectItem value="tokens">tokens/day</SelectItem>
              </SelectContent>
            </Select>
          </span>
        )}
      </div>
    </div>
  );
}

// One-shot route copy: apply another agent's strategy combo (type + ordered
// candidates, weights and windows included) onto this agent, replacing its
// current route. The confirm step is inline — no native dialog — so the
// destructive intent stays visible right where the click happens. Rendered in
// the taken-over onboarding flow only: copying a tuned route is the whole
// point before the agent has its own; once configured, the row stays out of
// the tab (bind/remove covers incremental tweaks).
export function CopyRouteRow({
  agent,
  routes,
  onChanged,
}: {
  agent: AgentId;
  routes: AgentRoute[];
  onChanged: () => void;
}) {
  const [src, setSrc] = useState<AgentId | null>(null);
  const [bump, setBump] = useState(0);
  const [busy, setBusy] = useState(false);
  const sources = routes.filter((r) => r.agent !== agent && r.bindings.length > 0);

  const mineMeta = AGENTS.find((m) => m.id === agent)!;
  const srcRoute = src ? routes.find((r) => r.agent === src) : undefined;
  const srcMeta = src ? AGENTS.find((m) => m.id === src)! : null;

  const apply = async () => {
    if (!src) return;
    setBusy(true);
    try {
      await api.applyAgentRoute(agent, src);
      setSrc(null);
      setBump((b) => b + 1);
      onChanged();
    } finally {
      setBusy(false);
    }
  };

  const content = (
    <>
      {srcRoute && srcMeta ? (
        <>
          <span className="truncate">
            Replace {mineMeta.label}'s route with {srcMeta.label}'s (
            {STRATEGIES.find((s) => s.id === srcRoute.strategy)?.label.toLowerCase()},{" "}
            {srcRoute.bindings.length} candidate{srcRoute.bindings.length === 1 ? "" : "s"})?
          </span>
          <Button size="sm" className="h-6 flex-none px-2 text-[11px]" onClick={apply} disabled={busy}>
            Replace
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 flex-none px-2 text-[11px]"
            onClick={() => setSrc(null)}
          >
            Cancel
          </Button>
        </>
      ) : (
        <>
          <span className="flex-none">Copy route from…</span>
          {/* key remounts the uncontrolled select after a cancelled pick so
              the trigger label resets */}
          <Select key={bump} onValueChange={(v) => setSrc(v as AgentId)}>
            <SelectTrigger
              size="sm"
              aria-label={`Copy route from another agent onto ${mineMeta.label}`}
              className="h-6 w-auto gap-1 bg-transparent px-1.5 text-[11px] dark:bg-transparent"
            >
              <SelectValue placeholder="Pick an agent" />
            </SelectTrigger>
            {/* wider than the trigger: "Claude Code · 2 candidates" must fit */}
            <SelectContent className="min-w-[230px]">
              {sources.map((r) => {
                const label = AGENTS.find((m) => m.id === r.agent)?.label ?? r.agent;
                return (
                  <SelectItem key={r.agent} value={r.agent}>
                    {label} · {r.bindings.length} candidate{r.bindings.length === 1 ? "" : "s"}
                  </SelectItem>
                );
              })}
            </SelectContent>
          </Select>
        </>
      )}
    </>
  );

  // Nothing to copy from until another agent has a route
  if (sources.length === 0) return null;
  return (
    <div className="flex items-center gap-2 text-[11px] text-mut">{content}</div>
  );
}

// Routes are owned by the host screen (single fetch) — a route created while
// this panel is mounted (first bind, route copy) must appear immediately.
export default function StrategyPanel({
  agent,
  routes,
  onChanged,
}: {
  agent: AgentId;
  routes: AgentRoute[] | null;
  onChanged?: () => void;
}) {
  // Only this tab's agent, and only once it has a route (bindings) to configure
  const mine = routes?.filter((r) => r.agent === agent) ?? null;
  if (!mine || mine.length === 0) return null;

  // No header row of its own: the closing note at the bottom of the screen
  // carries the strategy context (see Providers' footer line).
  return (
    <section className="mt-1">
      {mine.map((r) => (
        <RouteRow key={r.agent} route={r} onChanged={() => onChanged?.()} />
      ))}
    </section>
  );
}
