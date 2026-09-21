// The view grouped by model: every model the catalog serves, the providers that
// serve it, and the price range across them. The grouping is built from the
// rows the category filter left, so a group whose providers were all filtered
// out is simply absent.
import { type KeyPath, type Messages, type Translate } from "../../i18n";
import { fmtMoney } from "../../lib/format";
import type { CatalogEntry } from "../../api/types";
import { byTagRank } from "./order";
import { rateText, tierRates, type PriceTier } from "./prices";

// ── the view grouped by model ────────────────────────────────────────────

export type ShelfView = "provider" | "model";

export const VIEWS: { id: ShelfView; labelKey: KeyPath<Messages> }[] = [
  { id: "provider", labelKey: "shelf.viewByProvider" },
  { id: "model", labelKey: "shelf.viewByModel" },
];

/** The remembered view, defaulting to the provider table — and to it for any
    value this build does not know, the way `parseSort` treats an unknown sort
    key. */
export function parseView(v: string | null | undefined): ShelfView {
  return v === "model" ? "model" : "provider";
}

/** Every model id an entry speaks about: the ones its endpoints list, plus the
    one it prices. The union is not belt-and-braces — 15 of the catalog's 71
    priced entries price a model their own list spells differently
    (`openrouter` prices `gpt-5.2` while listing `openai/gpt-5.2`), and matching
    on the lists alone would leave those prices in no group at all. */
function servedModelIds(e: CatalogEntry): string[] {
  const ids = new Set<string>(e.models);
  for (const x of e.endpoints ?? []) {
    for (const m of x.models) ids.add(m);
  }
  if (e.price_ref) ids.add(e.price_ref.model_id);
  return [...ids];
}

/** This provider's published rate for `model` — exact ids only. The app has no
    model-id ladder on this side (the costing one lives in Rust), and inventing
    one here would make two visibly different ids read as one model. */
export function modelRate(e: CatalogEntry, model: string) {
  return e.price_ref && e.price_ref.model_id === model ? e.price_ref : null;
}

export type ModelGroupRow = {
  entry: CatalogEntry;
  rate: NonNullable<CatalogEntry["price_ref"]> | null;
  /** Input rate in the display currency, or null when this provider publishes
      none for this model — the sort key, and the range's raw material. */
  value: number | null;
};

export type ModelGroup = {
  model: string;
  name: string;
  rows: ModelGroupRow[];
  /** The priced rows' input rates, formatted and cheapest first — the header's
      count. */
  prices: string[];
  /** `min – max`, only when the priced rows share a currency and disagree; null
      otherwise (a range across currencies compares units). */
  range: string | null;
};

/** The name a group is shown under: the display name its priced rows agree on,
    else the raw id. Voted rather than taken from the first row, because the
    rows are ordered by price in the user's currency — "first" would relabel a
    group when the currency preference changes. The id is never prettified:
    `openai/gpt-5.2` and `gpt-5.2` are different ids, and giving the first an
    invented name would make two different models read alike. */
function groupName(entries: CatalogEntry[], model: string): string {
  const votes = new Map<string, number>();
  for (const e of entries) {
    const r = e.price_ref;
    if (r && r.model_id === model) votes.set(r.display_name, (votes.get(r.display_name) ?? 0) + 1);
  }
  let best: string | null = null;
  for (const [name, n] of votes) {
    const top = best === null ? -1 : (votes.get(best) ?? 0);
    if (n > top || (n === top && best !== null && name.localeCompare(best) < 0)) best = name;
  }
  return best ?? model;
}

/** The grouped body: one group per model, each holding the providers that speak
    about it, cheapest first.

    A group is worth showing when it has more than one row — one provider is not
    a comparison — unless something is being searched, in which case a single
    row is the answer that was asked for. Rows are built after the chip filter,
    so a group whose providers are all filtered out is simply absent. */
export function buildModelGroups(entries: CatalogEntry[], query: string): ModelGroup[] {
  const q = query.trim().toLowerCase();
  const byModel = new Map<string, CatalogEntry[]>();
  for (const e of entries) {
    for (const id of servedModelIds(e)) {
      const seen = byModel.get(id);
      // One entry contributes one row however many protocols it lists the id
      // under, which is the dedupe — a provider is a row, not a protocol.
      if (seen) seen.push(e);
      else byModel.set(id, [e]);
    }
  }

  const groups: ModelGroup[] = [];
  for (const [model, serving] of byModel) {
    const name = groupName(serving, model);
    const rows: ModelGroupRow[] = serving
      .filter(
        (e) =>
          q === "" ||
          e.name.toLowerCase().includes(q) ||
          model.toLowerCase().includes(q) ||
          name.toLowerCase().includes(q),
      )
      .map((e) => {
        const rate = modelRate(e, model);
        const published = rate ? Number(rate.input) : null;
        return { entry: e, rate, value: published !== null && Number.isFinite(published) ? published : null };
      })
      .sort((a, b) => {
        if (a.value === null || b.value === null) {
          if (a.value === null && b.value === null) return byTagRank(a.entry, b.entry);
          return a.value === null ? 1 : -1;
        }
        return a.value - b.value || byTagRank(a.entry, b.entry);
      });
    if (rows.length < (q === "" ? 2 : 1)) continue;
    // Each rate in the currency it is published in, and a range only when they
    // are all in the same one: `$1 – ¥8` would be a comparison of units.
    const priced = rows.flatMap((r) => (r.rate ? [r.rate] : []));
    const prices = priced.map((rate) => fmtMoney(Number(rate.input), rate.currency || "USD"));
    const currencies = new Set(priced.map((rate) => rate.currency || "USD"));
    const range =
      prices.length > 1 && currencies.size === 1 && new Set(prices).size > 1
        ? `${prices[0]} – ${prices[prices.length - 1]}`
        : null;
    groups.push({ model, name, rows, prices, range });
  }

  // Priced groups first, then the widest, then by name: with today's data two
  // thirds of the rows are dashes, and the groups worth reading should not be
  // interleaved with the empty ones.
  return groups.sort(
    (a, b) =>
      Number(b.prices.length > 0) - Number(a.prices.length > 0) ||
      b.rows.length - a.rows.length ||
      a.name.localeCompare(b.name),
  );
}

/** A group row's price cell: the rates for *this* model, without the model name
    (the group header has it) and without a plan provider's offer — "from ¥49
    /mo" is a price for the provider, and ranking it beside per-token rates
    would compare two different things. */
export function groupPriceText(rate: ModelGroupRow["rate"], t: Translate, tier: PriceTier): string {
  if (!rate) return t("common.none");
  const rates = tierRates(rate, tier);
  if (!Number.isFinite(Number(rates.input)) || !Number.isFinite(Number(rates.output))) {
    return t("common.none");
  }
  return rateText(rates, t);
}
