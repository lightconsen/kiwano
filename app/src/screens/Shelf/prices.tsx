// The shelf's price vocabulary: the two tiers a published rate can be read in,
// the chip that switches between them, and the sentences the price cells and
// the detail dialog are built from. Every helper is hook-free and takes the
// translator as a parameter, so the container's own `t` is the only one used.
import { fmtMoney, fmtTokens } from "../../lib/format";
import { type Translate } from "../../i18n";
import type { CatalogEntry, LongContextRates, ModelPrice } from "../../api/types";
import { tzLabel } from "../../lib/peak";
import { tiersOf, windowText } from "./labels";

/// How many models the detail dialog lists before it offers to expand. The
/// catalog's own lists are short (a curated handful per endpoint); this is for
/// the day the Hub publishes a long one, and it keeps the sections below the
/// list reachable.
export const MODEL_ROWS_SHOWN = 8;

/// Past this many models the section grows a filter of its own. Reading a list
/// stops working somewhere around here — that is the point at which a reader is
/// hunting for one model rather than taking in the set.
export const MODEL_SEARCH_ABOVE = 10;

/// Which of a row's two prices its cell is showing. The row's own figures are
/// the **peak** ones (the published table's rule), so that is where every cell
/// starts.
export type PriceTier = "peak" | "offPeak";

/** The rates to show for `tier`: the row's own (peak) figures, or the
    discounted pair. A row that publishes no schedule has only the one, which is
    why the off-peak answer is the peak one there. */
export function tierRates(
  rate: NonNullable<CatalogEntry["price_ref"]>,
  tier: PriceTier,
): { input: string; output: string; currency: string } {
  const off = rate.off_peak;
  if (tier === "offPeak" && off && rate.peak_hours) {
    return { input: off.in, output: off.out, currency: rate.currency };
  }
  return { input: rate.input, output: rate.output, currency: rate.currency };
}

/** A pair of rates, labelled and in the currency they are published in:
    `in ¥9 / out ¥27`. The two numbers are meaningless unlabelled — any two
    figures could sit either side of a slash, and which one a request spends
    most of is exactly what a reader is comparing. */
export function rateText(rate: { input: string; output: string; currency: string }, t: Translate): string {
  const cur = rate.currency || "USD";
  return `${t("shelf.priceIn")} ${fmtMoney(Number(rate.input), cur)} / ${t("shelf.priceOut")} ${fmtMoney(Number(rate.output), cur)}`;
}

/** The one line that says what a tiered price means: the peak rates, the
    off-peak rates, and when each applies — **on the vendor's clock**, named, so
    a reader is never left to assume their own. */
export function tierDetail(
  rate: NonNullable<CatalogEntry["price_ref"]>,
  tiers: NonNullable<ReturnType<typeof tiersOf>>,
  t: Translate,
): string {
  return [
    `${t("shelf.peakRates")} ${rateText(rate, t)}`,
    // The off-peak block spells its keys `in`/`out` (the published shape);
    // `rateText` takes the pair, so the mapping happens here.
    `${t("shelf.offPeakRates")} ${rateText(
      { input: tiers.off.in, output: tiers.off.out, currency: rate.currency },
      t,
    )}`,
    `${t("shelf.peakHours")}: ${windowText(tiers.hours, t)} (${t("shelf.vendorTime", {
      offset: tzLabel(tiers.hours.tz_offset),
    })})`,
  ].join(" · ");
}

/** The switch a tiered price carries: it reads as the tier the cell is showing
    and shows the other one when clicked, so "what does this model cost at
    night?" is answered in place — the numbers beside it change.

    Clicking must not open the row's detail dialog, hence the stopPropagation:
    the whole row is a button. */
