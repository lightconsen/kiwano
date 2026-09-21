// The screen's toolbar: the category chips, the provider/model view toggle, the
// search box and the Hub refresh. Every one of them is a control over state the
// container owns, so each arrives here as a value and the setter behind it.
import { Check, RefreshCw, Search } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useT } from "../../i18n";
import { CHIPS, TAG_ICON, type ChipId } from "./labels";
import { VIEWS, type ShelfView } from "./groups";

export function Toolbar({
  chip,
  setChip,
  view,
  rememberView,
  hubErr,
  query,
  setQuery,
  hub,
  refreshFromHub,
}: {
  chip: ChipId;
  setChip: (id: ChipId) => void;
  view: ShelfView;
  rememberView: (v: ShelfView) => void;
  /** Why the last Hub sync failed. Unlike the check it holds until the next
      try, which is what makes it readable. */
  hubErr: string | null;
  query: string;
  setQuery: (q: string) => void;
  hub: "idle" | "busy" | "done";
  refreshFromHub: () => void;
}) {
  const t = useT();
  return (
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
  );
}
