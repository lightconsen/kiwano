// Keys for app. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
export const app = {
  /** The top nav. Keys rather than labels: the array is module-level and
      `t()` is a hook, so the label is resolved at render. */
  nav: {
    apps: "Apps",
    models: "Models",
    dashboard: "Dashboard",
    settings: "Settings",
  },

  // Header
  gateway: "Gateway",

  // Footer status bar
  today: "Today",
  requests: "requests",
  tokens: "tokens",
  hubSynced: "Hub just synced",
  /** The status bar's ⟳. Its scope is the screen in front of you — nothing
      outside it (see lib/reload.ts), so the label names the screen rather than
      the app. A screen's own ⟳ keeps `common.refresh`: that one is the screen's
      control, this one is the shell's. */
  refreshScreen: "Refresh this screen",
  refreshScreenTitle:
    "Re-read what this screen is showing — filters, window and scroll position stay put",

  // Cost-alert notification
  /** The three ways a used amount is phrased before it is dropped into a
      notification body. */
  usedPlanWindow: "{pct}% of its plan window",
  usedTokens: "{count}k tokens",
  usedRequests: "{count} requests",
  notifyPlanTitle: "Kiwano plan limit",
  notifyCostTitle: "Kiwano cost alert",
  /** Feature alerts (forecast / anomaly / agent budget) carry their own body. */
  notifyFeatureTitle: "Kiwano alert",
  notifyPlanBody:
    "{provider} is at {used} and has reached its {limit}% limit — it is disabled until usage drops back under",
  notifyCostBody:
    "{provider} used {used} this period and has hit its limit of {limit} — watch your spending",

  // Billing chips (components/bits.tsx)
  billing: {
    payg: "PAYG",
    plan: "Plan",
    unl: "Unl",
  },

  // Credential-watch banner (under the nav; the finding's own note line
  // carries the rule names, e.g. "dlp: github-token ×1")
  credentialBannerTitle: "An API key or private key left the machine in a recent request.",
  credentialBannerCta: "View the request",
  credentialBannerDismiss: "Dismiss",
};
