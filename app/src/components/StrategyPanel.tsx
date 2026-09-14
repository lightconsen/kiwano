// Agent routing strategy panel (tech.md §4.7): strategy type + candidate ordering.
// Rendered inside a single agent's tab: shows that agent's route only (and
// nothing when it has no bindings yet); changes take effect immediately via
// the gateway's /reload.
import { useState } from "react";
import { SquarePen } from "lucide-react";

import { Dialog, DialogContent, DialogHeader, DialogTitle } from "@/components/ui/dialog";
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
  type AgentLimit,
  type AgentRef,
  type AgentRoute,
  type StrategyKind,
} from "../api/types";
import { agentMeta } from "../lib/agents";
import { useT, type KeyPath, type Messages, type Translate } from "../i18n";
import StrategyIcon from "./StrategyIcon";
import { Button } from "@/components/ui/button";

// Labels and hints are translation *keys*, resolved at render: `t` is a hook,
// so a module-level table cannot hold the strings themselves. The ids are the
// values sent to the backend and are never translated.
const STRATEGIES: { id: StrategyKind; label: KeyPath<Messages>; hint: KeyPath<Messages> }[] = [
  { id: "single", label: "strategy.single", hint: "strategy.singleHint" },
  { id: "failover", label: "strategy.failover", hint: "strategy.failoverHint" },
  { id: "roundrobin", label: "strategy.roundrobin", hint: "strategy.roundrobinHint" },
  { id: "timewindow", label: "strategy.timewindow", hint: "strategy.timewindowHint" },
  // "the primary" drifts: whoever holds the first slot right now, which a
  // billing limit or a reorder can change. The number is the agent's, not the
  // provider's — the old wording ("once the primary exceeds its daily
  // threshold") read as an allowance set on one provider.
  { id: "quota", label: "strategy.quota", hint: "strategy.quotaHint" },
];

/** The strategy's display label, lowercased so it reads as part of the
    mid-sentence confirmation ("failover, 2 candidates"). Lowercasing is a
    no-op on Chinese, so the same call serves both locales. */
function strategyLabel(t: Translate, id: StrategyKind): string {
  const s = STRATEGIES.find((x) => x.id === id);
  return (s ? t(s.label) : id).toLowerCase();
}

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
  const t = useT();
  const meta = agentMeta(route.agent);
  const strategy = STRATEGIES.find((s) => s.id === route.strategy);
  const hint = strategy ? t(strategy.hint) : "";
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
            aria-label={t("strategy.ariaFor", { agent: meta.label })}
            className="h-7 min-w-[92px] bg-transparent px-1.5 text-[11.5px] dark:bg-transparent"
          >
            {/* Static children replace the selected-item label: icon + name */}
            <SelectValue>
              <StrategyIcon id={route.strategy} className="size-3.5" />
              <span>{strategy && t(strategy.label)}</span>
            </SelectValue>
          </SelectTrigger>
          {/* wider than the trigger: the longest label ("Weighted round-robin") must fit */}
          <SelectContent className="min-w-[230px]">
            {STRATEGIES.map((s) => (
              <SelectItem key={s.id} value={s.id}>
                <StrategyIcon id={s.id} className="size-3.5" />
                {t(s.label)}
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
                {/* The value is the bare unit ("tokens"); the menu adds the
                    "/day" the reader needs. */}
                <SelectValue>
                  {(v) =>
                    v === "tokens" ? t("strategy.unitTokensDay") : t("strategy.unitRequestsDay")
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="requests">{t("strategy.unitRequestsDay")}</SelectItem>
                <SelectItem value="tokens">{t("strategy.unitTokensDay")}</SelectItem>
              </SelectContent>
            </Select>
          </span>
        )}
      </div>
    </div>
  );
}

/** How a stored ceiling reads: "50 CNY / month", "100 requests / day".
    A limit with no period measures everything it has ever done, which is worth
    saying out loud rather than leaving as a blank. */
function formatLimit(t: Translate, limit: AgentLimit): string {
  const unit =
    limit.limit_unit === "wan_tokens"
      ? t("strategy.limitUnitWanTokens")
      : limit.limit_unit && limit.limit_unit.length === 3
        ? limit.limit_unit
        : t("strategy.limitUnitRequests");
  const period = limit.reset_period ? LIMIT_PERIODS[limit.reset_period] : undefined;
  return [
    String(limit.period_limit),
    unit,
    period ? t(period) : t("strategy.limitPeriodAll"),
  ].join(" ");
}

/** The reset periods a limit can take, and what each reads as. The ids are what
    the backend stores; `all` is the absence of one. */
