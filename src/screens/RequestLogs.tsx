// Request logs (data-plane audit trail captured by the gateway, migration V5):
// paged metadata list + expandable detail with redacted headers and bodies.
import { Fragment, useEffect, useState } from "react";
import { ChevronDown, ChevronRight, Trash2 } from "lucide-react";
import { Button } from "@/components/ui/button";
import { api } from "../api/client";
import type { RequestLogDetail, RequestLogEntry } from "../api/types";
import { fmtLatency, fmtTokens } from "../lib/format";

const PAGE_SIZE = 50;

type StatusFilter = "all" | "ok" | "error";

const FILTERS: { id: StatusFilter; label: string }[] = [
  { id: "all", label: "All" },
  { id: "ok", label: "OK" },
  { id: "error", label: "Errors" },
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

function Detail({ id }: { id: number }) {
  const [d, setD] = useState<RequestLogDetail | null>(null);

  useEffect(() => {
    api.getRequestLog(id).then(setD).catch(() => {});
  }, [id]);

  if (!d) return <div className="px-6 pb-3 text-[11px] text-mut">Loading…</div>;

  const parse = (json: string | null): string => {
    if (!json) return "—";
    try {
      return JSON.stringify(JSON.parse(json), null, 2);
    } catch {
      return json;
    }
  };

  return (
    <div className="space-y-2 border-t border-line px-6 pb-3 pt-2">
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
            {parse(d.request_headers)}
          </pre>
        </div>
        <div>
          <div className="mb-1 text-[10px] font-medium text-mut">RESPONSE HEADERS</div>
          <pre className="max-h-32 overflow-auto rounded border border-line bg-surface2 p-2 font-mono text-[10px] leading-relaxed">
            {parse(d.response_headers)}
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

export default function RequestLogs() {
  const [filter, setFilter] = useState<StatusFilter>("all");
  const [page, setPage] = useState(1);
  const [rows, setRows] = useState<RequestLogEntry[]>([]);
  const [total, setTotal] = useState(0);
  const [openId, setOpenId] = useState<number | null>(null);
  const [loaded, setLoaded] = useState(false);

  const fetchPage = (p: number) => {
    api
      .listRequestLogs(p, PAGE_SIZE, filter === "all" ? undefined : { status: filter })
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
  }, [page, filter]);

  const pages = Math.max(1, Math.ceil(total / PAGE_SIZE));

  const onClear = () => {
    api.clearRequestLogs().then(() => {
      setPage(1);
      fetchPage(1);
    });
  };

  return (
    <section>
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <div className="flex overflow-hidden rounded-lg border border-line text-[12px]">
          {FILTERS.map((f, i) => (
            <button
              key={f.id}
              className={`seg h-7 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${filter === f.id ? " active" : ""}`}
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
          <span>{total.toLocaleString()} requests</span>
          <Button variant="outline" size="sm" className="h-7 px-2.5 text-[11px] text-mut" onClick={onClear} title="Delete every recorded request">
            <Trash2 className="h-3 w-3" />
            Clear
          </Button>
        </span>
      </div>

      <div className="p-4">
        <div className="overflow-hidden rounded-lg border border-line bg-surface">
          <table className="w-full text-[11.5px]">
            <thead>
              <tr className="border-b border-line text-left text-[10px] text-mut">
                <th className="px-3 py-2 font-medium">Time</th>
                <th className="px-2 py-2 font-medium">Agent</th>
                <th className="px-2 py-2 font-medium">Provider</th>
                <th className="px-2 py-2 font-medium">Path</th>
                <th className="px-2 py-2 font-medium">Status</th>
                <th className="px-2 py-2 text-right font-medium">Latency</th>
                <th className="px-2 py-2 text-right font-medium">Tokens</th>
                <th className="w-6 px-2 py-2" />
              </tr>
            </thead>
            <tbody className="font-mono">
              {rows.map((r) => (
                <Fragment key={r.id}>
                  <tr
                    className="cursor-pointer border-b border-line hover:bg-surface2"
                    onClick={() => setOpenId(openId === r.id ? null : r.id)}
                  >
                    <td className="whitespace-nowrap px-3 py-1.5 text-mut">{fmtTime(r.ts)}</td>
                    <td className="px-2 py-1.5 font-sans">{r.agent ?? "—"}</td>
                    <td className="px-2 py-1.5 font-sans">{r.provider_id ?? "—"}</td>
                    <td className="max-w-[180px] truncate px-2 py-1.5 text-mut">
                      {r.path}
                      {r.is_streaming && <span className="ml-1 text-[9.5px]">SSE</span>}
                    </td>
                    <td className="px-2 py-1.5">
                      <StatusPill code={r.status_code} />
                    </td>
                    <td className="text-right px-2 py-1.5">{fmtLatency(r.latency_ms)}</td>
                    <td className="text-right px-2 py-1.5 text-mut">
                      {r.input_tokens + r.output_tokens > 0
                        ? `${fmtTokens(r.input_tokens)} / ${fmtTokens(r.output_tokens)}`
                        : "—"}
                    </td>
                    <td className="px-2 py-1.5 text-mut">
                      {openId === r.id ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
                    </td>
                  </tr>
                  {openId === r.id && (
                    <tr>
                      <td colSpan={8} className="p-0">
                        <Detail id={r.id} />
                      </td>
                    </tr>
                  )}
                </Fragment>
              ))}
            </tbody>
          </table>
          {loaded && rows.length === 0 && (
            <div className="p-8 text-center text-[12px] text-mut">
              No requests recorded yet — traffic forwarded through the gateway shows up here
            </div>
          )}
        </div>

        {pages > 1 && (
          <div className="mt-3 flex items-center justify-center gap-3 text-[11.5px] text-mut">
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
      </div>
    </section>
  );
}
