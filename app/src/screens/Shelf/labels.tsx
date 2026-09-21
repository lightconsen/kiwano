// The shelf's label vocabulary: the maps the row's badges, the billing column
// and the price lines read, and the category badge itself. All module-level, so
// every map holds a dictionary key rather than text and each reader resolves it
// with its own `t` — see `screens/Providers/agents.tsx` for the same shape.
import { Boxes, Gift, House, Layers, ShieldCheck, type LucideIcon } from "lucide-react";
import { type KeyPath, type Messages, type Translate } from "../../i18n";
import type { CatalogBilling, CatalogEntry, Protocol } from "../../api/types";
import { type PeakHours } from "../../lib/peak";

/** The glyph each category wears, in the table and in the chip that filters for
    it — one map, so the badge and the filter can never show different icons.
    The table's legend is the filter row directly above it, which is why the
    column can get away with the icon alone.

    `local` has no chip (the shelf lists no local providers of its own) but is
    still a row that can appear. */
export const TAG_ICON: Record<CatalogEntry["tag"], LucideIcon> = {
  official: ShieldCheck,
  aggregate: Layers,
  third: Boxes,
  free: Gift,
  local: House,
};

// Chip icons speak the category's meaning; colors match the per-tag badge
// palette used in the table (tagChipStyle below). The labels are keys, not
// text: every map below is module-level, where no hook can run.
export const CHIPS: {
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

export function tagChipStyle(tag: CatalogEntry["tag"]): React.CSSProperties {
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

export const PROTO_STYLE: Record<Protocol, React.CSSProperties> = {
  anthropic: { background: "var(--orange-soft)", color: "var(--orange)" },
  openai: { background: "var(--surface2)", color: "var(--mut)" },
  gemini: { background: "var(--indigo-soft)", color: "var(--indigo)" },
};

/** One letter per protocol. The column is 17% of a 1000px window for what is
    usually one or two words, while the price column beside it is the one that
    truncates — and a protocol set of a few is a set a letter can name.

    The letter is not the whole answer: the chip keeps its colour, carries the
    full name as its hover title and its accessible name, and the detail dialog
    spells the protocols out. Sorting still uses the full string. */
export const PROTO_ABBR: Record<Protocol, string> = {
  anthropic: "A",
  openai: "O",
  gemini: "G",
};

/** The spelled-out name behind an abbreviation. Reused from the add form rather
    than duplicated: `Protocol` is one vocabulary across the two screens, and a
    second copy of it is a second thing to keep in step. */
export const PROTO_LABEL: Record<Protocol, KeyPath<Messages>> = {
  anthropic: "addProvider.protoAnthropic",
  openai: "addProvider.protoOpenai",
  gemini: "addProvider.protoGemini",
};

export const TAG_RANK: Record<CatalogEntry["tag"], number> = {
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
export function billingLabel(billing: CatalogBilling, t: Translate): string {
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
export function billingRank(billing: CatalogBilling): number | null {
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

export function tagLabel(tag: CatalogEntry["tag"], t: Translate): string {
  return TAG_LABEL[tag] ? t(TAG_LABEL[tag]) : tag;
}

/** The category badge: its glyph in the tag's colour, with the name as the
    accessible text and the tooltip — icon-only would announce nothing. An
    unknown Hub tag falls back to the neutral glyph rather than nothing at
    all, which is the same posture `tagLabel` takes with its text. */
export function TagBadge({ tag, t }: { tag: CatalogEntry["tag"]; t: Translate }) {
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
export function tiersOf(rate: CatalogEntry["price_ref"]) {
  if (!rate?.off_peak || !rate.peak_hours) return null;
  return { off: rate.off_peak, hours: rate.peak_hours };
}

/** The peak hours as text: "Mon/Tue/Wed/Thu/Fri 09:00–12:00, …". */
export function windowText(hours: PeakHours, t: Translate): string {
  return hours.windows
    .map(
      (w) =>
        `${w.days.map((d) => t(DAY_LABEL[d.trim().toLowerCase()] ?? "shelf.dayMon")).join("/")} ${w.start}–${w.end}`,
    )
    .join(t("shelf.windowSep"));
}
