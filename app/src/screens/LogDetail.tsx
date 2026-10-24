// One request log's own page, reached by opening a row of the Dashboard's log
// table (or the credential banner's deep link `#dashboard/log/<id>`).
//
// It **replaces** the Dashboard rather than floating over it (`Dashboard.tsx`
// returns it early), the same shape `SessionDetail` has and for the same
// reason: a redacted header dump and two bodies are a subject of their own, not
// a detail *of* the list behind it — and a dialog that large hides the row it
// came from.

import { useCallback, useEffect, useState } from "react";
import { Check, ChevronRight, Copy, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { errText, LoadFailed } from "../components/LoadFailed";
import { useReload } from "../lib/reload";
import { useT } from "../i18n";
import type { RequestLogDetail } from "../api/types";
import { copyText, fmtLogText, fmtTime, LogDetailBody } from "./RequestLogs";

/** Where the breadcrumb's root leads: the Dashboard, with no tail. */
const DASHBOARD_HASH = "#dashboard";

export default function LogDetail({ logId }: { logId: number }) {
  const t = useT();
  const [row, setRow] = useState<RequestLogDetail | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  const [copied, setCopied] = useState(false);

  // Read by id: the page is what replaces the list, and a deep link or a
  // refresh arrives with nothing else.
  const reload = useCallback(
    () =>
      api
        .getRequestLog(logId)
        .then((r) => {
          setRow(r);
          setLoaded(true);
          setErr(null);
        })
        .catch((e) => {
          setErr(errText(e));
          setLoaded(true);
        }),
    [logId],
  );
  useEffect(() => {
    void reload();
  }, [reload]);
  useReload(reload);

  // `<main>` survives this swap, so a page opened from halfway down the table
  // would paint at that offset — with the breadcrumb and title off-screen.
  useEffect(() => {
    const main = document.querySelector<HTMLElement>("main");
    if (main) main.scrollTop = 0;
  }, [logId]);

  const onCopy = () => {
    // Synchronous inside the gesture: the detail is already loaded here, so
    // nothing may await before the clipboard write (WKWebView ends the user
    // activation window at the first await).
    if (row && copyText(fmtLogText(row, row))) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    }
  };

  const title = t("logs.logTitle", { id: logId });
  const back = () => {
    window.location.hash = DASHBOARD_HASH;
  };

  return (
    <section>
      <div className="p-4">
        <nav aria-label={t("logs.logBreadcrumb")} className="mb-2">
          <ol className="flex items-center gap-1.5 text-[11px] text-mut">
            <li>
              <button className="cursor-pointer hover:text-fg" onClick={back}>
                {t("app.nav.dashboard")}
              </button>
            </li>
            <li aria-hidden="true">
              <ChevronRight className="h-3 w-3" />
            </li>
            <li aria-current="page" className="flex min-w-0 items-center">
              <span className="truncate font-mono">{title}</span>
              {row && (
                <span className="ml-1.5 truncate text-mut">· {fmtTime(row.ts)}</span>
              )}
            </li>
          </ol>
        </nav>

        <div className="flex items-start justify-between gap-4">
          <div className="min-w-0">
            <h2 className="text-[15px] font-semibold">{title}</h2>
            {row && (
              <div className="mt-0.5 font-mono text-[11px] text-mut">
                {row.method}{" "}
                <span className="text-ink">
                  {row.path}
                  {row.query ? `?${row.query}` : ""}
                </span>
              </div>
            )}
          </div>
          <Button
            variant="outline"
            size="sm"
            className="h-7 shrink-0 gap-1 px-2.5 text-[11px] text-mut"
            disabled={!row}
            onClick={onCopy}
          >
            {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            {copied ? t("common.copied") : t("common.copy")}
          </Button>
        </div>

        {err ? (
          <LoadFailed detail={err} onRetry={() => void reload()} />
        ) : !loaded ? (
          <div className="flex items-center justify-center gap-2 py-16 text-[12px] text-mut">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {t("common.loading")}
          </div>
        ) : !row ? (
          // The log was pruned or never existed: an answer, not a failure.
          <div className="rounded-lg border border-line bg-surface px-3.5 py-6 text-center text-[12px] text-mut">
            {t("logs.logGone")}
          </div>
        ) : (
          <div className="mt-1">
            <LogDetailBody d={row} />
          </div>
        )}
      </div>
    </section>
  );
}
