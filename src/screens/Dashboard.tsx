// Dashboard (design/index.html #s-dashboard)
import { useEffect, useState } from "react";
import { CircleDollarSign, Coins, Send, Timer } from "lucide-react";
import {
  Select,
  SelectContent,
  SelectItem,
  SelectTrigger,
  SelectValue,
} from "@/components/ui/select";
import { api } from "../api/client";
import type { DashboardData, DashboardWindow } from "../api/types";
import { fmtMoney, fmtTokens } from "../lib/format";
import RequestLogs from "./RequestLogs";

const WINDOWS: { id: DashboardWindow; label: string }[] = [
  { id: "today", label: "Today" },
  { id: "7d", label: "Last 7 days" },
  { id: "30d", label: "Last 30 days" },
];

/** Catmull-Rom → Bézier smoothing (programmatic equivalent of the prototype's hand-drawn C curves) */
function smoothPath(pts: [number, number][]): string {
  if (pts.length < 2) return pts.length ? `M${pts[0][0]},${pts[0][1]}` : "";
  let d = `M${pts[0][0]},${pts[0][1]}`;
  for (let i = 0; i < pts.length - 1; i++) {
    const p0 = pts[Math.max(0, i - 1)];
    const p1 = pts[i];
    const p2 = pts[i + 1];
    const p3 = pts[Math.min(pts.length - 1, i + 2)];
    const c1x = p1[0] + (p2[0] - p0[0]) / 6;
    const c1y = p1[1] + (p2[1] - p0[1]) / 6;
    const c2x = p2[0] - (p3[0] - p1[0]) / 6;
    const c2y = p2[1] - (p3[1] - p1[1]) / 6;
    d += ` C${c1x},${c1y} ${c2x},${c2y} ${p2[0]},${p2[1]}`;
  }
  return d;
}

function TrendChart({ data }: { data: DashboardData }) {
  const W = 900;
  const H = 140;
  const top = 22;
  const bottom = 112;
  // Both units get their own scale and their own tick column: requests on the
  // left, tokens on the right. Sharing one axis meant normalising each series
  // to its own maximum with nothing to read against, so two series that move
  // together — which requests and tokens usually do — drew one line.
  const left = 38;
  const right = 862;
  const n = data.trend.length;
  const x = (i: number) => left + (i * (right - left)) / Math.max(1, n - 1);
  // Empty DB gives max=0 → division yields NaN → SVG error; clamp with max(1, ·)
  const reqMax = Math.max(1, ...data.trend.map((t) => t.requests));
  const tokMax = Math.max(1, ...data.trend.map((t) => t.tokens));
  const yReq = (v: number) => bottom - (v / reqMax) * (bottom - top);
  const yTok = (v: number) => bottom - (v / tokMax) * (bottom - top);
  const reqPts = data.trend.map((t, i) => [x(i), yReq(t.requests)] as [number, number]);
  const tokPts = data.trend.map((t, i) => [x(i), yTok(t.tokens)] as [number, number]);
  const reqPath = smoothPath(reqPts);
  const areaPath = `${reqPath} L${right},${bottom} L${left},${bottom} Z`;
  const last = reqPts[reqPts.length - 1];
  // Grid and ticks are the same three levels, so a label always sits on a line.
  const ticks = [
    { y: top, req: reqMax, tok: tokMax },
    { y: (top + bottom) / 2, req: reqMax / 2, tok: tokMax / 2 },
    { y: bottom, req: 0, tok: 0 },
  ];
  const tick = (v: number) => (v >= 1000 ? `${Math.round(v / 1000)}k` : String(Math.round(v)));

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="mt-2 w-full" style={{ height: 128 }}>
      <defs>
        <linearGradient id="gk" x1="0" y1="0" x2="0" y2="1">
          <stop offset="0%" stopColor="var(--kiwi)" stopOpacity="0.22" />
          <stop offset="100%" stopColor="var(--kiwi)" stopOpacity="0" />
        </linearGradient>
      </defs>
      <g stroke="var(--line)" strokeDasharray="3 4">
        {ticks.map((t) => (
          <line key={t.y} x1={left} y1={t.y} x2={right} y2={t.y} />
        ))}
      </g>
      <path d={areaPath} fill="url(#gk)" />
      <path d={reqPath} fill="none" stroke="var(--kiwi)" strokeWidth="1.8" />
      {/* Solid, and warm: a dashed blue line beside a green one reads as the
          same family, and the dash was doing work the colour should do. */}
      <path d={smoothPath(tokPts)} fill="none" stroke="var(--orange)" strokeWidth="1.6" />
      {last && <circle cx={last[0]} cy={last[1]} r="3" fill="var(--kiwi)" stroke="var(--bg)" strokeWidth="1.5" />}
      <g fill="var(--mut)" fontSize="9.5" fontFamily="JetBrains Mono">
        {ticks.map((t) => (
          <g key={t.y}>
            <text x={left - 6} y={t.y + 3} textAnchor="end">
              {tick(t.req)}
            </text>
            <text x={right + 6} y={t.y + 3} textAnchor="start">
              {tick(t.tok)}
            </text>
          </g>
        ))}
        {data.trend.map((t, i) => (
          <text key={t.date} x={x(i) - 14} y={H - 8}>
            {t.date}
          </text>
        ))}
      </g>
    </svg>
  );
}

