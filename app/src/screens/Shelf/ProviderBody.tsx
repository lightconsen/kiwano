// The provider table's body: one row per entry the chip filter, the search and
// the sort left, or the single row that says there are none.
import type { CatalogEntry } from "../../api/types";
import { COLUMNS } from "./order";
import { Row } from "./Row";
import { type PriceTier } from "./prices";

export function ProviderBody({
  rows,
  hubUrl,
  onAdd,
  onOpen,
  emptyLabel,
}: {
  /** Already filtered and sorted — both are the container's memos. */
  rows: CatalogEntry[];
  hubUrl: string | null;
  onAdd: (e: CatalogEntry) => void;
  onOpen: (e: CatalogEntry, tier: PriceTier) => void;
  /** Which of the two empty states this is — see the container's `emptyLabel`. */
  emptyLabel: string;
}) {
  return (
    <tbody>
      {rows.map((e) => (
        <Row
          key={e.id}
          entry={e}
          hubUrl={hubUrl}
          onAdd={onAdd}
          onOpen={onOpen}
        />
      ))}
      {rows.length === 0 && (
        <tr>
          <td colSpan={COLUMNS.length} className="px-4 py-8 text-center text-[12px] text-mut">
            {emptyLabel}
          </td>
        </tr>
      )}
    </tbody>
  );
}
