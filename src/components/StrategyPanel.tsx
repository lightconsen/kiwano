// Agent routing strategy panel (tech.md §4.7): strategy type + candidate ordering.
// Rendered inside a single agent's tab: shows that agent's route only (and
// nothing when it has no bindings yet); changes take effect immediately via
// the gateway's /reload.
import { useCallback, useEffect, useState } from "react";

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

function RouteRow({ route, onChanged }: { route: AgentRoute; onChanged: () => void }) {
  const meta = AGENTS.find((m) => m.id === route.agent)!;
  const hint = STRATEGIES.find((s) => s.id === route.strategy)?.hint ?? "";
  const quota = parseQuota(route.config);

  const setStrategy = async (next: StrategyKind) => {
    if (next === "quota") {
      const config = JSON.stringify({ limit: quota.limit || 100, unit: quota.unit });
      await api.updateAgentStrategy(route.agent, next, config);
    } else {
      await api.updateAgentStrategy(route.agent, next, null);
    }
    onChanged();
  };

  const setQuota = async (patch: Partial<ReturnType<typeof parseQuota>>) => {
    const next = { ...quota, ...patch };
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
              className="h-6 w-16 rounded-md bg-transparent px-1.5 text-right font-mono text-[11px] dark:bg-transparent"
              value={quota.limit || ""}
              placeholder="100"
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

export default function StrategyPanel({ agent, onChanged }: { agent: AgentId; onChanged?: () => void }) {
  const [routes, setRoutes] = useState<AgentRoute[] | null>(null);

  const refetch = useCallback(() => {
    api.getAgentRoutes().then(setRoutes);
    // The host screen renders the candidate rows from the same routes — let it
    // refresh too (e.g. a strategy switch to roundrobin changes the row roles).
    onChanged?.();
  }, [onChanged]);
  useEffect(refetch, [refetch]);

  // Only this tab's agent, and only once it has a route (bindings) to configure
  const mine = routes?.filter((r) => r.agent === agent) ?? null;
  if (!mine || mine.length === 0) return null;

  return (
    <section className="mt-1">
      <div className="flex h-8 items-center gap-3 border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
        <span className="flex-none">Agent routing strategy</span>
        <span className="truncate">
          the provider rows above are the candidates in priority order · switching strategy keeps the
          order, the primary stays first · changes apply instantly
        </span>
      </div>
      {mine.map((r) => (
        <RouteRow key={r.agent} route={r} onChanged={refetch} />
      ))}
    </section>
  );
}