/** Share donut. Each segment is a circle whose dash length is its slice of the
    circumference — same hand-rolled SVG as the trend chart, no chart library.
    A hair of gap keeps adjacent segments apart; a lone segment is drawn whole. */
function Donut({
  slices,
  total,
  size = 108,
  thickness = 15,
}: {
  slices: { color: string; pct: number }[];
  total: number;
  size?: number;
  thickness?: number;
}) {
  const r = (size - thickness) / 2;
  const circumference = 2 * Math.PI * r;
  let used = 0;
  return (
    <svg width={size} height={size} viewBox={`0 0 ${size} ${size}`} className="shrink-0">
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="var(--surface2)"
        strokeWidth={thickness}
      />
      <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
        {slices.map((s, i) => {
          const gap = slices.length > 1 ? 1.5 : 0;
          const len = Math.max(0, (s.pct / 100) * circumference - gap);
          const dash = `${len} ${circumference - len}`;
          const offset = -(used / 100) * circumference;
          used += s.pct;
          return (
            <circle
              key={i}
              cx={size / 2}
              cy={size / 2}
              r={r}
              fill="none"
              stroke={s.color}
              strokeWidth={thickness}
              strokeDasharray={dash}
              strokeDashoffset={offset}
            />
          );
        })}
      </g>
      <text
        x="50%"
        y="48%"
        textAnchor="middle"
        fill="var(--ink)"
        fontSize="16"
        fontWeight="600"
        fontFamily="JetBrains Mono"
      >
        {total}
      </text>
      <text x="50%" y="62%" textAnchor="middle" fill="var(--mut)" fontSize="9">
        requests
      </text>
    </svg>
  );
}

function Delta({ pct, invert }: { pct: number; invert?: boolean }) {
  const up = pct >= 0;
  const good = invert ? !up : up;
  return (
    <span className="text-[10.5px] font-normal" style={{ color: good ? "var(--kiwi)" : "var(--red)" }}>
      {up ? "↑" : "↓"}
      {Math.abs(pct)}%
    </span>
  );
}

