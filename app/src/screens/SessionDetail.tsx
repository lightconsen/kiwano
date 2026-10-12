// One session's own page, reached by opening a row of the Sessions list.
//
// It **replaces** the Dashboard rather than floating over it (`Dashboard.tsx`
// returns it early), because what it shows is not a detail *of* the page behind
// it: it is a different subject, with its own breadcrumb back.
//
// The page exists for the parts a 900px table cannot hold — the tools and skills
// a session used, all four token buckets, the machine — and for the one thing a
// single merged row cannot say: **which ledger each number came from**. So the
// numbers are grouped by their source, and nothing is ever added across the two.

import { useCallback, useEffect, useState } from "react";
import { ChevronRight, Loader2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import { errText, LoadFailed } from "../components/LoadFailed";
import { useReload } from "../lib/reload";
import { fmtTokens } from "../lib/format";
import { useLocale, useT, type Translate } from "../i18n";
import type { HistoryCount, Session } from "../api/types";
import {
  fmtFullStamp,
  fmtSessionCost,
  SourceBadge,
} from "./RequestLogs/sessionBits";

/** Where the breadcrumb's root leads: the Dashboard, with no tail. */
const DASHBOARD_HASH = "#dashboard";

export default function SessionDetail({
  sessionId,
  onShowRequests,
}: {
  sessionId: string;
  /** Ask the Dashboard to show this session's requests: set the log card's
   *  session filter, switch it to Logs, and close this page. Owned by the
   *  Dashboard, which is the only component that outlives both views. */
  onShowRequests: (id: string) => void;
}) {
  const t = useT();
  const locale = useLocale();
  const [row, setRow] = useState<Session | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [err, setErr] = useState<string | null>(null);

  // Reads by id rather than taking the row from the list: this page *is* what
  // replaces the list, and a refresh or a pasted link arrives with nothing else.
  const reload = useCallback(
    () =>
      api
        .getSession(sessionId)
        .then((r) => {
          setRow(r);
          setLoaded(true);
          setErr(null);
        })
        .catch((e) => {
          setErr(errText(e));
          setLoaded(true);
        }),
    [sessionId],
  );
  useEffect(() => {
    void reload();
  }, [reload]);
  // The app's ⟳ reaches whatever is on screen; this is what is on screen.
  // (`reload` resolves only after the read lands, which is what the status bar's
  // spinner is waiting on.)
  useReload(reload);

  // `<main>` is the app's only scroll container and it survives this swap, so a
  // session opened from halfway down the list would paint at that offset — with
  // the breadcrumb and the title already scrolled off the top.
  useEffect(() => {
    // `scrollTop = 0` rather than `scrollTo`: it is the same jump without an
    // animation, and it is the form jsdom implements (the tests render this page).
    const main = document.querySelector<HTMLElement>("main");
    if (main) main.scrollTop = 0;
  }, [sessionId]);

  return (
    <section>
      {/* The same `p-4` the Dashboard's own content sits in: without it this page
          runs to the window's edges, which reads as a different kind of screen
          rather than the same one showing something else. */}
      <div className="p-4">
        <Breadcrumb title={row?.title ?? undefined} id={sessionId} />
        {err ? (
          <LoadFailed detail={err} onRetry={() => void reload()} />
        ) : !loaded ? (
          <div className="flex items-center justify-center gap-2 py-16 text-[12px] text-mut">
            <Loader2 className="h-3.5 w-3.5 animate-spin" />
            {t("common.loading")}
          </div>
        ) : !row ? (
          // Not a failure: neither ledger has this id, which is an answer. The
          // header and breadcrumb are above it either way, so there is a way out.
          <div className="rounded-lg border border-line bg-surface px-3.5 py-6 text-center text-[12px] text-mut">
            {t("dashboard.sessionGone")}
          </div>
        ) : (
          <Body
            row={row}
            t={t}
            locale={locale}
            onShowRequests={onShowRequests}
          />
        )}
      </div>
    </section>
  );
}

/** `Dashboard › <title>`, with the root going back.
 *
 * A `button` setting the hash rather than `history.back()`: a deep link from the
 * tray or a pasted URL has no entry behind it, and Back would leave the app.
 * The root's label is the nav's own key, so the two cannot drift. */
function Breadcrumb({ title, id }: { title?: string; id: string }) {
  const t = useT();
  const back = () => {
    window.location.hash = DASHBOARD_HASH;
  };
  return (
    <nav aria-label={t("dashboard.sessionBreadcrumb")} className="mb-2">
      <ol className="flex items-center gap-1.5 text-[11px] text-mut">
        <li>
          <button className="hover:text-fg" onClick={back}>
            {t("app.nav.dashboard")}
          </button>
        </li>
        <li aria-hidden="true">
          <ChevronRight className="h-3 w-3" />
        </li>
        <li aria-current="page" className="flex min-w-0 items-center">
          {/* A gateway-only session has no title, and then the id *is* its name. */}
          <span className="max-w-[520px] truncate font-mono" title={id}>
            {title ?? id}
          </span>
        </li>
      </ol>
    </nav>
  );
}

function Body({
  row,
  t,
  locale,
  onShowRequests,
}: {
  row: Session;
  t: Translate;
  locale: string;
  onShowRequests: (id: string) => void;
}) {
  return (
    <>
      <div className="flex items-start justify-between gap-4">
        {/* Read, not compared — so it wraps rather than ellipsising, the same
            rule the list's title column follows. */}
        <h2 className="text-[15px] font-semibold break-words">
          {row.title ?? row.session_id}
        </h2>
        <SourceBadge source={row.source} />
      </div>
      <div className="mt-1 mb-3 flex flex-wrap items-center gap-x-2 gap-y-1 text-[11px] text-mut">
        <span>{row.agent}</span>
        <Dotted />
        <span>{row.project ?? t("common.none")}</span>
        <Dotted />
        <span>
          {t("dashboard.sessionMachine")}: {row.machine ?? t("common.none")}
        </span>
        <Dotted />
        <span className="font-mono">{row.session_id}</span>
      </div>

      {/* Two columns, unconditionally: the window's minimum is 900px, so the
          page is always ~860px wide, and a breakpoint would only be a second way
          for the reader's window to change the answer. */}
      <div className="grid grid-cols-2 gap-3">
        {/* The gateway's half: what it metered itself. */}
        <Card
          title={t("dashboard.sessionFromGateway")}
          hint={t("dashboard.sessionFromGatewayHint")}
        >
          <Row
            label={t("dashboard.colRequests")}
            // `—` rather than 0 when the files are the only side that has this
            // session: a zero there would read as "it routed nothing", which is a
            // different statement from "it never saw it".
            value={
              row.source === "imported"
                ? t("common.none")
                : String(row.requests)
            }
          />
          <Row
            label={t("dashboard.sessionTokens")}
            title={t("dashboard.sessionTokensTitle", {
              input: fmtTokens(row.input_tokens),
              output: fmtTokens(row.output_tokens),
              read: fmtTokens(row.cache_read_tokens),
              write: fmtTokens(row.cache_creation_tokens),
            })}
            value={`${fmtTokens(row.input_tokens)} / ${fmtTokens(row.output_tokens)} / ${fmtTokens(
              row.cache_read_tokens,
            )} / ${fmtTokens(row.cache_creation_tokens)}`}
          />
          <Row
            label={t("dashboard.sessionCost")}
            // Per currency, never converted — import this page into the
            // Dashboard's converted totals and the rule stops meaning anything.
            value={fmtSessionCost(row, t)}
          />
        </Card>

        {/* The files' half: the dimensions the gateway cannot observe. */}
        <Card
          title={t("dashboard.sessionFromFiles")}
          hint={t("dashboard.sessionFromFilesHint")}
        >
          <Row
            label={t("dashboard.sessionStarted")}
            value={fmtFullStamp(row.started_at, locale)}
          />
          <Row
            label={t("dashboard.sessionEnded")}
            value={fmtFullStamp(row.ended_at, locale)}
          />
          <Row
            label={t("dashboard.sessionTurns")}
            value={
              row.source === "gateway" ? t("common.none") : String(row.turns)
            }
          />
        </Card>
      </div>

      <div className="mt-3 grid grid-cols-2 gap-3">
        <Card title={t("dashboard.sessionTools")}>
          <CountList
            counts={row.tool_calls}
            empty={
              row.source === "gateway"
                ? t("common.none")
                : t("dashboard.sessionNoTools")
            }
            t={t}
          />
        </Card>
        <Card title={t("dashboard.sessionSkills")}>
          <CountList
            counts={row.skills}
            empty={
              row.source === "gateway"
                ? t("common.none")
                : t("dashboard.sessionNoSkills")
            }
            t={t}
          />
        </Card>
      </div>

      <div className="mt-3 flex justify-end">
        <Button
          variant="outline"
          size="sm"
          className="h-7 gap-1 px-3 text-[11.5px]"
          onClick={() => onShowRequests(row.session_id)}
        >
          {t("dashboard.sessionShowRequests")}
        </Button>
      </div>
    </>
  );
}

function Dotted() {
  return <span aria-hidden="true">·</span>;
}

function Card({
  title,
  hint,
  children,
}: {
  title: string;
  hint?: string;
  children: React.ReactNode;
}) {
  return (
    <div className="rounded-lg border border-line bg-surface px-3.5 py-3">
      <div className="mb-2 text-[12px] font-semibold">{title}</div>
      {hint && (
        <div className="mb-2 text-[10.5px] leading-snug text-mut">{hint}</div>
      )}
      <div className="space-y-1.5">{children}</div>
    </div>
  );
}

/** One label/value line. `min-w-0` on the value is what lets a long one wrap
 *  inside the grid instead of pushing the card wider than the window. */
function Row({
  label,
  value,
  title,
}: {
  label: string;
  value: string;
  title?: string;
}) {
  return (
    <div className="flex items-baseline justify-between gap-3">
      <span className="shrink-0 text-[11px] text-mut">{label}</span>
      <span
        className="min-w-0 text-right font-mono text-[11.5px] break-words"
        title={title}
      >
        {value}
      </span>
    </div>
  );
}

/** How many tools or skills a session used, most-used first.
 *
 * Sorted here rather than trusted from the wire: the counts arrive in the order
 * the file happened to mention them, which is not an order a reader wants. Capped
 * because a session can touch thirty tools, and the list is a summary — the
 * remainder is *stated* rather than dropped. */
const COUNT_CAP = 10;

function CountList({
  counts,
  empty,
  t,
}: {
  counts: HistoryCount[];
  empty: string;
  t: Translate;
}) {
  if (counts.length === 0) {
    return <div className="text-[11px] text-mut">{empty}</div>;
  }
  const sorted = [...counts].sort(
    (a, b) => b.count - a.count || a.name.localeCompare(b.name),
  );
  const shown = sorted.slice(0, COUNT_CAP);
  const total = sorted.reduce((n, c) => n + c.count, 0);
  return (
    <div className="space-y-1">
      {shown.map((c) => (
        <div key={c.name} className="flex items-baseline gap-2 text-[11.5px]">
          <span className="min-w-0 flex-1 truncate font-sans" title={c.name}>
            {c.name}
          </span>
          {/* The share is what makes a long list readable — `Bash ×976` says
              more with a bar behind it than as a number. */}
          <span className="h-1 w-16 shrink-0 self-center rounded-full bg-surface2">
            <span
              className="block h-1 rounded-full"
              style={{
                width: `${Math.max(2, Math.round((c.count / total) * 100))}%`,
                background: "var(--kiwi)",
              }}
            />
          </span>
          <span className="w-10 shrink-0 text-right font-mono">{c.count}</span>
        </div>
      ))}
      {sorted.length > shown.length && (
        <div className="text-[10.5px] text-mut">
          {t("dashboard.sessionMore", { n: sorted.length - shown.length })}
        </div>
      )}
    </div>
  );
}
