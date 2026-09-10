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

type TrendMetric = "requests" | "tokens";

/** One metric as columns.
 *
 * Requests and tokens are different scales, so they get a chart each and a
 * switch between them rather than a shared axis: bar heights invite direct
 * comparison, which two units on one frame cannot honestly support. The
 * switch also means only one series is ever on screen, so identity never
 * rests on telling two colours apart.
 */
function TrendBars({ data, metric }: { data: DashboardData; metric: TrendMetric }) {
  const W = 900;
  const H = 140;
  const top = 22;
  const bottom = 112;
  const left = 52;
  const right = 884;
  const n = data.trend.length;
  const value = (t: DashboardData["trend"][number]) =>
    metric === "requests" ? t.requests : t.tokens;
  // Empty DB gives max=0 → division yields NaN → SVG error; clamp with max(1, ·)
  const max = Math.max(1, ...data.trend.map(value));
  const slot = (right - left) / Math.max(1, n);
  // Thin marks with a 2px gap between them, and a cap so a one-bucket window
  // ("today") does not draw a single slab the width of the card.
  const barWidth = Math.max(2, Math.min(slot - 2, 44));
  const color = metric === "requests" ? "var(--kiwi)" : "var(--orange)";
  const ticks = [
    { y: top, v: max },
    { y: (top + bottom) / 2, v: max / 2 },
    { y: bottom, v: 0 },
  ];
  const tick = (v: number) => (v >= 1000 ? `${Math.round(v / 1000)}k` : String(Math.round(v)));

  return (
    <svg viewBox={`0 0 ${W} ${H}`} className="mt-2 w-full" style={{ height: 128 }}>
      <g stroke="var(--line)" strokeDasharray="3 4">
        {ticks.map((t) => (
          <line key={t.y} x1={left} y1={t.y} x2={right} y2={t.y} />
        ))}
      </g>
      {data.trend.map((t, i) => {
        const v = value(t);
        const h = Math.max(1, (bottom - top) * (v / max));
        return (
          <rect
            key={t.date}
            x={left + slot * i + (slot - barWidth) / 2}
            y={bottom - h}
            width={barWidth}
            height={h}
            rx="2"
            fill={color}
          >
            {/* Native tooltip: the chart's only hover affordance so far. */}
            <title>{`${t.date} · ${tick(v)} ${metric}`}</title>
          </rect>
        );
      })}
      <g fill="var(--mut)" fontSize="9.5" fontFamily="JetBrains Mono">
        {ticks.map((t) => (
          <text key={t.y} x={left - 6} y={t.y + 3} textAnchor="end">
            {tick(t.v)}
          </text>
        ))}
        {data.trend.map((t, i) => (
          <text key={t.date} x={left + slot * (i + 0.5)} y={H - 8} textAnchor="middle">
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
  // The trend chart shows one metric at a time (see TrendBars).
  const [metric, setMetric] = useState<TrendMetric>("requests");
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
            <h3 className="text-[12.5px] font-semibold">Usage trend</h3>
            <div className="flex overflow-hidden rounded-lg border border-line text-[11.5px]">
              {(["requests", "tokens"] as TrendMetric[]).map((m, i) => (
                <button
                  key={m}
                  className={`seg h-6 border-line px-2.5 text-mut${i > 0 ? " border-l" : ""}${metric === m ? " active" : ""}`}
                  onClick={() => setMetric(m)}
                >
                  {m === "requests" ? "Requests" : "Tokens"}
                </button>
              ))}
            </div>
          </div>
          <TrendBars data={data} metric={metric} />
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
