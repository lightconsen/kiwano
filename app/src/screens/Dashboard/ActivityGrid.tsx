// The Activity card's grid layout: one cell per trend bucket, colour depth by
// volume — the shape a reader already knows from a contributions graph. The
// trend bars answer "how much, over time"; this answers "which buckets was this
// machine actually working in", where a bar of zero is easy to miss and a gray
// cell is not.
//
// It reads the same `DashboardData.trend` the bars do, so switching layouts
// cannot change the numbers — only how they are drawn.
import { useMemo, useRef, useState } from "react";
import type { KeyPath, Messages, Translate } from "../../i18n";
import type { TrendPoint } from "../../api/types";
import { formatMetric, metricValue, metricWord, type TrendMetric } from "./metrics";

/** One bucket's cell, as the ramp sees it.
 *
 * `future` is its own state rather than `none` because the "today" window plots
 * all 24 hours of the day: at 10am, the cell for 23:00 is not an hour in which
 * nothing happened — it is an hour that has not happened. Drawing those as empty
 * would be the grid asserting something it cannot know. */
export type CellState = "none" | "low" | "medium" | "high" | "future";

/** How deep each intensity is drawn, as a share of the accent. `none` is the
    card's own recessed surface rather than a translucent accent, so "nothing
    here" reads as a hole rather than as the faintest possible something. */
const RAMP_FILL: Record<"low" | "medium" | "high", string> = {
  low: "color-mix(in srgb, var(--kiwi) 28%, transparent)",
  medium: "color-mix(in srgb, var(--kiwi) 58%, transparent)",
  high: "var(--kiwi)",
};

/** Cells per row. Seven — a week's worth, and about as many as reads at a glance
    before the eye starts losing its place between rows. */
export const CELLS_PER_ROW = 7;

const LEGEND: { state: CellState; key: KeyPath<Messages> }[] = [
  { state: "none", key: "dashboard.activityLegendNone" },
  { state: "low", key: "dashboard.activityLegendLow" },
  { state: "medium", key: "dashboard.activityLegendMedium" },
  { state: "high", key: "dashboard.activityLegendHigh" },
  { state: "future", key: "dashboard.activityLegendFuture" },
];

/** The depth of `value` against `cuts` — the terciles of this window's own
    non-zero buckets, not fixed numbers. A reader whose busiest day is 40
    requests gets the same contrast as one whose busiest is 40,000; fixed
    thresholds would show the first a uniformly pale month. */
export function cellState(value: number, cuts: [number, number], future: boolean): CellState {
  if (future) return "future";
  if (value <= 0) return "none";
  if (value <= cuts[0]) return "low";
  if (value <= cuts[1]) return "medium";
  return "high";
}

/** The two cuts between the ramp's three intensity steps: the 33rd and 66th
    percentiles of the values above zero. Empty buckets are left out of the
    distribution — a month with two busy days and twenty-eight empty ones should
    still show those two as deep. */
export function rampCuts(values: number[]): [number, number] {
  const nonZero = values.filter((v) => v > 0).sort((a, b) => a - b);
  if (nonZero.length === 0) return [0, 0];
  const at = (p: number) => nonZero[Math.min(nonZero.length - 1, Math.floor(nonZero.length * p))];
  return [at(0.33), at(0.66)];
}

/** How big a cell is drawn, from how many there are. A month should read as a
    month rather than as a speck, and a two-year history should still fit in the
    card — so the size falls as the count climbs. */
export function cellSize(count: number): number {
  return count <= 21 ? 30 : count <= 70 ? 20 : 15;
}

/** Whether an hourly bucket is still in the future, on the reader's clock. The
    buckets are local (`usage_hourly` applies the configured offset), so they are
    read back as local times — comparing them against UTC would call the evening
    "not yet" all morning for a reader east of Greenwich. */
export function isFutureHour(p: TrendPoint, now: Date): boolean {
  const hour = Number(p.date.slice(0, 2));
  if (!Number.isFinite(hour)) return false;
  const at = new Date(`${p.day}T00:00:00`);
  at.setHours(hour);
  return at.getTime() > now.getTime();
}

