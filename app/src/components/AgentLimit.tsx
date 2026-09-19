// An agent's spend ceilings: the windows they are measured over, and the controls
// that set them.
//
// Its own module because two dialogs render it — a built-in agent's settings and
// a user-defined agent's — and because a ceiling is not part of a strategy: it
// holds under every one of them, `single` included.
//
// A *list* of windows, because an agent may hold several at once: a day's ceiling
// is what stops one runaway session and a month's is what stops a runaway month.
// Being over any one of them is being over, so they are edited as a set and
// written as a set.
import { useState } from "react";
import { Plus, X } from "lucide-react";

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
import { type AgentLimit, type AgentRef } from "../api/types";
import { useT, type KeyPath, type Messages, type Translate } from "../i18n";

/** The windows a ceiling can be measured over, in the order they are offered.
    `all` is the absence of one, which the backend spells the same way. */
export const LIMIT_PERIODS: Record<string, KeyPath<Messages>> = {
  day: "strategy.limitPeriodDay",
  weekly: "strategy.limitPeriodWeek",
  monthly: "strategy.limitPeriodMonth",
  yearly: "strategy.limitPeriodYear",
  all: "strategy.limitPeriodAll",
};

/** Every window id, in offer order. */
const PERIOD_IDS = Object.keys(LIMIT_PERIODS);

export function periodLabel(t: Translate, period: string): string {
  const key = LIMIT_PERIODS[period];
  return key ? t(key) : period;
}

/** One window as the form holds it: the amount is text while it is being typed. */
interface Draft {
  period: string;
  value: string;
  unit: string;
}

/** Read a stored window as the form's draft. */
function toDraft(l: AgentLimit): Draft {
  return {
    period: l.period,
    value: String(l.period_limit),
    unit: l.limit_unit ?? "requests",
  };
}

/** The draft as the backend wants it, or nothing when it is not a ceiling: blank,
    zero or unparseable is the absence of that window — the same reading the
    gateway applies to a stored zero, so the form cannot show a ceiling that would
    not be enforced. */
function toLimit(d: Draft): AgentLimit | null {
  const amount = Number(d.value);
  if (!Number.isFinite(amount) || amount <= 0) return null;
  return {
    period: d.period,
    period_limit: amount,
    limit_unit: d.unit === "requests" ? null : d.unit,
  };
}

