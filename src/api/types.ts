// Kiwano frontend data contract — fields aligned with the SQLite tables (tech.md §2.3/§4.7).
// The UI accesses data only via KiwanoApi in src/api/client.ts; components must not contain
// dev literals or Tauri invoke calls.

export type AgentId =
  | "claude"
  | "codex"
  | "gemini"
  | "grokbuild"
  | "claude-desktop"
  | "opencode"
  | "openclaw"
  | "hermes"
  | "pi";
export type Billing = "plan" | "payg" | "unl";
export type Protocol = "openai" | "anthropic" | "gemini";

export interface AgentMeta {
  id: AgentId;
  label: string;
  chip_char: string;
  chip_color: string;
  chip_border?: boolean;
}

/** Additive-mode agents: config keeps multiple providers coexisting, takeover
    writes a gateway entry and selects it (vs exclusive-switch mode). */
export const ADDITIVE_AGENTS: AgentId[] = ["opencode", "openclaw", "hermes", "pi"];

export const AGENTS: AgentMeta[] = [
  { id: "claude", label: "Claude Code", chip_char: "C", chip_color: "#D97757" },
  { id: "codex", label: "Codex", chip_char: "C", chip_color: "#0F0F0F", chip_border: true },
  { id: "gemini", label: "Gemini CLI", chip_char: "G", chip_color: "#1E6FEB" },
  { id: "grokbuild", label: "Grok Build", chip_char: "X", chip_color: "#1A1A1A", chip_border: true },
  { id: "claude-desktop", label: "Claude Desktop", chip_char: "D", chip_color: "#B45E51" },
  { id: "opencode", label: "OpenCode", chip_char: "O", chip_color: "#7C3AED" },
  { id: "openclaw", label: "OpenClaw", chip_char: "L", chip_color: "#EA580C" },
  { id: "hermes", label: "Hermes", chip_char: "H", chip_color: "#8B5CF6" },
  { id: "pi", label: "Pi", chip_char: "P", chip_color: "#DB2777" },
];

export interface ProviderHealth {
  /** ok=healthy (current) idle=standby off=disabled/not running down=failure */
  state: "ok" | "idle" | "off";
  latency_ms: number | null;
  /** Status column suffix note: disabled / not running / … (omitted when healthy) */
  note?: string;
}

export interface QuotaState {
  used: number;
  limit: number;
  /** requests=counted by requests (subscription) · wan_tokens=counted by 10k-token blocks ·
      otherwise a 3-letter ISO currency code (limit denominated in that currency) */
  unit: "requests" | "wan_tokens" | (string & {});
  /** Reset date YYYY-MM-DD (subscription period); null for payg limits */
  resets_at: string | null;
}

/** Last-7-days usage aggregate (aggregated from the usage table, tech.md §2.3) */
export interface UsageSummary {
  requests: number;
  input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  /** Estimated cost, in the provider's own currency; null for unl (not billed) */
  cost: number | null;
  /** Currency of `cost` — the provider's, never converted; absent when unpriced */
  cost_currency?: string | null;
  latency_ms: number | null;
  quota: QuotaState | null;
  /** Last-7-days trend samples (sparkline for payg types without a limit), y normalized to 0-14 */
  spark: number[] | null;
}

/** Plan-quota query config (providers.plan_query JSON): one of the
    known templates plus its extra credential fields. Templates without extra
    credentials authenticate with the provider's own API key. */
export interface PlanQuery {
  template: string;
  fields?: Record<string, string>;
}

/** Templates offered by the plan-query modal section (Grok is backend-stubbed
    and intentionally absent). Fields render per-template credential inputs. */
export const PLAN_QUERY_TEMPLATES: {
  id: string;
  label: string;
  fields: { key: string; label: string }[];
}[] = [
  { id: "kimi", label: "Kimi (monthly plan)", fields: [] },
  { id: "zhipu", label: "Zhipu GLM (personal)", fields: [] },
  {
    id: "zhipu_team",
    label: "Zhipu GLM (team)",
    fields: [
      { key: "organization_id", label: "Org ID" },
      { key: "project_id", label: "Project ID" },
    ],
  },
  { id: "minimax", label: "MiniMax", fields: [] },
  {
    id: "zenmux",
    label: "ZenMux",
    fields: [{ key: "quota_url", label: "Usage endpoint URL" }],
  },
  { id: "opencode_go", label: "OpenCode Go", fields: [] },
  {
    id: "volcengine",
    label: "Volcengine Ark",
    fields: [
      { key: "access_key_id", label: "AccessKey ID" },
      { key: "secret_access_key", label: "Secret AccessKey" },
    ],
  },
];