export function ActivityGrid({
  points,
  metric,
  currency,
  unit,
  t,
}: {
  points: TrendPoint[];
  metric: TrendMetric;
  currency: string;
  /** What one cell covers — a property of the window, printed in the caption.
      Buckets come as hours, days or weeks depending on the window, and the grid
      draws them all the same way: in the order they happened. */
  unit: "hour" | "day" | "week";
  t: Translate;
}) {
  const host = useRef<HTMLDivElement>(null);
  const cells = useRef<(HTMLDivElement | null)[]>([]);
  const [hover, setHover] = useState<number | null>(null);
  const [tip, setTip] = useState({ x: 0, y: 0 });

  const cols = Math.min(CELLS_PER_ROW, points.length) || 1;
  const cell = cellSize(points.length);

  const now = useMemo(() => new Date(), []);
  const cuts = useMemo(
    () => rampCuts(points.map((p) => metricValue(p, metric))),
    [points, metric],
  );
  const states = useMemo(
    () =>
      points.map((p) =>
        cellState(metricValue(p, metric), cuts, unit === "hour" && isFutureHour(p, now)),
      ),
    [points, metric, cuts, unit, now],
  );
  const counts = useMemo(() => {
    const n: Record<CellState, number> = { none: 0, low: 0, medium: 0, high: 0, future: 0 };
    for (const s of states) n[s] += 1;
    return n;
  }, [states]);

  const pointAt = (i: number) => {
    const el = cells.current[i];
    if (!el || !host.current) return;
    const box = el.getBoundingClientRect();
    const outer = host.current.getBoundingClientRect();
    setTip({ x: box.left - outer.left + box.width / 2, y: box.top - outer.top - 6 });
    setHover(i);
  };

  // Arrows walk the grid as it is drawn: a step along the row, a row at a time
  // down. The cells are in time order, so this is also the order they happened
  // in — there is no second axis to explain.
  const step = (from: number, key: string): number | null => {
    const by = (d: number) => {
      const next = from + d;
      return next >= 0 && next < points.length ? next : null;
    };
    switch (key) {
      case "ArrowRight":
        return by(1);
      case "ArrowLeft":
        return by(-1);
      case "ArrowDown":
        return by(cols);
      case "ArrowUp":
        return by(-cols);
      default:
        return null;
    }
  };

  const unitWord = t(
    unit === "hour"
      ? "dashboard.activityCellHour"
      : unit === "week"
        ? "dashboard.activityCellWeek"
        : "dashboard.activityCellDay",
  );
  const word = metricWord(metric, t);
  const shown = hover === null ? null : points[hover];

  return (
    <div ref={host} className="relative">
      <div
        tabIndex={0}
        role="img"
        aria-label={t("dashboard.activityGridAria", { metric: word, n: points.length })}
        className="mx-auto w-max max-w-full overflow-x-auto outline-none"
        onKeyDown={(e) => {
          const next = step(hover ?? 0, e.key);
          if (next === null) return;
          e.preventDefault();
          pointAt(next);
        }}
        onFocus={() => {
          if (hover === null && points.length > 0) pointAt(0);
        }}
        onBlur={() => setHover(null)}
        onPointerLeave={() => setHover(null)}
      >
        <div
          className="grid"
          style={{
            gridTemplateColumns: `repeat(${cols}, ${cell}px)`,
            gap: 2,
            justifyContent: "center",
          }}
        >
          {points.map((p, i) => {
            const state = states[i];
            return (
              <div
                key={`${p.day}-${p.date}`}
                ref={(el) => {
                  cells.current[i] = el;
                }}
                data-state={state}
                title={`${p.day} ${p.date} · ${formatMetric(metricValue(p, metric), metric, currency)} ${word}`}
                onPointerEnter={() => pointAt(i)}
                style={{
                  width: cell,
                  height: cell,
                  borderRadius: 3,
                  background:
                    state === "low" || state === "medium" || state === "high"
                      ? RAMP_FILL[state]
                      : state === "none"
                        ? "var(--surface2)"
                        : "transparent",
                  border: state === "future" ? "1px dashed var(--line)" : undefined,
                  outline: hover === i ? "2px solid var(--ink)" : undefined,
                  outlineOffset: 1,
                }}
              />
            );
          })}
        </div>
      </div>

      {/* The two ends, not every cell: the tooltip names a bucket exactly, and a
          label under each of thirty cells would be a smear. */}
      {points.length > 1 && (
        <div
          className="mx-auto mt-1 flex justify-between text-[9.5px]"
          style={{ color: "var(--mut)", width: cell * cols + 2 * (cols - 1) }}
        >
          <span>{points[0].date}</span>
          <span>{points[points.length - 1].date}</span>
        </div>
      )}

      <p className="mt-1.5 text-center text-[10.5px]" style={{ color: "var(--mut)" }}>
        {t("dashboard.activityCaption", { unit: unitWord, metric: word })}
      </p>

      {/* The shades, named — and counted. A reader who cannot tell two steps
          apart still gets the reading they encode, and the counts are what say
          how much of the window was empty rather than leaving it to the eye. */}
      <div
        className="mt-1.5 flex flex-wrap items-center justify-center gap-x-3 gap-y-1 text-[9.5px]"
        style={{ color: "var(--mut)" }}
      >
        {LEGEND.map(({ state, key }) => (
          <span key={state} className="flex items-center gap-1">
            <span
              style={{
                width: 10,
                height: 10,
                borderRadius: 2,
                background:
                  state === "low" || state === "medium" || state === "high"
                    ? RAMP_FILL[state]
                    : state === "none"
                      ? "var(--surface2)"
                      : "transparent",
                border: state === "future" ? "1px dashed var(--line)" : undefined,
              }}
            />
            <span>
              {t(key)}
              {counts[state] > 0 ? ` ${counts[state]}` : ""}
            </span>
          </span>
        ))}
      </div>

      {shown && (
        <div
          role="status"
          className="pointer-events-none absolute z-10 -translate-x-1/2 -translate-y-full rounded-md border px-2 py-[3px] text-[10.5px] whitespace-nowrap"
          style={{ left: tip.x, top: tip.y, background: "var(--panel)", borderColor: "var(--line)" }}
        >
          <span className="font-semibold">
            {formatMetric(metricValue(shown, metric), metric, currency)}
          </span>
          <span style={{ color: "var(--mut)" }}>
            {" "}
            {word} · {shown.day} {shown.date}
          </span>
        </div>
      )}
    </div>
  );
}
