// Agent 路由策略面板（tech.md §4.7）：策略类型 + 候选排序。
// 只展示有绑定的 Agent；改动经网关 /reload 即时生效。
import { useCallback, useEffect, useState } from "react";

import { ArrowDown, ArrowUp } from "lucide-react";

import { api } from "../api/client";
import { AGENTS, type AgentRoute, type StrategyKind } from "../api/types";
import { AgentChip, Logo } from "./bits";

const STRATEGIES: { id: StrategyKind; label: string; hint: string }[] = [
  { id: "single", label: "单一主选", hint: "固定使用主选 Provider" },
  { id: "failover", label: "故障转移", hint: "主选失败按序下沉备用，恢复后自动回切" },
  { id: "roundrobin", label: "加权轮询", hint: "新会话按权重轮转，同会话固定以保住上游 prompt cache" },
  { id: "timewindow", label: "峰谷窗口", hint: "按候选的本地时段窗口选择，未命中回主选" },
  { id: "quota", label: "额度下沉", hint: "主选当日用量超阈值后转用备用" },
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

  const move = async (idx: number, dir: -1 | 1) => {
    const ids = route.bindings.map((b) => b.provider_id);
    const j = idx + dir;
    if (j < 0 || j >= ids.length) return;
    [ids[idx], ids[j]] = [ids[j], ids[idx]];
    await api.reorderAgentBindings(route.agent, ids);
    onChanged();
  };

  return (
    <div className="flex items-center gap-3 border-b border-line px-4 py-2">
      <div className="flex w-[200px] flex-none items-center gap-2">
        <AgentChip meta={meta} />
        <select
          className="h-7 rounded-md border border-line bg-transparent px-1.5 text-[11.5px]"
          value={route.strategy}
          onChange={(e) => setStrategy(e.target.value as StrategyKind)}
          aria-label={`${meta.label} 策略`}
        >
          {STRATEGIES.map((s) => (
            <option key={s.id} value={s.id}>
              {s.label}
            </option>
          ))}
        </select>
      </div>

      <div className="flex min-w-0 flex-1 flex-wrap items-center gap-2 text-[11px] text-mut">
        <span className="truncate">{hint}</span>
        {route.strategy === "quota" && (
          <span className="flex flex-none items-center gap-1">
            <input
              type="number"
              min={1}
              className="h-6 w-16 rounded border border-line bg-transparent px-1.5 text-right font-mono text-[11px]"
              value={quota.limit || ""}
              placeholder="100"
              onChange={(e) => setQuota({ limit: Number(e.target.value) || 0 })}
            />
            <select
              className="h-6 rounded border border-line bg-transparent px-1 text-[11px]"
              value={quota.unit}
              onChange={(e) => setQuota({ unit: e.target.value as "requests" | "tokens" })}
            >
              <option value="requests">请求/日</option>
              <option value="tokens">tokens/日</option>
            </select>
          </span>
        )}
      </div>

      <div className="flex flex-none items-center gap-1.5">
        {route.bindings.map((b, i) => (
          <span
            key={b.provider_id}
            className="flex items-center gap-1 rounded-md border border-line py-0.5 pl-1 pr-0.5"
            title={`优先级 #${i} · 权重 ${b.weight}`}
          >
            <Logo char={b.logo_char} color={b.logo_color} size="w-4 h-4 text-[9px]" />
            <span className="max-w-24 truncate text-[11px]">{b.provider_name}</span>
            {i === 0 && (
              <span className="rounded px-1 text-[9.5px] font-medium" style={{ background: "var(--kiwi-soft)", color: "var(--kiwi)" }}>
                主
              </span>
            )}
            <span className="flex flex-col">
              <button
                className="text-mut hover:text-ink disabled:opacity-30"
                aria-label="上移"
                disabled={i === 0}
                onClick={() => move(i, -1)}
              >
                <ArrowUp className="h-2.5 w-2.5" />
              </button>
              <button
                className="text-mut hover:text-ink disabled:opacity-30"
                aria-label="下移"
                disabled={i === route.bindings.length - 1}
                onClick={() => move(i, 1)}
              >
                <ArrowDown className="h-2.5 w-2.5" />
              </button>
            </span>
          </span>
        ))}
        {route.bindings.length === 1 && (
          <span className="text-[10.5px] text-mut">单候选 · 添加供应商后可排序</span>
        )}
      </div>
    </div>
  );
}

export default function StrategyPanel() {
  const [routes, setRoutes] = useState<AgentRoute[] | null>(null);

  const refetch = useCallback(() => {
    api.getAgentRoutes().then(setRoutes);
  }, []);
  useEffect(refetch, [refetch]);

  if (!routes || routes.length === 0) return null;

  return (
    <section className="mt-1">
      <div className="flex h-8 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
        Agent 路由策略 · 失败/超限自动下沉，改动即时生效
      </div>
      {routes.map((r) => (
        <RouteRow key={r.agent} route={r} onChanged={refetch} />
      ))}
    </section>
  );
}
