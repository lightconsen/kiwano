// Home screen: Apps (local provider list, design/index.html #s-providers)
import { useCallback, useEffect, useMemo, useState, type FocusEvent, type ReactNode } from "react";

import { ArrowDown, ArrowUp, Pencil, Pin, Plus, RefreshCw, Trash2, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import {
  AGENTS,
  PLAN_TIER_LABELS,
  type AgentDetect,
  type AgentId,
  type AgentRoute,
  type CurrencyMeta,
  type PlanQuotaReport,
  type Provider,
  type StrategyBinding,
} from "../api/types";
import { AgentChip, BillTag, Dot, Logo, Ring, Sparkline } from "../components/bits";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { iconForEndpoint } from "@/components/icons/infer";
import StrategyPanel, { CopyRouteRow } from "../components/StrategyPanel";
import { fmtLatency, fmtMoney, fmtTokens } from "../lib/format";

// Agent filter segments — each renders the agent's brand logo (ported with
// the cc-switch icon set, see components/icons). Hover shows the full name.
const SEGMENTS: { id: AgentId | "all"; icon?: string }[] = [
  { id: "all" },
  { id: "claude", icon: "claudecode" },
  { id: "codex", icon: "openai" },
  { id: "gemini", icon: "gemini" },
  { id: "grokbuild", icon: "grok" },
  { id: "claude-desktop", icon: "claude" },
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
    manual entry. When already taken over but unbound, steer to binding an
    existing provider (bindSlot) or manual add. */
function AgentOnboarding({
  agent,
  installed,
  takenOver,
  busy,
  onTakeover,
  onAdd,
  bindSlot,
  copySlot,
}: {
  agent: AgentId;
  installed: boolean;
  takenOver: boolean;
  busy: boolean;
  onTakeover: () => void;
  onAdd: () => void;
  /** Extra first-candidate entry (bind an existing provider) for the taken-over branch */
  bindSlot?: ReactNode;
  /** Copy another agent's whole route — available right after Enable, before any binding exists */
  copySlot?: ReactNode;
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
          <div className="flex items-center gap-2">
            {bindSlot}
            <Button size="sm" className="h-7 gap-1 px-3 text-[12px] font-semibold" onClick={onAdd}>
              <Plus className="h-3.5 w-3.5" />
              Add provider
            </Button>
          </div>
          {copySlot}
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
  if (p.billing === "plan" && p.plan_limits) {
    const l = p.plan_limits;
    const parts = [
      l.five_hour != null ? `5h window ≤ ${l.five_hour}%` : null,
      l.weekly != null ? `weekly window ≤ ${l.weekly}%` : null,
    ]
      .filter(Boolean)
      .join(" · ");
    const tok = p.usage
      ? ` · in ${fmtTokens(p.usage.input_tokens)} · out ${fmtTokens(p.usage.output_tokens)}`
      : "";
    return `Plan · limit ${parts} · enforced against the provider's plan-quota utilization${tok}`;
  }
  const u = p.usage;
  if (!u) return "";
  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const unitLabel = u.quota.unit === "requests" || u.quota.unit === "wan_tokens" ? u.quota.unit : u.quota.unit;
    return `Plan · ${u.quota.used}/${u.quota.limit} ${unitLabel} this period (${pct}%) · resets ${u.quota.resets_at ?? ""} · in ${fmtTokens(u.input_tokens)} · out ${fmtTokens(u.output_tokens)}`;
  }
  if (p.billing === "unl") return "Local inference · no cost metering · works offline";
  if (u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    return `Pay as you go · ${fmtMoney(u.quota.used, u.quota.unit)} / ${fmtMoney(u.quota.limit, u.quota.unit)} this period (${pct}%) · in ${fmtTokens(u.input_tokens)} (cache ${fmtTokens(u.cache_read_tokens)}, billed at 1/10) · out ${fmtTokens(u.output_tokens)} · latency ${fmtLatency(u.latency_ms)}`;
  }
  return "Pay as you go · no limit set · 7-day usage trend";
}

/** Quota amount text: counted units render raw, currencies via fmtMoney */
function quotaAmountText(used: number, limit: number, unit: string): string {
  if (unit === "requests") return `${used}/${limit}`;
  if (unit === "wan_tokens") return `${used}/${limit}`;
  return `${fmtMoney(used, unit)} / ${fmtMoney(limit, unit)}`;
}

/** Percent-limit plan cell: the ring shows how close the live plan-quota
    utilization is to its configured ceiling (tightest window wins); the
    limit line reads from plan_limits. Independent of usage history, so it
    renders for brand-new providers too. */
function planPercentCell(
  p: Provider,
  plan: PlanQuotaReport | undefined,
  title: string,
): ReactNode {
  const l = p.plan_limits!;
  const util = (name: string) =>
    plan?.success ? plan.tiers.find((t) => t.name === name)?.utilization : undefined;
  const five = util("five_hour");
  const week = util("weekly_limit");
  const ratios = [
    l.five_hour != null && five != null ? (five / l.five_hour) * 100 : 0,
    l.weekly != null && week != null ? (week / l.weekly) * 100 : 0,
  ];
  const pct = Math.min(100, Math.round(Math.max(...ratios, 0)));
  const color = pct >= 95 ? "var(--red)" : pct >= 80 ? "var(--amber)" : "var(--kiwi)";
  const limitLine = [
    l.five_hour != null ? `5h ≤ ${l.five_hour}%` : null,
    l.weekly != null ? `wk ≤ ${l.weekly}%` : null,
  ]
    .filter(Boolean)
    .join(" · ");
  const planLine = plan?.success
    ? plan.tiers
        .map((t) => `${PLAN_TIER_LABELS[t.name] ?? t.name} ${Math.round(t.utilization)}%`)
        .join(" · ")
    : plan && !plan.success
      ? plan.error
      : null;
  const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
  return (
    <div className="flex w-[22%] items-center gap-2" title={title}>
      <Ring pct={pct} color={color} />
      <div className="min-w-0">
        <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
          <BillTag billing="plan" />
          <span>{limitLine}</span>
        </div>
        <div className="mt-0.5 text-[10.5px] text-mut">{p.plan_price ?? ""}</div>
        {/* Live plan quota: per-window utilization from the provider's own endpoint */}
        {planLine && (
          <div
            className="mt-0.5 truncate text-[10.5px]"
            style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
            title={plan && !plan.success ? (plan.error ?? "") : `${plan?.template}${plan?.cached ? " · cached" : ""}`}
          >
            {planLine}
          </div>
        )}
      </div>
    </div>
  );
}

function UsageCell({
  p,
  plan,
  pref,
}: {
  p: Provider;
  /** Plan-quota report for providers with a plan query (undefined = not fetched yet) */
  plan?: PlanQuotaReport;
  /** Preferred display currency (Settings) for cost-only payg cells */
  pref: string;
}) {
  const u = p.usage;

  // Percent-limit rows render even before the provider has any usage
  // history (usage summary still null): the ring/limit line need no totals.
  if (p.billing === "plan" && p.plan_limits) {
    return planPercentCell(p, plan, usageTitle(p));
  }

  if (!u) return <div className="w-[22%]" />;
  const title = usageTitle(p);

  if (p.billing === "plan" && u.quota) {
    const pct = Math.round((u.quota.used / u.quota.limit) * 100);
    const planLine = plan?.success
      ? plan.tiers
          .map((t) => `${PLAN_TIER_LABELS[t.name] ?? t.name} ${Math.round(t.utilization)}%`)
          .join(" · ")
      : plan && !plan.success
        ? plan.error
        : null;
    const maxUtil = plan?.success ? Math.max(...plan.tiers.map((t) => t.utilization), 0) : 0;
    return (
      <div className="flex w-[22%] items-center gap-2" title={title}>
        <Ring pct={pct} color={ringColor("plan", pct)} />
        <div className="min-w-0">
          <div className="flex items-center gap-1.5 font-mono text-[12.5px]">
            <BillTag billing="plan" />
            {u.quota.unit === "requests" || u.quota.unit === "wan_tokens" ? (
              <>
                {u.quota.used}/{u.quota.limit}{" "}
                <span className="font-normal text-mut">{u.quota.unit === "requests" ? "req" : "10k tok"}</span>
              </>
            ) : (
              <span>{quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit)}</span>
            )}
          </div>
          <div className="mt-0.5 text-[10.5px] text-mut">
            {[p.plan_price, u.quota.resets_at ? `resets ${u.quota.resets_at.slice(5)}` : null]
              .filter(Boolean)
              .join(" · ")}
          </div>
          {/* Live plan quota: per-window utilization from the provider's own endpoint */}
          {planLine && (
            <div
              className="mt-0.5 truncate text-[10.5px]"
              style={{ color: maxUtil >= 95 ? "var(--red)" : maxUtil >= 80 ? "var(--amber)" : "var(--mut)" }}
              title={plan && !plan.success ? (plan.error ?? "") : `${plan?.template}${plan?.cached ? " · cached" : ""}`}
            >
              {planLine}
            </div>
          )}
        </div>
      </div>
    );
  }

  // Percent-limit cell (extracted so it renders before usage history exists):
  // the ring tracks how close the live utilization is to its configured
  // ceiling (tightest window wins); legacy used/limit rows render above.
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
            {quotaAmountText(u.quota.used, u.quota.limit, u.quota.unit).split(" / ")[0]}{" "}
            <span className="font-normal text-mut">
              / {u.quota.unit === "requests" || u.quota.unit === "wan_tokens" ? u.quota.limit : fmtMoney(u.quota.limit, u.quota.unit)} limit
            </span>
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
        {fmtMoney(u.cost ?? 0, pref)} <span className="font-normal text-mut">· {u.requests} req</span>
      </div>
      {u.spark && (
        <span className="mt-1 block w-20">
          <Sparkline points={u.spark} />
        </span>
      )}
    </div>
  );
}

