// The Dashboard's one vocabulary for "which number is this chart showing".
//
// Both of the Activity card's layouts read the same trend buckets, so the
// metric is decided once here rather than in each: a bar and a cell for the same
// bucket must be able to disagree about nothing, and the surest way to keep that
// true is to have one function pick the number and one function print it.
import type { Translate, KeyPath, Messages } from "../../i18n";
import { fmtMoney, fmtTokens } from "../../lib/format";
import type { TrendPoint } from "../../api/types";

/** The three series the Activity card can show. `requests` and `tokens` are what
    the trend carried before the grid existed; `cost` is the window's own money,
    already converted to the display currency by the daemon. */
export type TrendMetric = "requests" | "tokens" | "cost";

/** Which layout the Activity card is drawn in. Same buckets either way. */
export type ChartLayout = "bars" | "grid";

/** One bucket's figure for `metric`. */
export function metricValue(p: TrendPoint, metric: TrendMetric): number {
  if (metric === "requests") return p.requests;
  if (metric === "tokens") return p.tokens;
  return p.cost;
}

/** The metric's name on the switch above the chart — the capitalised form, as a
    control reads. The readout and the aria-labels use `metricWord` instead. */
export function metricTabKey(metric: TrendMetric): KeyPath<Messages> {
  return metric === "requests"
    ? "dashboard.metricRequests"
    : metric === "tokens"
      ? "dashboard.metricTokens"
      : "dashboard.metricCost";
}

/** The metric's name inside a sentence ("40 requests", "· tokens") — the same
    word the tab uses, lower case, which is why it is a second key rather than a
    transformation of the first: not every language lower-cases by prefix. */
export function metricWord(metric: TrendMetric, t: Translate): string {
  return t(
    metric === "requests"
      ? "dashboard.trendMetricRequests"
      : metric === "tokens"
        ? "dashboard.trendMetricTokens"
        : "dashboard.trendMetricCost",
  );
}

/** One bucket's figure, printed the way every other screen prints it: money in
    the display currency, tokens abbreviated, requests as a plain count with
    separators. The chart's axis abbreviates further on its own (`k`/`M`) — a
    readout is a sentence, an axis is a ruler. */
export function formatMetric(value: number, metric: TrendMetric, currency: string): string {
  if (metric === "cost") return fmtMoney(value, currency);
  if (metric === "tokens") return fmtTokens(value);
  return value.toLocaleString();
}

/** What one bucket covers, which is a property of the window rather than of the
    reader's choice — and it is not a constant, so the caption has to say it.
    `usage_daily`/`usage_hourly` bucket by the local day or hour; past
    `TREND_DAILY_LIMIT_DAYS` the daemon sums a long history a week at a time.
    Detected from the buckets themselves (a week apart) rather than assumed from
    the window's name, so the label cannot drift from the data it describes. */
export function bucketUnit(points: TrendPoint[], window: string): "hour" | "day" | "week" {
  if (window === "today") return "hour";
  const last = points[points.length - 1];
  const prev = points[points.length - 2];
  if (!last || !prev) return "day";
  return dayGap(prev.day, last.day) >= 7 ? "week" : "day";
}

/** Whole days between two `YYYY-MM-DD` keys. Parsed as UTC noon so the answer
    cannot move with the reader's own zone — these are calendar labels, not
    instants. */
function dayGap(from: string, to: string): number {
  const ms = Date.parse(`${to}T12:00:00Z`) - Date.parse(`${from}T12:00:00Z`);
  return Math.round(ms / 86_400_000);
}
