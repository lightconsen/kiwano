// The table's ordering vocabulary: the columns and the keys they sort by, the
// remembered sort, and the comparators behind them. The default order is the
// one a reader with no preference gets, so it is written out here rather than
// left to the catalog's own.
import { type KeyPath, type Messages } from "../../i18n";
import type { CatalogEntry } from "../../api/types";
import { TAG_RANK, billingRank } from "./labels";

const SORT_KEYS = ["name", "billing", "price"] as const;
export type SortKey = (typeof SORT_KEYS)[number];
export type Sort = { key: SortKey; dir: 1 | -1 };

/** A remembered sort choice ("price:-1"), or null for the default order. A key
    this build does not know — a value written by a newer one — falls back to
    the default rather than sorting by nothing. */
export function parseSort(v: string | null | undefined): Sort | null {
  if (!v) return null;
  const [key, dir] = v.split(":");
  if (!SORT_KEYS.includes(key as SortKey)) return null;
  const d = Number(dir);
  return d === 1 || d === -1 ? { key: key as SortKey, dir: d } : null;
}

export const formatSort = (s: Sort | null): string | null => (s ? `${s.key}:${s.dir}` : null);

/** The comparable price of an entry: the representative model's input rate, as
    published. null means no price is published at all. */
function priceValue(e: CatalogEntry): number | null {
  const p = e.price_ref;
  if (!p) return null;
  const n = Number(p.input);
  return Number.isFinite(n) ? n : null;
}

export const COLUMNS: { key: SortKey | null; labelKey: KeyPath<Messages>; className: string }[] = [
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
export const byTagRank = (a: CatalogEntry, b: CatalogEntry): number =>
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
export function orderRows(rows: CatalogEntry[], sort: Sort | null): CatalogEntry[] {
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
