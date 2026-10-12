// Request logs (data-plane audit trail captured by the gateway, migration V5),
// rendered as the trailing card section of the Dashboard: paged metadata list
// with status filter, clear-all. Clicking a row opens a dialog with the full
// record (redacted headers + bodies) and a Copy button that puts the whole
// log as plain text on the clipboard.
import { useEffect, useRef, useState } from "react";
import { Check, ChevronRight, Copy, Download, X as XIcon } from "lucide-react";
import { save } from "@tauri-apps/plugin-dialog";
import { Button } from "@/components/ui/button";
import {
  Dialog,
  DialogContent,
  DialogHeader,
  DialogTitle,
} from "@/components/ui/dialog";
import { Input } from "@/components/ui/input";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import { errText } from "../components/LoadFailed";
import { useReload } from "../lib/reload";
import type {
  DashboardWindow,
  RequestLogDetail,
  RequestLogEntry,
  RequestLogFilter,
  Session,
} from "../api/types";
import { SessionsTable } from "./RequestLogs/SessionsTable";
import {
  localMidnightUtc,
  localMidnightUtcAfter,
  shiftLocalDate,
  todayLocalDate,
  windowSince,
} from "../lib/daterange";
import { fmtLatency, fmtTokens } from "../lib/format";
import { useT, type KeyPath, type Messages } from "../i18n";

const PAGE_SIZE = 10;

type StatusFilter = "all" | "ok" | "error";

// Keys, not labels: a module-level `const` cannot call the `useT` hook, so the
// display text is resolved at render where `t` is in scope.
const FILTERS: { id: StatusFilter; labelKey: KeyPath<Messages> }[] = [
  { id: "all", labelKey: "logs.filterAll" },
  { id: "ok", labelKey: "logs.filterOk" },
  { id: "error", labelKey: "logs.filterError" },
];

export type Tab = "logs" | "sessions";

/** The card's two views. The first is the audit trail this card has always
    been; the second is the same machine's sessions, which the gateway's traffic
    and the agents' own files each know half of. */
const TABS: { id: Tab; labelKey: KeyPath<Messages> }[] = [
  { id: "logs", labelKey: "logs.title" },
  { id: "sessions", labelKey: "logs.tabSessions" },
];

/** The Dashboard window's own name, for the sessions view's caption.
 *
 * The labels are the dashboard's, reused on purpose: this list is measured over
 * the same span those figures are (`windowSince`), so a second set of words for
 * it would be a second claim about the same window. */
function windowLabel(t: (k: KeyPath<Messages>) => string, win: DashboardWindow): string {
  switch (win) {
    case "today":
      return t("dashboard.windowToday");
    case "30d":
      return t("dashboard.window30d");
    case "all":
      return t("dashboard.windowAll");
    default:
      return t("dashboard.window7d");
  }
}

type RangeId = "any" | "today" | "7d" | "30d" | "custom";

const RANGES: { id: RangeId; labelKey: KeyPath<Messages> }[] = [
  { id: "any", labelKey: "logs.rangeAny" },
  { id: "today", labelKey: "logs.rangeToday" },
  { id: "7d", labelKey: "logs.range7d" },
  { id: "30d", labelKey: "logs.range30d" },
  { id: "custom", labelKey: "logs.rangeCustom" },
];

function fmtTime(ts: string): string {
  // RFC3339 → "MM-DD HH:MM:SS" (local tool: compact column beats full ISO)
  return ts.replace("T", " ").slice(5, 19);
}

