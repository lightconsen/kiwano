// Keys for settings. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
export const settings = {
  general: "General",
  displayCurrency: "Display currency",
  displayCurrencyNote: "For cost cards and usage limits",
  theme: "Theme",
  themeLight: "Light",
  themeDark: "Dark",
  launchAtLogin: "Launch at login",
  minimizeToTray: "Minimize to tray on close",
  logs: "Logs",
  logsNote: "Errors and warnings, one file, seven days",
  openFolder: "Open folder",

  // Local gateway
  localGateway: "Local gateway",
  agentTakeover: "Agent takeover",
  /** Sits inline after the "Agent takeover" label, so it keeps the leading "·"
      separator the label relies on. The count is of the agents pointing at the
      gateway; the per-agent controls are on the Apps page. */
  agentTakeoverSummary: "· {routed} of {total} agents routed through the gateway",
  manageInApps: "Manage in Apps",
  autoFailover: "Auto failover",
  autoFailoverNote: "Switch to a standby when the primary fails",
  requestLogs: "Request logs",
  compatShim: "Compat shim",
  compatShimNote: "Fix requests an upstream would refuse (new thinking params, null tool schemas, foreign thinking history); every change is recorded in the request log",
  dlpScan: "Credential watch",
  dlpScanNote: "Note a credential an agent sends (API keys, private keys) in the request log. Reports, never blocks — and needs request logging on to have somewhere to report",
  dlpAlert: "Alert",
  requestLogsNote: "Record every request with bodies, local only",
  logRetention: "Log retention",
  logRetentionNote: "Rows older than this are pruned every 6h",
  logBodyCap: "Body size limit",
  logBodyCapNote: "Larger bodies are stored truncated. Also the memory one stream may hold.",
  noLimit: "No limit",
  mb: "{count} MB",
  streamFirstByte: "First-byte wait",
  streamFirstByteNote:
    "Give up on an upstream that returns headers and then sends nothing. A cold local model can take a while — raise it, or turn it off",
  streamIdle: "Silent stream",
  streamIdleNote:
    "Give up on a stream that goes quiet mid-answer, rather than holding the connection until the client times out",
  seconds: "{count}s",
  keepAll: "Keep all",
  off: "Off",
  /** Same shape for 7/30/90 — the option is a number of days. */
  days: "{count} days",
  costAlert: "Cost alert",
  costAlertNote: "System notification when a period limit is reached",

  // Features panel — the request-logs applications, each an opt-in.
  features: "Features",
  featuresNote: "Optional capabilities built on the request log. All off by default.",
  featGroupAlerts: "Alerts",
  featCostForecast: "Cost forecast",
  featCostForecastNote: "warns before the month's slope passes a provider's limit",
  featAnomalyAlerts: "Anomaly detection",
  featAnomalyAlertsNote: "error spikes, latency outliers, traffic bursts vs your 7-day baseline",
  featAgentLimitAlerts: "Agent budget alerts",
  featAgentLimitAlertsNote: "notify when an agent hits its own per-period budget",
  featGroupAgent: "Agent feedback",
  featMcpSelfQuery: "Agent self-query (MCP)",
  featMcpSelfQueryNote: "agents can read their own aggregate stats via `kiwano mcp`; aggregates only",
  featRuleInjection: "Rule injection",
  featRuleInjectionNote: "`kiwano rules apply` writes insights-derived rules to CLAUDE.md/AGENTS.md — changes agent behavior",
  featGroupExperiments: "Experiments",
  featTuningAdvice: "Tuning advice",
  featTuningAdviceNote: "insights suggests retry-budget and route-health changes; advice only",
  featCacheExperiment: "Cache-shaping experiment",
  featCacheExperimentNote: "`kiwano cache-experiment` measures whether body normalization would help; read-only",

  // Privacy pledge
  privacyTitle: "Privacy (Kiwano pledge)",
  privacyUsage:
    "Kiwano sends nothing about your usage. The only outbound requests are the Hub catalog and pricing fetch, and the update check.",
  privacyNoKeys: "Neither carries your API keys or any request content",
  privacyKeysLocal: "Keys go only to the providers you configure, never to Kiwano",

  // Kiwano Hub
  hub: "Kiwano Hub",
  catalog: "Catalog",
  catalogNote: "skips the download when unchanged",
  syncing: "Syncing…",
  syncNow: "Sync now",
  /** The sync result line, joined with " · " when pricing also moved. */
  syncUnchanged: "Catalog up to date · {count} providers",
  syncSynced: "Synced {count} providers",
  syncPricing: "pricing v{version}",

  // About / update
  about: "About",
  version: "Version",
  autoCheckUpdates: "Auto-check for updates",
  autoCheckUpdatesNote: "Silent check at startup",
  updates: "Updates",
  downloading: "Downloading…",
  /** The About row's inline version, "v0.1.7 available". */
  versionAvailable: "v{version} available",
  /** The banner's standalone headline, which names the app. */
  updateAvailable: "Kiwano {version} is available",
  retryDownload: "Retry download",
  downloadInstall: "Download & install",
  upToDate: "Up to date",
  checking: "Checking…",
  checkForUpdates: "Check for updates",
  dismissUpdate: "Dismiss update notice",
  hideUntil: "Hide until a version newer than {version}",
};
