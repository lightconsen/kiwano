// Keys for addProvider. English is the source of truth for the key set;
// the zh-CN file must match this shape exactly (see ../types.ts).
//
// Protocol and billing names (OpenAI / Anthropic, Plan / Pay as
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

  apiKey: "API Key",
  keyKeep: "Leave blank to keep the current key",
  keyLocal: "Stored locally, readable only by you",
  showHideKey: "Show / hide key",

  endpointUrl: "Endpoint URL",
  endpointHintShelf: "from the catalog · one per protocol · shares the API key",
  addEndpoint: "Add endpoint",
  removeEndpoint: "Remove this endpoint",
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
  /** The one entry that charges both ways: the catalog names two arrangements,
      so the choice belongs to the user and the form will not save without it. */
  billingBoth: "This vendor charges both ways — pick the one this provider is",

  // ── Plan-mode percent limits ──
  usageLimits: "Usage limits",
  usageLimitsHint: "optional · % of each plan window",
  fiveHourWindow: "5-hour window",
  weeklyWindow: "Weekly window",
  planLimitsBody:
    "Kiwano stops routing to this provider once its live plan-quota utilization for a window reaches the percent, and resumes when usage drops back under. Blank leaves that window unlimited.",
  /** Rejected input: the backend keeps only this range and drops the rest
      without a word, so an out-of-range ceiling would save as nothing. */
  planLimitRange: "1–100, or blank for no limit",

  // ── Payg spending limit ──
  spendingLimit: "Spending limit",
  spendingLimitHint: "optional · leave blank to show the usage trend",
  currencyTitle: "{currency} — the currency this provider bills in",

  // ── Plan, but nothing that can be asked ──
  noQuotaEndpoint:
    "No quota endpoint · this vendor publishes no way to read plan usage with your key alone",

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

  // The plan-quota query is no longer chosen here: it comes from the catalog
  // entry, which only ever publishes a template that needs nothing but the
  // provider's own API key. The template labels below stay because the table
  // still maps a stored config's credential fields when one is re-saved.

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
