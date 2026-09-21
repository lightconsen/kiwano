// What a hand-added provider declares it charges per million tokens.
import { Plus, XIcon } from "lucide-react";
import type { Dispatch, SetStateAction } from "react";
import { Button } from "@/components/ui/button";
import { Input } from "@/components/ui/input";
import { Label } from "@/components/ui/label";
import { useT, type KeyPath, type Messages } from "../../i18n";
import type { DeclaredPrice } from "../../api/types";

// This table carries user-visible labels, so it is built inside the form where
// the translator is available rather than at module scope (a module-level `t`
// is impossible — it is a hook).

/// The four rates one declared-price row collects, in the price table's own
/// order: `DeclaredPrice` is read back with exactly these fields, and the order
/// is the one every price in the app is printed in (input, output, cache read,
/// cache write).
export const RATE_FIELDS: {
  key: keyof Omit<DeclaredPrice, "model_id">;
  labelKey: KeyPath<Messages>;
}[] = [
  { key: "input", labelKey: "addProvider.priceIn" },
  { key: "output", labelKey: "addProvider.priceOut" },
  { key: "cache_read", labelKey: "addProvider.priceCacheRead" },
  { key: "cache_creation", labelKey: "addProvider.priceCacheWrite" },
];

/** A declared rate: blank (a box not filled in yet), or a number that is zero
 * or more.
 *
 * Blank is allowed through because a row is entered field by field, and a
 * rate the user never states is zero — the backend reads it that way, and the
 * section's own note says so. Anything else is refused rather than dropped:
 * the price table parses these as numbers when it costs a request, so a stray
 * character would become the cost of *every* request this provider serves. */
export const invalidRate = (raw: string): boolean => {
  if (raw.trim() === "") return false;
  const rate = Number(raw);
  return !Number.isFinite(rate) || rate < 0;
};

/** The rates a hand-added provider declares, for a pay-as-you-go provider with
    no catalog entry behind it. Held as the form wrote them — blank boxes
    included — so a half-typed row survives a re-render; the blanks become zeros
    on the way out, which is the backend's own reading of a rate nobody stated. */
export function PricesSection({
  prices,
  setPrices,
  model,
  limitCurrency,
  badPrice,
}: {
  prices: DeclaredPrice[];
  setPrices: Dispatch<SetStateAction<DeclaredPrice[]>>;
  /** Only read to seed a new row with the model the form routes */
  model: string;
  limitCurrency: string;
  badPrice: boolean;
}) {
  const t = useT();
  /** Patch one row: its model id, or one of its four rates. */
  const setPrice = (i: number, patch: Partial<DeclaredPrice>) =>
    setPrices((rows) => rows.map((row, j) => (j === i ? { ...row, ...patch } : row)));

  /** A new row, seeded with the default model when the form has one and does
   * not already price it: the model this provider routes is the one whose rates
   * the user came here to write down. */
  const addPrice = () =>
    setPrices((rows) => [
      ...rows,
      {
        model_id: rows.some((r) => r.model_id.trim() === model.trim()) ? "" : model.trim(),
        input: "",
        output: "",
        cache_read: "",
        cache_creation: "",
      },
    ]);
  return (
    <div>
      <Label className="text-[11px] font-medium text-mut">
        {t("addProvider.prices")}{" "}
        <span className="ml-1 text-[10px]" style={{ color: "var(--kiwi)" }}>
          {t("addProvider.pricesHint", { currency: limitCurrency })}
        </span>
      </Label>
      <div className="mt-1 space-y-1.5">
        {prices.map((row, i) => (
          <div key={i} className="rounded-md border border-line px-2 py-1.5">
            <div className="flex items-center gap-1.5">
              <Input
                className="h-7 min-w-0 flex-1 bg-bg font-mono text-[11.5px] dark:bg-bg"
                value={row.model_id}
                onChange={(e) => setPrice(i, { model_id: e.target.value })}
                aria-label={t("addProvider.priceModel")}
                placeholder={t("addProvider.priceModelPlaceholder")}
              />
              <Button
                variant="ghost"
                size="icon-xs"
                className="flex-none text-mut"
                aria-label={t("addProvider.removePrice")}
                title={t("addProvider.removePrice")}
                onClick={() => setPrices((rows) => rows.filter((_, j) => j !== i))}
              >
                <XIcon />
              </Button>
            </div>
            <div className="mt-1 grid grid-cols-2 gap-x-2 gap-y-1">
              {RATE_FIELDS.map((f) => (
                <label key={f.key} className="flex min-w-0 items-center gap-1.5">
                  <span className="flex-none text-[10px] text-mut">
                    {t(f.labelKey)}
                  </span>
                  <Input
                    className="h-7 min-w-0 flex-1 bg-bg font-mono text-[11.5px] dark:bg-bg"
                    value={row[f.key]}
                    onChange={(e) => setPrice(i, { [f.key]: e.target.value })}
                    inputMode="decimal"
                    aria-invalid={invalidRate(row[f.key])}
                    aria-label={t(f.labelKey)}
                  />
                </label>
              ))}
            </div>
          </div>
        ))}
        <Button
          variant="outline"
          size="sm"
          className="h-7 w-full gap-1 text-[11px] text-mut"
          onClick={addPrice}
        >
          <Plus className="h-3 w-3" />
          {t("addProvider.addPrice")}
        </Button>
        <p className="text-[10.5px] text-mut">{t("addProvider.pricesBody")}</p>
        {badPrice && (
          <p className="text-[10.5px]" style={{ color: "var(--red)" }}>
            {t("addProvider.priceInvalid")}
          </p>
        )}
      </div>
    </div>
  );
}