function fmtBytes(n: number): string {
  if (n >= 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)}MB`;
  if (n >= 1024) return `${(n / 1024).toFixed(1)}KB`;
  return `${n}B`;
}

/** Generation throughput when the stream's first-token timing exists (output
    tokens over the generation phase = total minus first-token latency);
    otherwise end-to-end (output tokens over total latency). */
function fmtThroughput(r: RequestLogEntry): string {
  if (r.output_tokens <= 0 || r.latency_ms == null) return "—";
  if (r.first_token_ms != null && r.latency_ms > r.first_token_ms) {
    return `${(r.output_tokens / ((r.latency_ms - r.first_token_ms) / 1000)).toFixed(0)} tok/s`;
  }
  return `${(r.output_tokens / (r.latency_ms / 1000)).toFixed(0)} tok/s`;
}

/** Pretty-print stored header JSON; non-JSON passes through untouched. */
function prettyJson(json: string | null): string {
  if (!json) return "—";
  try {
    return JSON.stringify(JSON.parse(json), null, 2);
  } catch {
    return json;
  }
}

/** Whole-log plain text for the clipboard: metadata, then detail sections when available. */
function noteLines(notes: string | null): string[] {
  if (!notes) return [];
  const { dlp, rest } = splitNotes(notes);
  // The detector's lines are self-prefixed ("dlp: …"); the shim's need naming.
  return [...dlp, ...(rest.length > 0 ? ["sanitizer: " + rest.join("\n")] : [])];
}
function fmtLogText(r: RequestLogEntry, d?: RequestLogDetail | null): string {
  const lines = [
    `#${r.id} · ${r.ts}`,
    `${r.method} ${r.path}${r.query ? `?${r.query}` : ""}`,
    `agent: ${r.agent ?? "—"} · provider: ${r.provider_id ?? "—"} · model: ${r.model ?? "—"} · status: ${r.status_code}`,
    `latency: ${r.latency_ms ?? "—"}ms · tokens: in ${r.input_tokens} / out ${r.output_tokens}`,
    // Only when the upstream broke it out: a provider that does not report
    // thinking is not saying the model did none.
    ...(r.reasoning_tokens > 0 ? [`reasoning: ${r.reasoning_tokens} (of the output)`] : []),
    ...(r.usage_missing ? ["usage: not reported by the upstream"] : []),
    // What the gateway noted on the way through — detector findings and shim
    // rewrites are different audiences, so the dump keeps them on their own
    // lines rather than under one prefix.
    ...noteLines(r.request_notes),
  ];
  if (r.error_kind) lines.push(`error: ${r.error_kind} ${r.error_message ?? ""}`);
  if (d?.session_id) lines.push(`session: ${d.session_id}`);
  if (d) {
    lines.push("", "-- request headers --", prettyJson(d.request_headers));
    lines.push("", "-- response headers --", prettyJson(d.response_headers));
    if (d.request_body) lines.push("", "-- request body --", d.request_body);
    if (d.response_body) lines.push("", "-- response body --", d.response_body);
  }
  return lines.join("\n");
}

/** Synchronous clipboard write. WKWebView has no navigator.clipboard and only
    honors execCommand copies made inside the user gesture, so the legacy
    textarea path runs first; the async Clipboard API is the fallback. */
function copyText(text: string): boolean {
  try {
    const ta = document.createElement("textarea");
    ta.value = text;
    ta.style.position = "fixed";
    ta.style.opacity = "0";
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand("copy");
    ta.remove();
    if (ok) return true;
  } catch {
    // fall through to the async path
  }
  try {
    void navigator.clipboard.writeText(text);
    return true;
  } catch {
    return false;
  }
}

function StatusPill({ code }: { code: number }) {
  const ok = code < 400;
  return (
    <span
      className="inline-block rounded px-1.5 py-0.5 text-[10px] font-semibold"
      style={{
        color: ok ? "var(--kiwi)" : "var(--red)",
        background: ok ? "var(--kiwi-soft)" : "color-mix(in srgb, var(--red) 12%, transparent)",
      }}
    >
      {code}
    </span>
  );
}

/** The card's pager, shared by both views.
 *
 * One component rather than two copies: the views are siblings in one card, and
 * two page controls that drifted apart in spacing or wording would read as two
 * different mechanisms. What each view pages through is its own business — only
 * the control is shared. */
function Pager({
  page,
  pages,
  onGo,
}: {
  page: number;
  pages: number;
  onGo: (page: number) => void;
}) {
  const t = useT();
  return (
    <div className="flex items-center justify-center gap-3 pb-3 text-[11.5px] text-mut">
      <Button
        variant="outline"
        size="sm"
        className="h-7 px-2.5 text-[11px]"
        disabled={page <= 1}
        onClick={() => onGo(page - 1)}
      >
        {t("logs.prev")}
      </Button>
      <span className="font-mono">{t("logs.pageIndicator", { page, pages })}</span>
      <Button
        variant="outline"
        size="sm"
        className="h-7 px-2.5 text-[11px]"
        disabled={page >= pages}
        onClick={() => onGo(page + 1)}
      >
        {t("logs.next")}
      </Button>
    </div>
  );
}

/** The list-row marker for a credential-watch finding: the detector's note
    lines always lead `request_notes`, so a prefix test on the row is exact. */
function DlpBadge() {
  const t = useT();
  return (
    <span
      className="ml-1 inline-block rounded px-1 py-0.5 text-[9.5px] font-semibold"
      style={{
        color: "var(--red)",
        background: "color-mix(in srgb, var(--red) 12%, transparent)",
      }}
    >
      {t("logs.dlpBadge")}
    </span>
  );
}

/** A finding rides the row when its notes lead with the detector's prefix. */
export function hasCredentialFinding(notes: string | null): boolean {
  return notes?.startsWith("dlp:") ?? false;
}

/** Split gateway notes into the detector's lines and everything else (the
    shim's). The detail shows the two in separate blocks. */
function splitNotes(notes: string): { dlp: string[]; rest: string[] } {
  const dlp: string[] = [];
  const rest: string[] = [];
  for (const line of notes.split("\n")) {
    (line.startsWith("dlp: ") ? dlp : rest).push(line);
  }
  return { dlp, rest };
}

