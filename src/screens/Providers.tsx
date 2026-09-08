// Home screen: Apps (local provider list, design/index.html #s-providers, formerly "我的供应商")
import { useCallback, useEffect, useState } from "react";

import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { AGENTS, type AgentId, type Provider } from "../api/types";
import { AgentChip, BillTag, Dot, Logo, Ring, Sparkline } from "../components/bits";
import StrategyPanel from "../components/StrategyPanel";
import { fmtCny, fmtLatency, fmtTokens } from "../lib/format";

const SEGMENTS: { id: AgentId | "all"; label: string }[] = [
  { id: "all", label: "全部" },
  { id: "claude", label: "Claude Code" },
  { id: "codex", label: "Codex" },
  { id: "gemini", label: "Gemini CLI" },
];

function ringColor(billing: Provider["billing"], pct: number): string {
  if (billing === "plan") return "oklch(0.78 0.12 300)";
  if (pct >= 95) return "var(--red)";
  if (pct >= 80) return "var(--amber)";
  return "var(--kiwi)";
}

function usageTitle(p: Provider): string {
  const u = p.usage;
  if (!u) return "";
  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return `订阅套餐 · 本期 ${u.quota.used}/${u.quota.limit} 请求（${pct}%）· ${u.quota.resets_at ?? ""} 重置 · 输入 ${fmtTokens(u.input_tokens)} · 输出 ${fmtTokens(u.output_tokens)}`;
  }
  if (p.billing === "unl") return "本地推理 · 不计量费用 · 离线可用";
  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return `Pay as you go · 本期 ${fmtCny(u.cost ?? 0)} / ${fmtCny(u.quota.limit)}（${pct}%）· 输入 ${fmtTokens(u.input_tokens)}（缓存 ${fmtTokens(u.cache_read_tokens)}，按 1/10 计价）· 输出 ${fmtTokens(u.output_tokens)} · 延迟 ${fmtLatency(u.latency_ms)}`;
  }
  return "Pay as you go · 未设限额 · 近 7 日用量趋势";
}

function UsageCell({ p }: { p: Provider }) {
  const u = p.usage;
  if (!u) return <div className="w-[22%]" />;
  const title = usageTitle(p);

  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return (
      <div className="flex w-[22%] items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("plan", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="plan" />
            {u.quota.used}/{u.quota.limit} <span className="font-normal text-mut">请求</span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {[p.plan_price, u.quota.resets_at ? `${u.quota.resets_at.slice(5)} 重置` : null]
              .filter(Boolean)
              .join(" · ")}
          </div>
        </div>
      </div>
    );
  }

  if (p.billing === "unl") {
    return (
      <div className="w-[22%]" title={title}>
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="unl" />
          {u.requests} 次 <span className="font-normal text-mut">· {fmtTokens(u.input_tokens + u.output_tokens)} tok</span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">
          入 {fmtTokens(u.input_tokens)} · 出 {fmtTokens(u.output_tokens)}
        </div>
      </div>
    );
  }

  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return (
      <div className="flex w-[22%] items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("payg", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="payg" />
            {fmtCny(u.cost ?? 0)} <span className="font-normal text-mut">/ {fmtCny(u.quota.limit)} 限额</span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {fmtTokens(u.input_tokens + u.output_tokens)} tokens · 延迟 {fmtLatency(u.latency_ms)}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-[22%]" title={title}>
      <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
        <BillTag billing="payg" />
        {fmtCny(u.cost ?? 0)} <span className="font-normal text-mut">· {u.requests} 次</span>
      </div>
      {u.spark && (
        <span className="mt-1 block w-20">
          <Sparkline points={u.spark} />
        </span>
      )}
    </div>
  );
}

