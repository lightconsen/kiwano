// Keys for strategy. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// The strategy *ids* (`single`, `failover`, `roundrobin`, `timewindow`,
// `quota`) are sent to the backend and are never translated — only the labels
// and hints below are. The candidate counts are explicit one/other keys
// because there is no plural engine; zh-CN renders both the same way.
export const strategy = {
  // ── Strategy labels ──
  single: "Single primary",
  failover: "Failover",
  roundrobin: "Weighted round-robin",
  timewindow: "Time window",
  quota: "Quota fallback",

  // ── One-line description shown beside the select ──
  singleHint: "Always use the primary provider",
  failoverHint: "Fall through standbys in order on failure, switch back on recovery",
  roundrobinHint:
    "New sessions rotate by weight; sticky per session to keep the upstream prompt cache",
  timewindowHint:
    "Pick by each candidate's local time window; fall back to the primary when no window matches",
  quotaHint: "Once today's primary reaches the number below, send to standbys",

  // ── Quota-fallback unit select ──
  unitRequestsDay: "requests/day",
  unitTokensDay: "tokens/day",

  // ── Select trigger ──
  ariaFor: "{agent} strategy",

  // ── Copy-another-agent's-route row ──
  replaceConfirmOne:
    "Replace {mine}'s route with {source}'s ({strategy}, {count} candidate)?",
  replaceConfirmOther:
    "Replace {mine}'s route with {source}'s ({strategy}, {count} candidates)?",
  replace: "Replace",
  copyRouteFrom: "Copy route from…",
  copyRouteAria: "Copy route from another agent onto {agent}",
  pickAgent: "Pick an agent",
  candidateOne: "{agent} · {count} candidate",
  candidateOther: "{agent} · {count} candidates",


  // ── The agent's own ceiling (its own row, under the strategy) ──
  limitAdd: "Add a window",
  limitRemoveAria: "Remove this window",
  limitLabel: "Limit",
  limitNone: "No limit",
  limitEditAria: "Edit {agent}'s limit",
  limitTitle: "{agent}'s limit",
  limitPlaceholder: "none",
  limitAmountAria: "Limit amount",
  limitClear: "Clear",
  limitUnitRequests: "requests",
  limitUnitWanTokens: "10k tokens",
  limitPeriodDay: "per day",
  limitPeriodWeek: "per week",
  limitPeriodMonth: "per month",
  limitPeriodYear: "per year",
  limitPeriodAll: "in total",
  limitBody:
    "Measured across every provider this agent uses, and enforced under every strategy — including single, whose one-provider rule says nothing about how much may be spent. An agent can hold several windows at once, and being over any one of them refuses requests until that window resets.",
  limitMoneyNote:
    "A money limit counts only what the price table can price. A request whose model has no published price is metered but not costed, so it does not count toward the ceiling.",
};