function Detail({ d }: { d: RequestLogDetail | null }) {
  const t = useT();
  if (!d) return <div className="px-5 pb-4 text-[11px] text-mut">{t("common.loading")}</div>;

  return (
    <div className="space-y-2 px-5 pb-4 pt-3">
      {/* The list table trades this away for Model — it stays here, one per detail */}
      <div className="font-mono text-[11px] text-mut">
        {d.method} <span className="text-ink">{d.path}</span>
        {d.query ? `?${d.query}` : ""}
      </div>
      <div className="flex gap-6 text-[10.5px] text-mut">
        {d.session_id && (
          <span>
            {t("logs.detailSession")}{" "}
            <span className="font-mono text-ink">{d.session_id}</span>
          </span>
        )}
        {/* Which entry of the provider's key pool carried it (v33) — a label,
            `#<id>` or `primary`, never the key. Absent on rows written before
            that migration, which is why this is conditional and not a dash. */}
        {d.upstream_key && (
          <span>
            {t("logs.detailUpstreamKey")}{" "}
            <span className="font-mono text-ink">{d.upstream_key}</span>
          </span>
        )}
        {d.is_streaming && d.first_token_ms != null && (
          <span>
            {t("logs.firstToken")}{" "}
            <span className="font-mono text-ink">{fmtLatency(d.first_token_ms)}</span>
          </span>
        )}
        <span>
          {t("logs.detailRequest")}{" "}
          <span className="font-mono text-ink">{fmtBytes(d.request_size)}</span> ·{" "}
          {t("logs.detailResponse")}{" "}
          <span className="font-mono text-ink">{fmtBytes(d.response_size)}</span>
          {d.truncated && t("logs.detailBodyTruncated")}
        </span>
      </div>
      {d.error_kind && (
        <div className="rounded border border-line bg-surface2 p-2 text-[11px]" style={{ color: "var(--red)" }}>
          <span className="font-mono font-semibold">{d.error_kind}</span> {d.error_message}
        </div>
      )}
      {/* Not gated on error_kind on purpose: a sanitized request usually
          answered 200, and its explanation is not an error. */}
      {d.request_notes &&
        (() => {
          const { dlp, rest } = splitNotes(d.request_notes);
          return (
            <>
              {dlp.length > 0 && (
                <div
                  className="whitespace-pre-line rounded border p-2 text-[11px]"
                  style={{
                    color: "var(--red)",
                    borderColor: "color-mix(in srgb, var(--red) 35%, transparent)",
                    background: "color-mix(in srgb, var(--red) 8%, transparent)",
                  }}
                >
                  <span className="font-semibold">{t("logs.detailCredentialWatch")}</span>{" "}
                  {dlp.join("\n")}
                </div>
              )}
              {rest.length > 0 && (
                <div className="whitespace-pre-line rounded border border-line bg-surface2 p-2 text-[11px] text-mut">
                  <span className="font-semibold text-ink">{t("logs.detailNotes")}</span>{" "}
                  {rest.join("\n")}
                </div>
              )}
            </>
          );
        })()}
      <div className="grid grid-cols-2 gap-2">
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">
            {t("logs.detailRequestHeaders")}
          </div>
          <pre className="max-h-32 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {prettyJson(d.request_headers)}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">
            {t("logs.detailResponseHeaders")}
          </div>
          <pre className="max-h-32 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {prettyJson(d.response_headers)}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">
            {t("logs.detailRequestBody")}
          </div>
          <pre className="max-h-64 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {d.request_body ?? "—"}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">
            {t("logs.detailResponseBody")}
            {d.is_streaming ? t("logs.detailClientStream") : ""}
          </div>
          <pre className="max-h-64 overflow-auto whitespace-pre-wrap rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {d.response_body ?? "—"}
          </pre>
        </div>
      </div>
    </div>
  );
}

/** Full-record dialog: detail body + a Copy button that puts the whole log
    (metadata + headers + bodies) on the clipboard as plain text. */
