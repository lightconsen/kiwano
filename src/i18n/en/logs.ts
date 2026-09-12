// Keys for logs. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// Deliberately absent: the clipboard payload `fmtLogText` builds and the
// values `fmtTime`/`fmtBytes`/`fmtThroughput` produce. The payload is a
// diagnostic dump whose field names ("agent:", "-- request headers --") are
// identifiers a maintainer greps for, so it stays in one language.
export const logs = {
  title: "Logs",

  filterAll: "All",
  filterOk: "OK",
  filterError: "Errors",

  rangeAny: "Any time",
  rangeToday: "Today",
  range7d: "Last 7 days",
  range30d: "Last 30 days",
  rangeCustom: "Custom…",
  /** The select prints this when its value matches no range (see RANGES). */
  rangeFallback: "All time",

  /** "12 requests" beside the export button; `n` is pre-formatted. */
  requestCount: "{n} requests",
  exportCsv: "Export CSV",
  exportTitle: "Choose a date range and write it to a CSV file",

  colTime: "Time",
  colAgent: "Agent",
  colProvider: "Provider",
  colModel: "Model",
  colStatus: "Status",
  colLatency: "Latency",
  firstToken: "First token",
  colThroughput: "Tok/s",
  colTokens: "Tokens",

  empty: "No requests recorded yet — traffic forwarded through the gateway shows up here",

  prev: "Prev",
  next: "Next",
  pageIndicator: "{page} / {pages}",

  logTitle: "Log #{id}",

  detailSession: "Session",
  detailRequest: "Request",
  detailResponse: "Response",
  detailBodyTruncated: "(body truncated)",
  detailRequestHeaders: "REQUEST HEADERS",
  detailResponseHeaders: "RESPONSE HEADERS",
  detailRequestBody: "REQUEST BODY",
  detailResponseBody: "RESPONSE BODY",
  detailClientStream: " (client-visible stream)",

  exportDialogTitle: "Export logs",
  dateRange: "DATE RANGE",
  includeBodies: "Include request and response bodies",
  twoExtraColumns: "Two extra columns",
  exportNote: "Every request in the range is written, not just the {n} on screen.",
  exporting: "Exporting…",
  exported: "Exported {n} rows",
  exportCapped: "Capped at {n} rows — narrow the range for the rest.",
};
