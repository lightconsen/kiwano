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
  /** requests=counted by requests (subscription) cny=counted by amount (payg limit) */
  unit: "requests" | "cny";
  /** Reset date YYYY-MM-DD (subscription period); null for payg limits */
  resets_at: string | null;
}

/** Last-7-days usage aggregate (aggregated from the usage table, tech.md §2.3) */
export interface UsageSummary {
  requests: number;
  input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  /** Estimated cost (¥); null for unl since it is not billed */
  cost: number | null;
  latency_ms: number | null;
  quota: QuotaState | null;
  /** Last-7-days trend samples (sparkline for payg types without a limit), y normalized to 0-14 */
  spark: number[] | null;
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
  billing: Billing;
  /** Price row for subscription types (plan): ¥49/month */
  plan_price?: string;
  enabled: boolean;
  /** Derived from agent_bindings */
  agents: AgentId[];
  /** The row currently in use after list sorting (kiwi left bar + "in use" badge) */
  is_current: boolean;
  /** Badge text: backup #1 / local / … */
  status_badge?: string;
  /** Supplementary note for the Agent column: failover queue / N agents / unbound */
  agents_note?: string;
  health: ProviderHealth;
  usage: UsageSummary | null;
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
    limit_unit?: "requests" | "wan_tokens" | "cny";
    reset_period?: "monthly" | "weekly" | "yearly" | "none";
  };
  agents: AgentId[];
}

export interface CatalogEntry {
  id: string;
  name: string;
  logo_char: string;
  logo_color: string;
  logo_border?: boolean;
  /** Brand-mark key in src/components/icons registry; missing = letter avatar */
  icon?: string;
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
  price_note?: string;
  billing: Billing;
  users: string;
  blurb: string;
  added: boolean;
  /** One-liner shown when a free quota exists */
  free_offer?: string;
  /** Model options pre-populated in the add modal */
  models: string[];
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
  telemetry: boolean;
  /** Cost alert (spec §4.1 P1): system notification when usage reaches the per-period limit */
  cost_alert: boolean;
  hub_logged_in: boolean;
  /** Hub catalog sync endpoint (protocol v0: static JSON) */
  hub_url: string;
}

/** Hub catalog sync result (tech.md §3 Hub sync protocol) */
export interface HubSyncReport {
  fetched: number;
  synced_at: string;
  hub_url: string;
}

export interface GatewayStatus {
  running: boolean;
  port: number;
}

export interface FooterStats {
  today_requests: number;
  today_cost: number;
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

/** Agent strategy candidates (projection of agent_bindings rows, ascending by priority, 0 = primary) */
export interface StrategyBinding {
  provider_id: string;
  provider_name: string;
  logo_char: string;
  logo_color: string;
  priority: number;
  weight: number;
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
  listCatalog(): Promise<CatalogList>;
  getDashboard(window: DashboardWindow): Promise<DashboardData>;
  getSettings(): Promise<AppSettings>;
  updateSettings(patch: Partial<AppSettings>): Promise<AppSettings>;
  /** Fetch the Provider catalog from the Hub into the local cache (Models page prefers cache, falls back to static) */
  syncHub(): Promise<HubSyncReport>;
  /** Cost alert patrol (backend KV dedupe: returned at most once per Provider per reset period) */
  checkUsageAlerts(): Promise<UsageAlert[]>;
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
  /** Export the config plan to the given path (including API keys); returns the Provider count */
  exportConfig(path: string): Promise<number>;
  /** Import a config plan from a file (merged by name+base_url); returns a count report */
  importConfig(path: string): Promise<ConfigShareReport>;
  getFooterStats(): Promise<FooterStats>;
}
