// Kiwano frontend data contract — fields aligned with the SQLite tables (tech.md §2.3/§4.7).
// The UI accesses data only via KiwanoApi in src/api/client.ts; components must not contain
// dev literals or Tauri invoke calls.
//
// The two display constants at the foot of this file carry *dictionary keys*
// rather than labels. They are module-level tables that get rendered, so the
// translator cannot be called where they are declared — the key is resolved at
// the call site, which is the same shape the screens use for their own tables.
import type { KeyPath, Messages } from "../i18n";

export type AgentId =
  | "claude"
  | "codex"
  | "gemini"
  | "grokbuild"
  | "claude-desktop"
  | "opencode"
  | "openclaw"
  | "hermes"
  | "pi"
  | "workbuddy"
  | "codebuddy"
  | "mimo"
  | "kimi"
  | "qwen"
  | "cline";
/** An agent id as it travels through routes, bindings, usage and logs: a
    built-in's, or one the user defined. The closed `AgentId` above stays the
    type wherever the thing being named is a *built-in* — the takeover switch,
    the agent registry, the icon table — so a user-defined id cannot be fed to
    code that would go looking for its config file. */
export type AgentRef = AgentId | (string & {});

/** An agent the user defined (migration v16): a name for a route, its own key,
    and no config file anywhere. */
export interface CustomAgent {
  id: string;
  label: string;
  note: string | null;
  /** What this agent's clients speak, chosen when it was defined; null for one
      defined before the field existed, which is "not said" rather than a
      default. A *label*: nothing routes or validates by it. */
  protocol: Protocol | null;
  placeholder_key: string | null;
}

export type Billing = "plan" | "payg" | "unl";
/** A **catalog** entry's billing tag: the three a local provider can hold, plus
    `both` for a vendor that charges two ways at one address — Anthropic sells an
    API (metered) and Pro/Max (subscription), and both answer on the same host. A
    local row can only be one, so the add dialog asks the user to settle it and
    `both` is never stored. The same split Rust makes between `CatalogBilling` and
    `store::Billing`. */
export type CatalogBilling = Billing | "both";
export type Protocol = "openai" | "anthropic" | "gemini";

export interface AgentMeta {
  /** Built-in in the registry; a user-defined id in the entries the resolver
      derives (see lib/agents.ts). */
  id: AgentRef;
  label: string;
  chip_char: string;
  chip_color: string;
  chip_border?: boolean;
}

/** Additive-mode agents: config keeps multiple providers coexisting, takeover
    writes a gateway entry and selects it (vs exclusive-switch mode). */
export const ADDITIVE_AGENTS: AgentId[] = [
  "opencode",
  "openclaw",
  "hermes",
  "pi",
  "workbuddy",
  "codebuddy",
  "kimi",
  "qwen",
  "mimo",
];

export const AGENTS: AgentMeta[] = [
  { id: "claude", label: "Claude Code", chip_char: "C", chip_color: "#D97757" },
  { id: "codex", label: "Codex", chip_char: "C", chip_color: "#0F0F0F", chip_border: true },
  { id: "gemini", label: "Gemini CLI", chip_char: "G", chip_color: "#4285F4" },
  { id: "grokbuild", label: "Grok Build", chip_char: "X", chip_color: "#1A1A1A", chip_border: true },
  { id: "claude-desktop", label: "Claude Desktop", chip_char: "D", chip_color: "#B45E51" },
  { id: "opencode", label: "OpenCode", chip_char: "O", chip_color: "#7C3AED" },
  { id: "openclaw", label: "OpenClaw", chip_char: "L", chip_color: "#EA580C" },
  { id: "hermes", label: "Hermes", chip_char: "H", chip_color: "#8B5CF6" },
  { id: "pi", label: "Pi", chip_char: "P", chip_color: "#DB2777" },
  { id: "workbuddy", label: "WorkBuddy", chip_char: "W", chip_color: "#0052D9" },
  { id: "codebuddy", label: "CodeBuddy Code", chip_char: "B", chip_color: "#0EA5E9" },
  { id: "kimi", label: "Kimi Code CLI", chip_char: "K", chip_color: "#1783FF" },
  { id: "qwen", label: "Qwen Code", chip_char: "Q", chip_color: "#615CED" },
  { id: "cline", label: "Cline", chip_char: "C", chip_color: "#1C1C24", chip_border: true },
  { id: "mimo", label: "MiMo Code", chip_char: "M", chip_color: "#FF6900" },
];

