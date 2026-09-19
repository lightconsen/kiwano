// Keys for dashboard. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// The `WINDOWS` ids ("today"/"7d"/"30d") are what the backend filters on and
// are not here — only their labels are.
export const dashboard = {
  windowToday: "Today",
  window7d: "Last 7 days",
  window30d: "Last 30 days",
  /** Not a counted window: everything the store holds. Reads as the same word
      the two filters beside it use, which is the point — it is the same kind of
      "no narrowing" as they are, and the strip is read left to right as one
      row of choices. */
  windowAll: "All",

  allProviders: "All providers",
  allAgents: "All agents",
  /** Accessible names for the two filter comboboxes: the visible label is the
      current selection, so without these a screen reader announces an unnamed
      listbox and a test has no way to tell the two apart. */
  filterByProvider: "Filter by provider",
  filterByAgent: "Filter by agent",
  localSqlite: "Data stays in local SQLite",

  statRequests: "Requests",
  statTokens: "Tokens",
  statEstCost: "Est. cost",
  statAvgLatency: "Avg latency",
  atHubPrice: "at Hub price",
  /** The peak premium under the cost stat, shown only when there is one. Not a
      "saving": it prices the window's own tokens at the same rows' off-peak
      rate, which is the discount a caller who ran at peak did not get. */
  offPeakPremium: "peak premium",
  offPeakPremiumTitle:
    "What this window's tokens would have cost at their rows' off-peak rates. Zero when nothing you used publishes a time-of-day price, and zero when you were already off-peak.",
  tokensInOut: "in {input} / out {output}",
  /** Hover text on the tokens total; the cache-hit tier bills at 1/10. */
  tokensTitle: "in {input} (cache hits {cache}, billed at 1/10) · out {output}",

  usageTrend: "Usage trend",
  metricRequests: "Requests",
  metricTokens: "Tokens",
  /** The chart's readout and aria-label interpolate the metric's own name,
      which the raw id printed lowercase; the tab above uses the capital form. */
  trendMetricRequests: "requests",
  trendMetricTokens: "tokens",
  trendAria: "Usage trend · {metric} · {n} buckets",

  byProvider: "By provider",
  donutAria: "By provider · {n} requests",
  donutCenterLabel: "requests",
  notRoutingHere: "Not routing here: {reason}",
  blocked: "Blocked",
  noTraffic: "No traffic in this window.",

  byAgent: "By agent",
  colAgent: "Agent",
  colRequests: "Requests",
  colTokens: "Tokens",
  colCost: "Cost",
};
