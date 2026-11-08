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

/** The Dashboard, with no tail — the root the page grew up under. */
const DASHBOARD_HASH = "#dashboard";

/** A step in the breadcrumb's trail above the request: a label and where it
    leads. Passed in by whatever opened the log, because the same page is
    reached from the Dashboard's log table and from one provider's own page,
    and a request's trail should be the one it was walked down. */
export interface LogTrailStep {
  label: string;
  hash: string;
}

export default function LogDetail({
  logId,
  trail = [],
}: {
  logId: number;
  /** Steps between the nav root and the request, outermost first. Empty for
      the Dashboard's own log list — that trail is just "Dashboard". */
  trail?: LogTrailStep[];
}) {
  const t = useT();
  const [row, setRow] = useState<RequestLogDetail | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [err, setErr] = useState<string | null>(null);
  // What the click on Copy did, in words beside the button — the icon inside it
  // is the instant response, this is the one a reader catches after looking
  // away from the gesture. A fresh object per click, so clicking again while the
  // words are still up arms a new four seconds instead of inheriting the old
  // deadline.
  const [copyState, setCopyState] = useState<{ what: "copied" | "failed" } | null>(null);

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

  // Four seconds, the same budget the log table gives its export line
  // (RequestLogs.tsx): long enough to read after the gesture, short enough that
  // it does not outlive the fact it reports.
  useEffect(() => {
    if (!copyState) return;
    // Named `timer`, not `t`: `t` is the translator here.
    const timer = window.setTimeout(() => setCopyState(null), 4000);
    return () => window.clearTimeout(timer);
  }, [copyState]);

  const onCopy = () => {
    if (!row) return;
    // Synchronous inside the gesture: the detail is already loaded here, so
    // nothing may await before the clipboard write (WKWebView ends the user
    // activation window at the first await).
    setCopyState({ what: copyText(fmtLogText(row, row)) ? "copied" : "failed" });
  };

  const title = t("logs.logTitle", { id: logId });
  const back = () => {
    window.location.hash = trail.length > 0 ? trail[trail.length - 1].hash : DASHBOARD_HASH;
  };

  return (
    <section>
      <div className="p-4">
        <nav aria-label={t("logs.logBreadcrumb")} className="mb-2">
          <ol className="flex items-center gap-1.5 text-[11px] text-mut">
            {/* The root is the Dashboard when nothing above it was walked
                through; a log opened from a provider's page says Apps ›
                <provider> instead, which is the way it was reached. */}
            {trail.length === 0 && (
              <li>
                <button className="cursor-pointer hover:text-fg" onClick={back}>
                  {t("app.nav.dashboard")}
                </button>
              </li>
            )}
            {trail.map((step) => (
              <li key={step.hash} className="flex min-w-0 items-center gap-1.5">
                <button className="cursor-pointer truncate hover:text-fg" onClick={() => {
                  window.location.hash = step.hash;
                }}>
                  {step.label}
                </button>
                <span aria-hidden="true">
                  <ChevronRight className="h-3 w-3" />
                </span>
              </li>
            ))}
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
          <div className="flex shrink-0 items-center gap-2">
            {/* `role="status"` so the words reach a screen reader too: the
                button's label does not change, and nothing else on the page
                would announce the click. The element stays mounted (empty) so
                the change of its contents is what gets announced. */}
            <span
              role="status"
              className="text-[11px]"
              style={{ color: copyState?.what === "failed" ? "var(--red)" : "var(--kiwi)" }}
            >
              {copyState?.what === "failed"
                ? t("logs.copyFailed")
                : copyState?.what === "copied"
                  ? t("logs.copiedToClipboard")
                  : ""}
            </span>
            <Button
              variant="outline"
              size="sm"
              className="h-7 shrink-0 gap-1 px-2.5 text-[11px] text-mut"
              disabled={!row}
              onClick={onCopy}
            >
              {/* The tick is the in-place response; the word stays "Copy" so the
                  button does not move its own goalpost mid-gesture. */}
              {copyState?.what === "copied" ? (
                <Check className="h-3 w-3" />
              ) : (
                <Copy className="h-3 w-3" />
              )}
              {t("common.copy")}
            </Button>
          </div>
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
