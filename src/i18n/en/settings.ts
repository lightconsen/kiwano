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
  /** Sits inline after the "Agent takeover" heading, so it keeps the leading
      "·" separator the heading relies on. */
  agentTakeoverNote: "· hot-switching once pointed at the local gateway",
  coexist: "Coexist · multi-provider",
  placeholderKeyTitle:
    "Placeholder key assigned by the gateway, used for request attribution",
  takenOver: "Taken over",
  notTakenOver: "Not taken over",
  enableInApps: "Enable in Apps",
  autoFailover: "Auto failover",
  autoFailoverNote: "Switch to a standby when the primary fails",
  requestLogs: "Request logs",
  requestLogsNote: "Record every request with bodies, local only",
  logRetention: "Log retention",
  logRetentionNote: "Rows older than this are pruned every 6h",
  /** Same shape for 7/30/90 — the option is a number of days. */
  days: "{count} days",
  costAlert: "Cost alert",
  costAlertNote: "System notification when a period limit is reached",

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
