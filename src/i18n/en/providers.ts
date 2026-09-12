// Keys for providers. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// Several of these messages are assembled at the call site from optional
// fragments (the usage tooltip joins a 5h and a weekly bound with " · ", the
// plan cell joins per-window tier lines). The fragments are whole phrases in
// both languages and their order is fixed, so they are translated
// individually and joined; messages that read as one sentence carry named
// placeholders instead of being concatenated.
export const providers = {
  // ── Segment bar and list header ──
  /** Label of the "all agents" segment (agent names themselves are brand names). */
  all: "All",
  counts: "{providers} providers · {agents} agents bound",
  addProvider: "Add provider",
  refreshTitle: "Refresh providers and plan quotas (bypasses the 5-min quota cache)",

  // ── Per-agent onboarding (empty tab) ──
  takenOverTitle: "{agent} is taken over",
  takenOverBody:
    "The local gateway routes this agent's requests, but no provider is bound yet — add one so requests have somewhere to go.",
  startManaging: "Start managing {agent} with Kiwano",
  startManagingBody:
    "Kiwano backs up the current config (one-click restore later), imports the provider {agent} already uses (shared with other agents), and routes it through the local gateway — same upstream, instant switching afterwards.",
  enableKiwano: "Enable Kiwano",
  enabling: "Enabling…",
  addProviderFirst: "Add provider first",
  oauthNote:
    "Signed in with an official subscription (Claude / Gemini login, Codex ChatGPT)? Official OAuth can't be proxied yet — add a provider manually first.",

  // ── Provider rows ──
  unbound: "Unbound",
  inUse: "In use",
  editProvider: "Edit provider",
  deleteProvider: "Delete provider",
  clickAgain: "Click again to confirm",
  confirm: "Confirm",
  blocked: "Blocked",
  healthy: "Healthy {latency}ms",
  notRoutingTitle: "Not routing here: {reason}",

  // ── Usage / quota cell ──
  /** Plan row with configured percent limits: the {windows} fragment is the
      joined 5h/weekly bounds, {tokens} the optional in/out suffix. */
  usagePlanLimits: "Plan · limit {windows} · enforced against the provider's plan-quota utilization{tokens}",
  usageWindowFive: "5h window ≤ {percent}%",
  usageWindowWeekly: "weekly window ≤ {percent}%",
  /** Appended to `usagePlanLimits`; keeps the leading separator so it can be
      substituted as the empty string. */
  usageInOut: " · in {input} · out {output}",
  usagePlanQuota:
    "Plan · {used}/{limit} {unit} this period ({percent}%) · resets {resets} · in {input} · out {output}",
  usageLocalInference: "Local inference · no cost metering · works offline",
  usagePaygQuota:
    "Pay as you go · {used} / {limit} this period ({percent}%) · in {input} (cache {cache}, billed at 1/10) · out {output} · latency {latency}",
  usagePaygTrend: "Pay as you go · no limit set · 7-day usage trend",
  /** Tooltip for the live plan-quota line; {template} is a backend identifier. */
  planTemplateCached: "{template} · cached",
  planLimitFive: "5h ≤ {percent}%",
  planLimitWeekly: "wk ≤ {percent}%",
  unitReq: "req",
  unitTenKTok: "10k tok",
  resetsAt: "resets {date}",
  /** Local-inference row: the request count renders separately in mono, so
      only the trailing unit fragment is translated. */
  reqTokSuffix: "req · {tokens} tok",
  inOutTokens: "in {input} · out {output}",
  limitSuffix: "/ {limit} limit",
  tokensLatency: "{tokens} tokens · latency {latency}",
  reqSuffix: "· {requests} req",

  // ── Column headers ──
  colProvider: "Provider",
  colRole: "Role in strategy",
  colBoundAgents: "Bound agents",
  colUsage: "Usage / quota",
  colStatus: "Status",
  colPriority: "Priority",
  colActions: "Actions",

  // ── Agent-tab role cell ──
  weight: "weight",
  weightTitle: "Sessions rotate across candidates proportionally to their weights",
  windowStart: "Window start",
  windowEnd: "Window end",
  windowTitle:
    "Local time window this candidate serves; type digits like 0930 — the end must be later than the start. Clear both to remove the window",
  fallback: "Fallback",
  fallbackTitle: "Serves whenever no candidate's window matches",
  noWindow: "No window",
  noWindowTitle: "No window set — this candidate is never picked",
  primary: "Primary",
  primaryTitle: "Current route of this agent",
  standby: "Standby #{index}",

  // ── Candidate row actions ──
  moveUp: "Move up",
  moveDown: "Move down",
  makePrimary: "Make primary",
  makePrimaryTitle: "Make primary — move this candidate to the head of the queue",
  removeFromRouteAria: "Remove from route",
  removeFromRoute: "Remove from this route (other agents keep their binding)",

  // ── Bind-another-provider row ──
  bindAria: "Bind provider to route",
  allBound: "All providers are already bound",
  bindExisting: "Bind existing provider…",
  allInRoute: "All providers are already in this route — add a new one from the header",
  bindAnother: "Bind another provider to this route — it joins the queue tail as a standby",

  // ── Empty list ──
  none: "No providers yet —",

  // ── Closing note (one variant per tab state) ──
  footerNotTakenOver:
    "Kiwano does not route this agent yet — enable the takeover above; the stored route is kept and applies again as-is · API keys stay in the system keychain · requests never touch the Kiwano cloud",
  footerStrategy:
    "Agent routing strategy · rows above are the candidates in priority order (primary first) · switching applies instantly · API keys stay in the system keychain · requests never touch the Kiwano cloud",
  footerDefault:
    "Switching applies instantly (the agent is taken over by the local gateway; switching only changes routing) · API keys stay in the system keychain · requests never touch the Kiwano cloud",

  // ── Plan quota tiers ──
  // The backend reports these as machine keys (`five_hour`, `weekly_limit`,
  // `monthly`); the mapping lives in `PLAN_TIER_LABEL_KEYS` in api/types.ts.
  tierFiveHour: "5h window",
  tierWeekly: "Weekly",
  tierMonthly: "Monthly",
};