export function LimitSection({
  agent,
  limits,
  currencies,
  onChanged,
}: {
  agent: AgentRef;
  limits: AgentLimit[];
  /** The currencies a money ceiling may be written in: what this agent's own
      providers bill in (or, when none of them says, every currency the Hub
      prices). An agent's traffic spans providers that bill in their own, so the
      ceiling needs a currency of its own rather than borrowing whichever
      provider served last — and never the display currency, which is a
      preference about reading numbers rather than about money being spent. */
  currencies: string[];
  onChanged?: () => void;
}) {
  const t = useT();
  const [rows, setRows] = useState<Draft[]>(() => limits.map(toDraft));
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

  // The windows no row holds yet. `Add` is offered only while one is left, so the
  // form cannot hold two rows for the same window — which would be a ceiling that
  // depends on which row was read.
  const unused = PERIOD_IDS.filter((p) => !rows.some((r) => r.period === p));

  const setRow = (period: string, patch: Partial<Draft>) =>
    setRows((rs) => rs.map((r) => (r.period === period ? { ...r, ...patch } : r)));

  /** A money unit rather than a count: everything the two counting units are
      not. The gateway reads a stored unit the same way round — a 3-letter code
      is a currency, anything else is a count (`limits.rs`). */
  const isMoney = (unit: string) => unit !== "requests" && unit !== "wan_tokens";

  /** The currencies this row may be written in: the agent's, plus the one it is
      already in. A ceiling stored before its agent's providers changed — or set
      from the CLI — has to keep reading as itself rather than as blank. */
  const unitsFor = (r: Draft) =>
    isMoney(r.unit) && !currencies.includes(r.unit)
      ? [...currencies, r.unit].sort()
      : currencies;

  /** What a new ceiling starts in: the agent's currency when its providers
      agree on one — it is then the only money the ceiling could be in — and a
      request count when there is a choice to make or nothing to choose from. */
  const defaultUnit = currencies.length === 1 ? currencies[0] : "requests";

  const save = async () => {
    setBusy(true);
    setErr(null);
    try {
      // Committed as a set, and by Save rather than on change: the amounts are
      // typed, so "5" on the way to "50" is not a value anyone meant — and a
      // ceiling that took effect mid-word would refuse requests while the user was
      // still deciding.
      await api.setAgentLimits(
        agent,
        rows.map(toLimit).filter((l): l is AgentLimit => l !== null),
      );
      setSaved(true);
      window.setTimeout(() => setSaved(false), 1200);
      onChanged?.();
    } catch (e) {
      setErr(String(e));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div>
      {rows.length === 0 && <div className="text-[11px]">{t("strategy.limitNone")}</div>}

      {rows.map((r) => (
        <div key={r.period} className="mt-1.5 flex flex-wrap items-center gap-2">
          <Input
            type="number"
            min={0}
            value={r.value}
            placeholder={t("strategy.limitPlaceholder")}
            aria-label={`${t("strategy.limitAmountAria")} · ${periodLabel(t, r.period)}`}
            className="h-8 w-[68px] font-mono text-[12px]"
            onChange={(e) => setRow(r.period, { value: e.target.value })}
          />
          <Select
            value={r.unit}
            onValueChange={(v) => setRow(r.period, { unit: v ?? "requests" })}
          >
            <SelectTrigger
              size="sm"
              className="h-8 w-[112px] bg-surface2 text-[11.5px] dark:bg-surface2"
            >
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
              {unitsFor(r).map((c) => (
                <SelectItem key={c} value={c}>
                  {c}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Select value={r.period} onValueChange={(v) => v && setRow(r.period, { period: v })}>
            <SelectTrigger
              size="sm"
              className="h-8 w-[104px] bg-surface2 text-[11.5px] dark:bg-surface2"
            >
              <SelectValue>{(v) => periodLabel(t, String(v ?? "day"))}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {/* Its own window, plus the ones no other row holds. */}
              {PERIOD_IDS.filter((p) => p === r.period || unused.includes(p)).map((p) => (
                <SelectItem key={p} value={p}>
                  {periodLabel(t, p)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
          <Button
            variant="ghost"
            size="sm"
            className="h-8 w-8 shrink-0 px-0 text-mut hover:text-ink"
            aria-label={t("strategy.limitRemoveAria")}
            title={t("strategy.limitRemoveAria")}
            onClick={() => setRows((rs) => rs.filter((x) => x.period !== r.period))}
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      ))}

      <div className="mt-1.5 flex items-center gap-2">
        <Button
          variant="ghost"
          size="sm"
          className="h-7 gap-1 px-2 text-[12px]"
          disabled={unused.length === 0}
          onClick={() =>
            setRows((rs) => [...rs, { period: unused[0], value: "", unit: defaultUnit }])
          }
        >
          <Plus className="h-3.5 w-3.5" />
          {t("strategy.limitAdd")}
        </Button>
        <Button
          size="sm"
          className="ml-auto h-7 px-3 text-[12px] font-semibold"
          disabled={busy}
          onClick={save}
        >
          {saved ? t("common.done") : t("common.save")}
        </Button>
      </div>

      <p className="mt-1.5 text-[10.5px] leading-relaxed text-mut">
        {t("strategy.limitBody")}
      </p>
      <p className="mt-1 text-[10.5px] leading-relaxed text-mut">
        {t("strategy.limitMoneyNote")}
      </p>
      <p className="mt-1 text-[10.5px] leading-relaxed text-mut">
        {t("strategy.limitCurrencyNote")}
      </p>
      {err && <p className="mt-1 text-[11px] text-red-400">{err}</p>}
    </div>
  );
}