export interface ProviderHealth {
  /** `ok` — measured and answering. `error` — measured and not working: either
      nothing answered, or something did and refused the key (see `error`).
      `off` — parked by the user. `idle` — enabled with nothing known against it
      yet: no traffic in the window, no verdict on file. Today's equivalent of
      the blank cell. */
  state: "ok" | "error" | "idle" | "off";
  /** The round trip behind `state: "ok"`; null otherwise. */
  latency_ms: number | null;
  /** Status column suffix note: `Disabled` — omitted when there is nothing to
      say, which is the ordinary case. */
  note?: string;
  /** Where `latency_ms` came from, and these are not the same claim:
      `traffic` is this provider's own requests through the gateway (with the
      user's key), `probe` is the gateway's unsigned GET to its endpoint, and
      `test` is the Apps screen's own latency test — a real prompt, sent with the
      key, which is the strongest of the three. */
  source?: "traffic" | "probe" | "test";
  /** What the endpoint said when it said no: the vendor's own message for a
      refused key ("invalid API key"), or the transport error when nothing
      answered. Its presence is what separates "the key is wrong" from "nobody
      answered" — a 401 is the vendor responding. */
  error?: string;
  /** When the probe ran (RFC3339). `probe` only: a traffic average covers a
      window, not an instant. */
  checked_at?: string;
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
  /** Cache writes; part of the input-side denominator of the hit rate */
  cache_creation_tokens: number;
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
  /** Dictionary key, not the label: the names are product names that stay as
      they are in every language, and only the qualifier around them moves. */
  labelKey: KeyPath<Messages>;
  fields: { key: string; labelKey: KeyPath<Messages> }[];
}[] = [
  { id: "kimi", labelKey: "addProvider.tplKimi", fields: [] },
  { id: "zhipu", labelKey: "addProvider.tplZhipuPersonal", fields: [] },
  {
    id: "zhipu_team",
    labelKey: "addProvider.tplZhipuTeam",
    fields: [
      { key: "organization_id", labelKey: "addProvider.fieldOrgId" },
      { key: "project_id", labelKey: "addProvider.fieldProjectId" },
    ],
  },
  { id: "minimax", labelKey: "addProvider.tplMinimax", fields: [] },
  {
    id: "zenmux",
    labelKey: "addProvider.tplZenmux",
    fields: [{ key: "quota_url", labelKey: "addProvider.fieldQuotaUrl" }],
  },
  { id: "opencode_go", labelKey: "addProvider.tplOpencodeGo", fields: [] },
  {
    id: "volcengine",
    labelKey: "addProvider.tplVolcengine",
    fields: [
      { key: "access_key_id", labelKey: "addProvider.fieldAccessKeyId" },
      { key: "secret_access_key", labelKey: "addProvider.fieldSecretAccessKey" },
    ],
  },
];

/** Canonical tier-name -> the dictionary key for its label (the backend emits
    machine keys, so the mapping is by name and the label is resolved at render).
    The `?? t.name` fallback at the call sites is deliberate: a tier the backend
    adds before this list knows about it should read as its own id, not vanish. */
