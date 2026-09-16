// A read a screen could not make.
//
// Feedback lives in the screen rather than in a toast — the same call
// `RequestLogs`'s toolbar makes and for the same reason: a transient clip that
// fades leaves the reader who looked away with nothing, and this one stays until
// the next attempt. What is shared is only the strip and the wording (see
// i18n/en/common.ts on why that belongs in `common`) — *which* read failed and
// whether stale data stays on screen is each screen's own judgement, so the
// error state lives there and this renders it.
import { Button } from "@/components/ui/button";
import { useT } from "../i18n";

/** Tauri rejects an `invoke` with an `Error` or a bare string; both read badly
    raw, and a screen that shows one wants a string. */
export function errText(e: unknown): string {
  return e instanceof Error ? e.message : String(e);
}

export function LoadFailed({
  detail,
  onRetry,
  className,
}: {
  /** The raw failure. It rides in the tooltip: it is rarely a sentence. */
  detail: string;
  onRetry?: () => void;
  className?: string;
}) {
  const t = useT();
  return (
    <div
      role="alert"
      title={detail}
      className={`flex items-center gap-3 border-line px-4 py-2 text-[11px] ${className ?? ""}`}
      style={{
        background: "color-mix(in srgb, var(--red) 10%, transparent)",
        color: "var(--red)",
      }}
    >
      <span className="min-w-0 flex-1 truncate">{t("common.loadFailed")}</span>
      {onRetry && (
        <Button
          variant="ghost"
          size="xs"
          className="shrink-0 text-[11px]"
          onClick={onRetry}
        >
          {t("common.retry")}
        </Button>
      )}
    </div>
  );
}
