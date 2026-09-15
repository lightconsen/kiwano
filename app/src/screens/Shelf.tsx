// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
import { useCallback, useEffect, useMemo, useState } from "react";
import { Boxes, Check, ChevronDown, ChevronRight, Gauge, Gift, House, Layers, RefreshCw, Search, ShieldCheck, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { api } from "../api/client";
import { useReload } from "../lib/reload";
import type { CatalogBilling, CatalogEntry, LongContextRates, ModelPrice, ProbeReport, Protocol } from "../api/types";
import { tzLabel, type PeakHours } from "../lib/peak";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { hubAssetUrl, useHubUrl } from "../lib/hub";
import { fmtMoney, fmtTokens } from "../lib/format";
import { useT, type KeyPath, type Messages, type Translate } from "../i18n";

/** The glyph each category wears, in the table and in the chip that filters for
    it — one map, so the badge and the filter can never show different icons.
    The table's legend is the filter row directly above it, which is why the
    column can get away with the icon alone.

    `local` has no chip (the shelf lists no local providers of its own) but is
    still a row that can appear. */
const TAG_ICON: Record<CatalogEntry["tag"], LucideIcon> = {
  official: ShieldCheck,
  aggregate: Layers,
  third: Boxes,
  free: Gift,
  local: House,
};

// Chip icons speak the category's meaning; colors match the per-tag badge
// palette used in the table (tagChipStyle below). The labels are keys, not
// text: every map below is module-level, where no hook can run.
const CHIPS: {
  id: "all" | CatalogEntry["tag"];
  labelKey: KeyPath<Messages>;
  icon_color?: string;
}[] = [
  { id: "all", labelKey: "shelf.chipAll" },
  { id: "official", labelKey: "shelf.chipOfficial", icon_color: "var(--kiwi)" },
  { id: "aggregate", labelKey: "shelf.chipAggregate", icon_color: "var(--violet)" },
  { id: "third", labelKey: "shelf.chipThird", icon_color: "var(--blue)" },
  { id: "free", labelKey: "shelf.chipFree", icon_color: "var(--amber)" },
];

function tagChipStyle(tag: CatalogEntry["tag"]): React.CSSProperties {
  switch (tag) {
    case "official":
      return { background: "var(--kiwi-soft)", color: "var(--kiwi)" };
    case "free":
      return { background: "var(--amber-soft)", color: "var(--amber)" };
    case "aggregate":
      return { background: "var(--violet-soft)", color: "var(--violet)" };
    case "third":
      return { background: "var(--blue-soft)", color: "var(--blue)" };
    case "local":
      return { background: "var(--surface2)", color: "var(--mut)" };
  }
}

const PROTO_STYLE: Record<Protocol, React.CSSProperties> = {
  anthropic: { background: "var(--orange-soft)", color: "var(--orange)" },
  openai: { background: "var(--surface2)", color: "var(--mut)" },
};

/** One letter per protocol. The column is 17% of a 1000px window for what is
    usually one or two words, while the price column beside it is the one that
    truncates — and a protocol set of two is a set a letter can name.

    The letter is not the whole answer: the chip keeps its colour, carries the
    full name as its hover title and its accessible name, and the detail dialog
    spells the protocols out. Sorting still uses the full string. */
const PROTO_ABBR: Record<Protocol, string> = {
  anthropic: "A",
  openai: "O",
};

/** The spelled-out name behind an abbreviation. Reused from the add form rather
    than duplicated: `Protocol` is one vocabulary across the two screens, and a
    second copy of it is a second thing to keep in step. */
const PROTO_LABEL: Record<Protocol, KeyPath<Messages>> = {
  anthropic: "addProvider.protoAnthropic",
  openai: "addProvider.protoOpenai",
};

const TAG_RANK: Record<CatalogEntry["tag"], number> = {
  official: 0,
  aggregate: 1,
  third: 2,
  free: 3,
  local: 4,
};

const BILLING_LABEL: Record<CatalogBilling, KeyPath<Messages>> = {
  plan: "shelf.billingPlan",
  payg: "shelf.billingPayg",
  unl: "shelf.billingUnl",
  // A vendor that charges both ways at one address. The column is 104px wide, so
  // this is the shortest honest phrasing: the row's price cell still shows the
  // metered rates (see `rowPriceText`).
  both: "shelf.billingBoth",
};

/** Hub catalogs may carry a billing tag this build predates: show the raw
    tag rather than a blank cell. */
function billingLabel(billing: CatalogBilling, t: Translate): string {
  // The lookup can miss at runtime despite the `Record` type: a newer Hub
  // catalog may name a billing tag this build has never heard of.
  return BILLING_LABEL[billing] ? t(BILLING_LABEL[billing]) : billing;
}

/** Where a billing mode sits in the column's order, most committed first: a plan
    is something the user signed up for, pay-as-you-go is metered, and unlimited
    has no meter to read.
 *
    `null` for a value with no single rank: a tag this build does not know, and
    `both` — a vendor that charges two ways is not "more" or "less" committed than
    one that charges one. Neither is a rank, so those rows sort after the three
    whichever way the arrow points, the way an unpriced row behaves in the price
    ordering. */
const BILLING_RANK: Record<string, number> = { plan: 0, payg: 1, unl: 2 };
function billingRank(billing: CatalogBilling): number | null {
  return BILLING_RANK[billing] ?? null;
}

/** The row's category badge, derived from `tag` rather than carried in the
    payload — the same keys the filter chips use, so a badge and the chip that
    filters for it always read alike. There is no chip for `local` (the shelf
    lists no local providers of its own), hence its own key. */
const TAG_LABEL: Record<CatalogEntry["tag"], KeyPath<Messages>> = {
  official: "shelf.chipOfficial",
  aggregate: "shelf.chipAggregate",
  third: "shelf.chipThird",
  free: "shelf.chipFree",
  local: "shelf.tagLocal",
};

function tagLabel(tag: CatalogEntry["tag"], t: Translate): string {
  return TAG_LABEL[tag] ? t(TAG_LABEL[tag]) : tag;
}

/** The category badge: its glyph in the tag's colour, with the name as the
    accessible text and the tooltip — icon-only would announce nothing. An
    unknown Hub tag falls back to the neutral glyph rather than nothing at
    all, which is the same posture `tagLabel` takes with its text. */
function TagBadge({ tag, t }: { tag: CatalogEntry["tag"]; t: Translate }) {
  const Icon = TAG_ICON[tag] ?? Boxes;
  const name = tagLabel(tag, t);
  return (
    <span
      className="inline-flex items-center rounded px-1.5 py-0.5"
      style={tagChipStyle(tag)}
      title={name}
    >
      <Icon className="size-3" aria-hidden />
      <span className="sr-only">{name}</span>
    </span>
  );
}

/** The document's weekday names on their dictionary keys. The schedule speaks
    `mon`/`tue`/…, which is not a label in any language. */
const DAY_LABEL: Record<string, KeyPath<Messages>> = {
  mon: "shelf.dayMon",
  tue: "shelf.dayTue",
  wed: "shelf.dayWed",
  thu: "shelf.dayThu",
  fri: "shelf.dayFri",
  sat: "shelf.daySat",
  sun: "shelf.daySun",
};

/** A price's time-of-day tiers, or null when it publishes none. The two parts
    are only meaningful together — a discount with no window is just a
    different price — which is why they are read as one thing. */
function tiersOf(rate: CatalogEntry["price_ref"]) {
  if (!rate?.off_peak || !rate.peak_hours) return null;
  return { off: rate.off_peak, hours: rate.peak_hours };
}

/** The peak hours as text: "Mon/Tue/Wed/Thu/Fri 09:00–12:00, …". */
function windowText(hours: PeakHours, t: Translate): string {
  return hours.windows
    .map(
      (w) =>
        `${w.days.map((d) => t(DAY_LABEL[d.trim().toLowerCase()] ?? "shelf.dayMon")).join("/")} ${w.start}–${w.end}`,
    )
    .join(t("shelf.windowSep"));
}

/// How many models the detail dialog lists before it offers to expand. The
/// catalog's own lists are short (a curated handful per endpoint); this is for
/// the day the Hub publishes a long one, and it keeps the sections below the
/// list reachable.
const MODEL_ROWS_SHOWN = 8;

/// Past this many models the section grows a filter of its own. Reading a list
/// stops working somewhere around here — that is the point at which a reader is
/// hunting for one model rather than taking in the set.
const MODEL_SEARCH_ABOVE = 10;

/// Which of a row's two prices its cell is showing. The row's own figures are
/// the **peak** ones (the published table's rule), so that is where every cell
/// starts.
type PriceTier = "peak" | "offPeak";

/** The rates to show for `tier`: the row's own (peak) figures, or the
    discounted pair. A row that publishes no schedule has only the one, which is
    why the off-peak answer is the peak one there. */
function tierRates(
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
function rateText(rate: { input: string; output: string; currency: string }, t: Translate): string {
  const cur = rate.currency || "USD";
  return `${t("shelf.priceIn")} ${fmtMoney(Number(rate.input), cur)} / ${t("shelf.priceOut")} ${fmtMoney(Number(rate.output), cur)}`;
}

/** The one line that says what a tiered price means: the peak rates, the
    off-peak rates, and when each applies — **on the vendor's clock**, named, so
    a reader is never left to assume their own. */
function tierDetail(
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
function TierChip({
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
function modelRateText(price: ModelPrice, tier: PriceTier, t: Translate): string {
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
function bandText(band: LongContextRates, currency: string, t: Translate): string {
  return t("shelf.priceLongContext", {
    over: fmtTokens(band.over),
    rates: bandRatesText(band, currency, t),
  });
}

/** One model of a provider: its id, its rates, and its own tier switch. The
    switch is per model because the schedule is — a provider may price one model
    by time of day and another flatly. */
function ModelPriceRow({
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

/** One protocol as a marked letter — see PROTO_ABBR for why, and for what
    carries the full name. */
function ProtoChip({ protocol, t }: { protocol: Protocol; t: Translate }) {
  const name = t(PROTO_LABEL[protocol]);
  return (
    <span
      className="rounded px-1.5 py-0.5 font-mono text-[10px]"
      style={PROTO_STYLE[protocol]}
      title={name}
    >
      {/* The letter is the visual stand-in, the name is what a screen reader
          reads — an `aria-label` on a bare span is not reliably announced, and
          a chip that announces "O" is worse than no chip at all. */}
      <span aria-hidden>{PROTO_ABBR[protocol]}</span>
      <span className="sr-only">{name}</span>
    </span>
  );
}

const SORT_KEYS = ["name", "billing", "price"] as const;
type SortKey = (typeof SORT_KEYS)[number];
type Sort = { key: SortKey; dir: 1 | -1 };

/** A remembered sort choice ("price:-1"), or null for the default order. A key
    this build does not know — a value written by a newer one — falls back to
    the default rather than sorting by nothing. */
function parseSort(v: string | null | undefined): Sort | null {
  if (!v) return null;
  const [key, dir] = v.split(":");
  if (!SORT_KEYS.includes(key as SortKey)) return null;
  const d = Number(dir);
  return d === 1 || d === -1 ? { key: key as SortKey, dir: d } : null;
}

const formatSort = (s: Sort | null): string | null => (s ? `${s.key}:${s.dir}` : null);

/** The comparable price of an entry: the representative model's input rate, as
    published. null means no price is published at all. */
function priceValue(e: CatalogEntry): number | null {
  const p = e.price_ref;
  if (!p) return null;
  const n = Number(p.input);
  return Number.isFinite(n) ? n : null;
}

/** The price cell: the representative model's name and its in/out rates, or the
    provider's one-line description when it prices no model at all.

    The rates stay in the currency the entry publishes them in. Converting them
    would show a derived number that moves with a Settings choice and hides what
    the provider charges — the display currency is the Dashboard's, and the
    price a reader compares across providers has to be the price they pay. */
function priceText(e: CatalogEntry, t: Translate, tier: PriceTier): string {
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
function rowPriceText(e: CatalogEntry, t: Translate, tier: PriceTier): string {
  const rate = priceText(e, t, tier);
  // Everything that is not a subscription shows its per-token rates — including
  // `both`, whose metered half is exactly what these figures are. Its other half
  // is a monthly fee the catalog does not carry.
  if (e.billing !== "plan") return rate;
  const offer = e.desc ?? "";
  // Equal when the entry prices no model and `desc` is all there is.
  return offer && offer !== rate ? `${offer} · ${rate}` : rate;
}

// Probe verdict chip: green=usable, amber=route exists but needs a key,
// gray=route missing, red=broken/unreachable
const PROBE_LABEL: Record<ProbeReport["verdict"], KeyPath<Messages>> = {
  ok: "shelf.probeOk",
  auth: "shelf.probeAuth",
  unsupported: "shelf.probeUnsupported",
  error: "shelf.probeError",
  unreachable: "shelf.probeUnreachable",
};

function probeChipStyle(verdict: ProbeReport["verdict"]): React.CSSProperties {
  switch (verdict) {
    case "ok":
      return { background: "oklch(0.72 0.15 155 / .14)", color: "oklch(0.62 0.15 155)" };
    case "auth":
      return { background: "oklch(0.8 0.13 85 / .14)", color: "oklch(0.7 0.13 85)" };
    case "unsupported":
      return { background: "var(--surface2)", color: "var(--mut)" };
    case "error":
    case "unreachable":
      return { background: "oklch(0.6 0.2 25 / .14)", color: "var(--red)" };
  }
}

/** One endpoint card of the detail dialog: URL + keyless protocol probe.
    A 401/403 still counts as "auth" — the route exists, so the protocol is
    supported even without a key. */
function EndpointCard({ protocol, endpoint, models }: { protocol: Protocol; endpoint: string; models: string[] }) {
  const t = useT();
  const [probe, setProbe] = useState<ProbeReport | "loading" | null>(null);
  return (
    <div className="rounded-md border border-line px-2.5 py-2">
      <div className="flex items-center gap-2">
        <span className="rounded px-1.5 py-0.5 font-mono text-[10px]" style={PROTO_STYLE[protocol]}>
          {protocol}
        </span>
        <span className="min-w-0 truncate font-mono text-[11px] text-mut" title={endpoint}>
          {endpoint.replace(/^https?:\/\//, "")}
        </span>
        <Button
          variant="outline"
          size="xs"
          className="ml-auto flex-none gap-1 rounded px-2 text-[10.5px]"
          disabled={probe === "loading"}
          onClick={() => {
            setProbe("loading");
            api
              .testEndpoint(protocol, endpoint)
              .then(setProbe)
              .catch(() =>
                setProbe({
                  verdict: "error",
                  status: null,
                  latency_ms: 0,
                  detail: t("shelf.probeFailed"),
                }),
              );
          }}
        >
          <Gauge className="h-3 w-3" />
          {t("shelf.test")}
        </Button>
      </div>
      {probe && probe !== "loading" && (
        <div className="mt-1.5 flex items-center gap-2">
          <span className="rounded px-1.5 text-[10px]" style={probeChipStyle(probe.verdict)}>
            {t(PROBE_LABEL[probe.verdict])}
          </span>
          <span className="min-w-0 truncate text-[10.5px] text-mut" title={probe.detail}>
            {probe.detail}
          </span>
          <span className="ml-auto flex-none font-mono text-[10.5px] text-mut">{probe.latency_ms}ms</span>
        </div>
      )}
      {models.length > 0 && (
        <div className="mt-1.5 flex flex-wrap gap-1">
          {models.map((m) => (
            <span key={m} className="rounded bg-surface2 px-1.5 py-0.5 font-mono text-[10px] text-mut">
              {m}
            </span>
          ))}
        </div>
      )}
    </div>
  );
}

const COLUMNS: { key: SortKey | null; labelKey: KeyPath<Messages>; className: string }[] = [
  { key: "name", labelKey: "shelf.colName", className: "w-[150px]" },
  { key: null, labelKey: "shelf.colProtocol", className: "w-[72px]" },
  { key: null, labelKey: "shelf.colCategory", className: "w-[48px]" },
  { key: "billing", labelKey: "shelf.colBilling", className: "w-[104px]" },
  { key: "price", labelKey: "shelf.colPrice", className: "" },
  { key: null, labelKey: "shelf.colActions", className: "w-[70px] text-right" },
];

/** The default order, used when the user has not picked a column: providers by
    how much they can be trusted first, then by name. Alphabetical alone put
    whichever aggregators happened to be called `9527code` / `a6api` on the first
    screen — a new user's whole impression of the catalog decided by naming luck. */
const byTagRank = (a: CatalogEntry, b: CatalogEntry): number =>
  TAG_RANK[a.tag] - TAG_RANK[b.tag] || a.name.localeCompare(b.name);

/** Own providers first — but only as a tie-break.
 *
 * The shelf lists the whole catalog, and the entries already wired into the
 * local setup are the ones a reader is most often looking for, which is why they
 * lead the default order. They used to be pinned above *every* ordering, applied
 * as a pass after the column sort; because `sort` is stable that did not reorder
 * the list but split it into two runs, each internally sorted, and concatenated
 * them. Sorting by Billing then read Plan, Plan, Pay as you go, Unlimited, Pay as
 * you go… — a column that looks broken, and the same trick hid behind Price,
 * where a pinned expensive provider floated above cheaper ones.
 *
 * As a tie-break it costs nothing and still helps: prices repeat across the
 * catalog (the same model resold at the same rate), and there own-first is a
 * sensible way to order two rows the column cannot separate.
 */
const byAdded = (a: CatalogEntry, b: CatalogEntry): number => Number(b.added) - Number(a.added);

/** Rows in the user's chosen order (or the default one when they have not
    chosen). Sorting by price needs the currency conversion, and a row with no
    published price has no place in a price ordering — it stays last whichever
    way the arrow points, rather than the arrow flipping it to the top. */
function orderRows(rows: CatalogEntry[], sort: Sort | null): CatalogEntry[] {
  // Nothing picked: trust first, own providers above them.
  if (!sort) return [...rows].sort((a, b) => byAdded(a, b) || byTagRank(a, b));
  // Destructured so the narrowing survives into the comparator closure.
  const { key, dir } = sort;
  if (key === "name") {
    return [...rows].sort((a, b) => a.name.localeCompare(b.name) * dir || byAdded(a, b));
  }
  if (key === "billing") {
    return [...rows].sort((a, b) => {
      const ra = billingRank(a.billing);
      const rb = billingRank(b.billing);
      if (ra === null || rb === null) {
        if (ra === null && rb === null) return byAdded(a, b) || a.name.localeCompare(b.name);
        return ra === null ? 1 : -1;
      }
      // Within one mode by name, unflipped: the mode is what the column ranks,
      // and flipping it should not also reverse the names inside each group.
      return (ra - rb) * dir || byAdded(a, b) || a.name.localeCompare(b.name);
    });
  }
  return rows
    .map((e) => ({ e, p: priceValue(e) }))
    .sort((a, b) => {
      if (a.p === null || b.p === null) {
        if (a.p === null && b.p === null) return byAdded(a.e, b.e) || byTagRank(a.e, b.e);
        return a.p === null ? 1 : -1;
      }
      return (a.p - b.p) * dir || byAdded(a.e, b.e);
    })
    .map((x) => x.e);
}

// ── the view grouped by model ────────────────────────────────────────────

type ShelfView = "provider" | "model";

const VIEWS: { id: ShelfView; labelKey: KeyPath<Messages> }[] = [
  { id: "provider", labelKey: "shelf.viewByProvider" },
  { id: "model", labelKey: "shelf.viewByModel" },
];

/** The remembered view, defaulting to the provider table — and to it for any
    value this build does not know, the way `parseSort` treats an unknown sort
    key. */
function parseView(v: string | null | undefined): ShelfView {
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
function modelRate(e: CatalogEntry, model: string) {
  return e.price_ref && e.price_ref.model_id === model ? e.price_ref : null;
}

type ModelGroupRow = {
  entry: CatalogEntry;
  rate: NonNullable<CatalogEntry["price_ref"]> | null;
  /** Input rate in the display currency, or null when this provider publishes
      none for this model — the sort key, and the range's raw material. */
  value: number | null;
};

type ModelGroup = {
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
function buildModelGroups(entries: CatalogEntry[], query: string): ModelGroup[] {
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
function groupPriceText(rate: ModelGroupRow["rate"], t: Translate, tier: PriceTier): string {
  if (!rate) return t("common.none");
  const rates = tierRates(rate, tier);
  if (!Number.isFinite(Number(rates.input)) || !Number.isFinite(Number(rates.output))) {
    return t("common.none");
  }
  return rateText(rates, t);
}

function Row({
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

/** Model detail dialog: clicking a catalog row opens the full listing —
    every per-protocol endpoint with its models, pricing and an Add shortcut. */
function DetailDialog({
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

/** The list's remembered preferences: the column sort and the currency to price
    in. Both come from the settings blob, loaded once per screen; a save is
    fire-and-forget so a failed write never blocks the click that caused it. */
function useShelfPrefs() {
  const [sort, setSort] = useState<Sort | null>(null);
  const [view, setView] = useState<ShelfView>("provider");
  useEffect(() => {
    let alive = true;
    api
      .getSettings()
      .then((s) => {
        if (!alive) return;
        setSort(parseSort(s.shelf_sort));
        setView(parseView(s.shelf_view));
      })
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);
  const remember = (s: Sort | null) => {
    setSort(s);
    api.updateSettings({ shelf_sort: formatSort(s) }).catch(() => {});
  };
  // Persisted, not just held: App remounts the shelf after every mutation, so a
  // view in component state would snap back to the provider table the moment
  // the user added a provider while browsing models.
  const rememberView = (v: ShelfView) => {
    setView(v);
    api.updateSettings({ shelf_view: v }).catch(() => {});
  };
  return { sort, view, remember, rememberView };
}

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const t = useT();
  const hubUrl = useHubUrl();
  const { sort, view, remember, rememberView } = useShelfPrefs();
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [prices, setPrices] = useState<ModelPrice[]>([]);
  const [chip, setChip] = useState<(typeof CHIPS)[number]["id"]>("all");
  const [query, setQuery] = useState("");
  // Catalog row clicked → model detail dialog
  const [detail, setDetail] = useState<{ entry: CatalogEntry; tier: PriceTier } | null>(null);
  // Hub refresh: "busy" spins the glyph, "done" flashes a check. The header
  // bar has no room for a toast, so success is the icon and only a failure
  // takes up words.
  const [hub, setHub] = useState<"idle" | "busy" | "done">("idle");
  const [hubErr, setHubErr] = useState<string | null>(null);

  // Everything this page reads, in one handle the app's own status-bar reload
  // can await (lib/reload.ts): the catalog and the price mirror behind the
  // per-model view. What is *not* in here is the page's own state — the chip,
  // the search box, the sort — which is the point of reloading over remounting.
  const reload = useCallback(() => {
    const catalog = api.listCatalog().then(setCatalog);
    // The price mirror is a second, smaller read: the catalog names one
    // representative price per provider, and the dialog's per-model list needs
    // the rest of them.
    const prices = api
      .listModelPrices()
      .then(setPrices)
      .catch(() => {});
    return Promise.all([catalog, prices]);
  }, []);
  useEffect(() => {
    void reload();
  }, [reload]);
  useReload(reload);

  /** Pull the Hub catalog into the local cache, then re-read it. The sync is
      conditional on the Hub's side (manifest sha): a Hub that has not changed
      is still a successful check, it just skips the download. */
  const refreshFromHub = () => {
    setHub("busy");
    setHubErr(null);
    api
      .syncHub()
      .then(() => api.listCatalog())
      .then((c) => {
        setCatalog(c);
        setHub("done");
      })
      .catch((e) => {
        setHubErr(String(e));
        setHub("idle");
      });
  };

  // The check is an acknowledgement, not a state to read: fall back to the
  // refresh glyph. An error has no such timer — it holds until the next try.
  useEffect(() => {
    if (hub !== "done") return;
    // Named `timer`, not `t`: `t` is the translator here.
    const timer = setTimeout(() => setHub("idle"), 1600);
    return () => clearTimeout(timer);
  }, [hub]);

  // The category filter, shared by both views so neither can drift from the
  // chips: the provider view filters on names afterwards, the grouped view
  // builds its groups from these rows.
  const chipRows = useMemo(
    () => catalog?.entries.filter((e) => chip === "all" || e.tag === chip) ?? [],
    [catalog, chip],
  );

  const filtered = useMemo(() => {
    const rows = chipRows.filter((e) => e.name.toLowerCase().includes(query.trim().toLowerCase()));
    return orderRows(rows, sort);
  }, [chipRows, query, sort]);

  const groups = useMemo(() => buildModelGroups(chipRows, query), [chipRows, query]);

  // Collapsed models, by id. Held here rather than in the group rows because a
  // group is a <tbody>, not a component — and deliberately not persisted: it is
  // a reading position, not a preference.
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());
  const toggleGroup = (model: string) =>
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (!next.delete(model)) next.add(model);
      return next;
    });

  /** The grouped body under the current sort: a sort orders the providers
      *inside* each group and never the groups themselves.

      A view where a column reorders the models would need the columns to
      describe models, and they do not — protocol, category, billing and name
      are the provider's, which is why they mean the same thing in both views.
      The model order stays the grouping's own: models somebody prices first,
      then the widest, then by name. Without a sort the rows keep the order
      `buildModelGroups` gave them, cheapest first — the reading this view
      exists for. */
  const orderedGroups = useMemo(() => {
    if (!sort) return groups;
    // Destructured so the narrowing survives into the comparator closure.
    const { key, dir } = sort;
    const rowCmp = (a: ModelGroupRow, b: ModelGroupRow): number => {
      if (key === "price") {
        // A row with no price has no place in a price ordering: last, whichever
        // way the arrow points — "no price" is a category, not the cheapest one.
        if (a.value === null || b.value === null) {
          if (a.value === null && b.value === null) return byTagRank(a.entry, b.entry);
          return a.value === null ? 1 : -1;
        }
        return (a.value - b.value) * dir;
      }
      return a.entry.name.localeCompare(b.entry.name) * dir;
    };
    return groups.map((g) => ({ ...g, rows: [...g.rows].sort(rowCmp) }));
  }, [groups, sort]);

  if (!catalog)
    return <div className="p-8 text-center text-[12px] text-mut">{t("common.loading")}</div>;

  // Three-state: ascending → descending → back to the default order.
  const toggleSort = (key: SortKey) =>
    remember(sort?.key === key ? (sort.dir === 1 ? { key, dir: -1 } : null) : { key, dir: 1 });

  /** An empty catalog is not a failed search: there is no bundled fallback any
      more, so a machine that has never synced sees that message instead — and
      "no matching providers" would send the user off to rewrite a search term
      that was never wrong. */
  const emptyLabel = catalog.entries.length === 0 ? t("shelf.notSynced") : t("shelf.noMatches");

  return (
    <section>
      <div className="sticky top-0 z-20 flex h-11 items-center gap-2 border-b border-line bg-bg px-4">
        {/* No "From Kiwano Hub · N providers" line: the refresh button beside
            the search already says where the list comes from, and the count was
            a width the chips and the view toggle can use. */}
        <div className="flex gap-1.5">
          {CHIPS.map((c) => {
            // The chip's glyph is the tag's own, so the filter and the badge in
            // the column below can never drift apart.
            const Icon = c.id === "all" ? null : TAG_ICON[c.id];
            return (
              <button
                key={c.id}
                className={`chip inline-flex h-[26px] items-center gap-1 rounded-full border border-line px-2.5 text-[11.5px]${chip === c.id ? " active" : " text-mut"}`}
                onClick={() => setChip(c.id)}
              >
                {Icon && <Icon className="size-3" style={{ color: c.icon_color }} aria-hidden />}
                {t(c.labelKey)}
              </button>
            );
          })}
        </div>
        <div className="ml-auto flex items-center gap-2">
          {/* Two readings of one catalog: rows are providers, or rows are the
              models those providers serve. A segment group rather than a
              dropdown: there are exactly two, both fit on the row, and this is
              the control the screen is flipped with most — so it should cost
              one click and name the other reading without being opened. Same
              markup as the Dashboard's window and metric groups. */}
          <div className="flex flex-none overflow-hidden rounded-lg border border-line text-[12px]">
            {VIEWS.map((v, i) => (
              <button
                key={v.id}
                className={`seg h-7 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${view === v.id ? " active" : ""}`}
                onClick={() => rememberView(v.id)}
              >
                {t(v.labelKey)}
              </button>
            ))}
          </div>
          {/* A failed sync is worth the room it takes; the search box shifts
              left to make it, which is the point. */}
          {hubErr && (
            <span
              className="max-w-[220px] truncate text-[11px]"
              style={{ color: "var(--red)" }}
              title={hubErr}
            >
              {hubErr}
            </span>
          )}
          <div className="relative">
            <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-mut" />
            <input
              className="h-7 w-[190px] rounded-md border border-line bg-surface pl-7 pr-2 text-[12px]"
              placeholder={t("shelf.searchPlaceholder")}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
            />
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="h-7 w-7 flex-none px-0 text-mut"
            aria-label={t("shelf.refreshAria")}
            title={t("shelf.refreshTitle")}
            disabled={hub === "busy"}
            onClick={refreshFromHub}
          >
            {hub === "done" ? (
              <Check className="h-3.5 w-3.5" style={{ color: "var(--kiwi)" }} />
            ) : (
              <RefreshCw className={`h-3.5 w-3.5${hub === "busy" ? " animate-spin" : ""}`} />
            )}
          </Button>
        </div>
      </div>
      <table className="w-full border-collapse">
        <thead className="sticky top-11 z-10 bg-surface">
          <tr className="border-b border-line">
            {COLUMNS.map((c) => (
              <th
                key={c.labelKey}
                className={`px-2 py-1.5 text-left text-[11px] font-medium text-mut ${c.className} ${c.key ? "cursor-pointer select-none hover:text-foreground" : ""} ${c.key === "name" ? "pl-4" : ""} ${c.key === null && c.labelKey === "shelf.colActions" ? "pr-4" : ""}`}
                onClick={c.key ? () => toggleSort(c.key!) : undefined}
              >
                {t(c.labelKey)}
                {sort?.key === c.key && (
                  <span className="ml-0.5 text-[9px]">{sort.dir === 1 ? "▲" : "▼"}</span>
                )}
              </th>
            ))}
          </tr>
        </thead>
        {view === "provider" ? (
          <tbody>
            {filtered.map((e) => (
              <Row
                key={e.id}
                entry={e}
                hubUrl={hubUrl}
                onAdd={onAdd}
                onOpen={(entry, tier) => setDetail({ entry, tier })}
              />
            ))}
            {filtered.length === 0 && (
              <tr>
                <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-[12px] text-mut">
                  {emptyLabel}
                </td>
              </tr>
            )}
          </tbody>
        ) : (
          <>
            {orderedGroups.map((g) => {
              const range = g.range;
              const open = !collapsed.has(g.model);
              return (
                <tbody key={g.model}>
                  <tr className="border-t border-line" style={{ background: "var(--surface2)" }}>
                    {/* colgroup, not a stray cell: this header is what the
                        providers below it are grouped by. A bare `th` is bold
                        and centred, hence the explicit left/normal — and the
                        whole row is the collapse control, so it is a button
                        inside the header rather than a click on the header
                        itself (which is not focusable). */}
                    <th scope="colgroup" colSpan={COLUMNS.length} className="p-0 text-left">
                      <button
                        type="button"
                        className="flex w-full items-center gap-2 px-4 py-1.5 text-left text-[11.5px] font-semibold"
                        aria-expanded={open}
                        onClick={() => toggleGroup(g.model)}
                      >
                        {open ? (
                          <ChevronDown className="size-3.5 flex-none text-mut" aria-hidden />
                        ) : (
                          <ChevronRight className="size-3.5 flex-none text-mut" aria-hidden />
                        )}
                        <span className="truncate">{g.name}</span>
                        {g.name !== g.model && (
                          <span className="truncate font-mono text-[10.5px] font-normal text-mut">
                            {g.model}
                          </span>
                        )}
                        <span className="font-normal text-mut">
                          {t(g.rows.length === 1 ? "shelf.groupMetaOne" : "shelf.groupMeta", {
                            n: g.rows.length,
                            m: g.prices.length,
                          })}
                        </span>
                        {range && (
                          <span
                            className="ml-auto flex-none font-mono font-normal"
                            title={t("shelf.groupPriceTitle")}
                          >
                            {range}
                          </span>
                        )}
                      </button>
                    </th>
                  </tr>
                  {open &&
                    g.rows.map((r) => (
                      <Row
                        key={r.entry.id}
                        entry={r.entry}
                        hubUrl={hubUrl}
                        onAdd={onAdd}
                        onOpen={(entry, tier) => setDetail({ entry, tier })}
                        modelId={g.model}
                        indent
                      />
                    ))}
                </tbody>
              );
            })}
            {groups.length === 0 && (
              <tbody>
                <tr>
                  <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-[12px] text-mut">
                    {emptyLabel}
                  </td>
                </tr>
              </tbody>
            )}
          </>
        )}
      </table>
      {detail && (
        <DetailDialog
          entry={detail.entry}
          hubUrl={hubUrl}
          tier={detail.tier}
          prices={prices}
          onClose={() => setDetail(null)}
          onAdd={(e) => {
            setDetail(null);
            onAdd(e);
          }}
        />
      )}
    </section>
  );
}
