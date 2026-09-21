// The header's two buttons: the ⟳, which re-reads everything at once, and the
// "+", which opens the add-provider dialog.
import { Check, Plus, RefreshCw } from "lucide-react";
import { Button } from "@/components/ui/button";
import { useT } from "../../i18n";

/** The click itself lives in the container: it is three of the screen's reads
    awaited together, and the two flags it drives are the container's. */
export function HeaderActions({
  refreshing,
  refreshed,
  onRefresh,
  onAdd,
}: {
  refreshing: boolean;
  refreshed: boolean;
  onRefresh: () => void;
  onAdd: () => void;
}) {
  const t = useT();
  return (
    <>
    <Button
      variant="ghost"
      size="sm"
      className="ml-auto h-7 w-7 px-0 text-mut"
      aria-label={t("common.refresh")}
      title={t("providers.refreshTitle")}
      disabled={refreshing}
      onClick={onRefresh}
    >
      {refreshed ? (
        <Check className="h-3.5 w-3.5" style={{ color: "var(--kiwi)" }} />
      ) : (
        <RefreshCw className={`h-3.5 w-3.5${refreshing ? " animate-spin" : ""}`} />
      )}
    </Button>
    <Button
      size="sm"
      className="h-7 gap-1 px-2.5 text-[12px] font-semibold"
      onClick={onAdd}
    >
      <Plus className="h-3.5 w-3.5" />
      {t("providers.addProvider")}
    </Button>
    </>
  );
}