function LogDialog({ entry, onClose }: { entry: RequestLogEntry; onClose: () => void }) {
  const t = useT();
  const [copied, setCopied] = useState(false);
  const [d, setD] = useState<RequestLogDetail | null>(null);

  useEffect(() => {
    api.getRequestLog(entry.id).then(setD).catch(() => {});
  }, [entry.id]);

  const onCopy = () => {
    // Must stay synchronous inside the click gesture: awaiting anything here
    // (e.g. the detail fetch) ends the user-activation window and the copy
    // silently fails in WKWebView — so the detail is prefetched on open.
    if (copyText(fmtLogText(entry, d))) {
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1200);
    }
  };

  return (
    <Dialog open onOpenChange={(o) => !o && onClose()}>
      {/* overflow-x-hidden: same WKWebView hardening as AddProviderModal —
          long unbroken values in the header <pre>s must never widen the modal */}
      <DialogContent className="max-h-[min(660px,100dvh)] w-[calc(100%-2rem)] max-w-[720px] gap-0 overflow-x-hidden overflow-y-auto rounded-xl p-0 sm:max-w-[720px]">
        {/* pr-12 keeps the Copy button clear of the dialog's absolute X close button */}
        <DialogHeader className="flex h-11 flex-row items-center justify-between border-b border-line pl-4 pr-12">
          <DialogTitle className="text-[13px] font-semibold">
            {t("logs.logTitle", { id: entry.id })} ·{" "}
            <span className="font-mono text-[11.5px] font-normal text-mut">{fmtTime(entry.ts)}</span>
          </DialogTitle>
          <Button variant="outline" size="sm" className="h-7 gap-1 px-2.5 text-[11px] text-mut" onClick={onCopy}>
            {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            {copied ? t("common.copied") : t("common.copy")}
          </Button>
        </DialogHeader>
        <Detail d={d} />
      </DialogContent>
    </Dialog>
  );
}