const LIMIT_PERIODS: Record<string, KeyPath<Messages>> = {
  day: "strategy.limitPeriodDay",
  weekly: "strategy.limitPeriodWeek",
  monthly: "strategy.limitPeriodMonth",
  yearly: "strategy.limitPeriodYear",
};

/** Where a limit is edited. A dialog rather than an inline field because the
    readout is one line and the explanation is not: "no price, no count" is the
    kind of thing that has to be said in a sentence. */
function LimitDialog({
  agent,
  limit,
  open,
  onClose,
  onChanged,
  currency,
}: {
  agent: AgentRef;
  limit: AgentLimit | null;
  open: boolean;
  onClose: () => void;
  onChanged?: () => void;
  /** The currency a new money limit is written in. */
  currency: string;
}) {
  const t = useT();
  const meta = agentMeta(agent);
  const [unit, setUnit] = useState(limit?.limit_unit ?? "requests");
  const [value, setValue] = useState(limit ? String(limit.period_limit) : "");
  const [period, setPeriod] = useState(limit?.reset_period ?? "day");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      const amount = Number(value);
      await api.setAgentLimit(
        agent,
        // Blank, zero or unparseable is "no limit" rather than a ceiling of
        // nothing — the same reading the gateway applies to a stored zero.
        Number.isFinite(amount) && amount > 0
          ? {
              period_limit: amount,
              limit_unit: unit === "requests" ? null : unit,
              reset_period: period === "all" ? null : period,
            }
          : null,
      );
      onChanged?.();
      onClose();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <Dialog open={open} onOpenChange={(o) => !o && onClose()}>
      <DialogContent className="max-w-[460px]">
        <DialogHeader>
          <DialogTitle className="text-[13px]">
            {t("strategy.limitTitle", { agent: meta.label })}
          </DialogTitle>
        </DialogHeader>
        <div className="flex flex-col gap-3 px-5 pb-4 pt-1">
          <div className="flex items-center gap-2">
            <Input
              type="number"
              min={0}
              value={value}
              placeholder={t("strategy.limitPlaceholder")}
              aria-label={t("strategy.limitAmountAria")}
              className="h-8 w-28 font-mono text-[12px]"
              onChange={(e) => setValue(e.target.value)}
            />
            <Select value={unit} onValueChange={(v) => setUnit(v ?? "requests")}>
              <SelectTrigger size="sm" className="h-8 w-[180px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue>
                  {(v) =>
                    v === "wan_tokens"
                      ? t("strategy.limitUnitWanTokens")
                      : v === "requests"
                        ? t("strategy.limitUnitRequests")
                        : String(v ?? "")
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="requests">{t("strategy.limitUnitRequests")}</SelectItem>
                <SelectItem value="wan_tokens">{t("strategy.limitUnitWanTokens")}</SelectItem>
                {/* A money ceiling is written in the display currency, because an
                    agent's traffic spans providers that bill in their own. */}
                <SelectItem value={currency}>{currency}</SelectItem>
              </SelectContent>
            </Select>
            <Select value={period} onValueChange={(v) => setPeriod(v ?? "all")}>
              <SelectTrigger size="sm" className="h-8 w-[150px] bg-surface2 text-[11.5px] dark:bg-surface2">
                <SelectValue>
                  {(v) =>
                    v === "all"
                      ? t("strategy.limitPeriodAll")
                      : LIMIT_PERIODS[String(v)] && t(LIMIT_PERIODS[String(v)])
                  }
                </SelectValue>
              </SelectTrigger>
              <SelectContent>
                <SelectItem value="day">{t("strategy.limitPeriodDay")}</SelectItem>
                <SelectItem value="weekly">{t("strategy.limitPeriodWeek")}</SelectItem>
                <SelectItem value="monthly">{t("strategy.limitPeriodMonth")}</SelectItem>
                <SelectItem value="yearly">{t("strategy.limitPeriodYear")}</SelectItem>
                <SelectItem value="all">{t("strategy.limitPeriodAll")}</SelectItem>
              </SelectContent>
            </Select>
          </div>
          <p className="text-[11px] leading-relaxed text-mut">{t("strategy.limitBody")}</p>
          <p className="text-[11px] leading-relaxed text-mut">{t("strategy.limitMoneyNote")}</p>
          {err && <p className="text-[11px] text-red-400">{err}</p>}
          <div className="flex items-center gap-2">
            <Button size="sm" className="h-7 px-3 text-[12px] font-semibold" disabled={busy} onClick={save}>
              {t("common.save")}
            </Button>
            {limit && (
              <Button
                variant="ghost"
                size="sm"
                className="h-7 px-3 text-[12px]"
                disabled={busy}
                onClick={() => {
                  setValue("");
                  setUnit("requests");
                  setPeriod("day");
                }}
              >
                {t("strategy.limitClear")}
              </Button>
            )}
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/** The agent's own ceiling, as a row of its own under the strategy. Separate
    because it is separate: it holds under every strategy, `single` included,
    while the strategy above says only who serves the request. */
function LimitsRow({
  agent,
  limit,
  currency,
  onChanged,
}: {
  agent: AgentRef;
  limit: AgentLimit | null;
  currency: string;
  onChanged?: () => void;
}) {
  const t = useT();
  const [open, setOpen] = useState(false);
  return (
    <div className="flex items-center gap-3 border-b border-line px-4 py-2">
      <span className="w-[200px] flex-none text-[11px] text-mut">
        {t("strategy.limitLabel")}
      </span>
      <span className="min-w-0 flex-1 truncate text-[11px] text-ink">
        {limit ? formatLimit(t, limit) : t("strategy.limitNone")}
      </span>
      <Button
        variant="ghost"
        size="sm"
        className="h-6 w-6 shrink-0 px-0 text-mut hover:text-ink"
        aria-label={t("strategy.limitEditAria", { agent: agentMeta(agent).label })}
        title={t("strategy.limitEditAria", { agent: agentMeta(agent).label })}
        onClick={() => setOpen(true)}
      >
        <SquarePen className="h-3.5 w-3.5" />
      </Button>
      <LimitDialog
        agent={agent}
        limit={limit}
        open={open}
        currency={currency}
        onClose={() => setOpen(false)}
        onChanged={onChanged}
      />
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
  agent: AgentRef;
  routes: AgentRoute[];
  onChanged: () => void;
}) {
  const t = useT();
  const [src, setSrc] = useState<AgentRef | null>(null);
  const [bump, setBump] = useState(0);
  const [busy, setBusy] = useState(false);
  const sources = routes.filter((r) => r.agent !== agent && r.bindings.length > 0);

  const mineMeta = agentMeta(agent);
  const srcRoute = src ? routes.find((r) => r.agent === src) : undefined;
  const srcMeta = src ? agentMeta(src) : null;

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
            {t(
              srcRoute.bindings.length === 1
                ? "strategy.replaceConfirmOne"
                : "strategy.replaceConfirmOther",
              {
                mine: mineMeta.label,
                source: srcMeta.label,
                strategy: strategyLabel(t, srcRoute.strategy),
                count: srcRoute.bindings.length,
              },
            )}
          </span>
          <Button size="sm" className="h-6 flex-none px-2 text-[11px]" onClick={apply} disabled={busy}>
            {t("strategy.replace")}
          </Button>
          <Button
            size="sm"
            variant="ghost"
            className="h-6 flex-none px-2 text-[11px]"
            onClick={() => setSrc(null)}
          >
            {t("common.cancel")}
          </Button>
        </>
      ) : (
        <>
          <span className="flex-none">{t("strategy.copyRouteFrom")}</span>
          {/* key remounts the uncontrolled select after a cancelled pick so
              the trigger label resets */}
          <Select key={bump} onValueChange={(v) => setSrc(v as AgentRef)}>
            <SelectTrigger
              size="sm"
              aria-label={t("strategy.copyRouteAria", { agent: mineMeta.label })}
              className="h-6 w-auto gap-1 bg-transparent px-1.5 text-[11px] dark:bg-transparent"
            >
              <SelectValue placeholder={t("strategy.pickAgent")} />
            </SelectTrigger>
            {/* wider than the trigger: "Claude Code · 2 candidates" must fit */}
            <SelectContent className="min-w-[230px]">
              {sources.map((r) => {
                const label = agentMeta(r.agent).label;
                return (
                  <SelectItem key={r.agent} value={r.agent}>
                    {t(
                      r.bindings.length === 1
                        ? "strategy.candidateOne"
                        : "strategy.candidateOther",
                      { agent: label, count: r.bindings.length },
                    )}
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
  currency = "USD",
}: {
  agent: AgentRef;
  routes: AgentRoute[] | null;
  onChanged?: () => void;
  /** Display currency, for a money ceiling's unit. */
  currency?: string;
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
      {/* Under the strategy, not inside it: the ceiling holds whatever the
          strategy says, so it does not belong to any one of them. */}
      <LimitsRow
        agent={agent}
        limit={mine[0]?.limit ?? null}
        currency={currency}
        onChanged={onChanged}
      />
    </section>
  );
}
