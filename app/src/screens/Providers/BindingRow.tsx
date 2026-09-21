// An agent tab's candidate queue: one row per binding, the role column that
// reads it under the active strategy, and the row that binds one more.
import { useEffect, useState, type FocusEvent } from "react";

import { ArrowDown, ArrowUp, Pin, X } from "lucide-react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { iconForEndpoint } from "@/components/icons/infer";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../../api/client";
import { useT } from "../../i18n";
import {
  type AgentRef,
  type AgentRoute,
  type PlanQuotaReport,
  type Provider,
  type StrategyBinding,
} from "../../api/types";
import { HealthCell, IdentityCell, TestLatencyButton, rowState } from "./rows";
import { CacheCell, UsageCell } from "./usage";

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
  agent: AgentRef;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const t = useT();
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
    <span className="flex flex-none items-center gap-1" title={t("providers.weightTitle")}>
      <span className="text-[10.5px] text-mut">{t("providers.weight")}</span>
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
    Validation: each bound must be a valid HH:MM; invalid fields are
    highlighted red and the pair is simply not committed. An end earlier than
    the start is not an error — the engine reads it as a window crossing
    midnight (`in_window`, start > end). Plain "HH:MM" text fields instead of
    <input type="time">:
    the native control's click/stepper UI varies per WebView (Tauri's WKWebView
    offers nothing to click), while a masked text field behaves identically
    everywhere. */
function TimeRangeEditor({
  agent,
  b,
  onChanged,
}: {
  agent: AgentRef;
  b: StrategyBinding;
  onChanged: () => void;
}) {
  const t = useT();
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
  // Per-field red highlight: bad text flags its own bound, an incomplete pair
  // flags the missing side. `ce <= cs` stays valid on purpose: the engine
  // matches an inverted pair as an overnight window
  const badS = cs === null || (half && !s);
  const badE = ce === null || (half && !e);

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
      title={t("providers.windowTitle")}
      onBlur={onBlur}
    >
      <Input
        inputMode="numeric"
        placeholder="--:--"
        aria-label={t("providers.windowStart")}
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
        aria-label={t("providers.windowEnd")}
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
  agent: AgentRef;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  onChanged: () => void;
}) {
  const t = useT();
  const badge = (label: string, title: string) => (
    <span className="rounded px-1 text-[9.5px] font-medium" style={{ background: "var(--kiwi-soft)", color: "var(--kiwi)" }} title={title}>
      {label}
    </span>
  );
  return (
    <div className="flex w-[16%] min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1">
      {route.strategy === "roundrobin" ? (
        <WeightEditor agent={agent} b={b} onChanged={onChanged} />
      ) : route.strategy === "timewindow" ? (
        <>
          {idx === 0 &&
            !b.win_start &&
            badge(t("providers.fallback"), t("providers.fallbackTitle"))}
          {idx > 0 && !b.win_start && (
            <span className="text-[10.5px] text-mut" title={t("providers.noWindowTitle")}>
              {t("providers.noWindow")}
            </span>
          )}
          {/* The Fallback slot has no window by definition — no range editor
              there; give it one by pinning a windowed candidate to the top */}
          {(idx > 0 || !!b.win_start) && <TimeRangeEditor agent={agent} b={b} onChanged={onChanged} />}
        </>
      ) : idx === 0 ? (
        badge(t("providers.primary"), t("providers.primaryTitle"))
      ) : (
        <span className="text-[11px] text-mut">{t("providers.standby", { index: idx })}</span>
      )}
    </div>
  );
}