export const PLAN_TIER_LABEL_KEYS: Record<string, KeyPath<Messages>> = {
  five_hour: "providers.tierFiveHour",
  weekly_limit: "providers.tierWeekly",
  monthly: "providers.tierMonthly",
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

/** Currency metadata from the Hub price table (Settings selector + client conversion) */
/** One row of the price mirror — what the gateway charges with, which is not
    the same list as the catalog's one `price_ref` per provider. `off_peak` and
    `peak_hours` are the row's own schedule when it publishes one, and the row's
    `input`/`output` are then the **peak** figures. */
/** The rates that apply once a request's input passes `over`. The row's own
    rates are then the cheap band — the published listing is the one most requests
    pay — and a band carries no schedule of its own. */
export interface LongContextRates {
  /** Input tokens **above** which these rates apply. */
  over: number;
  in: string;
  out: string;
  cache_read: string;
  cache_creation: string;
}

export interface ModelPrice {
  /** The catalog entry this price belongs to; "" for the general price. */
  provider_id: string;
  model_id: string;
  display_name: string;
  input: string;
  output: string;
  cache_read: string;
  cache_creation: string;
  /** ISO code the figures are denominated in (the provider's own currency) */
  currency: string;
  off_peak?: { in: string; out: string; cache_read: string; cache_creation: string };
  peak_hours?: { tz_offset: number; windows: { days: string[]; start: string; end: string }[] };
  long_context?: LongContextRates;
}

export interface CurrencyMeta {
  /** ISO codes present in the Hub price table */
  currencies: string[];
  /** currency -> units of that currency per 1 USD (e.g. CNY: 7.1) */
  exchange_rates: Record<string, number>;
  /** The user's preferred display currency (Settings) */
  preferred: string;
}

/** One model's rates as the user declared them (per million tokens, TEXT
    decimals in the bundle's currency — parse before arithmetic, as everywhere
    prices travel). */
export interface DeclaredPrice {
  model_id: string;
  input: string;
  output: string;
  /** Zero when the user left the box blank: the vendor charges nothing for that
      bucket. Never the input rate — that would invent a charge. */
  cache_read: string;
  cache_creation: string;
}

/** The prices a user declared for their own provider (`providers.prices`).
 *
 * The second price source beside the Hub's: the Hub prices the models of *its*
 * catalog entries, so a provider that names no entry has no published rate to be
 * costed at — and the user is the only one who can say what it charges. These
 * figures cost that provider's requests and its spending limit is measured
 * against them, which is why they take precedence over the Hub's table. */
export interface DeclaredPrices {
  /** ISO code every figure is denominated in. The form fills it from the same
      picker the spending limit uses: both are about what this provider bills. */
  currency: string;
  models: DeclaredPrice[];
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
  /** The catalog entry this provider was added from, when it came from the
      shelf. The add/edit dialog reaches the entry through it for the endpoints
      and the billing currency the stored row may be missing. */
  catalog_id?: string | null;
  /** The currency this provider's figures are denominated in — what the user
      declared, else its catalog entry's, else USD. Read by a spending limit's
      unit picker, whose options are the currencies the limit's agent actually
      bills in. */
  currency: string;
  /** Second half of the endpoint subtitle: OpenAI compatible / qwen3:32b etc. */
  endpoint_note: string;
  /** Additional per-protocol endpoints (one provider serves multiple agent protocols) */
  endpoints?: { protocol: Protocol; endpoint: string }[];
  billing: Billing;
  /** Price row for subscription types (plan): ¥49/month */
  plan_price?: string;
  /** Raw limit unit (requests | wan_tokens | ISO currency) for edit prefill */
  limit_unit?: string;
  /** The model the add/edit form collected as this provider's default; absent
      when never set. Remembered for the dialog, not consulted when routing. */
  model_default?: string | null;
  /** Plan-quota query config; absent = not configured */
  plan_query?: PlanQuery | null;
  /** The prices the user declared for this provider (`providers.prices`).
      Absent = none declared, so its requests are priced from the Hub's table —
      or recorded unpriced where the Hub knows nothing about the model either. */
  prices?: DeclaredPrices | null;
  /** Percent-of-window ceilings for plan providers; absent = none set */
  plan_limits?: PlanLimits | null;
  enabled: boolean;
  /** Agents bound to this provider *and* routed through the gateway: a binding
      whose agent has its own config back is a stored route, not a live one, so
      it does not appear here (nor in serving_agents). */
  agents: AgentRef[];
  /** Agents this provider would serve a request for right now — the per-agent
      slice behind the agent tabs' local "In use" badge */
  serving_agents: AgentRef[];
  /** Agents for which this provider is the quota strategy's configured first
      backup while that agent's primary is over its threshold. Badged as
      "Fallback", not "In use": which backup actually serves depends on the
      gateway's breakers at request time. */
  fallback_agents?: AgentRef[];
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
  /** Agents to bind this provider to. Sent when adding — a new provider is
      useless unbound — and **omitted when editing**, where the bindings belong
      to the Apps screen's agent tabs. Sending them on an edit would promote this
      provider to primary for every agent it is already bound to, and rewrite the
      agent's strategy, as a side effect of saving anything else. */
  agents?: AgentRef[];
  /** Additional per-protocol endpoints to persist alongside the primary */
  endpoints?: { protocol: Protocol; endpoint: string }[];
  /** Plan-quota query config; null clears an existing config */
  plan_query?: PlanQuery | null;
  /** The prices the user declared for this provider.
   *
   * Omitted when the form is not speaking about prices — the provider is not
   * pay-as-you-go, or a catalog entry owns the form — and that omission keeps
   * whatever is stored, because the section that would show them is hidden.
   * An empty `models` list is the form saying "none": it clears the column. */
  prices?: DeclaredPrices;
  /**
   * The catalog entry this provider is being added from (shelf adds only).
   * Prices are published per catalog entry, so this is what the gateway looks
   * a request's price up by. Absent = no catalog entry; on update, absent or
   * empty keeps the stored value.
   */
  catalog_id?: string;
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

/** The price of the model that represents a provider, projected at build time
    from the data repo's `flagship` flag. Figures are per million tokens as TEXT
    decimals in `currency`, so parse before doing arithmetic. */
export interface CatalogPriceRef {
  model_id: string;
  /** e.g. "Claude Opus 5" — shown beside the figures */
  display_name: string;
  input: string;
  output: string;
  /** ISO code the figures are denominated in (the provider's own currency) */
  currency: string;
  /** The rates outside `peak_hours`. Present only when the provider publishes a
      schedule — and then `input`/`output` above are the **peak** ones. */
  off_peak?: { in: string; out: string; cache_read: string; cache_creation: string };
  /** When the peak rates apply, on the vendor's clock (`tz_offset`, minutes east
      of UTC). Read it with `lib/peak.ts`, never against the reader's zone. */
  peak_hours?: {
    tz_offset: number;
    windows: { days: string[]; start: string; end: string }[];
  };
  /** The rates above `over` input tokens, when the flagship is priced in bands.
      A second projection of the same thing `ModelPrice.long_context` carries: the
      shelf's price cell reads this one, the detail dialog reads the mirror's, and
      declaring only one of them makes half the screens disagree. */
  long_context?: LongContextRates;
}

/** What the app hands the Models page. The Hub publishes less than this: the
    primary endpoint (hoisted out of `endpoints`), the avatar colour, and
    `added` are filled in by the Rust view model on the way in, and the row's
    tag label is derived from `tag` through i18n. */
export interface CatalogEntry {
  id: string;
  name: string;
  /** Letter-avatar colour (the palette shared with locally-added providers) */
  logo_color: string;
  /** Hub-relative logo path ("logos/<id>.<ext>"), resolved against hub_url;
      absent offline, which falls back to the letter avatar */
  logo?: string;
  /** Tag category; drives the chip filter, the row sort and the badge text */
  tag: "official" | "aggregate" | "third" | "free" | "local";
  rating: number;
  /** Primary endpoint, pre-filled into the add modal */
  endpoint: string;
  /** Protocol of the primary endpoint (drives the add-modal selector) */
  protocol: Protocol;
  /** One line of prose: a price note, an audience line, a free-tier offer.
      Absent for the entries with nothing to say. */
  desc?: string;
  /** The provider's own site, from the Hub. Open it with `openUrl`. */
  website?: string;
  /** The representative model's price; absent for providers that price no model */
  price_ref?: CatalogPriceRef;
  /** The currency this provider bills in; the spending limit is denominated
      in it. Older catalog entries omit it → the app falls back to USD. */
  currency?: string;
  /** How to read this provider's plan usage, when the vendor's own endpoint can
      answer with nothing but the provider's key. Only four catalog entries carry
      it, and `billing === "plan"` does not imply it: the field is the difference
      between "bills by plan" and "can be asked how much of the plan is spent",
      and only the latter justifies offering a ceiling. */
  plan_query?: { template: string };
  billing: CatalogBilling;
  /** Derived at read time from the local provider list (same endpoint = added) */
  added: boolean;
  /** Models the primary endpoint serves; seeds the add modal's picker */
  models: string[];
  /** Every endpoint after the primary */
  endpoints?: { protocol: Protocol; endpoint: string; models: string[] }[];
}

export interface CatalogList {
  /** Total count of the Hub catalog (Models page header copy) */
  total: number;
  entries: CatalogEntry[];
}

export interface TrendPoint {
  /** Axis label for the bucket: `MM-DD` when bucketed by day, `HH:00` when
   *  the "today" window buckets by hour. */
  date: string;
  requests: number;
  tokens: number;
}

export type DashboardWindow = "today" | "7d" | "30d" | "all";

export interface DashboardData {
  window: DashboardWindow;
  requests: number;
  /** Against the window before this one; null when there is nothing to compare
      against (the "all" window, or an earlier window with no traffic). */
  requests_delta_pct: number | null;
  input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  cost: number;
  /** The peak premium: what these requests would have cost had they all run at
      their rows' off-peak rates. 0 when nothing in the window is tiered, and 0
      (correctly) for traffic that was already off-peak. */
  cost_off_peak: number;
  latency_ms: number;
  /** Same comparison for the average latency. Positive means slower, which the
      screen colours as the bad direction. */
  latency_delta_pct: number | null;
  trend: TrendPoint[];
  by_provider: {
    id: string;
    name: string;
    color: string;
    requests: number;
    pct: number;
    cost: number;
    /** The peak premium: what these requests would have cost had they all run at
            their rows' off-peak rates. 0 when nothing in the window is tiered, and 0
            (correctly) for traffic that was already off-peak. */
    cost_off_peak: number;
  }[];
  by_agent: {
    agent: AgentRef;
    label: string;
    requests: number;
    tokens: string;
    cost: number;
    cost_off_peak: number;
  }[];
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
  /** The files a takeover rewrites, as `~/.claude/settings.json`; empty when
      they cannot be resolved here (claude-desktop outside macOS) */
  config_paths: string[];
  /** Additive-mode agent (config keeps multiple providers; takeover writes a
      gateway entry and selects it) rather than exclusive-switch mode */
  additive: boolean;
  /** The protocols this agent's clients speak. On this row because it is *the*
      per-agent row the screen reads, the same way `additive` is a fact about
      the agent rather than about takeover state. A label: nothing routes by it. */
  protocols: Protocol[];
}

export interface AppSettings {
  language: string;
  theme: string;
  autostart: boolean;
  close_to_tray: boolean;
  gateway_listen: string;
  takeovers: TakeoverState[];
  /** The user's own agents. Not part of `takeovers`: that list answers "we
      rewrote this agent's config", and these have no config to rewrite. */
  custom_agents: CustomAgent[];
  auto_failover: boolean;
  request_logs: boolean;
  /** Compat shim: sanitize passthrough request bodies the upstream cannot
   *  parse (new thinking params, null tool schemas, foreign thinking history).
   *  Default on; every action is recorded in the request log. */
  compat_shim: boolean;
  /** Outbound credential detection: report a credential an agent is about to
   *  send, in the request log. "alert" (the default) records a finding; "off"
   *  runs no pass at all. It never blocks a request, and with request logging
   *  off there is no row to record one in. */
  dlp_mode: string;
  /** Request-log retention in days (gateway prunes older rows every 6h) */
  log_retention_days: number;
  /** Per-body capture cap in bytes; 0 stores every byte. */
  log_max_body_bytes: number;
  /** Abandon a stream whose first byte never arrives (0 = off) */
  stream_first_byte_secs: number;
  /** Abandon a stream that goes quiet mid-answer (0 = off) */
  stream_idle_secs: number;
  /** Cost alert (spec §4.1 P1): system notification when usage reaches the per-period limit */
  cost_alert: boolean;
  /** Preferred display currency for costs (ISO code; converted via the Hub rates) */
  preferred_currency: string;
  /** Auto-check for app updates at startup (silent; notification only) */
  auto_check_update: boolean;
  /** Version the user dismissed in the update banner (a newer one shows again) */
  dismissed_update: string | null;
  /** Minutes east of UTC (UTC+8 → 480). Day boundaries — "today", the daily
      chart buckets, alert reset periods — follow the user's day, not UTC's;
      the frontend keeps this current at startup. */
  tz_offset_minutes: number;
  hub_logged_in: boolean;
  /** Hub catalog sync endpoint (protocol v0: static JSON) */
  hub_url: string;
  /** Models list sort choice, remembered across sessions: "<key>:<dir>".
      null = the default order (added first, then tag rank, then name). */
  shelf_sort: string | null;
  /** Which Models view was last chosen: "model" for the grouping by model, null
      (or anything this build does not know) for the provider table. Kept loose
      like `shelf_sort`; `parseView` in the shelf is what narrows it. */
  shelf_view: string | null;
  // ── Features panel (docs/request-logs-applications.md): all opt-in, all
  // default off. None reach the gateway's forward path. ──
  /** Alert when the month's spend slope projects past a provider's limit */
  feat_cost_forecast: boolean;
  /** Alert on error-rate / latency / traffic anomalies vs the 7-day baseline */
  feat_anomaly_alerts: boolean;
  /** Alert when an agent hits its own per-period budget */
  feat_agent_limit_alerts: boolean;
  /** Let agents query their own aggregate stats over MCP (`kiwano mcp`) */
  feat_mcp_self_query: boolean;
  /** Allow `kiwano rules apply` to write insights-derived rules to CLAUDE.md/AGENTS.md */
  feat_rule_injection: boolean;
  /** Add tuning suggestions (retry budgets, route health) to insights */
  feat_tuning_advice: boolean;
  /** Enable the cache-shaping offline experiment (`kiwano cache-experiment`) */
  feat_cache_experiment: boolean;
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
  /** Providers the gateway is refusing to route, with its reason. The gateway
   *  decides this, so the card reads it rather than working out its own. */
  blocked: { id: string; reason: string }[];
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
  /** Display form (e.g. `sk-liv…mnop`) — the backend never sends the key itself. */
  masked: string;
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
  /** Which check raised this: provider_limit | plan_window | cost_forecast |
      anomaly | agent_limit. The first two are formatted here from the numbers;
      the rest arrive with their text in `message`. */
  kind: string;
  /** Pre-built notification text for the feature alerts; "" for legacy kinds. */
  message: string;
}

export type StrategyKind = "single" | "failover" | "roundrobin" | "timewindow" | "quota";

/** Agent installation detection (phase 1: existence only — one login-shell probe) */
/** One prompt round trip through a provider (the Apps row's Test button). */
export interface PromptLatency {
  provider_id: string;
  /** The model the ping was sent with — it decides the number as much as the
      network does. */
  model: string;
  latency_ms: number;
  status: number;
  /** The upstream's own words when it refused; null on success. */
  error: string | null;
}

export interface AgentDetect {
  agent: AgentId;
  installed: boolean;
  /** Resolved CLI binary path; null for claude-desktop */
  path: string | null;
  /** True when the hit came from a directory the user declared, not from the
      walk itself — the agent the detector could not find on its own. */
  manual?: boolean;
}

/// What a directory check found: the file it picked, and the version that file
/// printed. The file is half the answer — the user is looking at a directory and
/// wondering which of its files Kiwano means.
export interface AgentDirHit {
  path: string;
  version: string;
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
  agent: AgentRef;
  strategy: StrategyKind;
  /** Strategy JSON payload (quota: {"limit","unit"}; null otherwise) */
  config: string | null;
  bindings: StrategyBinding[];
  /** The agent's own ceilings, one per window; empty when it has none */
  limits: AgentLimit[];
}

/** One window of an agent's spend ceiling. Not part of the strategy: it holds
    under all of them, `single` included. An agent holds several — a day's and a
    month's answer different questions, and being over either is being over. */
export interface AgentLimit {
  /** `day` | `weekly` | `monthly` | `yearly` | `all` */
  period: string;
  period_limit: number;
  /** `requests` (default), `wan_tokens`, or a 3-letter currency code */
  limit_unit: string | null;
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
  /** Thinking tokens, a **slice of** `output_tokens` — never added to it. */
  reasoning_tokens: number;
  /** The upstream reported no usage at all, so every token number on this row
   *  is a zero that means "unknown" rather than "none". */
  usage_missing: boolean;
  latency_ms: number | null;
  first_token_ms: number | null;
  /** Redacted header JSON (auth headers stripped at capture time) */
  request_headers: string | null;
  response_headers: string | null;
  request_size: number;
  response_size: number;
  truncated: boolean;
  /** Recorded in `cost_currency`, never converted — the export states the cost
   *  as it was logged. */
  cost?: number | null;
  cost_currency?: string | null;
  /** What the compat shim changed about the request, one line per action
   *  (null when nothing was touched). */
  request_notes: string | null;
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
  /** RFC3339 UTC instant, inclusive lower bound on `ts`. Build it with
   *  `localMidnightUtc` — a `.000Z` shape sorts past the `+00:00` rows Rust
   *  writes and drops the row sitting exactly on the bound. */
  from?: string;
  /** RFC3339 UTC instant, EXCLUSIVE upper bound — `localMidnightUtcAfter` of
   *  the last day you want to see. */
  to?: string;
}

/** What one export wrote, so the UI can say so and warn when the slice was
 *  larger than the row cap. */
export interface RequestLogExport {
  rows_written: number;
  truncated: boolean;
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
  listProviders(filter?: AgentRef | "all"): Promise<Provider[]>;
  addProvider(input: NewProviderInput): Promise<Provider>;
  /** Update a provider (empty api_key means keep the existing key) */
  updateProvider(id: string, input: NewProviderInput): Promise<Provider>;
  /** Delete a provider; if it is the primary for some agent, the next candidate is promoted automatically */
  deleteProvider(id: string): Promise<void>;
  /** Enable = make this Provider the current route of its bound agents */
  /** Park a provider (or put it back): out of every route, keeping the row and
      its key. The agents bound to it fall through to their next candidate. */
  setProviderEnabled(id: string, enabled: boolean): Promise<void>;
  testLatency(endpoint: string): Promise<number>;
  /** Send one prompt through a stored provider and time the round trip */
  testProviderLatency(id: string): Promise<PromptLatency>;
  /** Protocol-aware probe: GET the protocol's models route; 401/403 still
      proves the protocol route exists (works without a key) */
  /** Probe the endpoint. A blank key is fine — a 401 still proves the route
      exists — and `providerId` lets the edit dialog fall back to the stored
      credential when the form holds none (it never shows the key). */
  testEndpoint(
    protocol: Protocol,
    endpoint: string,
    apiKey?: string,
    providerId?: string,
  ): Promise<ProbeReport>;
  /** Live model-name list from a provider endpoint (the API key is required:
      cloud providers reject anonymous /models calls) */
  /** Live model list. Needs a key — a typed one, or `providerId`'s stored key
      when that provider already answers on this endpoint. */
  listModels(
    protocol: Protocol,
    endpoint: string,
    apiKey: string,
    providerId?: string,
  ): Promise<string[]>;
  listCatalog(): Promise<CatalogList>;
  /** The price mirror: every model the gateway can cost, with its own rates. */
  listModelPrices(): Promise<ModelPrice[]>;
  /** Open a vendor's site in the user's browser (http/https only). */
  openUrl(url: string): Promise<void>;
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
  /** Define a user-defined agent: a named route with its own placeholder key */
  addCustomAgent(
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent>;
  /** Rename one. Its id, route and key are untouched — only the label moves */
  updateCustomAgent(
    id: string,
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent>;
  /** Every directory the detector looks in for a built-in agent — what "we
      could not find it" is measured against, from the walk itself. */
  agentSearchDirs(agent: AgentId): Promise<string[]>;
  /** Check a directory for a built-in agent's command without storing
      anything — what the dialog runs when one is picked. */
  verifyAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit>;
  /** Declare where a built-in agent lives, for one the detector cannot find.
      Resolves with what that directory's executable turned out to be. */
  setAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit>;
  /** Drop the declaration; the agent goes back to what the detector finds. */
  clearAgentDir(agent: AgentId): Promise<void>;
  /** Delete a user-defined agent, its route and its key (usage history stays) */
  removeCustomAgent(id: string): Promise<void>;

  /** Agent takeover switch (generates/deletes the placeholder key; config rewriting included from P1 on) */
  setTakeover(agent: AgentId, enabled: boolean): Promise<void>;
  /** Import config from CC Switch (auto-detects the ~/.cc-switch data source) */
  importCcSwitch(): Promise<ImportReport>;
  /** Agent routing strategy table (only agents with bindings) */
  getAgentRoutes(): Promise<AgentRoute[]>;
  /** Update an agent's strategy type (config only needed for quota: {"limit","unit"}) */
  updateAgentStrategy(agent: AgentRef, strategy: StrategyKind, config?: string | null): Promise<void>;
  /** Replace one agent's spend ceilings (an empty list clears them) */
  setAgentLimits(agent: AgentRef, limits: AgentLimit[]): Promise<void>;
  /** Reorder candidates: provider_id order → priority 0..n */
  reorderAgentBindings(agent: AgentRef, providerIds: string[]): Promise<void>;
  /** Patch one binding's strategy parameters (roundrobin weight / timewindow local window) */
  updateAgentBinding(
    agent: AgentRef,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ): Promise<void>;
  /** Bind a provider to an agent as a new candidate (appended at the queue tail) */
  addAgentBinding(agent: AgentRef, providerId: string): Promise<void>;
  /** Remove one agent's binding of a provider (other agents keep theirs) */
  removeAgentBinding(agent: AgentRef, providerId: string): Promise<void>;
  /** Copy another agent's whole route (strategy + ordered candidates) onto this one, replacing what it had */
  applyAgentRoute(target: AgentRef, source: AgentRef): Promise<void>;
  /**
   * Export the config plan to the given path; returns the Provider count.
   * API keys are omitted unless `includeKeys` is set — that flag is for a local
   * backup that never leaves the machine, not for a file that gets shared.
   */
  /**
   * No screen calls either of these, and that is settled rather than pending.
   *
   * They are left declared because the commands behind them work and the CLI
   * covers the same capability (`kiwano config export|import`, straight through
   * `kiwano-core::share`); what the desktop app lacks is an entry point. The
   * Settings section that held one was removed on 2026-09-10 (80eb138) with the
   * reason that its actions "didn't match the roadmap for config sharing" — the
   * roadmap being the community sharing in spec §4.1 P1, which is a different
   * feature from a local file. Rebuilding the entry means designing that, not
   * restoring this.
   */
  exportConfig(path: string, includeKeys?: boolean): Promise<number>;
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
  /** Write every row the filter matches to `path` as CSV. Unpaged, so it can
   *  exceed the list's page size; `truncated` says the cap was hit.
   *  The captured request/response bodies ride along as two extra columns: there
   *  is no metadata-only export, because the page and the file want the same
   *  payload. */
  exportRequestLogs(
    path: string,
    filter?: RequestLogFilter,
  ): Promise<RequestLogExport>;
  /** Delete every request-log row (bodies cascade) */
  clearRequestLogs(): Promise<void>;
  /** The newest credential-watch finding the user has not acknowledged
   *  (dismissed or clicked in the banner); null = nothing to show */
  checkCredentialFinding(): Promise<RequestLogEntry | null>;
  /** Acknowledge a finding by log id; a finding with a higher id shows again */
  ackCredentialFinding(id: number): Promise<void>;
  /** Reveal the app's log directory in the OS file manager */
  openLogFolder(): Promise<void>;
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
