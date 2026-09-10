// Dashboard (design/index.html #s-dashboard)
import {
  useEffect,
  useRef,
  useState,
  type KeyboardEvent as ReactKeyboardEvent,
} from "react";
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
 *
 * The readout is per-bar, not a crosshair: on a column chart the mark already
 * says where on the axis you are, so the bar is the hit target and the value
 * hangs off it. The arrow keys walk the same bars, giving the keyboard the
 * readout the pointer gets.
 */
function TrendBars({ data, metric }: { data: DashboardData; metric: TrendMetric }) {
  const W = 900;
  const H = 140;
  const top = 22;
  const bottom = 112;
  const left = 52;
  const right = 884;
  const TIP_H = 24; // the readout's own height; only offsets where it lands
  const n = data.trend.length;
  const value = (t: DashboardData["trend"][number]) =>
    metric === "requests" ? t.requests : t.tokens;
  // Empty DB gives max=0 → division yields NaN → SVG error; clamp with max(1, ·)
  const max = Math.max(1, ...data.trend.map(value));
  const slot = (right - left) / Math.max(1, n);
  // Thin marks with a 2px gap between them, and a cap so a short window does
  // not draw slabs the width of the card.
  const barWidth = Math.max(2, Math.min(slot - 2, 44));
  const color = metric === "requests" ? "var(--kiwi)" : "var(--orange)";
  const ticks = [
    { y: top, v: max },
    { y: (top + bottom) / 2, v: max / 2 },
    { y: bottom, v: 0 },
  ];
  const tick = (v: number) => (v >= 1000 ? `${Math.round(v / 1000)}k` : String(Math.round(v)));
  const barHeight = (v: number) => Math.max(1, (bottom - top) * (v / max));
  // "today" is 24 hourly bars: printing every label would overlap them into a
  // smear, so keep roughly eight (00:00, 03:00, …). Shorter windows are
  // unchanged — the divisor bottoms out at 1.
  const labelEvery = Math.ceil(n / 8);

  // Which bar the readout is about, and where to hang it. Both are measured in
  // wrapper pixels off the bar's own hit area: the SVG letterboxes (viewBox
  // aspect ≠ box), so its rendered box beats re-deriving the scale from W.
  const [hover, setHover] = useState<number | null>(null);
  const [tip, setTip] = useState({ x: 0, y: 0 });
  const wrap = useRef<HTMLDivElement>(null);
  const bars = useRef<(SVGRectElement | null)[]>([]);

  const pointAt = (i: number) => {
    setHover(i);
    const hit = bars.current[i];
    const box = wrap.current?.getBoundingClientRect();
    if (!hit || !box) return;
    const r = hit.getBoundingClientRect();
    // The hit area spans the full viewBox height, so its on-screen height is the
    // scale factor — close enough to place the readout on the bar's top edge,
    // where it reads as belonging to that bar rather than to the chart.
    const scale = r.height / H;
    const topPx = r.top - box.top + (bottom - barHeight(value(data.trend[i]))) * scale;
    setTip({ x: r.left + r.width / 2 - box.left, y: Math.max(0, topPx - TIP_H) });
  };

  const onKeyDown = (e: ReactKeyboardEvent<SVGSVGElement>) => {
    if (e.key !== "ArrowLeft" && e.key !== "ArrowRight") return;
    e.preventDefault();
    const step = e.key === "ArrowRight" ? 1 : -1;
    pointAt(Math.min(n - 1, Math.max(0, (hover ?? (step > 0 ? -1 : n)) + step)));
  };

  const shown = hover === null ? undefined : data.trend[hover];

  return (
    <div ref={wrap} className="relative mt-2">
      <svg
        viewBox={`0 0 ${W} ${H}`}
        className="w-full"
        style={{ height: 128 }}
        tabIndex={0}
        role="img"
        aria-label={`Usage trend · ${metric} · ${n} buckets`}
        onKeyDown={onKeyDown}
        onFocus={() => pointAt(hover ?? 0)}
        onBlur={() => setHover(null)}
        onPointerLeave={() => setHover(null)}
      >
        <g stroke="var(--line)" strokeDasharray="3 4">
          {ticks.map((t) => (
            <line key={t.y} x1={left} y1={t.y} x2={right} y2={t.y} />
          ))}
        </g>
        {data.trend.map((t, i) => {
          const v = value(t);
          const h = barHeight(v);
          return (
            <rect
              key={t.date}
              x={left + slot * i + (slot - barWidth) / 2}
              y={bottom - h}
              width={barWidth}
              height={h}
              rx="2"
              fill={color}
              // The hovered bar holds full strength while its neighbours recede,
              // which shows the chart answering without adding an outline.
              opacity={hover === null || hover === i ? 1 : 0.45}
            />
          );
        })}
        <g fill="var(--mut)" fontSize="9.5" fontFamily="JetBrains Mono">
          {ticks.map((t) => (
            <text key={t.y} x={left - 6} y={t.y + 3} textAnchor="end">
              {tick(t.v)}
            </text>
          ))}
          {data.trend.map((t, i) =>
            i % labelEvery === 0 ? (
              <text key={t.date} x={left + slot * (i + 0.5)} y={H - 8} textAnchor="middle">
                {t.date}
              </text>
            ) : null,
          )}
        </g>
        {/* Full-slot, full-height hit areas drawn last, so the pointer only has
            to be somewhere in the column rather than on the painted bar. Enter
            is enough to switch: every point in a column reads the same. */}
        {data.trend.map((t, i) => (
          <rect
            key={`hit-${t.date}`}
            ref={(el) => {
              bars.current[i] = el;
            }}
            x={left + slot * i}
            y={0}
            width={slot}
            height={H}
            // Hit the geometry without painting it: `all` ignores paint, where a
            // transparent fill rests on the engine counting zero alpha as ink.
            fill="none"
            style={{ pointerEvents: "all" }}
            onPointerEnter={() => pointAt(i)}
          />
        ))}
      </svg>
      {/* Value leads, bucket follows — the reader already knows the series. */}
      {shown && (
        <div
          role="status"
          className="pointer-events-none absolute z-10 -translate-x-1/2 rounded-md border px-2 py-[3px] text-[10.5px] whitespace-nowrap"
          style={{
            left: tip.x,
            top: tip.y,
            background: "var(--panel)",
            borderColor: "var(--line)",
          }}
        >
          <span className="font-semibold">{tick(value(shown))}</span>
          <span style={{ color: "var(--mut)" }}>
            {" "}
            {metric} · {shown.date}
          </span>
        </div>
      )}
    </div>
  );
}

