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
  refreshTitle:
    "Refresh providers, plan quotas and installed agents (bypasses the 5-min quota cache)",

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
    "Signed in with an official subscription (Claude login, Codex ChatGPT)? Official OAuth can't be proxied yet — add a provider manually first.",

  // ── Provider rows ──
  unbound: "Unbound",
  inUse: "In use",
  /** The quota strategy's first backup, once the primary is over its threshold.
      Not "In use": which backup actually serves is decided by the gateway's
      breakers at request time, so the honest claim is about the config. */
  quotaFallback: "Fallback",
  quotaFallbackTitle:
    "First in line once the primary passes its quota — which backup actually serves depends on the others still being reachable",
  editProvider: "Edit provider",
  deleteProvider: "Delete provider",
  clickAgain: "Click again to confirm",
  confirm: "Confirm",
  blocked: "Blocked",
  notRoutingTitle: "Not routing here: {reason}",

  // ── The Status column's measurements ──
  /** A latency the provider's own requests produced. */
  latencyTraffic: "Average of this provider's own requests over the last 24 hours",
  /** A latency the gateway's reachability check produced — a different claim,
      and the tooltip is where the difference is spelled out. */
  latencyProbe:
    "Reachability: the gateway asked the endpoint — an unsigned request, so this says something answers there, not that your key works · checked {time}",
  /** A latency the Apps screen's own test measured — a real prompt with the key. */
  latencyTest: "You tested it — a real prompt, sent with this provider's key · measured {time}",
  unreachable: "No answer",
  unreachableTitle: "The endpoint did not answer when it was last asked · checked {time}",
  /** Answered and refused the key: reachable, but not usable. */
  refused: "Key refused",
  refusedTitle: "The endpoint answered and refused this key: {error} · checked {time}",

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
  // ── Deleting a provider, and parking one instead ──
  /** The sentence a row shows while its delete is armed, in clauses joined by
      " · " — see `deleteConsequence`. */
  delRemovedFrom: "removed from {agents}",
  delPromotes: "{provider} becomes the primary for {agent}",
  delEmpties: "{agent} is left with no provider",
  disable: "Disable",
  enable: "Enable",
  disableTitle: "Park it: out of every route, keeping the row and its key",
  enableTitle: "Put it back into the routes it is bound to",

  // ── Latency test (a prompt round trip, one per provider row) ──
  testLatency: "Test latency",
  testLatencyTitle: "Send one prompt and time the round trip",
  testLatencyFailed: "failed",

  // ── User-defined agents (migration v16) ──
  // The + in the strip opens a menu rather than creating outright, so it needs
  // its own wording: the menu is "add an agent", and creating one is the first
  // thing it offers.
  /** The + at the end of the agent strip. */
  addAgentMenu: "Add an agent",
  addAgentMenuTitle: "Define your own, or point Kiwano at one it cannot find",
  /** First item in that menu. */
  newAgent: "Create a custom agent",
  // ── Pointing Kiwano at a built-in it could not find ──
  notDetected: "Not found on this machine",
  addAgentTitle: "Add {agent}",
  declaredAgentTitle: "{agent}'s install directory",
  installDir: "Install directory",
  installDirPlaceholder: "/opt/custom/bin",
  browse: "Browse…",
  /** Heading over the directories the detector tried. The count is there
      because the list scrolls: it is how the user knows there is more of it. */
  searchedDirs: "Looked in {count} directories, and not found",
  /** Asks the machine again — for the tool the user just installed. */
  checkAgain: "Check again",
  checkAgainTitle: "Look for this agent again on this machine",
  /** The probe found it after all: nothing to point at. */
  foundTitle: "Kiwano found it",
  foundBody:
    "The agent is installed here, so there is nothing to point at — its tab is already in the strip, on the Apps screen behind this dialog.",
  addAgentNote:
    "Point it at the directory holding this agent's command and Kiwano will confirm that command runs — not that it is this agent's, since version output has no shared shape to check.",
  declaredAgentNote:
    "You pointed Kiwano here, and this is where it found the agent. Saving checks the command still runs — not that it is this agent's, since version output has no shared shape to check.",
  addAgent: "Add agent",
  alreadyDeclared: "Added",
  editAgentName: "Edit agent name",
  agentName: "Name",
  agentNamePlaceholder: "Long tasks",
  agentNote: "Note",
  agentNotePlaceholder: "Optional — what this route is for",
  /** Under the form: what the user is and is not choosing. */
  agentIdNote:
    "The id is derived from the name and stays fixed. Nothing on disk changes: this agent is a route, so you point a client at the gateway with its key rather than taking over a config file.",
  createAgent: "Create",
  routeEmptyTitle: "{agent} has no candidates yet",
  routeEmptyBody:
    "Bind an existing provider below, and requests that arrive with this agent's key will route through the candidates in order.",
  /** The credentials dialog's title, and the name on the icon that opens it. */
  accessTab: "Access",
  accessEndpoint: "Endpoint",
  accessKey: "API Key",
  accessNote:
    "Any client pointed at this endpoint with this key routes by this agent's strategy, on /v1/messages or /v1/chat/completions.",
  deleteAgent: "Delete this agent",
  deleteAgentConfirm: "Click again to delete",
  deleteAgentNote: "Its usage history stays.",


  // ── A built-in agent's own settings (the row above its table, and its dialog) ──
  /** The gear's title, and the dialog's heading — the same words for both. */
  agentSettingsFor: "{agent} settings",
  agentGeneralTab: "General",
  agentConfigFilesNote: "Backed up before a takeover, restored when it is turned off.",

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

  // ── Plan quota tiers ──
  // The backend reports these as machine keys (`five_hour`, `weekly_limit`,
  // `monthly`); the mapping lives in `PLAN_TIER_LABEL_KEYS` in api/types.ts.
  tierFiveHour: "5h window",
  tierWeekly: "Weekly",
  tierMonthly: "Monthly",
};
