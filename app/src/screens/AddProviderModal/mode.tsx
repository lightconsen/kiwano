// The three things above the form: the mode switch, the catalog picker it opens
// on, and the chip naming the entry the form is bound to.
import { Store } from "lucide-react";
import { ProviderLogo } from "@/components/icons/ProviderLogo";
import { Input } from "@/components/ui/input";
import { useT } from "../../i18n";
import { hubAssetUrl } from "../../lib/hub";
import type { CatalogEntry } from "../../api/types";

/** Which of the two modes the form is in. Hidden while editing — no models are
    involved in an existing provider — so the container decides whether it
    renders at all; which one is showing is its state. */
export function ModeSwitch({
  mode,
  shelf,
  setMode,
}: {
  mode: "shelf" | "custom";
  /** The entry already picked, whose name the button carries once there is one */
  shelf: CatalogEntry | null;
  setMode: (mode: "shelf" | "custom") => void;
}) {
  const t = useT();
  return (
    <div className="flex overflow-hidden rounded-md border border-line text-[11.5px]">
      <div
        className="btn flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-center gap-1 font-medium"
        style={mode === "shelf" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
        onClick={() => setMode("shelf")}
      >
        <Store className="h-3 w-3" />
        {shelf
          ? t("addProvider.fromModelsNamed", { name: shelf.name })
          : t("addProvider.fromModels")}
      </div>
      <div
        className="btn flex h-8 min-w-0 flex-1 cursor-pointer items-center justify-center"
        style={mode === "custom" ? { background: "var(--kiwi-soft)", color: "var(--kiwi)" } : { color: "var(--mut)" }}
        onClick={() => setMode("custom")}
      >
        {t("addProvider.custom")}
      </div>
    </div>
  );
}

/** The picker the "From Models" mode opens on: the entries the Hub publishes
    that are not added yet, filtered by the query box. The read behind it is the
    container's, and so is the prefill a click runs. */
export function CatalogPicker({
  query,
  setQuery,
  catalog,
  hubUrl,
  setShelf,
  applyShelf,
}: {
  query: string;
  setQuery: (q: string) => void;
  /** Null until the first read lands: the list shows the loading note until it does */
  catalog: CatalogEntry[] | null;
  hubUrl: string | null;
  setShelf: (e: CatalogEntry | null) => void;
  applyShelf: (e: CatalogEntry) => void;
}) {
  const t = useT();
  return (
    <div className="mt-3">
      <Input
        className="h-8 bg-bg text-[12px] dark:bg-bg"
        placeholder={t("addProvider.searchCatalog")}
        value={query}
        onChange={(e) => setQuery(e.target.value)}
      />
      {/* rows carry logo + name only (endpoint URLs were dropped: they
          ellipsized awkwardly in the narrow list); overflow-x-hidden
          stays as WKWebView hardening */}
      <div className="mt-1.5 max-h-40 overflow-x-hidden overflow-y-auto rounded-md border border-line">
        {(catalog ?? [])
          .filter(
            (e) =>
              !e.added &&
              e.name.toLowerCase().includes(query.trim().toLowerCase()),
          )
          .map((e) => (
            <button
              key={e.id}
              className="btn flex w-full items-center gap-2 border-b border-line px-2.5 py-1.5 text-left last:border-b-0 hover:bg-surface2"
              onClick={() => {
                setShelf(e);
                applyShelf(e);
              }}
            >
              <ProviderLogo
                logo={e.logo && hubUrl ? hubAssetUrl(hubUrl, e.logo) : undefined}
                name={e.name}
                color={e.logo_color}
                size={16}
              />
              <span className="min-w-0 truncate text-[12px] font-medium">{e.name}</span>
            </button>
          ))}
        {catalog && catalog.filter((e) => !e.added && e.name.toLowerCase().includes(query.trim().toLowerCase())).length === 0 && (
          <div className="px-2.5 py-3 text-center text-[11px] text-mut">
            {t("addProvider.noMatching")}
          </div>
        )}
        {!catalog && (
          <div className="px-2.5 py-3 text-center text-[11px] text-mut">
            {t("common.loading")}
          </div>
        )}
      </div>
    </div>
  );
}

/** The entry the form is bound to, with the way back to the picker. */
export function EntryChip({
  shelf,
  hubUrl,
  setShelf,
}: {
  shelf: CatalogEntry;
  hubUrl: string | null;
  setShelf: (e: CatalogEntry | null) => void;
}) {
  const t = useT();
  return (
    <div className="mt-3 flex items-center gap-2 rounded-md border border-line px-2.5 py-1.5">
      <ProviderLogo
        logo={shelf.logo && hubUrl ? hubAssetUrl(hubUrl, shelf.logo) : undefined}
        name={shelf.name}
        color={shelf.logo_color}
        size={16}
      />
      <span className="min-w-0 truncate text-[12px] font-medium">{shelf.name}</span>
      <button
        className="ml-auto flex-none text-[11px] font-medium"
        style={{ color: "var(--kiwi)" }}
        onClick={() => setShelf(null)}
      >
        {t("addProvider.change")}
      </button>
    </div>
  );
}
