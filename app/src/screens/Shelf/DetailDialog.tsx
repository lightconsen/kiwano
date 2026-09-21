// The listing behind a catalog row: the dialog a row click opens, and the only
// place the shelf spells a provider out — its endpoints, its schedule and every
// model it serves with the rates the gateway charges by.
import { useState } from "react";
import { Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "../../api/client";
import { useT } from "../../i18n";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { hubAssetUrl } from "../../lib/hub";
import type { CatalogEntry, ModelPrice } from "../../api/types";
import { billingLabel, tagChipStyle, tagLabel, tiersOf } from "./labels";
import {
  MODEL_ROWS_SHOWN,
  MODEL_SEARCH_ABOVE,
  priceText,
  tierDetail,
  type PriceTier,
} from "./prices";
import { EndpointCard } from "./endpoints";
import { ModelPriceRow } from "./ModelPriceRow";

/** Model detail dialog: clicking a catalog row opens the full listing —
    every per-protocol endpoint with its models, pricing and an Add shortcut. */
export function DetailDialog({
  entry,
  hubUrl,
  tier,
  prices,
  onClose,
  onAdd,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  /// The tier the row was showing when it was opened; the dialog spells both
  /// out below the hero either way.
  tier: PriceTier;
  /// The price mirror, for the per-model list below.
  prices: ModelPrice[];
  onClose: () => void;
  onAdd: (e: CatalogEntry) => void;
}) {
  const t = useT();
  const logo = entry.logo && hubUrl ? hubAssetUrl(hubUrl, entry.logo) : undefined;
  const price = priceText(entry, t, tier);
  const endpoints = [
    { protocol: entry.protocol, endpoint: entry.endpoint, models: entry.models },
    ...(entry.endpoints ?? []),
  ];
  const website = entry.website;
  const [showAllModels, setShowAllModels] = useState(false);
  const [modelQuery, setModelQuery] = useState("");

  /** The mirror row for one model, in the order the gateway prices in: this
      provider's own row first, then — the gateway's own tolerance for a model
      the provider does not price — the model's lowest-`provider_id` row, which
      is what such a model is actually billed at. Showing "no price" there would
      put the dialog at odds with the bill. `localeCompare` sorts the empty
      (general) id first, as the gateway does; the Hub no longer publishes those,
      but a table seeded before it stopped still has them.

      Matching is exact, so a model the mirror spells differently has no row at
      all — the gateway's normalized and prefix matching is not reproduced here,
      and that difference is known. */
  const priceFor = (modelId: string) => {
    const same = (a: string, b: string) => a.trim().toLowerCase() === b.trim().toLowerCase();
    const forModel = prices.filter((p) => same(p.model_id, modelId));
    const own = forModel.find((p) => same(p.provider_id, entry.id));
    if (own) return own;
    return [...forModel].sort((a, b) => a.provider_id.localeCompare(b.provider_id))[0] ?? null;
  };

  // The models this provider serves, ordered so that a capped list keeps what a
  // reader came for. The representative model — the one the shelf row priced,
  // and so the one that made them open this — leads; then the rest the mirror
  // prices, since showing prices is what the list is for; then everything else,
  // declared or not, by id.
  const hero = entry.price_ref?.model_id;
  const rank = (m: { id: string; price: ModelPrice | null }) =>
    m.id === hero ? 0 : m.price ? 1 : 2;
  const models = [
    ...new Set([hero, ...endpoints.flatMap((e) => e.models)].filter((id): id is string => !!id)),
  ]
    .map((id) => ({ id, price: priceFor(id) }))
    .sort((a, b) => rank(a) - rank(b) || a.id.localeCompare(b.id));

  // The section's own filter. It matches the model's id and the mirror's
  // display name for it: the two differ often enough — `deepseek-chat (V3)`
  // against `DeepSeek Chat (V3)` — that matching the id alone would refuse a
  // query a reader can see the answer to. Case-insensitive, and the cap applies
  // to whatever it leaves, so a broad query cannot grow the section back.
  const needle = modelQuery.trim().toLowerCase();
  const matched = needle
    ? models.filter(
        (m) =>
          m.id.toLowerCase().includes(needle) ||
          (m.price?.display_name ?? "").toLowerCase().includes(needle),
      )
    : models;
  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      {/* overflow-x-hidden: same WKWebView hardening as the other modals */}
      <DialogContent className="max-h-[min(600px,100dvh)] w-[calc(100%-2rem)] max-w-[480px] gap-0 overflow-x-hidden overflow-y-auto rounded-xl p-0 sm:max-w-[480px]">
        <DialogHeader className="flex h-11 flex-row items-center justify-between border-b border-line pl-4 pr-12">
          <DialogTitle className="flex items-center gap-2 text-[13px] font-semibold">
            <ProviderLogo logo={logo} name={entry.name} color={entry.logo_color} size={18} />
            {entry.name}
          </DialogTitle>
        </DialogHeader>

        <div className="px-4 py-3.5">
          {/* hero: category · rating · the representative model's price */}
          <div className="flex flex-wrap items-center gap-2">
            <span className="rounded px-1.5 text-[10px]" style={tagChipStyle(entry.tag)}>
              {tagLabel(entry.tag, t)}
            </span>
            <span className="text-[11.5px] text-mut">★ {entry.rating.toFixed(1)}</span>
            {entry.price_ref && <span className="ml-auto text-[12px] font-medium">{price}</span>}
          </div>

          {/* The provider's one line of prose (`desc`), which is also what the
              price cell falls back to for the eleven entries that price no
              model. */}
          {entry.desc && <p className="mt-2 text-[11.5px] leading-relaxed text-mut">{entry.desc}</p>}

          {/* A tiered price spelled out: the hero's figures are the peak ones,
              and the alternative is not a footnote — it is what the same model
              costs most of the week. */}
          {entry.price_ref &&
            (() => {
              const tiers = tiersOf(entry.price_ref);
              if (!tiers) return null;
              return (
                <p className="mt-1.5 text-[11px] leading-relaxed text-mut">
                  {tierDetail(entry.price_ref, tiers, t)}
                </p>
              );
            })()}

          <div className="mt-2.5 flex flex-wrap items-center gap-4 text-[11.5px] text-mut">
            <span>
              {t("shelf.billing")}{" "}
              <span className="text-ink">{billingLabel(entry.billing, t)}</span>
            </span>
            {website && (
              <button
                type="button"
                className="cursor-pointer hover:underline"
                style={{ color: "var(--kiwi)" }}
                title={website}
                onClick={() => api.openUrl(website).catch(() => {})}
              >
                {t("shelf.website")}
              </button>
            )}
          </div>

          {/* one card per protocol: endpoint URL + keyless Test + models */}
          <div className="mt-3">
            <div className="mb-1 text-[10px] font-medium text-mut">{t("shelf.endpoints")}</div>
            <div className="space-y-1.5">
              {endpoints.map((e) => (
                <EndpointCard key={e.protocol} protocol={e.protocol} endpoint={e.endpoint} models={e.models} />
              ))}
            </div>
          </div>
          {/* Every model this provider serves, with the rates the gateway
              charges by. The catalog names one representative price per
              provider, so without this the rest of them — and any schedule of
              their own — are invisible. */}
          {models.length > 0 && (
            <div className="mt-3">
              {/* The heading counts what the filter has left, so it never
                  disagrees with the rows under it. */}
              <div className="mb-1 text-[10px] font-medium text-mut">
                {needle
                  ? t("shelf.modelsPricesFiltered", { n: matched.length, total: models.length })
                  : t("shelf.modelsPrices", { n: models.length })}
              </div>
              {models.length > MODEL_SEARCH_ABOVE && (
                <div className="relative mb-1.5">
                  <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-mut" />
                  <input
                    className="h-7 w-full rounded-md border border-line bg-surface pl-7 pr-2 text-[12px]"
                    placeholder={t("shelf.searchModels")}
                    value={modelQuery}
                    onChange={(e) => setModelQuery(e.target.value)}
                  />
                </div>
              )}
              {matched.length === 0 ? (
                <div className="rounded-md border border-line px-2.5 py-2 text-[10.5px] text-mut">
                  {t("shelf.noMatchingModels")}
                </div>
              ) : (
                <div className="space-y-1.5">
                  {(showAllModels ? matched : matched.slice(0, MODEL_ROWS_SHOWN)).map((m) => (
                    <ModelPriceRow key={m.id} modelId={m.id} price={m.price} t={t} />
                  ))}
                </div>
              )}
              {matched.length > MODEL_ROWS_SHOWN && (
                <button
                  type="button"
                  className="mt-1.5 cursor-pointer text-[10.5px] hover:underline"
                  style={{ color: "var(--kiwi)" }}
                  onClick={() => setShowAllModels((v) => !v)}
                >
                  {showAllModels
                    ? t("shelf.showFewerModels")
                    : t("shelf.showAllModels", { n: matched.length - MODEL_ROWS_SHOWN })}
                </button>
              )}
            </div>
          )}
        </div>

        <DialogFooter className="mx-0 mb-0 flex-row justify-end gap-2 rounded-b-xl border-t border-line bg-transparent px-4 py-3">
          {entry.added && (
            <span className="mr-auto self-center text-[11px] text-mut">
              {t("shelf.alreadyAdded")}
            </span>
          )}
          <Button variant="outline" size="sm" onClick={onClose}>
            {t("common.close")}
          </Button>
          {!entry.added && (
            <Button size="sm" className="font-semibold" onClick={() => onAdd(entry)}>
              {entry.tag === "local" ? t("shelf.connect") : t("shelf.add")}
            </Button>
          )}
        </DialogFooter>
      </DialogContent>
    </Dialog>
  );
}
