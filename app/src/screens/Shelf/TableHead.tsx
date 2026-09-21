// The table's column header: every column the table has, the three that sort
// it, and the arrow the current sort wears. It is the table's own child, so it
// is a `<thead>` rather than a wrapper that would have to be unwrapped.
import { useT } from "../../i18n";
import { COLUMNS, type Sort, type SortKey } from "./order";

export function TableHead({
  sort,
  toggleSort,
}: {
  sort: Sort | null;
  /** Three-state: ascending → descending → back to the default order. The
      container owns the cycle; this only knows which column was clicked. */
  toggleSort: (key: SortKey) => void;
}) {
  const t = useT();
  return (
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
  );
}
