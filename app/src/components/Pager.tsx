// The page control, for every list that has one.
//
// Extracted from the request-log card when the session page's tool calls grew a
// second list: two controls that drifted apart in spacing or wording would read
// as two different mechanisms, and a reader who has learned one should have
// learned the other. What each list pages through stays its own business.
//
// Its labels live in `common`, not in any one screen's namespace, for the reason
// it exists: it is nobody's control in particular.

import { Button } from "@/components/ui/button";
import { useT } from "../i18n";

export function Pager({
  page,
  pages,
  onGo,
}: {
  page: number;
  pages: number;
  /** The page asked for. The owner clamps and re-reads — a pager that decided
   *  what a valid page is would be a second opinion about its own list. */
  onGo: (page: number) => void;
}) {
  const t = useT();
  return (
    <div className="flex items-center justify-center gap-3 py-1.5 text-[11.5px] text-mut">
      <Button
        variant="outline"
        size="sm"
        className="h-7 px-2.5 text-[11px]"
        disabled={page <= 1}
        onClick={() => onGo(page - 1)}
      >
        {t("common.prev")}
      </Button>
      <span className="font-mono">{t("common.pageIndicator", { page, pages })}</span>
      <Button
        variant="outline"
        size="sm"
        className="h-7 px-2.5 text-[11px]"
        disabled={page >= pages}
        onClick={() => onGo(page + 1)}
      >
        {t("common.next")}
      </Button>
    </div>
  );
}
