// The grouped body: one <tbody> per model, each under the header that names it
// and collapses it. A fragment rather than a wrapper — these have to be the
// table's own children, and a wrapper element would be a row no table has.
import { ChevronDown, ChevronRight } from "lucide-react";
import { useT } from "../../i18n";
import type { CatalogEntry } from "../../api/types";
import { COLUMNS } from "./order";
import { Row } from "./Row";
import { type PriceTier } from "./prices";
import { type ModelGroup } from "./groups";

export function GroupedBody({
  groups,
  collapsed,
  toggleGroup,
  hubUrl,
  onAdd,
  onOpen,
  emptyLabel,
}: {
  /** Already ordered: a sort orders the providers *inside* each group and never
      the groups themselves, so the order arrives here decided. */
  groups: ModelGroup[];
  /** Collapsed models by id. Held by the container rather than here so that
      flipping to the provider view and back does not drop the reading
      position — the state resets when the screen remounts, as it always has. */
  collapsed: Set<string>;
  toggleGroup: (model: string) => void;
  hubUrl: string | null;
  onAdd: (e: CatalogEntry) => void;
  onOpen: (e: CatalogEntry, tier: PriceTier) => void;
  /** Which of the two empty states this is — see the container's `emptyLabel`. */
  emptyLabel: string;
}) {
  const t = useT();
  return (
    <>
      {groups.map((g) => {
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
                  onOpen={onOpen}
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
  );
}
