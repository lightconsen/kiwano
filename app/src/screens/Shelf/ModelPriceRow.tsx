// One model of a provider, in the detail dialog's per-model list.
import { useState } from "react";
import { type Translate } from "../../i18n";
import type { ModelPrice } from "../../api/types";
import { TierChip, bandText, modelRateText, type PriceTier } from "./prices";

/** One model of a provider: its id, its rates, and its own tier switch. The
    switch is per model because the schedule is — a provider may price one model
    by time of day and another flatly. */
export function ModelPriceRow({
  modelId,
  price,
  t,
}: {
  modelId: string;
  price: ModelPrice | null;
  t: Translate;
}) {
  const [tier, setTier] = useState<PriceTier>("peak");
  if (!price) {
    return (
      <div className="flex items-center gap-2 rounded-md border border-line px-2.5 py-2">
        <span className="min-w-0 truncate font-mono text-[11px]" title={modelId}>
          {modelId}
        </span>
        <span className="text-[10.5px] text-mut">{t("shelf.noPublishedPrice")}</span>
      </div>
    );
  }
  return (
    <div className="rounded-md border border-line px-2.5 py-2">
      <div className="flex items-center gap-2">
        <span className="min-w-0 truncate font-mono text-[11px]" title={price.model_id}>
          {price.model_id}
        </span>
        <TierChip
          rate={price}
          tier={tier}
          onToggle={() => setTier((v) => (v === "peak" ? "offPeak" : "peak"))}
          t={t}
        />
      </div>
      <div className="mt-1 text-[10.5px] text-mut">{modelRateText(price, tier, t)}</div>
      {/* The step-up, on its own line and outside the tier switch: a band is
          chosen by how long the request is, not by what the reader wants to
          look at, and it carries no schedule for the switch to flip. */}
      {price.long_context && (
        <div className="mt-0.5 text-[10.5px] text-mut">
          {bandText(price.long_context, price.currency || "USD", t)}
        </div>
      )}
    </div>
  );
}