export default function Dashboard() {
  const [win, setWin] = useState<DashboardWindow>("7d");
  // "all" = no filter; otherwise the provider id / agent id
  const [providerFilter, setProviderFilter] = useState("all");
  const [agentFilter, setAgentFilter] = useState("all");
  const [data, setData] = useState<DashboardData | null>(null);
  // Costs arrive already converted to the preferred currency (Settings)
  const [pref, setPref] = useState("CNY");

  useEffect(() => {
    api.getCurrencyMeta().then((m) => setPref(m.preferred)).catch(() => {});
  }, []);

  useEffect(() => {
    api
      .getDashboard(
        win,
        providerFilter === "all" ? undefined : providerFilter,
        agentFilter === "all" ? undefined : agentFilter,
      )
      .then((d) => {
        setData(d);
        // A window switch can drop the selected provider/agent from the
        // window's active set — fall back to All instead of a dead selection
        if (providerFilter !== "all" && !d.filter_providers.some((p) => p.id === providerFilter)) {
          setProviderFilter("all");
        }
        if (agentFilter !== "all" && !d.filter_agents.some((a) => a.id === agentFilter)) {
          setAgentFilter("all");
        }
      });
  }, [win, providerFilter, agentFilter]);

  if (!data) return <div className="p-8 text-center text-[12px] text-mut">Loading…</div>;

  const totalTokens = data.input_tokens + data.output_tokens;

  return (
    <section>
      <div className="flex h-11 items-center gap-2 border-b border-line px-4">
        <div className="flex overflow-hidden rounded-lg border border-line text-[12px]">
          {WINDOWS.map((w, i) => (
            <button
              key={w.id}
              className={`seg h-7 border-line px-3 text-mut${i > 0 ? " border-l" : ""}${win === w.id ? " active" : ""}`}
              onClick={() => setWin(w.id)}
            >
              {w.label}
            </button>
          ))}
        </div>
        {/* Filter options come from the window's active set (traffic in the
            window), independent of the current selection */}
        <Select value={providerFilter} onValueChange={(v) => setProviderFilter(v ?? "all")}>
          <SelectTrigger size="sm" className="h-7 bg-surface text-[12px] text-mut dark:bg-surface">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All providers</SelectItem>
            {data.filter_providers.map((p) => (
              <SelectItem key={p.id} value={p.id}>
                {p.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <Select value={agentFilter} onValueChange={(v) => setAgentFilter(v ?? "all")}>
          <SelectTrigger size="sm" className="h-7 bg-surface text-[12px] text-mut dark:bg-surface">
            <SelectValue />
          </SelectTrigger>
          <SelectContent>
            <SelectItem value="all">All agents</SelectItem>
            {data.filter_agents.map((a) => (
              <SelectItem key={a.id} value={a.id}>
                {a.label}
              </SelectItem>
            ))}
          </SelectContent>
        </Select>
        <span className="ml-auto text-[11px] text-mut">Data stays in local SQLite</span>
      </div>

      <div className="p-4">
        {/* Stats bar */}
        <div className="flex divide-line rounded-lg border border-line bg-surface">
          <div className="flex-1 p-3">
            <div className="flex items-center gap-1 text-[10.5px] text-mut">
              <Send className="h-3 w-3" />
              Requests
            </div>
            <div className="mt-1 font-mono text-[19px] font-semibold">
              {data.requests.toLocaleString()} <Delta pct={data.requests_delta_pct} />
            </div>
          </div>
          <div className="flex-1 p-3">
            <div className="flex items-center gap-1 text-[10.5px] text-mut">
              <Coins className="h-3 w-3" />
              Tokens
            </div>
            <div
              className="mt-1 font-mono text-[19px] font-semibold"
              title={`in ${fmtTokens(data.input_tokens)} (cache hits ${fmtTokens(data.cache_read_tokens)}, billed at 1/10) · out ${fmtTokens(data.output_tokens)}`}
            >
              {fmtTokens(totalTokens)}{" "}
              <span className="text-[10.5px] font-normal text-mut">
                in {fmtTokens(data.input_tokens)} / out {fmtTokens(data.output_tokens)}
              </span>
            </div>
          </div>
          <div className="flex-1 p-3">
            <div className="flex items-center gap-1 text-[10.5px] text-mut">
              <CircleDollarSign className="h-3 w-3" />
              Est. cost
            </div>
            <div className="mt-1 font-mono text-[19px] font-semibold">
              {fmtMoney(data.cost, pref)} <span className="text-[10.5px] font-normal text-mut">at Hub price</span>
            </div>
          </div>
          <div className="flex-1 p-3">
            <div className="flex items-center gap-1 text-[10.5px] text-mut">
              <Timer className="h-3 w-3" />
              Avg latency
            </div>
            <div className="mt-1 font-mono text-[19px] font-semibold">
              {(Math.round(data.latency_ms / 100) / 10).toFixed(1)}s <Delta pct={-data.latency_delta_pct} invert />
            </div>
          </div>
        </div>

        {/* Request trend */}
        <div className="mt-3 rounded-lg border border-line bg-surface p-3.5">
          <div className="flex items-center justify-between">
            <h3 className="text-[12.5px] font-semibold">Request trend</h3>
            <div className="flex items-center gap-3 text-[10.5px] text-mut">
              <span className="flex items-center gap-1">
                <span className="h-1 w-2 rounded-full" style={{ background: "var(--kiwi)" }} />
                Requests
              </span>
              <span className="flex items-center gap-1">
                <span className="h-1 w-2 rounded-full" style={{ background: "var(--orange)" }} />
                Tokens
              </span>
            </div>
          </div>
          <TrendChart data={data} />
        </div>

        <div className="mt-3 grid grid-cols-2 gap-3">
          {/* Provider breakdown */}
          <div className="rounded-lg border border-line bg-surface p-3.5">
            <h3 className="text-[12.5px] font-semibold">By provider</h3>
            {data.by_provider.length === 0 ? (
              <div className="mt-3 text-[11.5px] text-mut">No traffic in this window.</div>
            ) : (
              <div className="mt-2.5 flex items-center gap-4">
                <Donut
                  slices={data.by_provider.map((p) => ({ color: p.color, pct: p.pct }))}
                  total={data.by_provider.reduce((sum, p) => sum + p.requests, 0)}
                />
                <div className="min-w-0 flex-1 space-y-2">
                  {data.by_provider.map((p) => (
                    <div key={p.name} className="flex items-center justify-between gap-2 text-[11.5px]">
                      <span className="flex min-w-0 items-center gap-1.5">
                        <span className="h-2 w-2 shrink-0 rounded-sm" style={{ background: p.color }} />
                        <span className="truncate">{p.name}</span>
                      </span>
                      <span className="shrink-0 font-mono text-mut">
                        {p.pct}% · {fmtMoney(p.cost, pref)}
                      </span>
                    </div>
                  ))}
                </div>
              </div>
            )}
          </div>
          {/* Agent breakdown */}
          <div className="rounded-lg border border-line bg-surface p-3.5">
            <h3 className="text-[12.5px] font-semibold">By agent</h3>
            <table className="mt-2 w-full text-[11.5px]">
              <thead>
                <tr className="text-left text-mut text-[10px]">
                  <th className="pb-1.5 font-medium">Agent</th>
                  <th className="pb-1.5 text-right font-medium">Requests</th>
                  <th className="pb-1.5 text-right font-medium">Tokens</th>
                  <th className="pb-1.5 text-right font-medium">Cost</th>
                </tr>
              </thead>
              <tbody className="font-mono">
                {data.by_agent.map((a) => (
                  <tr key={a.agent} className="border-t border-line">
                    <td className="py-1.5 font-sans">{a.label}</td>
                    <td className="text-right">{a.requests}</td>
                    <td className="text-right">{a.tokens}</td>
                    <td className="text-right">{fmtMoney(a.cost, pref)}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </div>

        {/* Full data-plane request log (gateway audit trail) */}
        <RequestLogs />
      </div>
    </section>
  );
}