/** Canonical tier-name -> display label (backend emits machine keys) */
export const PLAN_TIER_LABELS: Record<string, string> = {
  five_hour: "5h window",
  weekly_limit: "Weekly",
  monthly: "Monthly",
};

/** One usage window of a token plan (5h / weekly / monthly …) */
export interface PlanTier {
  name: string;
  /** Percent of the window already used (unclamped — upstream's honest number) */
  utilization: number;
  resets_at: string | null;
  /** Absolute used / window cap when the endpoint reports amounts; percentage-only endpoints leave both null */
  used: number | null;
  limit: number | null;
  unit: string | null;
}

/** Result of one plan quota query (possibly served from the 5-min backend cache) */
export interface PlanQuotaReport {
  provider_id: string;
  template: string;
  /** false = deterministic failure; error carries the user-facing reason */
  success: boolean;
  error: string | null;
  /** Plan metadata from the endpoint (Zhipu level, ZenMux tier, Volcengine plan) */
  note: string | null;
  tiers: PlanTier[];
  /** Epoch millis of the original query */
  queried_at: number;
  /** true when served from the 5-minute cache */
  cached: boolean;
}

/** Currency metadata from the bundled price table (Settings selector + client conversion) */
export interface CurrencyMeta {
  /** ISO codes present in the bundled price table */
  currencies: string[];
  /** currency -> units of that currency per 1 USD (e.g. CNY: 7.1) */
  exchange_rates: Record<string, number>;
  /** The user's preferred display currency (Settings) */
  preferred: string;
}

/** Plan-mode percent limits (providers.plan_limits JSON): per-window
    utilization ceilings enforced by the app patrol. */
export interface PlanLimits {
  five_hour?: number;
  weekly?: number;
}

export interface Provider {
  id: string;
  name: string;
  logo_char: string;
  logo_color: string;
  logo_border?: boolean;
  endpoint: string;
  protocol: Protocol;
  /** Second half of the endpoint subtitle: OpenAI compatible / qwen3:32b etc. */
  endpoint_note: string;
  /** Additional per-protocol endpoints (one provider serves multiple agent protocols) */
  endpoints?: { protocol: Protocol; endpoint: string }[];
  billing: Billing;
  /** Price row for subscription types (plan): ¥49/month */
  plan_price?: string;
  /** Raw limit unit (requests | wan_tokens | ISO currency) for edit prefill */
  limit_unit?: string;
  /** Plan-quota query config; absent = not configured */
  plan_query?: PlanQuery | null;
  /** Percent-of-window ceilings for plan providers; absent = none set */
  plan_limits?: PlanLimits | null;
  enabled: boolean;
  /** Derived from agent_bindings */
  agents: AgentId[];
  /** Agents this provider would serve a request for right now — the per-agent
      slice behind the agent tabs' local "In use" badge */
  serving_agents: AgentId[];
  /** Collapsed across agents (the All tab badge); agent tabs use serving_agents */
  is_current: boolean;
  /** Badge text: backup #1 / local / … */
  status_badge?: string;
  /** Supplementary note for the Agent column: failover queue / N agents / unbound */
  agents_note?: string;
  health: ProviderHealth;
  usage: UsageSummary | null;
  /** Advanced forwarding settings (timeout / retries / custom headers); absent = all defaults */
  advanced?: {
    timeout_secs?: number | null;
    retries?: number | null;
    headers?: Record<string, string>;
  };
}

export interface NewProviderInput {
  name: string;
  api_key: string;
  endpoint: string;
  protocol: Protocol;
  model_default: string;
  billing: Billing;
  billing_config: {
    limit_value?: number;
    /** requests | wan_tokens | 3-letter ISO currency code */
    limit_unit?: "requests" | "wan_tokens" | (string & {});
    reset_period?: "monthly" | "weekly" | "yearly" | "none";
    /** Plan providers only: percent-of-window ceilings */
    plan_limits?: PlanLimits | null;
  };
  agents: AgentId[];
  /** Additional per-protocol endpoints to persist alongside the primary */
  endpoints?: { protocol: Protocol; endpoint: string }[];
  /** Plan-quota query config; null clears an existing config */
  plan_query?: PlanQuery | null;
  /**
   * Advanced forwarding settings. Absent in an update = keep existing
   * values; a present object is an authoritative snapshot (null clears).
   */
  advanced?: {
    timeout_secs?: number | null;
    retries?: number | null;
    headers?: Record<string, string>;
  };
}