/** Share donut. Each segment is a circle whose dash length is its slice of the
    circumference — same hand-rolled SVG as the trend chart, no chart library.
    A hair of gap keeps adjacent segments apart; a lone segment is drawn whole.

    Hovering a segment (or its legend row, which shares the state) dims the
    others and moves the centre readout onto that slice. The centre rather than
    a floating tooltip: the hole is empty space right beside the pointer, and a
    15px ring is too thin to hang a label off. */
function Donut({
  slices,
  total,
  active,
  onHover,
  size = 108,
  thickness = 15,
}: {
  slices: { color: string; pct: number; value: number; label: string }[];
  total: number;
  active: number | null;
  onHover: (i: number | null) => void;
  size?: number;
  thickness?: number;
}) {
  const r = (size - thickness) / 2;
  const circumference = 2 * Math.PI * r;
  // The hole is 78px across at the default size; a provider name longer than
  // this would run under the ring, so it gets clipped with an ellipsis.
  const shown = active === null ? null : slices[active];
  const label = shown && (shown.label.length > 10 ? `${shown.label.slice(0, 9)}…` : shown.label);
  let used = 0;
  const arcs = slices.map((s, i) => {
    const gap = slices.length > 1 ? 1.5 : 0;
    const len = Math.max(0, (s.pct / 100) * circumference - gap);
    used += s.pct;
    return { i, dash: `${len} ${circumference - len}`, offset: -((used - s.pct) / 100) * circumference };
  });
  return (
    <svg
      width={size}
      height={size}
      viewBox={`0 0 ${size} ${size}`}
      className="shrink-0"
      role="img"
      aria-label={`By provider · ${total} requests`}
      onPointerLeave={() => onHover(null)}
    >
      <circle
        cx={size / 2}
        cy={size / 2}
        r={r}
        fill="none"
        stroke="var(--surface2)"
        strokeWidth={thickness}
      />
      <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
        {arcs.map(({ i, dash, offset }) => (
          <circle
            key={i}
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke={slices[i].color}
            strokeWidth={thickness}
            strokeDasharray={dash}
            strokeDashoffset={offset}
            opacity={active === null || active === i ? 1 : 0.35}
          />
        ))}
      </g>
      {/* Hit rings after the painted ones: a wider invisible stroke, so the
          pointer only has to be near the ring, not on its 15px face. */}
      <g transform={`rotate(-90 ${size / 2} ${size / 2})`}>
        {arcs.map(({ i, dash, offset }) => (
          <circle
            key={`hit-${i}`}
            cx={size / 2}
            cy={size / 2}
            r={r}
            fill="none"
            stroke="none"
            strokeWidth={thickness + 12}
            strokeDasharray={dash}
            strokeDashoffset={offset}
            style={{ pointerEvents: "stroke", cursor: "pointer" }}
            onPointerEnter={() => onHover(i)}
          />
        ))}
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
        {shown ? shown.value : total}
      </text>
      <text x="50%" y="62%" textAnchor="middle" fill="var(--mut)" fontSize="9">
        {shown ? label : "requests"}
      </text>
    </svg>
  );
}

/** "By provider": the donut and its legend are one control.
 *
 * Pointing at a ring segment or at a legend row sets the same state, so the
 * pair lifts together and the donut's centre becomes that provider's readout.
 * The legend rows are the wide target and the keyboard's way in — the ring is
 * 15px thick, which is not something to ask anyone to hit. */
function ProviderBreakdown({
  rows,
  pref,
}: {
  rows: DashboardData["by_provider"];
  pref: string;
}) {
  const [active, setActive] = useState<number | null>(null);
  return (
    <div className="mt-2.5 flex items-center gap-4">
      <Donut
        slices={rows.map((p) => ({
          color: p.color,
          pct: p.pct,
          value: p.requests,
          label: p.name,
        }))}
        total={rows.reduce((sum, p) => sum + p.requests, 0)}
        active={active}
        onHover={setActive}
      />
      {/* Clearing on the list's leave rather than each row's keeps a move
          between rows from blanking the readout on the way. */}
      <div className="min-w-0 flex-1 space-y-1" onPointerLeave={() => setActive(null)}>
        {rows.map((p, i) => (
          <div
            key={p.name}
            tabIndex={0}
            className="-mx-1.5 flex items-center justify-between gap-2 rounded px-1.5 py-0.5 text-[11.5px]"
            style={{ background: active === i ? "var(--surface2)" : "transparent" }}
            onPointerEnter={() => setActive(i)}
            onFocus={() => setActive(i)}
            onBlur={() => setActive(null)}
          >
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
              <ProviderBreakdown rows={data.by_provider} pref={pref} />
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
        <RequestLogs
          agent={agentFilter === "all" ? undefined : agentFilter}
          providerId={providerFilter === "all" ? undefined : providerFilter}
        />
      </div>
    </section>
  );
}
