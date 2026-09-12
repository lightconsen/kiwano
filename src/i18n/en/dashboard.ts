// Keys for dashboard. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// The `WINDOWS` ids ("today"/"7d"/"30d") are what the backend filters on and
// are not here — only their labels are.
export const dashboard = {
  windowToday: "Today",
  window7d: "Last 7 days",
  window30d: "Last 30 days",

  allProviders: "All providers",
  allAgents: "All agents",
  localSqlite: "Data stays in local SQLite",

  statRequests: "Requests",
  statTokens: "Tokens",
  statEstCost: "Est. cost",
  statAvgLatency: "Avg latency",
  atHubPrice: "at Hub price",
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
