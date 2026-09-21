// One catalog row of the provider table — and of the grouped view, where the
// same row is nested under its model's header and priced for that model.
import { useState } from "react";
import { Button } from "@/components/ui/button";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { useT } from "../../i18n";
import { hubAssetUrl } from "../../lib/hub";
import type { CatalogEntry } from "../../api/types";
import { TagBadge, billingLabel } from "./labels";
import { ProtoChip } from "./endpoints";
import { TierChip, bandText, rowPriceText, type PriceTier } from "./prices";
import { groupPriceText, modelRate } from "./groups";

export function Row({
  entry,
  hubUrl,
  onAdd,
  onOpen,
  modelId,
  indent,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  onAdd: (e: CatalogEntry) => void;
  /// Handed the tier the row was showing, so the dialog opens on the same
  /// figures the click came from.
  onOpen: (e: CatalogEntry, tier: PriceTier) => void;
  /** Set by the grouped view: the model whose group this row sits in. It is the
      only thing that differs between the two views' rows — the price cell then
      speaks for that model instead of for the provider. */
  modelId?: string;
  /** …and this row is nested under that group's header, so the name cell starts
      where the header's model name does (past its chevron). */
  indent?: boolean;
}) {
  const t = useT();
  const logo = entry.logo && hubUrl ? hubAssetUrl(hubUrl, entry.logo) : undefined;
  const rate = modelId ? modelRate(entry, modelId) : (entry.price_ref ?? null);
  // Which of this row's two prices the cell shows. Held per row and not
  // remembered: it is a peek at the other number ("now or tonight?"), not a
  // preference — and the column's sort and a group's range stay on the
  // published rate, which is what they have always ranked by.
  const [tier, setTier] = useState<PriceTier>("peak");
  const price = modelId ? groupPriceText(rate, t, tier) : rowPriceText(entry, t, tier);
  /** The tooltip: the same figures plus the unit they are quoted in, which the
      table has no room to repeat on every row. A cell with no rates behind it
      (a plan's offer, a provider that prices no model) says only what it says. */
  const priceHint = rate
    ? [
        t("shelf.priceUnit", { rates: price }),
        // The cell has room for the headline rate alone, and the headline is the
        // band most requests pay — so the step-up lives here.
        rate.long_context ? bandText(rate.long_context, rate.currency || "USD", t) : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : price;
  const toggleTier = () => setTier((v) => (v === "peak" ? "offPeak" : "peak"));
  return (
    <tr
      className="cursor-pointer border-t border-line hover:bg-surface2"
      onClick={() => onOpen(entry, tier)}
    >
      <td className={`py-2 ${indent ? "pl-10" : "pl-4"} pr-2`}>
        <div className="flex items-center gap-2">
          <ProviderLogo logo={logo} name={entry.name} color={entry.logo_color} />
          <span className="truncate text-[12.5px] font-semibold" title={entry.name}>
            {entry.name}
          </span>
        </div>
      </td>
      <td className="px-2 py-2">
        {/* All supported protocols on one horizontal line (endpoint URLs
            live in the detail dialog, not the table) */}
        <div className="flex items-center gap-1">
          <ProtoChip protocol={entry.protocol} t={t} />
          {(entry.endpoints ?? []).map((e) => (
            <ProtoChip key={e.protocol} protocol={e.protocol} t={t} />
          ))}
        </div>
      </td>
      <td className="px-2 py-2">
        <TagBadge tag={entry.tag} t={t} />
      </td>
      <td className="px-2 py-2 text-[11.5px] text-mut">
        {/* How you pay for it, spelled out: this is a column of its own rather
            than a marker inside the price cell, and "Pay as you go" is the
            label the detail dialog uses. `billingLabel` also passes an unknown
            Hub tag through as itself. */}
        <span className="block truncate" title={billingLabel(entry.billing, t)}>
          {billingLabel(entry.billing, t)}
        </span>
      </td>
      <td className="px-2 py-2 text-[11.5px] text-mut">
        {/* What you pay: the offer first for a plan provider (see
            `rowPriceText`), then the representative model's rates, or the
            provider's own one-liner when it prices nothing. `title` because the
            cell can truncate. */}
        <span className="flex items-center">
          <span className="min-w-0 truncate" title={priceHint}>
            {price}
          </span>
          {/* `rate` is exactly the price this cell is showing, so it is also
              exactly when there is a second one to switch to. */}
          {rate && <TierChip rate={rate} tier={tier} onToggle={toggleTier} t={t} />}
        </span>
      </td>
      <td className="px-4 py-2 text-right" onClick={(e) => e.stopPropagation()}>
        {entry.added ? (
          <span className="text-[10.5px] text-mut">{t("shelf.added")}</span>
        ) : (
          <Button size="xs" className="rounded px-2 text-[10.5px] font-semibold" onClick={() => onAdd(entry)}>
            {entry.tag === "local" ? t("shelf.connect") : t("shelf.add")}
          </Button>
        )}
      </td>
    </tr>
  );
}