export interface CatalogEntry {
  id: string;
  name: string;
  logo_char: string;
  logo_color: string;
  logo_border?: boolean;
  /** Brand-mark key in src/components/icons registry; missing = letter avatar */
  icon?: string;
  /** Hub-relative logo path ("logos/<id>.<ext>"), resolved against hub_url;
      missing/offline falls back to `icon` then the letter avatar */
  logo?: string;
  /** Tag category shown on the card's top-right label (Models page chip filter) */
  tag: "official" | "aggregate" | "third" | "free" | "local";
  tag_label: string;
  rating: number;
  /** Endpoint pre-filled into the add modal */
  endpoint: string;
  /** Protocol fingerprint of the endpoint (drives the add-modal protocol selector) */
  protocol: Protocol;
  /** Price line: ¥4 /M in · ¥16 /M out etc. */
  price_line: string;
  /** The currency this provider bills in; the spending limit is denominated
      in it. Older catalog entries omit it → the app falls back to USD. */
  currency?: string;
  price_note?: string;
  billing: Billing;
  users: string;
  blurb: string;
  /** Derived at read time from the local provider list (same endpoint = added) */
  added: boolean;
  /** One-liner shown when a free quota exists */
  free_offer?: string;
  /** Model options pre-populated in the add modal */
  models: string[];
  /** Additional per-protocol endpoints merged from former sibling entries */
  endpoints?: { protocol: Protocol; endpoint: string; models: string[] }[];
}

export interface CatalogList {
  /** Total count of the Hub catalog (Models page header copy) */
  total: number;
  entries: CatalogEntry[];
}

export interface TrendPoint {
  date: string;
  requests: number;
  tokens: number;
}

export type DashboardWindow = "today" | "7d" | "30d";

export interface DashboardData {
  window: DashboardWindow;
  requests: number;
  requests_delta_pct: number;
  input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  cost: number;
  latency_ms: number;
  latency_delta_pct: number;
  trend: TrendPoint[];
  by_provider: { name: string; color: string; pct: number; cost: number }[];
  by_agent: { agent: AgentId; label: string; requests: number; tokens: string; cost: number }[];
  /** Filter select options: providers/agents with traffic in the window,
      computed independent of the active filter */
  filter_providers: { id: string; label: string }[];
  filter_agents: { id: string; label: string }[];
}

export interface TakeoverState {
  agent: AgentId;
  label: string;
  /** Placeholder key assigned by the gateway; null when not taken over */
  placeholder_key: string | null;
  enabled: boolean;
  /** Additive-mode agent (config keeps multiple providers; takeover writes a
      gateway entry and selects it) rather than exclusive-switch mode */
  additive: boolean;
}

export interface AppSettings {
  language: string;
  theme: string;
  autostart: boolean;
  close_to_tray: boolean;
  gateway_listen: string;
  takeovers: TakeoverState[];
  auto_failover: boolean;
  request_logs: boolean;
  /** Request-log retention in days (gateway prunes older rows every 6h) */
  log_retention_days: number;
  telemetry: boolean;
  /** Cost alert (spec §4.1 P1): system notification when usage reaches the per-period limit */
  cost_alert: boolean;
  /** Preferred display currency for costs (ISO code; converted via the bundled rates) */
  preferred_currency: string;
  /** Auto-check for app updates at startup (silent; notification only) */
  auto_check_update: boolean;
  /** Version the user dismissed in the update banner (a newer one shows again) */
  dismissed_update: string | null;
  hub_logged_in: boolean;
  /** Hub catalog sync endpoint (protocol v0: static JSON) */
  hub_url: string;
}

/** Hub catalog sync result (tech.md §3 Hub sync protocol) */
export interface HubSyncReport {
  fetched: number;
  synced_at: string;
  hub_url: string;
  /** The manifest hash matched the cache, so the catalog was not re-downloaded */
  unchanged: boolean;
  /** Version of the price table now cached; absent when the Hub has none */
  pricing_version?: number;
  /** The pricing half was already current, so models.json was not re-downloaded */
  pricing_unchanged: boolean;
}

export interface GatewayStatus {
  running: boolean;
  port: number;
}

export interface FooterStats {
  today_requests: number;
  /** Tokens consumed today (input + output) */
  today_tokens: number;
  hub_synced: boolean;
  version: string;
}

/** CC Switch import result (tech.md §4.5: v3.x only) */
export interface ImportReport {
  imported: number;
  skipped: number;
  detail: string[];
}

/** Config-plan import result (spec §4.1 P1 config sharing) */
export interface ConfigShareReport {
  providers_added: number;
  providers_kept: number;
  routes_applied: number;
}

