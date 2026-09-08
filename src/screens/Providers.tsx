// Home screen: Apps (local provider list, design/index.html #s-providers)
import { useCallback, useEffect, useMemo, useState } from "react";

import { Plus, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { AGENTS, type AgentDetect, type AgentId, type Provider } from "../api/types";
import { AgentChip, BillTag, Dot, Logo, Ring, Sparkline } from "../components/bits";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import StrategyPanel from "../components/StrategyPanel";
import { fmtCny, fmtLatency, fmtTokens } from "../lib/format";

// Agent filter segments — each renders the agent's brand logo (ported with
// the cc-switch icon set, see components/icons). Hover shows the full name.
const SEGMENTS: { id: AgentId | "all"; icon?: string; char?: string; color?: string }[] = [
  { id: "all" },
  { id: "claude", icon: "claude" },
  { id: "codex", icon: "openai" },
  { id: "gemini", icon: "gemini" },
  { id: "grokbuild", icon: "grok" },
  // Claude Desktop shares the Claude mark — render the design-system "D"
  // chip instead, so the two Claude agents stay distinguishable.
  { id: "claude-desktop", char: "D", color: "#B45E51" },
  { id: "opencode", icon: "opencode" },
  { id: "openclaw", icon: "openclaw" },
  { id: "hermes", icon: "hermes" },
  { id: "pi", icon: "pi" },
];

function segmentLabel(id: AgentId | "all"): string {
  if (id === "all") return "All";
  return AGENTS.find((a) => a.id === id)?.label ?? id;
}

const SEGMENT_ICON: Partial<Record<AgentId, string>> = Object.fromEntries(
  SEGMENTS.filter((s) => s.icon).map((s) => [s.id, s.icon!]),
);

/** First-touch state for an agent tab with no bound providers: one click to
    back up + take over (importing the provider the agent already uses), or
    manual entry. When already taken over but unbound, steer to manual add. */
function AgentOnboarding({
  agent,
  installed,
  takenOver,
  busy,
  onTakeover,
  onAdd,
}: {
  agent: AgentId;
  installed: boolean;
  takenOver: boolean;
  busy: boolean;
  onTakeover: () => void;
  onAdd: () => void;
}) {
  const meta = AGENTS.find((m) => m.id === agent)!;
  return (
    <div className="flex flex-col items-center gap-3 px-4 py-10 text-center">
      <ProviderLogo icon={SEGMENT_ICON[agent]} char={meta.chip_char} name={meta.label} size={36} />
      {takenOver ? (
        <>
          <div className="text-[13px] font-semibold">{meta.label} is taken over</div>
          <div className="max-w-[430px] text-[12px] text-mut">
            The local gateway routes this agent's requests, but no provider is bound yet — add one so
            requests have somewhere to go.
          </div>
          <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
            <Plus className="h-3.5 w-3.5" />
            Add provider
          </Button>
        </>
      ) : (
        <>
          <div className="text-[13px] font-semibold">Start managing {meta.label} with Kiwano</div>
          <div className="max-w-[460px] text-[12px] text-mut">
            Kiwano backs up the current config (one-click restore later), imports the provider{" "}
            {meta.label} already uses (shared with other agents), and routes it through the local
            gateway — same upstream, instant switching afterwards.
          </div>
          <div className="flex gap-2">
            <Button
              size="sm"
              className="h-7 px-3 text-[12px] font-semibold"
              disabled={busy}
              onClick={onTakeover}
            >
              {busy ? "Enabling…" : "Enable Kiwano"}
            </Button>
            <Button variant="ghost" size="sm" className="h-7 px-3 text-[12px]" onClick={onAdd}>
              Add provider first
            </Button>
          </div>
          {installed && (
            <div className="max-w-[470px] text-[11px] text-mut">
              Signed in with an official subscription (Claude / Gemini login, Codex ChatGPT)? Official
              OAuth can't be proxied yet — add a provider manually first.
            </div>
          )}
        </>
      )}
    </div>
  );
}

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
    return `Plan · ${u.quota.used}/${u.quota.limit} requests this period (${pct}%) · resets ${u.quota.resets_at ?? ""} · in ${fmtTokens(u.input_tokens)} · out ${fmtTokens(u.output_tokens)}`;
  }
  if (p.billing === "unl") return "Local inference · no cost metering · works offline";
  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return `Pay as you go · ${fmtCny(u.cost ?? 0)} / ${fmtCny(u.quota.limit)} this period (${pct}%) · in ${fmtTokens(u.input_tokens)} (cache ${fmtTokens(u.cache_read_tokens)}, billed at 1/10) · out ${fmtTokens(u.output_tokens)} · latency ${fmtLatency(u.latency_ms)}`;
  }
  return "Pay as you go · no limit set · 7-day usage trend";
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
            {u.quota.used}/{u.quota.limit} <span className="font-normal text-mut">req</span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {[p.plan_price, u.quota.resets_at ? `resets ${u.quota.resets_at.slice(5)}` : null]
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
          {u.requests} <span className="font-normal text-mut">req · {fmtTokens(u.input_tokens + u.output_tokens)} tok</span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">
          in {fmtTokens(u.input_tokens)} · out {fmtTokens(u.output_tokens)}
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
            {fmtCny(u.cost ?? 0)} <span className="font-normal text-mut">/ {fmtCny(u.quota.limit)} limit</span>
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {fmtTokens(u.input_tokens + u.output_tokens)} tokens · latency {fmtLatency(u.latency_ms)}
          </div>
        </div>
      </div>
    );
  }

  return (
    <div className="w-[22%]" title={title}>
      <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
        <BillTag billing="payg" />
        {fmtCny(u.cost ?? 0)} <span className="font-normal text-mut">· {u.requests} req</span>
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
                In use
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
          <span className="text-[11px] text-mut">Unbound</span>
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
            <Dot state="ok" />Healthy {p.health.latency_ms}ms
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
            Edit
          </Button>
        ) : (
          <Button
            variant="outline"
            size="sm"
            className="h-7 whitespace-nowrap border-kiwi-dim bg-kiwi-soft font-semibold text-kiwi text-[11.5px] dark:border-kiwi-dim dark:bg-kiwi-soft hover:bg-kiwi-soft dark:hover:bg-kiwi-soft"
            onClick={() => onEnable(p.id)}
          >
            Enable
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className={`h-7 whitespace-nowrap border border-line px-1.5 text-[10.5px]${confirmDel ? "" : " text-mut"}`}
          style={confirmDel ? { color: "var(--red)", borderColor: "var(--red)" } : undefined}
          aria-label="Delete"
          title={confirmDel ? "Click again to confirm" : "Delete provider"}
          onClick={() => (confirmDel ? onDelete(p) : setConfirmDel(true))}
        >
          {confirmDel ? "Confirm" : <Trash2 className="h-3.5 w-3.5" />}
        </Button>
      </div>
    </div>
  );
}