/** One agent-tab row: the provider joined with its strategy binding. */
export function BindingRow({
  p,
  route,
  b,
  idx,
  plan,
  blocked,
  onMove,
  onChanged,
}: {
  p: Provider;
  route: AgentRoute;
  b: StrategyBinding;
  idx: number;
  plan?: PlanQuotaReport;
  blocked?: string;
  onMove: (idx: number, dir: -1 | 1) => void;
  onChanged: () => void;
}) {
  const t = useT();
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

  // Local "In use": serving this agent right now, not merely any agent. A
  // quota-over first backup reads as fallback instead — first in line, which
  // only the gateway's breakers can promote to serving.
  const inUse = p.serving_agents.includes(route.agent);
  const isFallback = p.fallback_agents?.includes(route.agent) === true;
  const state = rowState(p, inUse, blocked);

  return (
    <div className={`row group flex h-[58px] items-center border-b border-line px-4${state ? ` ${state}` : ""}`}>
      <IdentityCell p={p} inUse={inUse} isFallback={isFallback} />

      <RoleCell agent={route.agent} route={route} b={b} idx={idx} onChanged={onChanged} />

      <UsageCell p={p} plan={plan} blocked={blocked} />

      <CacheCell u={p.usage} />

      <HealthCell p={p} blocked={blocked} />

      {/* Candidate ordering / membership, plus the provider's own latency test:
          provider *management* (edit, park, delete) stays on the All tab. The
          test belongs here because it measures the provider rather than this
          binding — it records the same verdict the All row reads, so the two tabs
          cannot end up disagreeing about what it found, and parking the provider
          (which is a global act, and would drop it from every agent's route) is
          deliberately not offered from a view scoped to one agent. Actions reveal
          on hover.

          The Pin slot renders on every row — invisible on the primary, which is
          already primary — so the rows of one route keep identical column
          alignment. It leads the group for that reason: the group is packed to
          the right, so an empty slot at its start costs nothing, while the same
          slot further in sits *between* two visible buttons and reads as a gap
          one button wide. */}
      {/* Same min-width as the header's trailing cell and the All tab's action
          group: the three shrink together, which is what keeps every column
          header above its own values. */}
      <div className="flex min-w-[140px] flex-1 items-center justify-end gap-1.5 opacity-0 transition-opacity group-focus-within:opacity-100 group-hover:opacity-100">
        {route.strategy !== "roundrobin" && (
          <Button
            variant="ghost"
            size="sm"
            className={`h-7 border border-line px-1.5 text-mut${idx === 0 ? " invisible" : ""}`}
            aria-label={t("providers.makePrimary")}
            title={t("providers.makePrimaryTitle")}
            onClick={makePrimary}
          >
            <Pin className="h-3.5 w-3.5" />
          </Button>
        )}
        <TestLatencyButton provider={p} onTested={onChanged} />
        {route.bindings.length > 1 && (
          <span className="flex flex-col">
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label={t("providers.moveUp")}
              disabled={idx === 0}
              onClick={() => onMove(idx, -1)}
            >
              <ArrowUp className="h-3 w-3" />
            </button>
            <button
              className="text-mut hover:text-ink disabled:opacity-30"
              aria-label={t("providers.moveDown")}
              disabled={idx === route.bindings.length - 1}
              onClick={() => onMove(idx, 1)}
            >
              <ArrowDown className="h-3 w-3" />
            </button>
          </span>
        )}
        <Button
          variant="ghost"
          size="sm"
          className="h-7 border border-line px-1.5 text-mut hover:text-ink"
          aria-label={t("providers.removeFromRouteAria")}
          title={t("providers.removeFromRoute")}
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
export function BindProviderSelect({
  providers,
  boundIds,
  onPick,
}: {
  providers: Provider[];
  boundIds: Set<string>;
  onPick: (providerId: string) => void;
}) {
  const t = useT();
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
        aria-label={t("providers.bindAria")}
        className="h-7 w-[210px] bg-transparent text-[12px] dark:bg-transparent disabled:opacity-50"
      >
        <SelectValue
          placeholder={full ? t("providers.allBound") : t("providers.bindExisting")}
        />
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
export function AddBindingRow({
  agent,
  providers,
  boundIds,
  onChanged,
}: {
  agent: AgentRef;
  providers: Provider[];
  boundIds: Set<string>;
  onChanged: () => void;
}) {
  const t = useT();
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
          ? t("providers.allInRoute")
          : t("providers.bindAnother")}
      </span>
    </div>
  );
}
