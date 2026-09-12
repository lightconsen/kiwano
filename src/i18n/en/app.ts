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

  // Cost-alert notification
  /** The three ways a used amount is phrased before it is dropped into a
      notification body. */
  usedPlanWindow: "{pct}% of its plan window",
  usedTokens: "{count}k tokens",
  usedRequests: "{count} requests",
  notifyPlanTitle: "Kiwano plan limit",
  notifyCostTitle: "Kiwano cost alert",
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
};
