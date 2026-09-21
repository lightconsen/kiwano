// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
//
// The container: it owns the screen's state, the reads and the effects behind
// it, and composes the pieces that live in `screens/Shelf/` — the same shape
// its sibling `screens/Providers.tsx` has.
import { useCallback, useEffect, useMemo, useState } from "react";
import { Check, ChevronDown, ChevronRight, RefreshCw, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { useReload } from "../lib/reload";
import type { CatalogEntry, ModelPrice } from "../api/types";
import { useHubUrl } from "../lib/hub";
import { useT } from "../i18n";
import { CHIPS, TAG_ICON } from "./Shelf/labels";
import { COLUMNS, byTagRank, orderRows, type SortKey } from "./Shelf/order";
import { VIEWS, buildModelGroups, type ModelGroupRow } from "./Shelf/groups";
import { type PriceTier } from "./Shelf/prices";
import { Row } from "./Shelf/Row";
import { DetailDialog } from "./Shelf/DetailDialog";
import { useShelfPrefs } from "./Shelf/prefs";

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