/** First grid cell: brand mark + name + endpoint subtitle. Shared by the All-tab
    management rows and the agent-tab strategy rows. The All tab badges the
    collapsed is_current; agent tabs pass inUse to badge membership in that
    agent's serving set only. */
function IdentityCell({ p, inUse }: { p: Provider; inUse?: boolean }) {
  // Brand mark inferred from the endpoint host; unknown hosts keep the letter avatar
  const brandIcon = iconForEndpoint(p.endpoint);
  const showInUse = inUse ?? p.is_current;
  return (
    <div className="flex min-w-0 w-[34%] items-center gap-2.5">
      {brandIcon ? (
        <ProviderLogo icon={brandIcon} name={p.name} size={32} />
      ) : (
        <Logo char={p.logo_char} color={p.logo_color} border={p.logo_border} />
      )}
      <div className="min-w-0">
        <div className="flex items-center gap-1.5">
          <span className="text-[13px] font-semibold">{p.name}</span>
          {showInUse && (
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
  );
}

/** Health cell: green dot + latency when healthy, muted note otherwise. */
function HealthCell({ p }: { p: Provider }) {
  return (
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
  );
}

function ProviderRow({
  p,
  plan,
  pref,
  onEdit,
  onDelete,
}: {
  p: Provider;
  plan?: PlanQuotaReport;
  pref: string;
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
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${p.is_current ? " current" : ""}`}>
      <IdentityCell p={p} />

      <div className="flex w-[22%] items-center">
        {p.agents.length === 0 ? (
          <span className="text-[11px] text-mut">Unbound</span>
        ) : (
          <>
            {/* Mini-logos, slightly overlapping (earlier agents on top); the
                opaque chip background keeps marks legible where they overlap */}
            {p.agents.map((a, i) => (
              <span
                key={a}
                className={`relative inline-flex rounded-[4px] bg-bg${i > 0 ? "-ml-0.5" : ""}`}
                style={{ zIndex: p.agents.length - i }}
              >
                <AgentChip meta={AGENTS.find((m) => m.id === a)!} size={14} />
              </span>
            ))}
            {p.agents_note && <span className="ml-1.5 text-[11px] text-mut">{p.agents_note}</span>}
          </>
        )}
      </div>

      <UsageCell p={p} plan={plan} pref={pref} />

      <HealthCell p={p} />

      {/* Actions stay out of the resting row: reveal on hover / keyboard focus,
          or while a delete confirmation is pending. Binding / unbinding happens
          in the agent tabs, so there is no Enable action here. */}
      <div
        className={`flex flex-1 items-center justify-end gap-1.5 transition-opacity${
          confirmDel ? "" : " opacity-0 group-hover:opacity-100 focus-within:opacity-100"
        }`}
      >
        <Button
          variant="ghost"
          size="sm"
          className="h-7 whitespace-nowrap border border-line px-1.5 text-[10.5px] text-mut"
          aria-label="Edit"
          title="Edit provider"
          onClick={() => onEdit(p)}
        >
          <Pencil className="h-3.5 w-3.5" />
        </Button>
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

// ── Agent-tab strategy rows ──
//
// Inside an agent tab the list is the agent's routing candidate queue: each row
// shows its role under the active strategy (primary / standby order / weight /
// time window) with that strategy's per-binding parameters editable inline.
// Provider management (edit / enable / delete) stays on the All tab.

/** Roundrobin weight editor: commits on blur or Enter, clamped to ≥ 1. */
function WeightEditor({
  agent,
  b,
  onChanged,
}: {
  agent: AgentId;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const [val, setVal] = useState(String(b.weight));
  useEffect(() => setVal(String(b.weight)), [b.weight]);
  const commit = () => {
    const n = Math.max(1, Math.round(Number(val) || 1));
    if (n === b.weight) {
      setVal(String(b.weight));
      return;
    }
    api.updateAgentBinding(agent, b.provider_id, { weight: n }).then(onChanged);
  };
  return (
    <span className="flex flex-none items-center gap-1" title="Sessions rotate across candidates proportionally to their weights">
      <span className="text-[10.5px] text-mut">weight</span>
      <Input
        type="number"
        min={1}
        className="h-6 w-12 rounded-md bg-transparent px-1.5 text-right font-mono text-[11px] dark:bg-transparent"
        value={val}
        onChange={(e) => setVal(e.target.value)}
        onBlur={commit}
        onKeyDown={(e) => e.key === "Enter" && commit()}
      />
    </span>
  );
}

/** Timewindow range editor: one control owning both bounds, committed as a
    pair (a half window would never match — the backend clears them together).
    Validation: each bound must be a valid HH:MM and the end must be later
    than the start; invalid fields are highlighted red and the pair is simply
    not committed. Plain "HH:MM" text fields instead of <input type="time">:
    the native control's click/stepper UI varies per WebView (Tauri's WKWebView
    offers nothing to click), while a masked text field behaves identically
    everywhere. */
function TimeRangeEditor({
  agent,
  b,
  onChanged,
}: {
  agent: AgentId;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const [s, setS] = useState(b.win_start ?? "");
  const [e, setE] = useState(b.win_end ?? "");
  const [left, setLeft] = useState(false); // focus has left the pair once
  useEffect(() => {
    setS(b.win_start ?? "");
    setE(b.win_end ?? "");
    setLeft(false);
  }, [b.win_start, b.win_end]);
  // Digits only; the colon is inserted automatically once past the hours
  const mask = (raw: string) => {
    const d = raw.replace(/[^0-9]/g, "").slice(0, 4);
    return d.length > 2 ? `${d.slice(0, 2)}:${d.slice(2)}` : d;
  };
  // Complete a masked entry: "9" -> "09:00", "0930" -> "09:30"; null = invalid
  const complete = (t: string): string | null => {
    const d = t.replace(/[^0-9]/g, "");
    if (!d) return "";
    if (d.length <= 2) {
      const h = Number(d);
      return h <= 23 ? `${d.padStart(2, "0")}:00` : null;
    }
    const h = Number(d.slice(0, 2));
    const m = Number(d.slice(2, 4));
    return h <= 23 && m <= 59 ? `${d.slice(0, 2).padStart(2, "0")}:${d.slice(2, 4).padStart(2, "0")}` : null;
  };

  const cs = complete(s);
  const ce = complete(e);
  // An incomplete pair only matters once editing is done: while the focus is
  // still inside, a single filled bound is just work in progress
  const half = left && !!cs !== !!ce;
  // Per-field red highlight: bad text flags its own bound, an end that is not
  // later than the start flags the end bound, an incomplete pair flags the
  // missing side
  const badS = cs === null || (half && !s);
  const badE = ce === null || (!!cs && !!ce && ce <= cs) || (half && !e);

  const commit = () => {
    setLeft(true);
    if (badS || badE) return; // invalid: stay red until fixed
    if (cs === (b.win_start ?? "") && ce === (b.win_end ?? "")) return;
    api.updateAgentBinding(agent, b.provider_id, { win_start: cs, win_end: ce }).then(onChanged);
  };
  // Commit only when the focus leaves the pair: moving between the two bounds
  // is mid-edit, and committing on each blur would wipe the first field before
  // the second one is filled.
  const onBlur = (ev: FocusEvent) => {
    if (ev.currentTarget.contains(ev.relatedTarget as Node | null)) return;
    commit();
  };
  const field =
    "h-5 w-[46px] border-0 bg-transparent p-0 text-center font-mono text-[11px] focus-visible:ring-0 dark:bg-transparent";
  return (
    <span
      className="flex flex-none items-center gap-1.5 rounded-md border border-line px-1.5 py-0.5"
      title="Local time window this candidate serves; type digits like 0930 — the end must be later than the start. Clear both to remove the window"
      onBlur={onBlur}
    >
      <Input
        inputMode="numeric"
        placeholder="--:--"
        aria-label="Window start"
        className={field}
        aria-invalid={badS}
        value={s}
        onChange={(ev) => setS(mask(ev.target.value))}
        onKeyDown={(ev) => ev.key === "Enter" && commit()}
      />
      <span className="text-[10.5px] text-mut">–</span>
      <Input
        inputMode="numeric"
        placeholder="--:--"
        aria-label="Window end"
        className={field}
        aria-invalid={badE}
        value={e}
        onChange={(ev) => setE(mask(ev.target.value))}
        onKeyDown={(ev) => ev.key === "Enter" && commit()}
      />
    </span>
  );
}

/** Role column: what this binding does under the active strategy, plus that
    strategy's per-binding parameters (weight / window) editable inline. */
function RoleCell({
  agent,
  route,
  b,
  idx,
  onChanged,
}: {
  agent: AgentId;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  onChanged: () => void;
}) {
  const badge = (label: string, title: string) => (
    <span className="rounded px-1 text-[9.5px] font-medium" style={{ background: "var(--kiwi-soft)", color: "var(--kiwi)" }} title={title}>
      {label}
    </span>
  );
  return (
    <div className="flex w-[22%] min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
      {route.strategy === "roundrobin" ? (
        <WeightEditor agent={agent} b={b} onChanged={onChanged} />
      ) : route.strategy === "timewindow" ? (
        <>
          {idx === 0 && !b.win_start && badge("Fallback", "Serves whenever no candidate's window matches")}
          {idx > 0 && !b.win_start && (
            <span className="text-[10.5px] text-mut" title="No window set — this candidate is never picked">
              No window
            </span>
          )}
          {/* The Fallback slot has no window by definition — no range editor
              there; give it one by pinning a windowed candidate to the top */}
          {(idx > 0 || !!b.win_start) && <TimeRangeEditor agent={agent} b={b} onChanged={onChanged} />}
        </>
      ) : idx === 0 ? (
        badge("Primary", "Current route of this agent")
      ) : (
        <span className="text-[11px] text-mut">Standby #{idx}</span>
      )}
    </div>
  );
}

/** One agent-tab row: the provider joined with its strategy binding. */
function BindingRow({
  p,
  route,
  b,
  idx,
  plan,
  pref,
  onMove,
  onChanged,
}: {
  p: Provider;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  plan?: PlanQuotaReport;
  pref: string;
  onMove: (idx: number, dir: -1 | 1) => void;
  onChanged: () => void;
}) {
  // Pin: promote this candidate to the queue head (the primary slot) — the
  // same reorder the arrows do, just straight to the top. Meaningless under
  // roundrobin (every candidate serves by weight), so it is hidden there.
  const makePrimary = () => {
    if (idx === 0) return;
    const ids = route.bindings.map((x) => x.provider_id);
    ids.unshift(ids.splice(idx, 1)[0]);
    api.reorderAgentBindings(route.agent, ids).then(onChanged);
  };
  // Unbind: removes only this agent's binding; other agents keep theirs.
  // Unbinding the last candidate drops the tab back to the onboarding state.
  const unbind = () => api.removeAgentBinding(route.agent, b.provider_id).then(onChanged);

  // Local "In use": serving this agent right now, not merely any agent
  const inUse = p.serving_agents.includes(route.agent);

  return (
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${inUse ? " current" : ""}`}>
      <IdentityCell p={p} inUse={inUse} />

      <RoleCell agent={route.agent} route={route} b={b} idx={idx} onChanged={onChanged} />

      <UsageCell p={p} plan={plan} pref={pref} />

      <HealthCell p={p} />

      {/* Candidate ordering / membership is the only action here — provider
          management (edit / enable / delete) stays on the All tab. Actions
          reveal on hover; the Pin slot renders on every row (invisible for the
          primary) so resting rows keep identical column alignment. */}
      <div className="flex flex-1 items-center justify-end gap-1.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
        {route.bindings.length > 1 && (
          <span className="flex flex-col">
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label="Move up"
              disabled={idx === 0}
              onClick={() => onMove(idx, -1)}
            >
              <ArrowUp className="h-3 w-3" />
            </button>
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label="Move down"
              disabled={idx === route.bindings.length - 1}
              onClick={() => onMove(idx, 1)}
            >
              <ArrowDown className="h-3 w-3" />
            </button>
          </span>
        )}
        {route.strategy !== "roundrobin" && (
          <Button
            variant="ghost"
            size="sm"
            className={`h-7 border border-line px-1.5 text-mut${idx === 0 ? " invisible" : ""}`}
            aria-label="Make primary"
            title="Make primary — move this candidate to the head of the queue"
            onClick={makePrimary}
          >
            <Pin className="h-3.5 w-3.5" />
          </Button>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 border border-line px-1.5 text-mut hover:text-ink"
          aria-label="Remove from route"
          title="Remove from this route (other agents keep their binding)"
          onClick={unbind}
        >
          <X className="h-3.5 w-3.5" />
        </Button>
      </div>
    </div>
  );
}

/** Select that offers providers not yet bound to the agent; picking one binds
    it to the route tail. Shared by the list-bottom add row and the onboarding
    empty state. When every provider is already in the route the trigger stays
    visible but disabled, so the row keeps its shape. */
function BindProviderSelect({
  providers,
  boundIds,
  onPick,
}: {
  providers: Provider[];
  boundIds: Set<string>;
  onPick: (providerId: string) => void;
}) {
  const available = providers.filter((p) => !boundIds.has(p.id));
  const [sel, setSel] = useState("");
  const full = available.length === 0;
  return (
    <Select
      value={sel}
      disabled={full}
      onValueChange={(pid) => {
        setSel("");
        if (pid) onPick(pid);
      }}
    >
      <SelectTrigger
        size="sm"
        aria-label="Bind provider to route"
        className="h-7 w-[210px] bg-transparent text-[12px] dark:bg-transparent disabled:opacity-50"
      >
        <SelectValue placeholder={full ? "All providers are already bound" : "Bind existing provider…"} />
      </SelectTrigger>
      <SelectContent className="min-w-[220px]">
        {available.map((p) => {
          const icon = iconForEndpoint(p.endpoint);
          return (
            <SelectItem key={p.id} value={p.id} className="text-[12px]">
              {icon && <ProviderLogo icon={icon} name={p.name} size={14} />}
              {p.name}
            </SelectItem>
          );
        })}
      </SelectContent>
    </Select>
  );
}

/** Bottom row of an agent tab with candidates: bind one more provider to this
    route (it joins the queue tail as a standby). */
function AddBindingRow({
  agent,
  providers,
  boundIds,
  onChanged,
}: {
  agent: AgentId;
  providers: Provider[];
  boundIds: Set<string>;
  onChanged: () => void;
}) {
  const available = providers.filter((p) => !boundIds.has(p.id));
  return (
    <div className="flex h-11 items-center gap-3 border-b border-line px-4">
      <BindProviderSelect
        providers={providers}
        boundIds={boundIds}
        onPick={(pid) => api.addAgentBinding(agent, pid).then(onChanged)}
      />
      <span className="flex-1 truncate text-[11px] text-mut">
        {available.length === 0
          ? "All providers are already in this route — add a new one from the header"
          : "Bind another provider to this route — it joins the queue tail as a standby"}
      </span>
    </div>
  );
}

export default function Providers({
  onAdd,
  onEdit,
  agentDetect = null,
  agentVersions = {},
  initialAgent = null,
}: {
  onAdd: () => void;
  onEdit: (p: Provider) => void;
  /** Phase 1 detection result; null = probe unavailable → show every agent */
  agentDetect?: AgentDetect[] | null;
  /** Phase 2 versions by agent id (arrive async, tooltip only) */
  agentVersions?: Partial<Record<AgentId, string>>;
  /** Deep-linked agent segment (#providers/<agent>, e.g. from Settings takeover rows) */
  initialAgent?: AgentId | null;
}) {
  const [providers, setProviders] = useState<Provider[] | null>(null);
  const [routes, setRoutes] = useState<AgentRoute[] | null>(null);
  const [seg, setSeg] = useState<AgentId | "all">(initialAgent ?? "all");
  const [takenOver, setTakenOver] = useState<Set<AgentId> | null>(null);
  const [enabling, setEnabling] = useState(false);
  // Plan-quota reports per provider, auto-refreshed on load
  const [planQuotas, setPlanQuotas] = useState<Record<string, PlanQuotaReport>>({});
  const [quotaBusy, setQuotaBusy] = useState(false);
  // Currency metadata (preferred display currency for cost cells)
  const [currencyMeta, setCurrencyMeta] = useState<CurrencyMeta | null>(null);

  // Segment switch that keeps the #providers/<agent> deep link truthful
  const pickSeg = (id: AgentId | "all") => {
    setSeg(id);
    window.location.hash = id === "all" ? "providers" : `providers/${id}`;
  };

  // Adopt a deep-linked segment arriving while mounted (App re-parses the hash)
  useEffect(() => {
    if (initialAgent && initialAgent !== seg) setSeg(initialAgent);
  }, [initialAgent, seg]);

  const refetch = useCallback(() => {
    api.listProviders().then(setProviders);
    // Per-agent strategy routes drive the agent-tab candidate rows
    api.getAgentRoutes().then(setRoutes).catch(() => setRoutes(null));
    // Takeover state drives the per-agent onboarding panel
    api
      .getSettings()
      .then((s) => setTakenOver(new Set(s.takeovers.filter((t) => t.enabled).map((t) => t.agent))))
      .catch(() => {});
  }, []);
  useEffect(refetch, [refetch]);

  // Currency metadata: preferred display currency + conversion rates
  useEffect(() => {
    api.getCurrencyMeta().then(setCurrencyMeta).catch(() => {});
  }, []);

  // Auto plan-quota refresh on load: one cached call per provider configured
  // with a plan query (the 5-min backend cache keeps this cheap)
  useEffect(() => {
    if (!providers) return;
    for (const p of providers) {
      if (!p.plan_query) continue;
      api
        .getPlanQuota(p.id)
        .then((r) => setPlanQuotas((m) => ({ ...m, [p.id]: r })))
        .catch(() => {});
    }
  }, [providers]);

  // Manual refresh: bypasses the backend's 5-minute quota cache
  const refreshQuotas = async () => {
    if (!providers || quotaBusy) return;
    setQuotaBusy(true);
    try {
      const results = await Promise.all(
        providers
          .filter((p) => p.plan_query)
          .map((p) => api.getPlanQuota(p.id, true).then((r) => [p.id, r] as const).catch(() => null)),
      );
      const fresh: Record<string, PlanQuotaReport> = {};
      for (const r of results) {
        if (r) fresh[r[0]] = r[1];
      }
      setPlanQuotas((m) => ({ ...m, ...fresh }));
    } finally {
      setQuotaBusy(false);
    }
  };

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
    if (seg !== "all" && !visibleSegments.some((s) => s.id === seg)) {
      setSeg("all");
      window.location.hash = "providers";
    }
  }, [visibleSegments, seg]);

  const onDelete = async (p: Provider) => {
    await api.deleteProvider(p.id);
    refetch();
  };

  // Swap two candidates in the current agent's strategy queue (priority order)
  const onMoveBinding = async (idx: number, dir: -1 | 1) => {
    if (seg === "all") return;
    const route = routes?.find((r) => r.agent === seg);
    if (!route) return;
    const ids = route.bindings.map((b) => b.provider_id);
    const j = idx + dir;
    if (j < 0 || j >= ids.length) return;
    [ids[idx], ids[j]] = [ids[j], ids[idx]];
    await api.reorderAgentBindings(seg, ids);
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
  // Inside an agent tab with bindings the list IS the strategy candidate queue
  const route = seg === "all" ? null : (routes?.find((r) => r.agent === seg) ?? null);
  const byId = new Map(providers.map((p) => [p.id, p]));
  // Disabling a takeover keeps the stored route (re-enabling restores it), but
  // the agent config no longer points at the gateway — the route is dormant.
  // An agent tab for an agent that is not taken over shows the (re-)takeover
  // onboarding instead of the dormant binding rows.
  const notTakenOver = seg !== "all" && !(takenOver?.has(seg) ?? false);

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
                onClick={() => pickSeg(s.id)}
              >
                {s.icon ? (
                  <ProviderLogo icon={s.icon} name={label} size={15} />
                ) : (
                  <span className="text-[12px] text-mut">{label}</span>
                )}
              </button>
            );
          })}
        </div>
        <span className="ml-1.5 text-[11.5px] text-mut">
          {providers.length} providers · {agentsBound} agents bound
        </span>
        {providers.some((p) => p.plan_query) && (
          <Button
            variant="ghost"
            size="sm"
            className="ml-auto h-7 w-7 px-0 text-mut"
            aria-label="Refresh plan quotas"
            title="Refresh plan quotas (bypasses the 5-min cache)"
            disabled={quotaBusy}
            onClick={refreshQuotas}
          >
            <RefreshCw className={`h-3.5 w-3.5${quotaBusy ? " animate-spin" : ""}`} />
          </Button>
        )}
        <Button
          size="sm"
          className={`h-7 gap-1 px-2.5 text-[12px] font-semibold${providers.some((p) => p.plan_query) ? "" : " ml-auto"}`}
          onClick={onAdd}
        >
          <Plus className="h-3.5 w-3.5" />
          Add provider
        </Button>
      </div>

      {notTakenOver ? (
        <AgentOnboarding
          agent={seg}
          installed={
            agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : true
          }
          takenOver={false}
          busy={enabling}
          onTakeover={() => onTakeover(seg)}
          onAdd={onAdd}
        />
      ) : (
        <>
          <div className="flex h-7 items-center border-b border-line px-4 text-[10.5px] text-mut" style={{ background: "var(--surface)" }}>
            <span className="w-[34%]">Provider</span>
            <span className="w-[22%]">{route ? "Role in strategy" : "Bound agents"}</span>
            <span className="w-[22%]">Usage / quota</span>
            <span className="w-[14%]">Status</span>
            <span className="flex-1 text-right">{route ? "Priority" : "Actions"}</span>
          </div>

          {route
            ? route.bindings.map((b, i) => {
                const p = byId.get(b.provider_id);
                // Skip a binding whose provider row vanished (deleted mid-session)
                return p ? (
                  <BindingRow
                    key={b.provider_id}
                    p={p}
                    route={route}
                    b={b}
                    idx={i}
                    plan={planQuotas[p.id]}
                    pref={currencyMeta?.preferred ?? "CNY"}
                    onMove={onMoveBinding}
                    onChanged={refetch}
                  />
                ) : null;
              })
            : filtered.map((p) => (
                <ProviderRow
                  key={p.id}
                  p={p}
                  plan={planQuotas[p.id]}
                  pref={currencyMeta?.preferred ?? "CNY"}
                  onEdit={onEdit}
                  onDelete={onDelete}
                />
              ))}

          {/* Bind-one-more entry under the candidate queue of an agent tab */}
          {route && (
            <AddBindingRow
              agent={route.agent}
              providers={providers}
              boundIds={new Set(route.bindings.map((b) => b.provider_id))}
              onChanged={refetch}
            />
          )}

          {filtered.length === 0 && seg !== "all" && (
            <AgentOnboarding
              agent={seg}
              installed={
                agentDetect ? (agentDetect.find((d) => d.agent === seg)?.installed ?? false) : true
              }
              takenOver={true}
              busy={enabling}
              onTakeover={() => onTakeover(seg)}
              onAdd={onAdd}
              bindSlot={
                <BindProviderSelect
                  providers={providers}
                  boundIds={new Set(providers.filter((p) => p.agents.includes(seg)).map((p) => p.id))}
                  onPick={(pid) => api.addAgentBinding(seg, pid).then(refetch)}
                />
              }
              copySlot={
                <CopyRouteRow agent={seg} routes={routes ?? []} onChanged={refetch} />
              }
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
        </>
      )}

      {/* Strategy config lives in the agent's own tab, only once it is taken over.
          Routes come from here (single fetch): a route created while the panel is
          mounted (first bind / copy) must show up without a tab switch. */}
      {seg !== "all" && (takenOver?.has(seg) ?? false) && (
        <StrategyPanel agent={seg} routes={routes} onChanged={refetch} />
      )}

      {/* Single closing note. In a taken-over agent tab with a route it also
          carries the strategy context (the StrategyPanel select row has no
          header of its own). */}
      <div className="mt-auto truncate px-4 py-3 text-[10.5px] text-mut">
        {notTakenOver
          ? "Kiwano does not route this agent yet — enable the takeover above; the stored route is kept and applies again as-is · API keys stay in the system keychain · requests never touch the Kiwano cloud"
          : seg !== "all" &&
              (takenOver?.has(seg) ?? false) &&
              (routes?.some((r) => r.agent === seg) ?? false)
            ? "Agent routing strategy · rows above are the candidates in priority order (primary first) · switching applies instantly · API keys stay in the system keychain · requests never touch the Kiwano cloud"
            : "Switching applies instantly (the agent is taken over by the local gateway; switching only changes routing) · API keys stay in the system keychain · requests never touch the Kiwano cloud"}
      </div>
    </section>
  );
}
