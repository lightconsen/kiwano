// One agent's spend ceiling: how a stored one reads, and the controls that set it.
//
// Its own module because two dialogs render it — a built-in agent's settings and
// a user-defined agent's — and because the ceiling is not part of a strategy: it
// holds under every one of them, `single` included, which is why it is a section
// of *settings* rather than a row of the strategy panel it used to sit in.
import { useState } from "react";

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

/** The reset periods a limit can take, and what each reads as. The ids are what
    the backend stores; `all` is the absence of one. */
export const LIMIT_PERIODS: Record<string, KeyPath<Messages>> = {
  day: "strategy.limitPeriodDay",
  weekly: "strategy.limitPeriodWeek",
  monthly: "strategy.limitPeriodMonth",
  yearly: "strategy.limitPeriodYear",
};

/** How a stored ceiling reads: "50 CNY / month", "100 requests / day". A limit
    with no period measures everything it has ever done, which is worth saying
    out loud rather than leaving as a blank. */
export function formatLimit(t: Translate, limit: AgentLimit): string {
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

/** The ceiling's section, for an agent's settings dialog.
 *
 * Committed by Save rather than on change: the amount is typed, so "5" on the way
 * to "50" is not a value anyone meant, and a ceiling that took effect mid-word
 * would refuse requests while the user was still deciding.
 */
export function LimitSection({
  agent,
  limit,
  currency,
  onChanged,
}: {
  agent: AgentRef;
  limit: AgentLimit | null;
  /** Display currency: the unit a money ceiling is written in. An agent's traffic
      spans providers that bill in their own, so the ceiling needs one of its
      own rather than borrowing whichever provider served last. */
  currency: string;
  onChanged?: () => void;
}) {
  const t = useT();
  const [unit, setUnit] = useState(limit?.limit_unit ?? "requests");
  const [value, setValue] = useState(limit ? String(limit.period_limit) : "");
  const [period, setPeriod] = useState(limit?.reset_period ?? "day");
  const [busy, setBusy] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [saved, setSaved] = useState(false);

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
      {/* No heading of its own: the tab that shows this names it. What the line
          keeps is the answer — the ceiling in force right now. */}
      <div className="text-[11px]">
        {limit ? formatLimit(t, limit) : t("strategy.limitNone")}
      </div>

      {/* Two lines rather than one: the amount, its unit and the period come to
          more than the dialog is wide, and a row that overflows a modal takes the
          buttons with it. The numbers read as one decision, the commit as the
          next. */}
      <div className="mt-1.5 flex flex-wrap items-center gap-2">
        <Input
          type="number"
          min={0}
          value={value}
          placeholder={t("strategy.limitPlaceholder")}
          aria-label={t("strategy.limitAmountAria")}
          className="h-8 w-[68px] font-mono text-[12px]"
          onChange={(e) => setValue(e.target.value)}
        />
        <Select value={unit} onValueChange={(v) => setUnit(v ?? "requests")}>
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
            <SelectItem value={currency}>{currency}</SelectItem>
          </SelectContent>
        </Select>
        <Select value={period} onValueChange={(v) => setPeriod(v ?? "all")}>
          <SelectTrigger
            size="sm"
            className="h-8 w-[104px] bg-surface2 text-[11.5px] dark:bg-surface2"
          >
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

      <div className="mt-1.5 flex items-center gap-2">
        <Button
          size="sm"
          className="h-7 px-3 text-[12px] font-semibold"
          disabled={busy}
          onClick={save}
        >
          {saved ? t("common.done") : t("common.save")}
        </Button>
        {limit && (
          <Button
            variant="ghost"
            size="sm"
            className="h-7 px-2 text-[12px]"
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

      <p className="mt-1.5 text-[10.5px] leading-relaxed text-mut">
        {t("strategy.limitBody")}
      </p>
      <p className="mt-1 text-[10.5px] leading-relaxed text-mut">
        {t("strategy.limitMoneyNote")}
      </p>
      {err && <p className="mt-1 text-[11px] text-red-400">{err}</p>}
    </div>
  );
}