/** A Provider's rotating keys (spec §4.1 P1 multi-key rotation; the primary key lives on the Provider) */
export interface ApiKeyEntry {
  id: number;
  api_key: string;
  label?: string;
  enabled: boolean;
  created_at: string;
}

/** Cost alert hit (backend already dedupes per period; the frontend just forwards it as a system notification) */
export interface UsageAlert {
  provider_id: string;
  provider_name: string;
  used: number;
  limit: number;
  /** requests | wan_tokens */
  unit: string;
}

export type StrategyKind = "single" | "failover" | "roundrobin" | "timewindow" | "quota";

/** Agent installation detection (phase 1: existence only — one login-shell probe) */
export interface AgentDetect {
  agent: AgentId;
  installed: boolean;
  /** Resolved CLI binary path; null for claude-desktop */
  path: string | null;
}

/** Agent version probe (phase 2: async `--version`; null when the probe failed) */
export interface AgentVersionEntry {
  agent: AgentId;
  version: string | null;
}

/** Agent strategy candidates (projection of agent_bindings rows, ascending by priority, 0 = primary) */
export interface StrategyBinding {
  provider_id: string;
  provider_name: string;
  logo_char: string;
  logo_color: string;
  priority: number;
  weight: number;
  /** Local "HH:MM" window bounds (timewindow strategy); null = windowless fallback */
  win_start: string | null;
  win_end: string | null;
  enabled: boolean;
}

/** Agent routing strategy (agent_strategies + agent_bindings, tech.md §4.7) */
export interface AgentRoute {
  agent: AgentId;
  strategy: StrategyKind;
  /** Strategy JSON payload (quota: {"limit","unit"}; null otherwise) */
  config: string | null;
  bindings: StrategyBinding[];
}

/** One data-plane request recorded by the gateway (metadata row; bodies live in the detail view) */
export interface RequestLogEntry {
  id: number;
  ts: string;
  method: string;
  path: string;
  query: string | null;
  agent: string | null;
  /** key = placeholder-key attribution · path_fallback = matched by path protocol */
  attribution: string | null;
  provider_id: string | null;
  model: string | null;
  status_code: number;
  error_kind: string | null;
  error_message: string | null;
  session_id: string | null;
  is_streaming: boolean;
  input_tokens: number;
  output_tokens: number;
  cache_read_tokens: number;
  cache_creation_tokens: number;
  latency_ms: number | null;
  first_token_ms: number | null;
  /** Redacted header JSON (auth headers stripped at capture time) */
  request_headers: string | null;
  response_headers: string | null;
  request_size: number;
  response_size: number;
  truncated: boolean;
}

/** Detail view: metadata + the captured request/response bodies */
export interface RequestLogDetail extends RequestLogEntry {
  request_body: string | null;
  response_body: string | null;
}

export interface RequestLogList {
  rows: RequestLogEntry[];
  total: number;
}

export interface RequestLogFilter {
  agent?: string;
  provider_id?: string;
  status?: "ok" | "error";
}

/** Protocol-aware endpoint probe (GET the protocol's models route; works
    without a key — 401/403 still proves the route exists) */
export interface ProbeReport {
  /** ok=route answers · auth=route exists, key required/invalid ·
      unsupported=404/405 · error=bad status/non-JSON · unreachable=no connect */
  verdict: "ok" | "auth" | "unsupported" | "error" | "unreachable";
  status: number | null;
  latency_ms: number;
  /** Human-readable explanation for the UI chip */
  detail: string;
}

/** A pending app update found on the release channel */
export interface UpdateInfo {
  version: string;
  notes: string | null;
  /** Release publish time, unix seconds */
  pub_date: number | null;
}

/** Download progress payload */
export interface UpdateProgress {
  downloaded: number;
  total: number | null;
}

