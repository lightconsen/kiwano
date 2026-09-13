// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
import { useEffect, useMemo, useState } from "react";
import { Boxes, Check, ChevronDown, ChevronRight, Gauge, Gift, House, Layers, RefreshCw, Search, ShieldCheck, type LucideIcon } from "lucide-react";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogFooter,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import type { Billing, CatalogEntry, ProbeReport, Protocol } from "../api/types";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { hubAssetUrl, useHubUrl } from "../lib/hub";
import { convertAmount, fmtMoney } from "../lib/format";
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
  gemini: { background: "var(--indigo-soft)", color: "var(--indigo)" },
};

/** One letter per protocol. The column is 17% of a 1000px window for what is
    usually one or two words, while the price column beside it is the one that
    truncates — and a protocol set of exactly three is a set a letter can name.

    The letter is not the whole answer: the chip keeps its colour, carries the
    full name as its hover title and its accessible name, and the detail dialog
    spells the protocols out. Sorting still uses the full string. */
const PROTO_ABBR: Record<Protocol, string> = {
  anthropic: "A",
  openai: "O",
  gemini: "G",
};

/** The spelled-out name behind an abbreviation. Reused from the add form rather
    than duplicated: `Protocol` is one vocabulary across the two screens, and a
    second copy of it is a second thing to keep in step. */
const PROTO_LABEL: Record<Protocol, KeyPath<Messages>> = {
  anthropic: "addProvider.protoAnthropic",
  openai: "addProvider.protoOpenai",
  gemini: "addProvider.protoGemini",
};

const TAG_RANK: Record<CatalogEntry["tag"], number> = {
  official: 0,
  aggregate: 1,
  third: 2,
  free: 3,
  local: 4,
};

const BILLING_LABEL: Record<Billing, KeyPath<Messages>> = {
  plan: "shelf.billingPlan",
  payg: "shelf.billingPayg",
  unl: "shelf.billingUnl",
};

/** Sort order for the billing column — see `compare`. */
const BILLING_RANK: Record<Billing, number> = { plan: 0, payg: 1, unl: 2 };

/** Hub catalogs may carry a billing tag this build predates: show the raw
    tag rather than a blank cell. */