export default function RequestLogs({
  agent,
  providerId,
  initialOpenId,
  window: win = "7d",
  tab,
  onTab,
  sessionFilter,
  onSessionFilter,
  onOpenSession,
}: {
  /** The page's provider/agent filters. They narrow this table the same way
   *  they narrow the cards above it — the reader set one slice and expects
   *  every panel to be answering for it. */
  agent?: string;
  providerId?: string;
  /** Deep-linked detail (`#dashboard/log/<id>`, e.g. from the credential
   *  banner): open that log's dialog even when the row is not on the current
   *  page. Consumed on open — the hash is rewritten so a refresh or a
   *  screen switch does not spring the dialog back up. */
  initialOpenId?: number;
  /** The Dashboard's own window selector, which the **sessions** view follows
   *  and the log table deliberately does not (the table is an audit trail with
   *  its own range, in the export dialog). It is passed down rather than
   *  re-derived so the two panels cannot end up measuring different spans. */
  window?: DashboardWindow;
  /** Which view is showing, and the session the log table is narrowed to.
   *
   *  Both are **controlled by the Dashboard**, which is the component that
   *  survives a trip into a session's own page: coming back has to land where
   *  the reader left, and the session page is what now sets the filter (it is
   *  the thing that knows a row was opened). */
  tab: Tab;
  onTab: (tab: Tab) => void;
  sessionFilter: string | null;
  onSessionFilter: (id: string | null) => void;
  /** A session row was opened: show that session's own page. The Dashboard owns
   *  the navigation, because it owns the route. */
  onOpenSession: (id: string) => void;
} = { tab: "logs" } as never) {
  const t = useT();
  // The sessions view's own read: the rows, whether one has landed, its error,
  // and a sequence guard for the same reason the log table has one.
  const [sessions, setSessions] = useState<Session[]>([]);
  const [sessionsPage, setSessionsPage] = useState(1);
  const [sessionsTotal, setSessionsTotal] = useState(0);
  const [sessionsLoaded, setSessionsLoaded] = useState(false);
  const [sessionsErr, setSessionsErr] = useState<string | null>(null);
  const seqSessions = useRef(0);
  const [filter, setFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [rows, setRows] = useState<RequestLogEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [openId, setOpenId] = useState<number | null>(null);
  // A deep-linked dialog whose row is off the current page: `openId` resolves
  // against the fetched page, so an outside open keeps its own entry.
  const [externalOpen, setExternalOpen] = useState<RequestLogEntry | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [range, setRange] = useState<RangeId>("any");
  const [fromDate, setFromDate] = useState(todayLocalDate);
  const [toDate, setToDate] = useState(todayLocalDate);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);
  // Kept apart from `err`, which belongs to the export flow: one channel would
  // let a page read that succeeded afterwards clear an export warning the
  // reader has not acted on yet.
  const [loadErr, setLoadErr] = useState<string | null>(null);
  // A page read races another page read: the pager, a filter change and the
  // app's ⟳ can all be in flight at once, and a slow answer for page 2 landing
  // after page 3 would put page 2's rows under a pager that says 3.
  const seq = useRef(0);

  // What the table is showing: the status tab plus the page's own filters.
  const buildFilter = (): RequestLogFilter => ({
    ...(filter === "all" ? {} : { status: filter }),
    agent,
    provider_id: providerId,
    // The session view's drill-down; absent unless a row there was clicked, so
    // the export dialog's own filter is unchanged for every other reader.
    ...(sessionFilter ? { session_id: sessionFilter } : {}),
  });

  // What the file covers: the above, narrowed by the range chosen in the
  // export dialog. Deliberately not the same slice as the table — the range
  // belongs to the report, and asking for it up front meant committing to one
  // before knowing what you were exporting.
  const exportFilter = (): RequestLogFilter => ({
    ...buildFilter(),
    ...(range === "any"
      ? {}
      : {
          from: localMidnightUtc(fromDate),
          to: localMidnightUtcAfter(toDate),
        }),
  });

  // A deep-link already shown and dismissed: without this, the hash-reset
  // round-trip (App clears `logId` only after the row fetch lands) would let
  // the dialog spring back open from a stale `initialOpenId`.
  const [spentOpenId, setSpentOpenId] = useState<number | null>(null);

  // The page filters change what the result set is, not just which rows of it
  // are on screen, so the pager has to go back to the start. Keyed by value
  // because the props are plain strings by the time they arrive.
  const scope = `${agent ?? ""} ${providerId ?? ""} ${sessionFilter ?? ""}`;
  useEffect(() => {
    setPage(1);
    setOpenId(null);
    setExternalOpen(null);
    setSpentOpenId(null);
  }, [scope]);

  // Consume a deep link: fetch the row straight by id rather than paging the
  // table to find it, then rewrite the hash so the link is spent. The dialog's
  // existence is *not* coupled to this read (the render derives it from
  // `initialOpenId` below), precisely so that a fetch failure or a race cannot
  // swallow the banner's click — the dialog opens on the skeleton and fills in
  // when the detail lands.
  useEffect(() => {
    if (initialOpenId == null) return;
    let alive = true;
    api
      .getRequestLog(initialOpenId)
      .then((d) => {
        if (alive && d) setExternalOpen(d);
      })
      .catch(() => {})
      .finally(() => {
        if (alive) window.location.hash = "#dashboard";
      });
    return () => {
      alive = false;
    };
  }, [initialOpenId]);

  // The deep-linked dialog: a full row when one came back, otherwise a stub
  // that only carries the id — the dialog's own read fills the rest in. Never
  // waits on `getRequestLog`, so the dialog is open the moment the hash lands.
  // `spentOpenId` gates the stub: dismissing it marks it spent, so the id's
  // hash-reset round-trip cannot spring the dialog back open.
  const externalEntry =
    externalOpen ??
    (initialOpenId != null && spentOpenId !== initialOpenId
      ? rows.find((r) => r.id === initialOpenId) ?? {
          id: initialOpenId,
          ts: "",
          method: "",
          path: "",
          query: null,
          agent: null,
          attribution: null,
          provider_id: null,
          model: null,
          status_code: 0,
          error_kind: null,
          error_message: null,
          session_id: null,
          is_streaming: false,
          input_tokens: 0,
          output_tokens: 0,
          cache_read_tokens: 0,
          cache_creation_tokens: 0,
          reasoning_tokens: 0,
          usage_missing: false,
          latency_ms: null,
          first_token_ms: null,
          request_headers: null,
          response_headers: null,
          request_size: 0,
          response_size: 0,
          truncated: false,
          request_notes: null,
        }
      : null);

  // Returns what it started, so the shell's ⟳ can await it (`lib/reload.ts`) — a
  // reader that resolves before its rows land would let the spinner stop on a
  // screen that has not updated. The failures are absorbed here rather than
  // rethrown: the table says what went wrong, and a rejection would reach the
  // click handler as an unhandled rejection instead.
  //
  // `loaded` is deliberately left false on failure. It used to be set either
  // way, which dropped a failed read into the "No requests recorded yet" empty
  // state — the table reported a broken read as an empty one.
  const fetchPage = (p: number): Promise<void> => {
    const mine = ++seq.current;
    return api
      .listRequestLogs(p, PAGE_SIZE, buildFilter())
      .then((r) => {
        if (mine !== seq.current) return;
        setRows(r.rows);
        setTotal(r.total);
        setLoadErr(null);
        setLoaded(true);
        // A row the reader had expanded can be gone after a refresh; without
        // this the dialog would spring back open if that id ever reappeared.
        setOpenId((id) => (id != null && !r.rows.some((x) => x.id === id) ? null : id));
      })
      .catch((e) => {
        if (mine === seq.current) setLoadErr(errText(e));
      });
  };

  /** Read the sessions view's rows. The window is the Dashboard's, expressed as
      an instant (`windowSince`) rather than a day count: the figures above this
      card are whole local days, and a list measured in rolling hours would
      disagree with them at the day boundary. */
  const fetchSessions = (page: number) => {
    const seq = ++seqSessions.current;
    setSessionsErr(null);
    api
      .listSessions(agent, undefined, undefined, windowSince(win), page, PAGE_SIZE)
      .then((listed) => {
        if (seq !== seqSessions.current) return;
        setSessions(listed.rows);
        setSessionsTotal(listed.total);
        setSessionsLoaded(true);
      })
      .catch((e) => {
        if (seq !== seqSessions.current) return;
        setSessionsErr(errText(e));
        setSessionsLoaded(true);
      });
  };

  // This card is part of what the Dashboard is showing, so the app's ⟳ has to
  // reach it too — it is embedded there (`Dashboard.tsx`). `page` and `tab` are
  // not captured stale: `useReload` keeps the latest render's closure. A ⟳ on
  // the Logs view does not refresh the sessions view (it is not mounted); that
  // view reads when it is next opened.
  useReload(() => {
    if (tab === "sessions") {
      fetchSessions(sessionsPage);
      return Promise.resolve();
    }
    return fetchPage(page);
  });

  useEffect(() => {
    if (tab !== "logs") return;
    void fetchPage(page);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, filter, scope, tab]);

  // The sessions view reads on every open, so it can never show rows from a
  // window or an agent the reader has since changed.
  // The sessions view reads on every open and on every page, so it can never
  // show rows from a window, an agent or a page the reader has since changed.
  // The page goes back to the first one when the *slice* changes, for the same
  // reason the log table's does.
  useEffect(() => {
    if (tab !== "sessions") return;
    void fetchSessions(sessionsPage);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [tab, agent, win, sessionsPage]);

  useEffect(() => {
    setSessionsPage(1);
  }, [agent, win]);

  // Presets move both ends at once, so React batches them into one refetch.
  const applyRange = (id: RangeId) => {
    setRange(id);
    const today = todayLocalDate();
    if (id === "today") {
      setFromDate(today);
      setToDate(today);
    } else if (id === "7d" || id === "30d") {
      setFromDate(shiftLocalDate(today, id === "7d" ? -6 : -29));
      setToDate(today);
    }
  };

  const onExport = async () => {
    setErr(null);
    setNotice(null);
    setExporting(true);
    try {
      // `save()` has to be the first await in the gesture: WKWebView drops the
      // user activation across a tick and the dialog then never opens.
      const path = await save({
        title: t("logs.exportDialogTitle"),
        defaultPath: `kiwano-logs-${fromDate}_${toDate}.csv`,
        filters: [{ name: "CSV", extensions: ["csv"] }],
      });
      if (!path) {
        // Cancelling is a decision, not a failure: say nothing.
        setExporting(false);
        return;
      }
      const r = await api.exportRequestLogs(path, exportFilter());
      if (r.truncated) {
        setErr(t("logs.exportCapped", { n: r.rows_written.toLocaleString() }));
      } else {
        setNotice(t("logs.exported", { n: r.rows_written.toLocaleString() }));
        setExportOpen(false);
      }
    } catch (e) {
      setErr(e instanceof Error ? e.message : String(e));
    } finally {
      setExporting(false);
    }
  };

  useEffect(() => {
    if (!notice) return;
    // Named `timer`, not `t`: `t` is the translator here.
    const timer = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(timer);
  }, [notice]);

  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const openLog = openId != null ? rows.find((r) => r.id === openId) : undefined;

  return (
    <div className="mt-3 rounded-lg border border-line bg-surface">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-3.5 py-2.5">
        {/* The two views of this card. `h-7` and the active tab's weight keep
            this switch from reading as one control with the status filter
            beside it, which is the same `.seg` idiom one size down. */}
        <div
          role="tablist"
          className="flex overflow-hidden rounded-lg border border-line text-[12px]"
        >
          {TABS.map((x, i) => (
            <button
              key={x.id}
              role="tab"
              aria-selected={tab === x.id}
              className={`seg h-7 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${tab === x.id ? " active font-semibold" : ""}`}
              onClick={() => {
                onTab(x.id);
                // A tab switch is a change of view, not of result set: the
                // pager and the open dialog belong to the view being left.
                setOpenId(null);
                setErr(null);
                setNotice(null);
                setExportOpen(false);
              }}
            >
              {t(x.labelKey)}
            </button>
          ))}
        </div>
        {tab === "logs" && (
          <div className="flex overflow-hidden rounded-lg border border-line text-[12px]">
            {FILTERS.map((f, i) => (
              <button
                key={f.id}
                className={`seg h-6 border-line px-2.5 text-mut${i > 0 ? " border-l" : ""}${filter === f.id ? " active" : ""}`}
                onClick={() => {
                  setFilter(f.id);
                  setPage(1);
                  setOpenId(null);
                }}
              >
                {t(f.labelKey)}
              </button>
            ))}
          </div>
        )}
        {tab === "logs" && sessionFilter && (
          // Which conversation is being shown, and the way out of it. The id is
          // shortened for the chip; the full value is its title, because the id
          // is what a reader greps for.
          <span className="flex items-center gap-1.5 rounded-md border border-line px-2 py-0.5 text-[11px] text-mut">
            {t("logs.sessionFilter")}
            <span className="max-w-[180px] truncate font-mono" title={sessionFilter}>
              {sessionFilter}
            </span>
            <button
              className="text-mut hover:text-fg"
              aria-label={t("logs.clearSessionFilter")}
              onClick={() => onSessionFilter(null)}
            >
              <XIcon className="h-3 w-3" />
            </button>
          </span>
        )}
        {tab === "sessions" && providerId && (
          // The provider filter above has no meaning here and must not look
          // like it does: a session is a conversation, and an imported one was
          // never routed by this gateway at all.
          <span className="text-[11px] text-mut">{t("logs.sessionsProviderNotApplied")}</span>
        )}
        <span className="ml-auto flex items-center gap-3 text-[11px] text-mut">
          {/* Feedback lives here rather than in a toast: a transient clip that
              fades, or the error until the next attempt. The read's failure and
              the export's are separate channels — neither clears the other. */}
          {loadErr && (
            <span style={{ color: "var(--red)" }} title={loadErr}>
              {t("common.loadFailed")}
            </span>
          )}
          {err && <span style={{ color: "var(--red)" }}>{err}</span>}
          {notice && <span style={{ color: "var(--kiwi)" }}>{notice}</span>}
          {tab === "sessions" && sessionsErr && (
            <span style={{ color: "var(--red)" }} title={sessionsErr}>
              {t("common.loadFailed")}
            </span>
          )}
          {tab === "logs" ? (
            <>
              <span>{t("logs.requestCount", { n: total.toLocaleString() })}</span>
              <Button
                variant="outline"
                size="sm"
                className="h-6 gap-1 px-2 text-[11px]"
                disabled={!loaded}
                onClick={() => {
                  setErr(null);
                  setNotice(null);
                  setExportOpen(true);
                }}
                title={t("logs.exportTitle")}
              >
                <Download className="h-3 w-3" />
                {t("logs.exportCsv")}
              </Button>
            </>
          ) : (
            // No export here: there is no sessions export endpoint, and a button
            // that had to invent one would be a second code path pretending to
            // be the first. The window is named because the reader cannot see
            // it — the selector it follows is above the card.
            <>
              <span title={t("logs.sessionsWindowTitle")}>{windowLabel(t, win)}</span>
              <span>{t("logs.sessionsCount", { n: sessionsTotal.toLocaleString() })}</span>
            </>
          )}
        </span>
      </div>

      {tab === "sessions" ? (
        <>
        <SessionsTable
          rows={sessions}
          loaded={sessionsLoaded}
          onOpenSession={(id) => {
            // Opening a row leaves this card for that session's own page. The
            // filter this used to set still exists — the page sets it, from a
            // button that says what it does, which is where a reader can see it.
            setOpenId(null);
            onOpenSession(id);
          }}
        />
        {sessionsLoaded && sessionsTotal > 0 && (
          <Pager
            page={sessionsPage}
            pages={Math.max(1, Math.ceil(sessionsTotal / PAGE_SIZE))}
            onGo={(p) => setSessionsPage(p)}
          />
        )}
        </>
      ) : (
      <div>
        <div className="overflow-hidden">
          <table className="w-full text-[11.5px]">
            <thead>
              <tr className="border-b border-line text-left text-[10px] text-mut">
                <th className="px-3 py-2 font-medium">{t("logs.colTime")}</th>
                <th className="px-2 py-2 font-medium">{t("logs.colAgent")}</th>
                <th className="px-2 py-2 font-medium">{t("logs.colProvider")}</th>
                {/* Model beats Path in the list: per-agent paths repeat (claude →
                    /v1/messages, codex → /v1/chat/completions…) while the model
                    differs per request; the path lives in the detail dialog */}
                <th className="px-2 py-2 font-medium">{t("logs.colModel")}</th>
                <th className="px-2 py-2 font-medium">{t("logs.colStatus")}</th>
                <th className="px-2 py-2 text-right font-medium">{t("logs.colLatency")}</th>
                <th className="px-2 py-2 text-right font-medium">{t("logs.firstToken")}</th>
                <th className="px-2 py-2 text-right font-medium">{t("logs.colThroughput")}</th>
                <th className="px-2 py-2 text-right font-medium">{t("logs.colTokens")}</th>
                <th className="w-6 px-2 py-2" />
              </tr>
            </thead>
            <tbody className="font-mono">
              {rows.map((r) => (
                <tr
                  key={r.id}
                  className="cursor-pointer border-b border-line hover:bg-surface2"
                  onClick={() => setOpenId(r.id)}
                >
                  <td className="whitespace-nowrap px-3 py-1.5 text-mut">{fmtTime(r.ts)}</td>
                  <td className="px-2 py-1.5 font-sans">{r.agent ?? "—"}</td>
                  <td className="px-2 py-1.5 font-sans">{r.provider_id ?? "—"}</td>
                  <td className="max-w-[180px] truncate px-2 py-1.5 text-mut">{r.model ?? "—"}</td>
                  <td className="whitespace-nowrap px-2 py-1.5">
                    <StatusPill code={r.status_code} />
                    {hasCredentialFinding(r.request_notes) && <DlpBadge />}
                  </td>
                  <td className="text-right px-2 py-1.5">
                    {fmtLatency(r.latency_ms)}
                    {r.is_streaming && <span className="ml-1 text-[9.5px]">SSE</span>}
                  </td>
                  <td className="text-right px-2 py-1.5 text-mut">{fmtLatency(r.first_token_ms)}</td>
                  <td className="whitespace-nowrap text-right px-2 py-1.5 text-mut">{fmtThroughput(r)}</td>
                  <td className="text-right px-2 py-1.5 text-mut">
                    {r.input_tokens + r.output_tokens > 0
                      ? `${fmtTokens(r.input_tokens)} / ${fmtTokens(r.output_tokens)}`
                      : "—"}
                  </td>
                  <td className="px-2 py-1.5 text-mut">
                    <ChevronRight className="h-3 w-3" />
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
          {loaded && rows.length === 0 && (
            <div className="p-8 text-center text-[12px] text-mut">
              {/* A session the gateway never routed has no rows here, and that
                  empty result is correct — saying "no requests recorded yet"
                  would read as a filter that failed. */}
              {sessionFilter ? t("logs.sessionNoTraffic") : t("logs.empty")}
            </div>
          )}
        </div>

        {loaded && total > 0 && (
          <Pager page={page} pages={pages} onGo={(p) => setPage(p)} />
        )}

        {openLog ? (
          <LogDialog entry={openLog} onClose={() => setOpenId(null)} />
        ) : (
          externalEntry &&
          <LogDialog
            entry={externalEntry}
            onClose={() => {
              setExternalOpen(null);
              if (initialOpenId != null) setSpentOpenId(initialOpenId);
            }}
          />
        )}

        {/* The range lives here, not in the toolbar: the table is for browsing
            and the file is a report, and asking for the range up front made you
            commit to one before you knew what you were exporting. */}
        <Dialog open={exportOpen} onOpenChange={(o) => !o && setExportOpen(false)}>
          <DialogContent className="max-w-[360px]">
            <DialogHeader>
              <DialogTitle className="text-[13px]">{t("logs.exportDialogTitle")}</DialogTitle>
            </DialogHeader>
            <div className="space-y-3 px-5 pb-4 pt-1">
              <div>
                <div className="mb-1 text-[10px] font-medium text-mut">{t("logs.dateRange")}</div>
                <Select
                  value={range}
                  onValueChange={(v) => applyRange((v ?? "any") as RangeId)}
                >
                  <SelectTrigger className="w-full bg-surface2 text-[12px] dark:bg-surface2">
                    <SelectValue>
                      {(v) => {
                        const r = RANGES.find((x) => x.id === v);
                        return r ? t(r.labelKey) : t("logs.rangeFallback");
                      }}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {RANGES.map((r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {t(r.labelKey)}
                      </SelectItem>
                    ))}
                  </SelectContent>
                </Select>
              </div>

              {range === "custom" && (
                <div className="flex items-center gap-2 text-[11px] text-mut">
                  <Input
                    type="date"
                    value={fromDate}
                    max={toDate}
                    onChange={(e) => {
                      setFromDate(e.target.value);
                      // The native calendar stays open once a day is picked,
                      // and there is nothing left to do in it — close it rather
                      // than making the reader dismiss it by hand.
                      e.target.blur();
                    }}
                    className="h-8 flex-1 text-[11px]"
                  />
                  <span>→</span>
                  <Input
                    type="date"
                    value={toDate}
                    min={fromDate}
                    max={todayLocalDate()}
                    onChange={(e) => {
                      setToDate(e.target.value);
                      e.target.blur();
                    }}
                    className="h-8 flex-1 text-[11px]"
                  />
                </div>
              )}

              {/* The file is not the page: say so, or the range reads as a
                  filter on the table behind the dialog. */}
              <p className="text-[11px] leading-relaxed text-mut">
                {t("logs.exportNote", { n: PAGE_SIZE })}
              </p>
              {err && (
                <div className="text-[11px]" style={{ color: "var(--red)" }}>
                  {err}
                </div>
              )}

              <div className="flex justify-end gap-2 pt-1">
                <Button
                  variant="ghost"
                  size="sm"
                  className="h-7 px-3 text-[12px]"
                  onClick={() => setExportOpen(false)}
                >
                  {t("common.cancel")}
                </Button>
                <Button
                  size="sm"
                  className="h-7 gap-1 px-3 text-[12px] font-semibold"
                  disabled={exporting}
                  onClick={onExport}
                >
                  <Download className="h-3 w-3" />
                  {exporting ? t("logs.exporting") : t("logs.exportCsv")}
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>
      </div>
      )}
    </div>
  );
}