function ProviderRow({
  p,
  onEnable,
  onEdit,
  onDelete,
}: {
  p: Provider;
  onEnable: (id: string) => void;
  onEdit: (p: Provider) => void;
  onDelete: (p: Provider) => void;
}) {
  // Delete is a two-step confirm: the first click enters the confirm state; a second click within 3 seconds actually deletes
  const [confirmDel, setConfirmDel] = useState(false);
  useEffect(() => {
    if (!confirmDel) return;
    const t = setTimeout(() => setConfirmDel(false), 3000);
    return () => clearTimeout(t);
  }, [confirmDel]);
  return (
    <div className={`row flex h-[58px] items-center border-b border-line px-4${p.is_current ? " current" : ""}`}>
      <div className="flex min-w-0 w-[34%] items-center gap-2.5">
        <Logo char={p.logo_char} color={p.logo_color} border={p.logo_border} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5">
            <span className="text-[13px] font-semibold">{p.name}</span>
            {p.is_current && (
              <span className="rounded px-1.5 py-px text-[10px] font-medium" style={{ background: "var(--kiwi)", color: "oklch(0.18 0.03 132)" }}>
                使用中
              </span>
            )}
            {p.status_badge && (
              <span className="rounded px-1.5 py-px text-[10px] font-medium text-mut" style={{ background: "var(--surface2)" }}>
                {p.status_badge}
              </span>
            )}
          </div>
          <div className="mt-0.5 truncate font-mono text-[11px] text-mut">
            {p.endpoint} · {p.endpoint_note}
          </div>
        </div>
      </div>

      <div className="flex w-[22%] items-center gap-1">
        {p.agents.length === 0 ? (
          <span className="text-[11px] text-mut">未绑定</span>
        ) : (
          <>
            {p.agents.map((a) => (
              <AgentChip key={a} meta={AGENTS.find((m) => m.id === a)!} />
            ))}
            {p.agents_note && <span className="ml-1 text-[11px] text-mut">{p.agents_note}</span>}
          </>
        )}
      </div>

      <UsageCell p={p} />

      <div className="w-[14%]">
        {p.health.state === "ok" ? (
          <span className="flex items-center gap-1.5 text-[11.5px]" style={{ color: "var(--kiwi)" }}>
            <Dot state="ok" />健康 {p.health.latency_ms}ms
          </span>
        ) : (
          <span className="flex items-center gap-1.5 text-[11.5px] text-mut">
            <Dot state={p.health.state} />
            {p.health.note ?? `${p.health.latency_ms}ms`}
          </span>
        )}
      </div>

      <div className="flex flex-1 items-center justify-end gap-1.5">
        {p.is_current ? (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 whitespace-nowrap border border-line px-2.5 text-[11.5px]"
            onClick={() => onEdit(p)}
          >
            编辑
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="h-7 whitespace-nowrap border-kiwi-dim bg-kiwi-soft font-semibold text-kiwi text-[11.5px] dark:border-kiwi-dim dark:bg-kiwi-soft hover:bg-kiwi-soft dark:hover:bg-kiwi-soft"
            onClick={() => onEnable(p.id)}
          >
            启用
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className={`h-7 whitespace-nowrap border border-line px-1.5 text-[10.5px]${confirmDel ? "" : " text-mut"}`}
          style={confirmDel ? { color: "var(--red)", borderColor: "var(--red)" } : undefined}
          aria-label="删除"
          title={confirmDel ? "再次点击确认删除" : "删除供应商"}
          onClick={() => (confirmDel ? onDelete(p) : setConfirmDel(true))}
        >
          {confirmDel ? "确认删除" : <Trash2 className="h-3.5 w-3.5" />}
        </Button>
      </div>
    </div>
  );
}

export default function Providers({ onAdd, onEdit }: { onAdd: () => void; onEdit: (p: Provider) => void }) {
  const [providers, setProviders] = useState<Provider[] | null>(null);
  const [seg, setSeg] = useState<AgentId | "all">("all");

  const refetch = useCallback(() => {
    api.listProviders().then(setProviders);
  }, []);
  useEffect(refetch, [refetch]);

  const onEnable = async (id: string) => {
    await api.enableProvider(id);
    refetch();
  };

  const onDelete = async (p: Provider) => {
    await api.deleteProvider(p.id);
    refetch();
  };

  if (!providers) return <div className="p-8 text-center text-[12px] text-mut">加载中…</div>;

  const filtered = providers.filter((p) => seg === "all" || p.agents.includes(seg));
  const agentsBound = new Set(providers.flatMap((p) => p.agents)).size;

  return (
    <section>
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <div className="flex overflow-hidden rounded-lg border border-line text-[12px]">
          {SEGMENTS.map((s, i) => (
            <button
              key={s.id}
              className={`seg h-7 px-3 text-mut${i > 0 ? " border-l border-line" : ""}${seg === s.id ? " active" : ""}`}
              onClick={() => setSeg(s.id)}
            >
              {s.label}
            </button>
          ))}
        </div>
        <span className="ml-1.5 text-[11.5px] text-mut">
          {providers.length} 个供应商 · {agentsBound} 个 Agent 已绑定
        </span>
        <Button size="sm" className="ml-auto h-7 gap-1 px-2.5 text-[12px] font-semibold" onClick={onAdd}>
          <Plus className="h-3.5 w-3.5" />
          添加供应商
        </Button>
      </div>

      <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
        <span className="w-[34%]">供应商</span>
        <span className="w-[22%]">绑定的 Agent</span>
        <span className="w-[22%]">用量 / 额度</span>
        <span className="w-[14%]">状态</span>
        <span className="flex-1 text-right">操作</span>
      </div>

      {filtered.map((p) => (
        <ProviderRow key={p.id} p={p} onEnable={onEnable} onEdit={onEdit} onDelete={onDelete} />
      ))}

      {filtered.length === 0 && (
        <div className="px-4 py-8 text-center text-[12px] text-mut">
          该 Agent 尚未绑定 Provider —{" "}
          <button className="font-semibold" style={{ color: "var(--kiwi)" }} onClick={onAdd}>
            添加供应商
          </button>
        </div>
      )}

      <StrategyPanel />

      <div className="px-4 py-3 text-[10.5px] text-mut">
        切换即时生效（Agent 已接管至本地网关，切换仅改路由）· API Key 存于本机系统钥匙串 · 请求不经 Kiwano 云端
      </div>
    </section>
  );
}