function billingLabel(billing: Billing, t: Translate): string {
  // The lookup can miss at runtime despite the `Record` type: a newer Hub
  // catalog may name a billing tag this build has never heard of.
  return BILLING_LABEL[billing] ? t(BILLING_LABEL[billing]) : billing;
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

/** The currency to price in, and the rates to get there. An empty rate table
    converts nothing, which is also `convertAmount`'s behaviour for a code it
    has no rate for. */
type Money = { rates: Record<string, number>; to: string };

const SORT_KEYS = ["name", "protocol", "tag", "billing", "price"] as const;
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

/** The comparable price of an entry: the representative model's input rate, in
    the display currency. null means no price is published at all. */
function priceValue(e: CatalogEntry, money: Money): number | null {
  const p = e.price_ref;
  if (!p) return null;
  const n = Number(p.input);
  return Number.isFinite(n) ? convertAmount(n, p.currency || "USD", money.to, money.rates) : null;
}

/** The price cell: the representative model's name and its in/out rates, or the
    provider's one-line description when it prices no model at all. */
function priceText(e: CatalogEntry, money: Money): string {
  const p = e.price_ref;
  if (!p) return e.desc ?? "";
  const inRate = Number(p.input);
  const outRate = Number(p.output);
  if (!Number.isFinite(inRate) || !Number.isFinite(outRate)) return e.desc ?? "";
  const cur = p.currency || "USD";
  const conv = (n: number) => fmtMoney(convertAmount(n, cur, money.to, money.rates), money.to);
  return `${p.display_name} · ${conv(inRate)} / ${conv(outRate)}`;
}

/** The price cell's text. A plan provider is bought rather than metered, so its
    `desc` leads — "from ¥49 /mo" is what the reader would actually pay — and
    the rates after it say what the same models cost per token. For every other
    billing mode the two are the same sentence, and this is `priceText`. */
function rowPriceText(e: CatalogEntry, money: Money): string {
  const rate = priceText(e, money);
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
  { key: "protocol", labelKey: "shelf.colProtocol", className: "w-[72px]" },
  { key: "tag", labelKey: "shelf.colCategory", className: "w-[48px]" },
  { key: "billing", labelKey: "shelf.colBilling", className: "w-[104px]" },
  { key: "price", labelKey: "shelf.colPrice", className: "" },
  { key: null, labelKey: "shelf.colActions", className: "w-[70px] text-right" },
];

/** The default order, used when the user has not picked a column: providers by
    how much they can be trusted first, then by name. Alphabetical alone put
    whichever aggregators happened to be called `9527code` / `a6api` on the first
    screen — a new user's whole impression of the catalog decided by naming luck.
    An explicit column sort replaces this rank, never the `added` pin below. */
const byTagRank = (a: CatalogEntry, b: CatalogEntry): number =>
  TAG_RANK[a.tag] - TAG_RANK[b.tag] || a.name.localeCompare(b.name);

function compare(key: Exclude<SortKey, "price">, a: CatalogEntry, b: CatalogEntry): number {
  switch (key) {
    case "name":
      return a.name.localeCompare(b.name);
    case "protocol":
      return a.protocol.localeCompare(b.protocol) || a.name.localeCompare(b.name);
    case "tag":
      return TAG_RANK[a.tag] - TAG_RANK[b.tag] || a.name.localeCompare(b.name);
    case "billing":
      // Ranked, not alphabetical: a subscription is the thing you decide to
      // buy, so sorting this column gathers the plans at the top. A tag this
      // build has never seen sorts last rather than into the middle.
      return (
        (BILLING_RANK[a.billing] ?? 9) - (BILLING_RANK[b.billing] ?? 9) ||
        a.name.localeCompare(b.name)
      );
  }
}

/** Rows in the user's chosen order (or the default one when they have not
    chosen). Sorting by price needs the currency conversion, and a row with no
    published price has no place in a price ordering — it stays last whichever
    way the arrow points, rather than the arrow flipping it to the top. */
function orderRows(rows: CatalogEntry[], sort: Sort | null, money: Money): CatalogEntry[] {
  if (!sort) return [...rows].sort(byTagRank);
  // Destructured so the narrowing survives into the comparator closure.
  const { key, dir } = sort;
  if (key !== "price") return [...rows].sort((a, b) => compare(key, a, b) * dir);
  return rows
    .map((e) => ({ e, p: priceValue(e, money) }))
    .sort((a, b) => {
      if (a.p === null || b.p === null) {
        if (a.p === null && b.p === null) return byTagRank(a.e, b.e);
        return a.p === null ? 1 : -1;
      }
      return (a.p - b.p) * dir;
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
  /** The priced rows' input rates, formatted and cheapest first: the header's
      count and its range. */
  prices: string[];
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
function buildModelGroups(entries: CatalogEntry[], money: Money, query: string): ModelGroup[] {
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
        const converted = rate
          ? convertAmount(Number(rate.input), rate.currency || "USD", money.to, money.rates)
          : null;
        return { entry: e, rate, value: converted !== null && Number.isFinite(converted) ? converted : null };
      })
      .sort((a, b) => {
        if (a.value === null || b.value === null) {
          if (a.value === null && b.value === null) return byTagRank(a.entry, b.entry);
          return a.value === null ? 1 : -1;
        }
        return a.value - b.value || byTagRank(a.entry, b.entry);
      });
    if (rows.length < (q === "" ? 2 : 1)) continue;
    const prices = rows.flatMap((r) =>
      r.rate
        ? [
            fmtMoney(
              convertAmount(Number(r.rate.input), r.rate.currency || "USD", money.to, money.rates),
              money.to,
            ),
          ]
        : [],
    );
    groups.push({ model, name, rows, prices });
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
function groupPriceText(rate: ModelGroupRow["rate"], money: Money, t: Translate): string {
  if (!rate) return t("common.none");
  const inRate = Number(rate.input);
  const outRate = Number(rate.output);
  if (!Number.isFinite(inRate) || !Number.isFinite(outRate)) return t("common.none");
  const conv = (n: number) => fmtMoney(convertAmount(n, rate.currency || "USD", money.to, money.rates), money.to);
  return `${conv(inRate)} / ${conv(outRate)}`;
}

/** The header's price range, shown only when the rows disagree: equal prices
    are already stated by every row below, and `¥36 – ¥36` is noise. Compared as
    formatted strings, so two currencies that print alike count as equal. */
function groupPriceRange(g: ModelGroup): string | null {
  if (new Set(g.prices).size < 2) return null;
  return `${g.prices[0]} – ${g.prices[g.prices.length - 1]}`;
}

function Row({
  entry,
  hubUrl,
  money,
  onAdd,
  onOpen,
  modelId,
  indent,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  money: Money;
  onAdd: (e: CatalogEntry) => void;
  onOpen: (e: CatalogEntry) => void;
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
  const price = modelId
    ? groupPriceText(modelRate(entry, modelId), money, t)
    : rowPriceText(entry, money);
  return (
    <tr className="cursor-pointer border-t border-line hover:bg-surface2" onClick={() => onOpen(entry)}>
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
        <span className="block truncate" title={price}>
          {price}
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
  money,
  onClose,
  onAdd,
}: {
  entry: CatalogEntry;
  hubUrl: string | null;
  money: Money;
  onClose: () => void;
  onAdd: (e: CatalogEntry) => void;
}) {
  const t = useT();
  const logo = entry.logo && hubUrl ? hubAssetUrl(hubUrl, entry.logo) : undefined;
  const price = priceText(entry, money);
  const endpoints = [
    { protocol: entry.protocol, endpoint: entry.endpoint, models: entry.models },
    ...(entry.endpoints ?? []),
  ];
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

          <div className="mt-2.5 flex gap-4 text-[11.5px] text-mut">
            <span>
              {t("shelf.billing")}{" "}
              <span className="text-ink">{billingLabel(entry.billing, t)}</span>
            </span>
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
  const [money, setMoney] = useState<Money>({ rates: {}, to: "USD" });
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
    api
      .getCurrencyMeta()
      .then((m) => alive && setMoney({ rates: m.exchange_rates, to: m.preferred }))
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
  return { sort, money, view, remember, rememberView };
}

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const t = useT();
  const hubUrl = useHubUrl();
  const { sort, money, view, remember, rememberView } = useShelfPrefs();
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [chip, setChip] = useState<(typeof CHIPS)[number]["id"]>("all");
  const [query, setQuery] = useState("");
  // Catalog row clicked → model detail dialog
  const [detail, setDetail] = useState<CatalogEntry | null>(null);
  // Hub refresh: "busy" spins the glyph, "done" flashes a check. The header
  // bar has no room for a toast, so success is the icon and only a failure
  // takes up words.
  const [hub, setHub] = useState<"idle" | "busy" | "done">("idle");
  const [hubErr, setHubErr] = useState<string | null>(null);

  useEffect(() => {
    api.listCatalog().then(setCatalog);
  }, []);

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
    // Added (or connected) providers pin to the top of any ordering — they are
    // the ones already wired into the local setup. The sort is stable, so a
    // picked column keeps its order inside each group.
    return orderRows(rows, sort, money).sort((a, b) => Number(b.added) - Number(a.added));
  }, [chipRows, query, sort, money]);

  const groups = useMemo(() => buildModelGroups(chipRows, money, query), [chipRows, money, query]);

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

  /** The grouped body under the current sort.
      The columns describe *rows* — a protocol, a category, a billing mode are
      the provider's — so a click orders the providers inside every group. Two
      of them also say something about a group, and those order the groups
      themselves: a name sort makes the model list itself alphabetical, and a
      price sort puts the models with the cheapest entry point first. The other
      columns leave the groups in `buildModelGroups`' order, because a group of
      providers has no single protocol or category to sort by.
      Without a sort, groups keep that order and rows stay cheapest first — the
      reading this view exists for. */
  const orderedGroups = useMemo(() => {
    if (!sort) return groups;
    // Destructured so the narrowing survives into the comparator closures.
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
      return compare(key, a.entry, b.entry) * dir;
    };
    const cheapest = (g: ModelGroup) =>
      g.rows.reduce((min, r) => (r.value !== null && r.value < min ? r.value : min), Number.POSITIVE_INFINITY);
    const groupCmp = (a: ModelGroup, b: ModelGroup): number => {
      if (key === "name") return a.name.localeCompare(b.name) * dir;
      if (key === "price") {
        const av = cheapest(a);
        const bv = cheapest(b);
        // Same rule as a row: a group nobody prices goes last both ways rather
        // than reading as free. `Infinity - Infinity` is NaN, hence the guards.
        if (!Number.isFinite(av) || !Number.isFinite(bv)) {
          return Number.isFinite(av) ? -1 : Number.isFinite(bv) ? 1 : 0;
        }
        return (av - bv) * dir;
      }
      return 0; // the group order is the grouping's own
    };
    // `sort` is stable, so the groups this comparator cannot order stay in the
    // order the grouping built.
    return [...groups]
      .sort(groupCmp)
      .map((g) => ({ ...g, rows: [...g.rows].sort(rowCmp) }));
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
              models those providers serve. */}
          <Select value={view} onValueChange={(v) => rememberView(parseView(v))}>
            <SelectTrigger className="h-7 w-[132px] flex-none bg-surface text-[11.5px]">
              <SelectValue>{(v) => t(VIEWS.find((x) => x.id === v)?.labelKey ?? VIEWS[0].labelKey)}</SelectValue>
            </SelectTrigger>
            <SelectContent>
              {VIEWS.map((v) => (
                <SelectItem key={v.id} value={v.id}>
                  {t(v.labelKey)}
                </SelectItem>
              ))}
            </SelectContent>
          </Select>
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
              <Row key={e.id} entry={e} hubUrl={hubUrl} money={money} onAdd={onAdd} onOpen={setDetail} />
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
              const range = groupPriceRange(g);
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
                        money={money}
                        onAdd={onAdd}
                        onOpen={setDetail}
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
          entry={detail}
          hubUrl={hubUrl}
          money={money}
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
