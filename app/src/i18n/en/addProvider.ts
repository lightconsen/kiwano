// Keys for addProvider. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// Protocol and billing names (OpenAI / Anthropic / Gemini API, Plan / Pay as
// you go) are terms of art and stay close to their English form; the helper
// text under each field is the bulk of this namespace.
export const addProvider = {
  // ── Dialog chrome ──
  titleAdd: "Add provider",
  titleEdit: "Edit provider",

  // ── Mode switch ──
  fromModels: "From Models",
  /** Mode label carrying the chosen catalog entry's name. */
  fromModelsNamed: "From Models: {name}",
  custom: "Custom",

  // ── Catalog picker ──
  searchCatalog: "Search the catalog…",
  noMatching: "No matching providers",
  change: "Change",

  // ── Form fields ──
  name: "Name",
  protocol: "Protocol",
  protoOpenai: "OpenAI-compatible",
  protoAnthropic: "Anthropic",
  protoGemini: "Gemini API",

  apiKey: "API Key",
  keyKeep: "Leave blank to keep the current key",
  keyLocal: "Stored locally, readable only by you",
  showHideKey: "Show / hide key",

  endpointUrl: "Endpoint URL",
  endpointHintShelf: "from the catalog · one per protocol · shares the API key",
  endpointHint: "one per protocol · shares the API key",
  test: "Test",

  // ── Inline probe verdicts (full detail lives in the tooltip) ──
  probeOk: "OK",
  probeAuth: "Auth",
  probeUnsupported: "404",
  probeError: "Error",
  probeUnreachable: "Down",

  // ── Default model ──
  defaultModel: "Default model",
  fetch: "Fetch",
  fetching: "Fetching…",
  errorNoKey: "Enter the API key first — providers reject anonymous model lists",
  errorNoModels: "The endpoint returned no models",
  modelIdPlaceholder: "model-id",

  // ── Billing ──
  billing: "Billing",
  billingPlan: "Plan",
  billingPayg: "Pay as you go",
  billingUnlimited: "Unlimited",
  billingFixed: "fixed for this provider",
  billingFromCatalog: "from the catalog",
  /** A catalog row carrying a tag this build does not know. */
  billingUnknown: "Unrecognized billing tag “{billing}” · pick a mode",

  // ── Plan-mode percent limits ──
  usageLimits: "Usage limits",
  usageLimitsHint: "optional · % of each plan window",
  planFiveHourPlaceholder: "e.g. 20 · blank = no limit",
  planWeeklyPlaceholder: "e.g. 60 · blank = no limit",
  fiveHourWindow: "5-hour window",
  weeklyWindow: "Weekly window",
  planLimitsBody:
    "Kiwano stops routing to this provider once its live plan-quota utilization for a window reaches the percent, and resumes when usage drops back under. Takes effect when a plan query is configured.",

  // ── Payg spending limit ──
  spendingLimit: "Spending limit",
  spendingLimitHint: "optional · leave blank to show the usage trend",
  currencyTitle: "{currency} — the currency this provider bills in",

  // ── Unlimited ──
  noQuotaConfig: "No quota config · usage info hidden in lists",

  // ── Agent binding ──
  bindAgents: "Bind to agents after saving",
  selectAgents: "Select agents…",

  // ── Rotating keys (edit mode) ──
  rotatingKeys: "Rotating keys",
  rotatingKeysHint: "rotate with the primary key · avoids rate limits",
  deleteKey: "Delete key",
  addKeyPlaceholder: "sk-… add key",
  labelPlaceholder: "Label",

  // ── Plan quota query (edit mode) ──
  planQuotaQuery: "Plan quota query",
  template: "Template",
  /** Select sentinel label for "no plan query configured". The stored value
      stays the literal "none". */
  none: "None",
  planQueryBody:
    "Queries the plan usage with the provider's API key (5-min cache); quota chips refresh on the Providers page.",

  // ── Advanced forwarding settings ──
  advanced: "Advanced (timeout / retries / headers)",
  timeoutLabel: "Timeout (s)",
  retriesLabel: "Retries",
  advancedBody:
    "Blank = gateway default · timeout caps time to response headers, never an in-flight stream · retries apply to this provider before failover",
  customHeaders: "Custom headers",
  customHeadersHint: "merged last · can override the API key header",
  headerNamePlaceholder: "Header-Name",
  headerValuePlaceholder: "value",
  removeHeader: "Remove header",
  addHeader: "Add header",

  // ── Footer ──
  saving: "Saving…",
  saveEnable: "Save & enable",

  // ── Plan-query templates ──
  // The ids and field keys live in `PLAN_QUERY_TEMPLATES` (api/types.ts); these
  // are the labels it points at. The product names stay as they are in every
  // language — only the qualifier around them is translated.
  tplKimi: "Kimi (monthly plan)",
  tplZhipuPersonal: "Zhipu GLM (personal)",
  tplZhipuTeam: "Zhipu GLM (team)",
  tplMinimax: "MiniMax",
  tplZenmux: "ZenMux",
  tplOpencodeGo: "OpenCode Go",
  tplVolcengine: "Volcengine Ark",
  fieldOrgId: "Org ID",
  fieldProjectId: "Project ID",
  fieldQuotaUrl: "Usage endpoint URL",
  fieldAccessKeyId: "AccessKey ID",
  fieldSecretAccessKey: "Secret AccessKey",
};
