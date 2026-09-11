// Request logs (data-plane audit trail captured by the gateway, migration V5),
// rendered as the trailing card section of the Dashboard: paged metadata list
// with status filter, clear-all. Clicking a row opens a dialog with the full
// record (redacted headers + bodies) and a Copy button that puts the whole
// log as plain text on the clipboard.
import { useEffect, useState } from "react";
import { Check, ChevronRight, Copy, Download } from "lucide-react";
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
import type { RequestLogDetail, RequestLogEntry, RequestLogFilter } from "../api/types";
import {
  localMidnightUtc,
  localMidnightUtcAfter,
  shiftLocalDate,
  todayLocalDate,
} from "../lib/daterange";
import { fmtLatency, fmtTokens } from "../lib/format";

const PAGE_SIZE = 10;

type StatusFilter = "all" | "ok" | "error";

const FILTERS: { id: StatusFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "ok", label: "OK" },
  { id: "error", label: "Errors" },
];

type RangeId = "any" | "today" | "7d" | "30d" | "custom";

const RANGES: { id: RangeId; label: string }[] = [
  { id: "any", label: "Any time" },
  { id: "today", label: "Today" },
  { id: "7d", label: "Last 7 days" },
  { id: "30d", label: "Last 30 days" },
  { id: "custom", label: "Custom…" },
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
function fmtLogText(r: RequestLogEntry, d?: RequestLogDetail | null): string {
  const lines = [
    `#${r.id} · ${r.ts}`,
    `${r.method} ${r.path}${r.query ? `?${r.query}` : ""}`,
    `agent: ${r.agent ?? "—"} · provider: ${r.provider_id ?? "—"} · model: ${r.model ?? "—"} · status: ${r.status_code}`,
    `latency: ${r.latency_ms ?? "—"}ms · tokens: in ${r.input_tokens} / out ${r.output_tokens}`,
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

function Detail({ d }: { d: RequestLogDetail | null }) {
  if (!d) return <div className="px-5 pb-4 text-[11px] text-mut">Loading…</div>;

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
            Session <span className="font-mono text-ink">{d.session_id}</span>
          </span>
        )}
        {d.is_streaming && d.first_token_ms != null && (
          <span>
            First token <span className="font-mono text-ink">{fmtLatency(d.first_token_ms)}</span>
          </span>
        )}
        <span>
          Request <span className="font-mono text-ink">{fmtBytes(d.request_size)}</span> · Response{" "}
          <span className="font-mono text-ink">{fmtBytes(d.response_size)}</span>
          {d.truncated && " (body truncated)"}
        </span>
      </div>
      {d.error_kind && (
        <div className="rounded border border-line bg-surface2 p-2 text-[11px]" style={{ color: "var(--red)" }}>
          <span className="font-mono font-semibold">{d.error_kind}</span> {d.error_message}
        </div>
      )}
      <div className="grid grid-cols-2 gap-2">
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">REQUEST HEADERS</div>
          <pre className="max-h-32 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {prettyJson(d.request_headers)}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">RESPONSE HEADERS</div>
          <pre className="max-h-32 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {prettyJson(d.response_headers)}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">REQUEST BODY</div>
          <pre className="max-h-64 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {d.request_body ?? "—"}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">RESPONSE BODY{d.is_streaming ? " (client-visible stream)" : ""}</div>
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
            Log #{entry.id} · <span className="font-mono text-[11.5px] font-normal text-mut">{fmtTime(entry.ts)}</span>
          </DialogTitle>
          <Button variant="outline" size="sm" className="h-7 gap-1 px-2.5 text-[11px] text-mut" onClick={onCopy}>
            {copied ? <Check className="h-3 w-3" /> : <Copy className="h-3 w-3" />}
            {copied ? "Copied" : "Copy"}
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
}: {
  /** The page's provider/agent filters. They narrow this table the same way
   *  they narrow the cards above it — the reader set one slice and expects
   *  every panel to be answering for it. */
  agent?: string;
  providerId?: string;
} = {}) {
  const [filter, setFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [rows, setRows] = useState<RequestLogEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [openId, setOpenId] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [range, setRange] = useState<RangeId>("any");
  const [fromDate, setFromDate] = useState(todayLocalDate);
  const [toDate, setToDate] = useState(todayLocalDate);
  const [exportOpen, setExportOpen] = useState(false);
  const [exporting, setExporting] = useState(false);
  const [notice, setNotice] = useState<string | null>(null);
  const [err, setErr] = useState<string | null>(null);

  // What the table is showing: the status tab plus the page's own filters.
  const buildFilter = (): RequestLogFilter => ({
    ...(filter === "all" ? {} : { status: filter }),
    agent,
    provider_id: providerId,
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

  // The page filters change what the result set is, not just which rows of it
  // are on screen, so the pager has to go back to the start. Keyed by value
  // because the props are plain strings by the time they arrive.
  const scope = `${agent ?? ""} ${providerId ?? ""}`;
  useEffect(() => {
    setPage(1);
    setOpenId(null);
  }, [scope]);

  const fetchPage = (p: number) => {
    api
      .listRequestLogs(p, PAGE_SIZE, buildFilter())
      .then((r) => {
        setRows(r.rows);
        setTotal(r.total);
        setLoaded(true);
      })
      .catch(() => setLoaded(true));
  };

  useEffect(() => {
    fetchPage(page);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [page, filter, scope]);

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
        title: "Export logs",
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
        setErr(
          `Capped at ${r.rows_written.toLocaleString()} rows — narrow the range for the rest.`,
        );
      } else {
        setNotice(`Exported ${r.rows_written.toLocaleString()} rows`);
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
    const t = window.setTimeout(() => setNotice(null), 4000);
    return () => window.clearTimeout(t);
  }, [notice]);

  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));
  const openLog = openId != null ? rows.find((r) => r.id === openId) : undefined;

  return (
    <div className="mt-3 rounded-lg border border-line bg-surface">
      <div className="flex flex-wrap items-center gap-x-3 gap-y-2 border-b border-line px-3.5 py-2.5">
        <h3 className="text-[12.5px] font-semibold">Logs</h3>
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
              {f.label}
            </button>
          ))}
        </div>
        <span className="ml-auto flex items-center gap-3 text-[11px] text-mut">
          {/* Feedback lives here rather than in a toast: a transient clip that
              fades, or the error until the next attempt. */}
          {err && <span style={{ color: "var(--red)" }}>{err}</span>}
          {notice && <span style={{ color: "var(--kiwi)" }}>{notice}</span>}
          <span>{total.toLocaleString()} requests</span>
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
            title="Choose a date range and write it to a CSV file"
          >
            <Download className="h-3 w-3" />
            Export CSV
          </Button>
        </span>
      </div>

      <div>
        <div className="overflow-hidden">
          <table className="w-full text-[11.5px]">
            <thead>
              <tr className="border-b border-line text-left text-[10px] text-mut">
                <th className="px-3 py-2 font-medium">Time</th>
                <th className="px-2 py-2 font-medium">Agent</th>
                <th className="px-2 py-2 font-medium">Provider</th>
                {/* Model beats Path in the list: per-agent paths repeat (claude →
                    /v1/messages, codex → /v1/chat/completions…) while the model
                    differs per request; the path lives in the detail dialog */}
                <th className="px-2 py-2 font-medium">Model</th>
                <th className="px-2 py-2 font-medium">Status</th>
                <th className="px-2 py-2 text-right font-medium">Latency</th>
                <th className="px-2 py-2 text-right font-medium">First token</th>
                <th className="px-2 py-2 text-right font-medium">Tok/s</th>
                <th className="px-2 py-2 text-right font-medium">Tokens</th>
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
                  <td className="px-2 py-1.5">
                    <StatusPill code={r.status_code} />
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
              No requests recorded yet — traffic forwarded through the gateway shows up here
            </div>
          )}
        </div>

        {loaded && total > 0 && (
          <div className="flex items-center justify-center gap-3 pb-3 text-[11.5px] text-mut">
            <Button variant="outline" size="sm" className="h-7 px-2.5 text-[11px]" disabled={page <= 1} onClick={() => setPage(page - 1)}>
              Prev
            </Button>
            <span className="font-mono">
              {page} / {pages}
            </span>
            <Button variant="outline" size="sm" className="h-7 px-2.5 text-[11px]" disabled={page >= pages} onClick={() => setPage(page + 1)}>
              Next
            </Button>
          </div>
        )}

        {openLog && <LogDialog entry={openLog} onClose={() => setOpenId(null)} />}

        {/* The range lives here, not in the toolbar: the table is for browsing
            and the file is a report, and asking for the range up front made you
            commit to one before you knew what you were exporting. */}
        <Dialog open={exportOpen} onOpenChange={(o) => !o && setExportOpen(false)}>
          <DialogContent className="max-w-[360px]">
            <DialogHeader>
              <DialogTitle className="text-[13px]">Export logs</DialogTitle>
            </DialogHeader>
            <div className="space-y-3 px-5 pb-4 pt-1">
              <div>
                <div className="mb-1 text-[10px] font-medium text-mut">DATE RANGE</div>
                <Select
                  value={range}
                  onValueChange={(v) => applyRange((v ?? "any") as RangeId)}
                >
                  <SelectTrigger className="w-full bg-surface2 text-[12px] dark:bg-surface2">
                    <SelectValue>
                      {(v) => RANGES.find((r) => r.id === v)?.label ?? "All time"}
                    </SelectValue>
                  </SelectTrigger>
                  <SelectContent>
                    {RANGES.map((r) => (
                      <SelectItem key={r.id} value={r.id}>
                        {r.label}
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
                Every request in the range is written, not just the {PAGE_SIZE} on screen.
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
                  Cancel
                </Button>
                <Button
                  size="sm"
                  className="h-7 gap-1 px-3 text-[12px] font-semibold"
                  disabled={exporting}
                  onClick={onExport}
                >
                  <Download className="h-3 w-3" />
                  {exporting ? "Exporting…" : "Export CSV"}
                </Button>
              </div>
            </div>
          </DialogContent>
        </Dialog>
      </div>
    </div>
  );
}
