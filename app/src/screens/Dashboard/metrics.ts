// The Dashboard's one vocabulary for "which number is this chart showing".
//
// The Activity card picks its metric once, here, rather than inside the chart:
// the switch, the readout and the column colour have to agree about what the
// marks mean, and the surest way to keep them agreeing is to have one function
// pick the number and one function print it.
import type { KeyPath, Messages } from "../../i18n";
import { fmtMoney, fmtTokens } from "../../lib/format";
import type { TrendPoint } from "../../api/types";

/** The three series the Activity card can show: requests and tokens are what
    the trend has always carried; `cost` is the window's own money, already
    converted to the display currency by the daemon. */
export type TrendMetric = "requests" | "tokens" | "cost";

/** One bucket's figure for `metric`. */
export function metricValue(p: TrendPoint, metric: TrendMetric): number {
  if (metric === "requests") return p.requests;
  if (metric === "tokens") return p.tokens;
  return p.cost;
}

/** The metric's name on the switch above the chart — the capitalised form, as a
    control reads. */
export function metricTabKey(metric: TrendMetric): KeyPath<Messages> {
  return metric === "requests"
    ? "dashboard.metricRequests"
    : metric === "tokens"
      ? "dashboard.metricTokens"
      : "dashboard.metricCost";
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