export interface KiwanoApi {
  getGatewayStatus(): Promise<GatewayStatus>;
  listProviders(filter?: AgentId | "all"): Promise<Provider[]>;
  addProvider(input: NewProviderInput): Promise<Provider>;
  /** Update a provider (empty api_key means keep the existing key) */
  updateProvider(id: string, input: NewProviderInput): Promise<Provider>;
  /** Delete a provider; if it is the primary for some agent, the next candidate is promoted automatically */
  deleteProvider(id: string): Promise<void>;
  /** Enable = make this Provider the current route of its bound agents */
  enableProvider(id: string): Promise<void>;
  testLatency(endpoint: string): Promise<number>;
  /** Protocol-aware probe: GET the protocol's models route; 401/403 still
      proves the protocol route exists (works without a key) */
  testEndpoint(protocol: Protocol, endpoint: string, apiKey?: string): Promise<ProbeReport>;
  /** Live model-name list from a provider endpoint (the API key is required:
      cloud providers reject anonymous /models calls) */
  listModels(protocol: Protocol, endpoint: string, apiKey: string): Promise<string[]>;
  listCatalog(): Promise<CatalogList>;
  getDashboard(window: DashboardWindow, providerId?: string, agentId?: string): Promise<DashboardData>;
  getSettings(): Promise<AppSettings>;
  updateSettings(patch: Partial<AppSettings>): Promise<AppSettings>;
  /** Fetch the Provider catalog from the Hub into the local cache (Models page prefers cache, falls back to static) */
  syncHub(): Promise<HubSyncReport>;
  /** Cost alert patrol (backend KV dedupe: returned at most once per Provider per reset period) */
  checkUsageAlerts(): Promise<UsageAlert[]>;
  /** Token-plan quota query; 5-min backend cache, force bypasses it */
  getPlanQuota(providerId: string, force?: boolean): Promise<PlanQuotaReport>;
  /** Currency metadata for the Settings selector + client-side conversion */
  getCurrencyMeta(): Promise<CurrencyMeta>;
  /** A Provider's list of rotating keys (excluding the primary key) */
  listApiKeys(providerId: string): Promise<ApiKeyEntry[]>;
  /** Append a rotating key (the gateway's key pool hot-reloads immediately) */
  addApiKey(providerId: string, apiKey: string, label?: string): Promise<ApiKeyEntry>;
  deleteApiKey(id: number): Promise<void>;
  /** Agent takeover switch (generates/deletes the placeholder key; config rewriting included from P1 on) */
  setTakeover(agent: AgentId, enabled: boolean): Promise<void>;
  /** Import config from CC Switch (auto-detects the ~/.cc-switch data source) */
  importCcSwitch(): Promise<ImportReport>;
  /** Agent routing strategy table (only agents with bindings) */
  getAgentRoutes(): Promise<AgentRoute[]>;
  /** Update an agent's strategy type (config only needed for quota: {"limit","unit"}) */
  updateAgentStrategy(agent: AgentId, strategy: StrategyKind, config?: string | null): Promise<void>;
  /** Reorder candidates: provider_id order → priority 0..n */
  reorderAgentBindings(agent: AgentId, providerIds: string[]): Promise<void>;
  /** Patch one binding's strategy parameters (roundrobin weight / timewindow local window) */
  updateAgentBinding(
    agent: AgentId,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ): Promise<void>;
  /** Bind a provider to an agent as a new candidate (appended at the queue tail) */
  addAgentBinding(agent: AgentId, providerId: string): Promise<void>;
  /** Remove one agent's binding of a provider (other agents keep theirs) */
  removeAgentBinding(agent: AgentId, providerId: string): Promise<void>;
  /** Copy another agent's whole route (strategy + ordered candidates) onto this one, replacing what it had */
  applyAgentRoute(target: AgentId, source: AgentId): Promise<void>;
  /** Export the config plan to the given path (including API keys); returns the Provider count */
  exportConfig(path: string): Promise<number>;
  /** Import a config plan from a file (merged by name+base_url); returns a count report */
  importConfig(path: string): Promise<ConfigShareReport>;
  /** Phase 1 agent detection: installed state per agent (fast; null path = probed unavailable) */
  detectAgents(): Promise<AgentDetect[]>;
  /** Phase 2 agent versions: slow `--version` probes, fetched after detection; null = probe failed */
  probeAgentVersions(): Promise<AgentVersionEntry[]>;
  /** Paged data-plane request log (metadata only; bodies via getRequestLog) */
  listRequestLogs(page: number, pageSize: number, filter?: RequestLogFilter): Promise<RequestLogList>;
  /** One request-log row with captured bodies; null when the row was pruned */
  getRequestLog(id: number): Promise<RequestLogDetail | null>;
  /** Delete every request-log row (bodies cascade) */
  clearRequestLogs(): Promise<void>;
  getFooterStats(): Promise<FooterStats>;
  /** Check GitHub Releases for a newer version; null = up to date */
  checkAppUpdate(): Promise<UpdateInfo | null>;
  /** Update found by the silent startup check; null = none found (or disabled) */
  getPendingUpdate(): Promise<UpdateInfo | null>;
  /** Download, verify and install the pending update, then relaunch */
  downloadAndInstallAppUpdate(): Promise<void>;
  /** Subscribe to download progress; returns an unsubscribe function */
  onUpdateProgress(cb: (p: UpdateProgress) => void): Promise<() => void>;
}