export function TierChip({
  rate,
  tier,
  onToggle,
  t,
}: {
  rate: NonNullable<CatalogEntry["price_ref"]>;
  tier: PriceTier;
  onToggle: () => void;
  t: Translate;
}) {
  const tiers = tiersOf(rate);
  if (!tiers) return null;
  const offPeak = tier === "offPeak";
  // Both states are marked, and by what they mean: the peak rate is the
  // expensive one, so it wears the palette's "mind this" amber — not the red,
  // which this app reserves for failures — and off-peak, the discount, wears
  // the kiwi green. A neutral chip said nothing about either.
  return (
    <button
      type="button"
      className="ml-1.5 flex-none cursor-pointer rounded px-1 font-mono text-[10px]"
      style={
        offPeak
          ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" }
          : { background: "var(--amber-soft)", color: "var(--amber)" }
      }
      title={`${tierDetail(rate, tiers, t)} · ${t("shelf.tierPriceSwitch")}`}
      aria-pressed={offPeak}
      onClick={(e) => {
        e.stopPropagation();
        onToggle();
      }}
    >
      {offPeak ? t("shelf.tierPriceOffPeak") : t("shelf.tierPricePeak")}
    </button>
  );
}

/** One model and the four rates a request against it can spend, in the currency
    the row is published in. The tier decides which numbers the first two are —
    the cache pair follows the same block, so a discounted schedule discounts
    them too. */
export function modelRateText(price: ModelPrice, tier: PriceTier, t: Translate): string {
  const cur = price.currency || "USD";
  const shown = tierRates(price, tier);
  const cache = tier === "offPeak" && price.off_peak ? price.off_peak : price;
  const fmt = (n: string) => fmtMoney(Number(n), cur);
  return [
    `${t("shelf.priceIn")} ${fmt(shown.input)}`,
    `${t("shelf.priceOut")} ${fmt(shown.output)}`,
    `${t("shelf.priceCacheRead")} ${fmt(cache.cache_read)}`,
    `${t("shelf.priceCacheWrite")} ${fmt(cache.cache_creation)}`,
  ].join(" · ");
}

/** A length band's rates, formatted exactly like the row's above them — same
    order, same wording, same currency. A band written differently from the line
    it sits under would read as a different kind of number. */
function bandRatesText(band: LongContextRates, currency: string, t: Translate): string {
  const fmt = (n: string) => fmtMoney(Number(n), currency);
  return [
    `${t("shelf.priceIn")} ${fmt(band.in)}`,
    `${t("shelf.priceOut")} ${fmt(band.out)}`,
    `${t("shelf.priceCacheRead")} ${fmt(band.cache_read)}`,
    `${t("shelf.priceCacheWrite")} ${fmt(band.cache_creation)}`,
  ].join(" · ");
}

/** The sentence that says a model's price steps up past an input size. */
export function bandText(band: LongContextRates, currency: string, t: Translate): string {
  return t("shelf.priceLongContext", {
    over: fmtTokens(band.over),
    rates: bandRatesText(band, currency, t),
  });
}

/** The price cell: the representative model's name and its in/out rates, or the
    provider's one-line description when it prices no model at all.

    The rates stay in the currency the entry publishes them in. Converting them
    would show a derived number that moves with a Settings choice and hides what
    the provider charges — the display currency is the Dashboard's, and the
    price a reader compares across providers has to be the price they pay. */
export function priceText(e: CatalogEntry, t: Translate, tier: PriceTier): string {
  const p = e.price_ref;
  if (!p) return e.desc ?? "";
  const rates = tierRates(p, tier);
  if (!Number.isFinite(Number(rates.input)) || !Number.isFinite(Number(rates.output))) {
    return e.desc ?? "";
  }
  return `${p.display_name} · ${rateText(rates, t)}`;
}

/** The price cell's text. A plan provider is bought rather than metered, so its
    `desc` leads — "from ¥49 /mo" is what the reader would actually pay — and
    the rates after it say what the same models cost per token. For every other
    billing mode the two are the same sentence, and this is `priceText`. */
export function rowPriceText(e: CatalogEntry, t: Translate, tier: PriceTier): string {
  const rate = priceText(e, t, tier);
  // Everything that is not a subscription shows its per-token rates — including
  // `both`, whose metered half is exactly what these figures are. Its other half
  // is a monthly fee the catalog does not carry.
  if (e.billing !== "plan") return rate;
  const offer = e.desc ?? "";
  // Equal when the entry prices no model and `desc` is all there is.
  return offer && offer !== rate ? `${offer} · ${rate}` : rate;
}