export default function Providers({
  onAdd,
  onEdit,
  agentDetect = null,
  agentVersions = {},
}: {
  onAdd: () => void;
  onEdit: (p: Provider) => void;
  /** Phase 1 detection result; null = probe unavailable → show every agent */
  agentDetect?: AgentDetect[] | null;
  /** Phase 2 versions by agent id (arrive async, tooltip only) */
  agentVersions?: Partial<Record<AgentId, string>>;
}) {
  const [providers, setProviders] = useState<Provider[] | null>(null);
  const [seg, setSeg] = useState<AgentId | "all">("all");
  const [takenOver, setTakenOver] = useState<Set<AgentId> | null>(null);
  const [enabling, setEnabling] = useState(false);

  const refetch = useCallback(() => {
    api.listProviders().then(setProviders);
    // Takeover state drives the per-agent onboarding panel
    api
      .getSettings()
      .then((s) => setTakenOver(new Set(s.takeovers.filter((t) => t.enabled).map((t) => t.agent))))
      .catch(() => {});
  }, []);
  useEffect(refetch, [refetch]);

  // Only agents that phase 1 detected as installed get a segment; a failed
  // probe (null) keeps every agent visible.
  const visibleSegments = useMemo(
    () =>
      SEGMENTS.filter(
        (s) => s.id === "all" || !agentDetect || agentDetect.find((d) => d.agent === s.id)?.installed,
      ),
    [agentDetect],
  );

  // Drop a hidden segment if the detection result changed under us.
  useEffect(() => {
    if (seg !== "all" && !visibleSegments.some((s) => s.id === seg)) setSeg("all");
  }, [visibleSegments, seg]);

  const onEnable = async (id: string) => {
    await api.enableProvider(id);
    refetch();
  };

  const onDelete = async (p: Provider) => {
    await api.deleteProvider(p.id);
    refetch();
  };

  const onTakeover = async (agent: AgentId) => {
    setEnabling(true);
    try {
      await api.setTakeover(agent, true);
    } finally {
      setEnabling(false);
      refetch();
    }
  };

  if (!providers) return <div className="p-8 text-center text-[12px] text-mut">Loading…</div>;

  const filtered = providers.filter((p) => seg === "all" || p.agents.includes(seg));
  const agentsBound = new Set(providers.flatMap((p) => p.agents)).size;

  return (
    <section className="flex min-h-full flex-col">
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <div className="flex max-w-full overflow-x-auto rounded-lg border border-line text-[12px]">
          {visibleSegments.map((s, i) => {
            const label = segmentLabel(s.id);
            const ver = s.id === "all" ? undefined : agentVersions[s.id];
            return (
              <button
                key={s.id}
                title={s.id === "all" ? label : ver ? `${label} · v${ver}` : label}
                aria-label={label}
                className={`seg flex h-7 shrink-0 items-center justify-center px-2.5${i > 0 ? " border-l border-line" : ""}${seg === s.id ? " active" : ""}`}
                onClick={() => setSeg(s.id)}
              >
                {s.id === "all" ? (
                  <span className="text-[12px] text-mut">{label}</span>
                ) : (
                  <ProviderLogo icon={s.icon} char={s.char} color={s.color} name={label} size={15} />
                )}
              </button>
            );
          })}
        </div>
        <span className="ml-1.5 text-[11.5px] text-mut">
          {providers.length} providers · {agentsBound} agents bound
        </span>
        <Button size="sm" className="ml-auto h-7 gap-1 px-2.5 text-[12px] font-semibold" onClick={onAdd}>
          <Plus className="h-3.5 w-3.5" />
          Add provider
        </Button>
      </div>

      <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
        <span className="w-[34%]">Provider</span>
        <span className="w-[22%]">Bound agents</span>
        <span className="w-[22%]">Usage / quota</span>
        <span className="w-[14%]">Status</span>
        <span className="flex-1 text-right">Actions</span>
      </div>

      {filtered.map((p) => (
        <ProviderRow key={p.id} p={p} onEnable={onEnable} onEdit={onEdit} onDelete={onDelete} />
      ))}

      {filtered.length === 0 && seg !== "all" && (
        <AgentOnboarding
          agent={seg}
          installed={
            agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : true
          }
          takenOver={takenOver?.has(seg) ?? false}
          busy={enabling}
          onTakeover={() => onTakeover(seg)}
          onAdd={onAdd}
        />
      )}

      {filtered.length === 0 && seg === "all" && (
        <div className="px-4 py-8 text-center text-[12px] text-mut">
          No providers yet —{" "}
          <button className="font-semibold" style={{ color: "var(--kiwi)" }} onClick={onAdd}>
            Add provider
          </button>
        </div>
      )}

      <StrategyPanel />

      <div className="mt-auto px-4 py-3 text-[10.5px] text-mut">
        Switching applies instantly (the agent is taken over by the local gateway; switching only changes routing) · API keys stay in the system keychain · requests never touch the Kiwano cloud
      </div>
    </section>
  );
}
