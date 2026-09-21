// Models: Kiwano Hub cloud catalog as a sortable table (desktop-tool density)
//
// The container: it owns the screen's state, the reads and the effects behind
// it, and composes the pieces that live in `screens/Shelf/` — the same shape
// its sibling `screens/Providers.tsx` has.
import { useCallback, useEffect, useMemo, useState } from "react";
import { api } from "../api/client";
import { useReload } from "../lib/reload";
import type { CatalogEntry, ModelPrice } from "../api/types";
import { useHubUrl } from "../lib/hub";
import { useT } from "../i18n";
import { type ChipId } from "./Shelf/labels";
import { byTagRank, orderRows, type SortKey } from "./Shelf/order";
import { buildModelGroups, type ModelGroupRow } from "./Shelf/groups";
import { type PriceTier } from "./Shelf/prices";
import { Toolbar } from "./Shelf/Toolbar";
import { TableHead } from "./Shelf/TableHead";
import { ProviderBody } from "./Shelf/ProviderBody";
import { GroupedBody } from "./Shelf/GroupedBody";
import { DetailDialog } from "./Shelf/DetailDialog";
import { useShelfPrefs } from "./Shelf/prefs";

export default function Shelf({ onAdd }: { onAdd: (preset: CatalogEntry) => void }) {
  const t = useT();
  const hubUrl = useHubUrl();
  const { sort, view, remember, rememberView } = useShelfPrefs();
  const [catalog, setCatalog] = useState<{ total: number; entries: CatalogEntry[] } | null>(null);
  const [prices, setPrices] = useState<ModelPrice[]>([]);
  const [chip, setChip] = useState<ChipId>("all");
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
      <Toolbar
        chip={chip}
        setChip={setChip}
        view={view}
        rememberView={rememberView}
        hubErr={hubErr}
        query={query}
        setQuery={setQuery}
        hub={hub}
        refreshFromHub={refreshFromHub}
      />
      <table className="w-full border-collapse">
        <TableHead sort={sort} toggleSort={toggleSort} />
        {view === "provider" ? (
          <ProviderBody
            rows={filtered}
            hubUrl={hubUrl}
            onAdd={onAdd}
            onOpen={(entry, tier) => setDetail({ entry, tier })}
            emptyLabel={emptyLabel}
          />
        ) : (
          <GroupedBody
            groups={orderedGroups}
            collapsed={collapsed}
            toggleGroup={toggleGroup}
            hubUrl={hubUrl}
            onAdd={onAdd}
            onOpen={(entry, tier) => setDetail({ entry, tier })}
            emptyLabel={emptyLabel}
          />
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
