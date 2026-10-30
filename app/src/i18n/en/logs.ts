// Keys for logs. English is the source of truth for the key set;
// every other locale's file must match this shape exactly (see ../types.ts).
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


  logBreadcrumb: "Request log",
  logGone: "This request log is no longer available",
  logTitle: "Log #{id}",

  detailSession: "Session",
  detailUpstreamKey: "Upstream key",
  detailNotes: "Gateway notes:",
  /** Row badge + detail block for a credential-watch finding (dlp:* notes). */
  dlpBadge: "key",
  detailCredentialWatch: "Credential watch: an API key or private key left the machine in this request.",
  detailRequest: "Request",
  detailResponse: "Response",
  detailBodyTruncated: "(body truncated)",
  /** The render cap, not the capture cap: the store still holds the whole
      body and Copy still copies it; this line says what the screen shows. */
  detailBodyCapped: "showing the first 256 KB",
  detailBodyExpand: "Show all",
  detailRequestHeaders: "REQUEST HEADERS",
  detailResponseHeaders: "RESPONSE HEADERS",
  detailRequestBody: "REQUEST BODY",
  detailResponseBody: "RESPONSE BODY",
  detailClientStream: " (client-visible stream)",

  exportDialogTitle: "Export logs",
  dateRange: "DATE RANGE",
  exportNote: "Every request in the range is written, not just the {n} on screen.",
  exporting: "Exporting…",
  exported: "Exported {n} rows",
  exportCapped: "Capped at {n} rows — narrow the range for the rest.",

  // ── The sessions view (the card's second tab) ──
  tabSessions: "Sessions",
  sessionsCount: "{n} sessions",
  sessionsEmptyUnmanaged: "No sessions for the agents you have taken over — take one over and its history appears here",
  unpricedRows: "{n} unpriced",
  costPlusUnpriced: "{amounts} (+{n} unpriced)",
  sessionsWindowTitle: "The window the Dashboard's selector is set to",
  sessionsProviderNotApplied: "The provider filter does not apply to sessions",
  sessionsEmpty:
    "No sessions in this window — the gateway's own traffic appears as it happens, and the agents' earlier history comes from `kiwano history import`",
  colRequests: "REQ",
  colTitle: "TITLE",
  colLast: "LAST",
  colLastTitle: "when the session was last active",
  colProject: "PROJECT",
  colMachine: "machine",
  colSource: "SOURCE",
  colTurns: "TURNS",
  colCost: "COST",
  colTokensTitle: "in / out — the cache read and write buckets are on each row",
  tokensCellTitle: "in {input} · out {output} · cache read {read} · cache write {write}",
  sourceGateway: "gateway",
  sourceImported: "imported",
  sourceBoth: "both",
  sourceTitle:
    "Which ledger this session was found in. The two are never added: traffic supplies the requests, the tokens and the cost, the agent's files supply the project, the span, the turns and the tools — and a dash marks the side that never saw it.",
  sessionFilter: "Session",
  clearSessionFilter: "Clear the session filter",
  sessionNoTraffic:
    "No gateway traffic for this session — it came from the agent's own files",
  sessionRowTitle: "Show this session's requests · {id}",
};
