// The spending ceilings: the plan's two percent windows, the pay-as-you-go
// amount, and the notes that stand in for either.
import { Gauge, Infinity as InfinityIcon } from "lucide-react";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { useT } from "../../i18n";

/// The ceilings offered as one-click chips. These are the percentages people
/// actually set: some headroom, a lot of headroom, or "only once the window is
/// full" — which is a value, not the same as leaving the box blank.
export const PLAN_PRESET_PCTS = [80, 90, 100];

/** A ceiling is a percentage in (0, 100], or blank for "no ceiling".
 *
 * Anything else is not a value to save. The backend keeps only that range and
 * silently drops the rest, so `150` — or `0`, or a stray letter — used to look
 * accepted and store nothing at all: a routing policy the user believed they
 * had set. The dialog says so instead, and refuses the save. */
export const invalidPct = (raw: string): boolean => {
  if (raw.trim() === "") return false;
  const pct = Number(raw);
  return !Number.isFinite(pct) || pct <= 0 || pct > 100;
};

/** A plan provider whose vendor publishes no quota query: there is nothing to
    measure a ceiling against, so the form offers none. */
export function PlanQuotaNotice() {
  const t = useT();
  return (
    <div className="flex h-8 items-center gap-1.5 text-[11.5px] text-mut">
      <Gauge className="h-3.5 w-3.5" />
      {t("addProvider.noQuotaEndpoint")}
    </div>
  );
}

/** The plan's two windows. Both are percentages of what the vendor's own
    endpoint reports, so both are gated on a quota query existing at all. */
export function PlanCeilings({
  planFiveHour,
  setPlanFiveHour,
  planWeekly,
  setPlanWeekly,
}: {
  planFiveHour: string;
  setPlanFiveHour: (v: string) => void;
  planWeekly: string;
  setPlanWeekly: (v: string) => void;
}) {
  const t = useT();
  /** One plan-window ceiling: a field that reads as a percentage, the presets
      beside the window's name, and the reason when it cannot be saved. */
  const ceilingField = (value: string, setValue: (v: string) => void, name: string) => {
    const bad = invalidPct(value);
    return (
      <div>
        <div className="flex items-center gap-1.5">
          <Input
            className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
            value={value}
            onChange={(e) => setValue(e.target.value)}
            inputMode="decimal"
            aria-invalid={bad}
          />
          <span className="text-[11.5px] text-mut">%</span>
        </div>
        <div className="mt-1 flex flex-wrap items-center gap-1">
          <p className="text-[10px] text-mut">{name}</p>
          {PLAN_PRESET_PCTS.map((pct) => (
            <button
              key={pct}
              type="button"
              className="chip rounded border border-line px-1.5 text-[10px] text-mut hover:text-foreground"
              onClick={() => setValue(String(pct))}
            >
              {pct}%
            </button>
          ))}
        </div>
        {bad && (
          <p className="mt-1 text-[10px]" style={{ color: "var(--red)" }}>
            {t("addProvider.planLimitRange")}
          </p>
        )}
      </div>
    );
  };
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.usageLimits")}
        <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
          {t("addProvider.usageLimitsHint")}
        </span>
      </Label>
      <div className="mt-1 grid grid-cols-2 gap-2">
        {ceilingField(planFiveHour, setPlanFiveHour, t("addProvider.fiveHourWindow"))}
        {ceilingField(planWeekly, setPlanWeekly, t("addProvider.weeklyWindow"))}
      </div>
      <p className="mt-1.5 text-[10.5px] text-mut">{t("addProvider.planLimitsBody")}</p>
    </div>
  );
}

/** The pay-as-you-go spending limit, and the currency it is denominated in.
    A catalog entry states the currency its prices are in — the limit is measured
    against those — so the picker is locked while the entry owns the form. */
export function PaygLimit({
  limitValue,
  setLimitValue,
  limitCurrency,
  setLimitCurrency,
  currencies,
  fromEntry,
}: {
  limitValue: string;
  setLimitValue: (v: string) => void;
  limitCurrency: string;
  setLimitCurrency: (v: string) => void;
  /** Every currency the Hub prices — the container reads it once per open */
  currencies: string[];
  fromEntry: boolean;
}) {
  const t = useT();
  /** What the picker offers. */
  const currencyOptions = Array.from(
    new Set([...(currencies.length ? currencies : [limitCurrency]), limitCurrency]),
  );
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.spendingLimit")}{" "}
        <span className="text-[10px]" style={{ color: "var(--kiwi)" }}>
          {t("addProvider.spendingLimitHint")}
        </span>
      </Label>
      <div className="mt-1 flex gap-1.5">
        <Input
          className="h-8 min-w-0 flex-1 bg-bg font-mono text-[12px] dark:bg-bg"
          value={limitValue}
          onChange={(e) => setLimitValue(e.target.value)}
          placeholder="50"
        />
        {/* A catalog entry states the currency its prices are in, and
            the limit is measured against those — so it is that while the
            entry owns the form. A provider typed in by hand has no such
            statement, and this is where the user makes it. */}
        <Select
          value={limitCurrency}
          disabled={fromEntry}
          onValueChange={(v) => v && setLimitCurrency(v)}
        >
          <SelectTrigger
            className="w-[84px] bg-bg text-[12px] dark:bg-bg"
            aria-label={t("addProvider.spendingLimit")}
            title={t("addProvider.currencyTitle", { currency: limitCurrency })}
          >
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            {currencyOptions.map((c) => (
              <SelectItem key={c} value={c}>
                {c}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
      </div>
    </div>
  );
}

/** An unlimited provider has no ceiling to configure at all. */
export function UnlimitedNote() {
  const t = useT();
  return (
    <div className="flex h-8 items-center gap-1.5 text-[11.5px] text-mut">
      <InfinityIcon className="h-3.5 w-3.5" />
      {t("addProvider.noQuotaConfig")}
    </div>
  );
}
