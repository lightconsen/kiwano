// Browser-dev data source — numbers match the design/index.html prototype verbatim.
// During integration src/api/client.ts switches to the Tauri invoke implementation and this file is retired.
import type { AgentDirHit, AgentId, AgentLimit, AgentRoute, ApiKeyEntry, AppSettings, Billing, CatalogEntry, CatalogList, ConfigShareReport, CurrencyMeta, DashboardData, DashboardWindow, FooterStats, GatewayStatus, HubSyncReport, ImportReport, KiwanoApi, ModelPrice, NewProviderInput, PlanLimits, PlanQuotaReport, PlanQuery, ProbeReport, Protocol, Provider, ProviderHealth, RequestLogDetail, RequestLogEntry, RequestLogExport, RequestLogFilter, RequestLogList, StrategyBinding, StrategyKind, UpdateInfo, UpdateProgress, UsageAlert, UsageSummary, CustomAgent, PromptLatency } from "./types";
import { AGENTS } from "./types";
// The same formatter the screens print with: a fixture that formats its own
// tokens is a fixture that can disagree with the page about how they read.
import { fmtTokens } from "../lib/format";

function protocolNote(protocol: NewProviderInput["protocol"]): string {
  return protocol === "openai" ? "OpenAI-compatible" : "Anthropic";
}

// endpoint_note suffix mirrors vm::endpoint_note ("OpenAI-compatible · +Anthropic")
function endpointNote(
  protocol: NewProviderInput["protocol"],
  endpoints?: { protocol: Protocol }[],
): string {
  const tags = (endpoints ?? []).map((e) =>
    `+${e.protocol === "openai" ? "OpenAI" : "Anthropic"}`,
  );
  return [protocolNote(protocol), ...tags].join(" · ");
}

// Mirrors the merged bundled catalog (protocol siblings folded in; runapi.co
// was dead and dropped in favor of the live runapi.host)
const CATALOG_TOTAL = 82;

// Endpoint identity: host+path, lowercased, scheme and trailing slashes
// stripped — mirrors vm::endpoint_key on the backend
function endpointKey(s: string): string {
  return s
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .replace(/\/+$/, "");
}

/** The mock's stand-in for `vm::stored_key_for`: the provider's key, but only
    for an endpoint that provider already answers on — the same guard the real one
    applies, so a probe aimed somewhere else gets no key. */
function storedKeyFor(providerId: string | undefined, endpoint: string): string | null {
  if (!providerId) return null;
  const p = providers.find((x) => x.id === providerId);
  if (!p) return null;
  const known = [p.endpoint, ...(p.endpoints ?? []).map((e) => e.endpoint)].map(endpointKey);
  return known.includes(endpointKey(endpoint)) ? "sk-stored" : null;
}

const providers: Provider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    logo_char: "D",
    catalog_id: "deepseek",
    currency: "CNY",
    model_default: "deepseek-chat (V3)",
    logo_color: "#4D6BFE",
    endpoint: "api.deepseek.com",
    endpoint_note: "OpenAI-compatible · +Anthropic",
    protocol: "openai",
    endpoints: [{ protocol: "anthropic", endpoint: "api.deepseek.com/anthropic" }],
    billing: "payg",
    limit_unit: "CNY",
    // What the user declared this provider charges — per million tokens, in the
    // currency its limit is in. Shown on a shelf-added row because the declared
    // rung outranks the catalog's, so the edit dialog is where a reader can see
    // and correct the figures a request is actually costed by.
    prices: {
      currency: "CNY",
      models: [
        { model_id: "deepseek-chat", input: "2", output: "8", cache_read: "0.2", cache_creation: "0" },
        { model_id: "deepseek-reasoner", input: "4", output: "16", cache_read: "0.4", cache_creation: "0" },
      ],
    },
    enabled: true,
    agents: ["claude", "codex"],
    serving_agents: [],
    is_current: true,
    agents_note: "2 agents",
    // Measured by its own requests (`vm::health_vm`'s first source): a provider
    // with traffic in the last day is never probed.
    health: { state: "ok", latency_ms: 1100, source: "traffic" },
    usage: {
      requests: 796,
      input_tokens: 5_400_000,
      cache_read_tokens: 4_300_000,
      cache_creation_tokens: 600_000,
      output_tokens: 800_000,
      cost: 28.6,
      cost_currency: "CNY",
      latency_ms: 1100,
      quota: { used: 28.6, limit: 50, unit: "CNY", resets_at: null },
      spark: null,
    },
  },
  {
    id: "kimi",
    name: "Kimi (Moonshot)",
    logo_char: "K",
    catalog_id: "kimi",
    currency: "CNY",
    logo_color: "#111111",
    logo_border: true,
    endpoint: "api.moonshot.cn",
    endpoint_note: "OpenAI-compatible",
    protocol: "openai",
    billing: "plan",
    plan_price: "¥49/mo",
    limit_unit: "requests",
    plan_query: { template: "kimi" },
    enabled: true,
    agents: ["claude"],
    serving_agents: [],
    is_current: false,
    agents_note: "Failover queue",
    health: { state: "ok", latency_ms: 287, source: "traffic" },
    usage: {
      requests: 295,
      input_tokens: 1_600_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 300_000,
      cost: 10.9,
      cost_currency: "CNY",
      latency_ms: 287,
      quota: { used: 295, limit: 460, unit: "requests", resets_at: "2026-09-30" },
      spark: null,
    },
  },
  {
    id: "glm",
    name: "GLM (Zhipu)",
    logo_char: "G",
    catalog_id: "zhipu-glm",
    currency: "CNY",
    logo_color: "#3859FF",
    endpoint: "open.bigmodel.cn",
    endpoint_note: "OpenAI-compatible · +Anthropic",
    protocol: "openai",
    endpoints: [{ protocol: "anthropic", endpoint: "open.bigmodel.cn/api/anthropic" }],
    billing: "payg",
    enabled: false,
    agents: [],
    serving_agents: [],
    is_current: false,
    health: { state: "off", latency_ms: null, note: "Disabled" },
    usage: {
      requests: 77,
      input_tokens: 0,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 0,
      cost: 6.7,
      cost_currency: "CNY",
      latency_ms: null,
      quota: null,
      spark: [11, 9, 10, 6, 8, 4, 5],
    },
  },
  {
    id: "ollama",
    name: "Ollama",
    logo_char: "O",
    currency: "CNY",
    logo_color: "#1c1c1e",
    logo_border: true,
    endpoint: "localhost:11434",
    endpoint_note: "qwen3:32b",
    protocol: "openai",
    billing: "unl",
    enabled: true,
    agents: ["opencode"],
    serving_agents: [],
    is_current: false,
    status_badge: "Local",
    agents_note: "1 agent",
    // A local server nothing is listening on: the probe asked and got nothing.
    health: { state: "error", latency_ms: null, source: "probe", checked_at: "2026-09-15T07:20:00Z" },
    usage: {
      requests: 31,
      input_tokens: 150_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 50_000,
      cost: null,
      latency_ms: null,
      quota: null,
      spark: null,
    },
  },
];

// ── The billing × limit × strategy matrix ───────────────────────────────────
//
// `UsageCellBody` branches on a provider's billing and on which limit it
// carries, and the rows above cover four corners of that: payg + a currency
// limit, plan + a quota, unlimited, and a parked one. This block is the rest of
// the table — one row per combination the cell can render, including the two
// shapes the gateway reports *refusing* to route (an over-spend and an
// over-ceiling). Each row is also bound to an agent whose strategy makes the
// columns around the cell vary: a failover standby, a weighted rotation, a
// night window, a quota ceiling.
//
// `pnpm dev` → Apps → All is the page they exist for. They are deliberately
// absent from the Dashboard fixtures, which are built from PROVIDER_MIX: a demo
// table whose totals move every time a cell gains a state is a worse demo, and
// those numbers are asserted against each other.
type MatrixSpec = {
  id: string;
  /** The combination, not a vendor — the Provider column is its own legend. */
  name: string;
  logo: string;
  color: string;
  billing: Billing;
  /** Payg: the unit its spending limit is counted in. */
  limit_unit?: string;
  plan_price?: string;
  plan_limits?: PlanLimits;
  plan_query?: PlanQuery;
  usage: UsageSummary | null;
  /** The gateway's own sentence, for a row it is refusing to route. */
  blocked?: string;
  /** What the Status column says. Absent = enabled with nothing measured yet,
      which is the blank cell (see `health` in the row builder below). */
  health?: ProviderHealth;
};

const MATRIX: MatrixSpec[] = [
  {
    id: "m-payg-cny",
    name: "PAYG · CNY limit",
    logo: "¥",
    color: "#4D6BFE",
    billing: "payg",
    limit_unit: "CNY",
    usage: {
      requests: 796,
      input_tokens: 5_400_000,
      cache_read_tokens: 4_300_000,
      cache_creation_tokens: 600_000,
      output_tokens: 800_000,
      cost: 28.6,
      cost_currency: "CNY",
      latency_ms: 1100,
      quota: { used: 28.6, limit: 50, unit: "CNY", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-req",
    name: "PAYG · request limit",
    logo: "R",
    color: "#7C3AED",
    billing: "payg",
    limit_unit: "requests",
    usage: {
      requests: 640,
      input_tokens: 2_100_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 420_000,
      cost: 42.5,
      cost_currency: "CNY",
      latency_ms: 860,
      quota: { used: 640, limit: 1000, unit: "requests", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-wan",
    name: "PAYG · 10k-token limit",
    logo: "T",
    color: "#0EA5E9",
    billing: "payg",
    limit_unit: "wan_tokens",
    usage: {
      requests: 210,
      input_tokens: 900_000,
      cache_read_tokens: 120_000,
      cache_creation_tokens: 300_000,
      output_tokens: 300_000,
      cost: 18.2,
      cost_currency: "CNY",
      latency_ms: 1220,
      quota: { used: 120, limit: 500, unit: "wan_tokens", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-payg-trend",
    name: "PAYG · no limit",
    logo: "~",
    color: "#16A34A",
    billing: "payg",
    usage: {
      requests: 148,
      input_tokens: 610_000,
      cache_read_tokens: 40_000,
      cache_creation_tokens: 0,
      output_tokens: 190_000,
      cost: 12.4,
      cost_currency: "CNY",
      latency_ms: 940,
      quota: null,
      spark: [4, 7, 3, 9, 6, 11, 8],
    },
  },
  {
    id: "m-payg-unpriced",
    name: "PAYG · unpriced",
    logo: "?",
    color: "#6B7280",
    billing: "payg",
    usage: {
      requests: 12,
      input_tokens: 44_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 9_800,
      cost: null,
      cost_currency: null,
      latency_ms: 1310,
      quota: null,
      // One day of use: the series that has no span to draw, and the reason the
      // sparkline draws a single sample as a dot.
      spark: [3],
    },
  },
  {
    id: "m-payg-over",
    name: "PAYG · over its limit",
    logo: "!",
    color: "#B91C1C",
    billing: "payg",
    limit_unit: "CNY",
    // `LimitState::describe`'s Spend arm with no reset window, which is what a
    // provider limited only by the form carries (`reset_period` is the CLI's and
    // an import's to set).
    blocked: "52.40 of 50.00 CNY this period",
    usage: {
      requests: 1_204,
      input_tokens: 8_100_000,
      cache_read_tokens: 5_600_000,
      cache_creation_tokens: 900_000,
      output_tokens: 1_100_000,
      cost: 52.4,
      cost_currency: "CNY",
      latency_ms: 1180,
      quota: { used: 52.4, limit: 50, unit: "CNY", resets_at: null },
      spark: null,
    },
  },
  {
    id: "m-plan-quota",
    name: "PLAN · plan quota",
    logo: "P",
    color: "#D97757",
    billing: "plan",
    plan_price: "¥49/mo",
    plan_query: { template: "kimi" },
    usage: {
      requests: 295,
      input_tokens: 1_600_000,
      cache_read_tokens: 300_000,
      cache_creation_tokens: 100_000,
      output_tokens: 300_000,
      cost: 10.9,
      cost_currency: "CNY",
      latency_ms: 287,
      quota: { used: 295, limit: 460, unit: "requests", resets_at: "2026-09-30" },
      spark: null,
    },
  },
  {
    id: "m-plan-ceiling",
    name: "PLAN · 50% / 90% ceiling",
    logo: "%",
    color: "#DB2777",
    billing: "plan",
    plan_price: "¥20/mo",
    // The report's five-hour utilization is 42.5%, so this ceiling is at 85% —
    // the amber ring. No usage summary: the cell renders from the live report
    // and the ceiling alone.
    plan_limits: { five_hour: 50, weekly: 90 },
    plan_query: { template: "zhipu" },
    usage: null,
    // Nothing has run through it, so the only thing that could have measured it
    // is the row's own Test — which is the case that button exists for.
    health: { state: "ok", latency_ms: 218, source: "test", checked_at: "2026-09-15T11:38:00Z" },
  },
  {
    id: "m-plan-over",
    name: "PLAN · ceiling reached",
    logo: "!",
    color: "#991B1B",
    billing: "plan",
    plan_price: "¥20/mo",
    // Below the reported 42.5%: the ring is full and the gateway refuses.
    // `LimitState::describe`'s PlanWindow arm, verbatim.
    plan_limits: { five_hour: 40, weekly: 100 },
    plan_query: { template: "minimax" },
    blocked: "five_hour window at 43% of a 40% ceiling",
    usage: null,
  },
  {
    id: "m-plan-none",
    name: "PLAN · no quota endpoint",
    logo: "∅",
    color: "#A16207",
    billing: "plan",
    plan_price: "¥99/mo",
    // A plan that publishes no way to read its usage and carries no ceiling:
    // the cell has no plan branch to take.
    usage: {
      requests: 12,
      input_tokens: 88_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 21_000,
      cost: 3.2,
      cost_currency: "CNY",
      latency_ms: 700,
      quota: null,
      spark: null,
    },
  },
  {
    id: "m-key-refused",
    name: "PAYG · key refused",
    logo: "×",
    color: "#9F1239",
    billing: "payg",
    usage: null,
    // Reachable, and unusable: the vendor answered and said no. The cell has to
    // read as the key rather than as silence — a 401 is not "nobody is home".
    health: {
      state: "error",
      latency_ms: 60,
      source: "test",
      checked_at: "2026-09-15T11:38:00Z",
      error: "invalid API key",
    },
  },
  {
    id: "m-unl",
    name: "UNL · local",
    logo: "∞",
    color: "#1C1C1E",
    billing: "unl",
    usage: {
      requests: 31,
      input_tokens: 150_000,
      cache_read_tokens: 0,
      cache_creation_tokens: 0,
      output_tokens: 50_000,
      cost: null,
      cost_currency: null,
      latency_ms: null,
      quota: null,
      spark: null,
    },
  },
  {
    id: "m-unl-empty",
    name: "UNL · no history yet",
    logo: "·",
    color: "#3F3F46",
    billing: "unl",
    // A provider nothing has run through: the cell renders nothing at all, which
    // is a state too (the ring and the limit line need no totals, this one has
    // neither).
    usage: null,
    // Nothing has run through it, so the prober is the only thing that can say
    // anything about it — which is the case the loop exists for.
    health: { state: "ok", latency_ms: 18, source: "probe", checked_at: "2026-09-15T07:20:00Z" },
  },
];

for (const m of MATRIX) {
  providers.push({
    id: m.id,
    name: m.name,
    logo_char: m.logo,
    logo_color: m.color,
    endpoint: `demo.local/${m.id}`,
    endpoint_note: protocolNote("openai"),
    protocol: "openai",
    // What its rates would be denominated in; the fixture prices everything in CNY.
    currency: "CNY",
    billing: m.billing,
    limit_unit: m.limit_unit,
    plan_price: m.plan_price,
    plan_limits: m.plan_limits,
    plan_query: m.plan_query,
    enabled: true,
    // Both of these are derived from the routes by `listProviders`; the static
    // pair here is what an unbound row keeps.
    agents: [],
    serving_agents: [],
    is_current: false,
    // `vm::health_vm`'s first rule, so the fixture cannot say something the
    // backend would not: a provider with latency in its own usage is measured by
    // its own requests, and only a row with nothing to measure carries whatever
    // the spec says (a probe verdict, or nothing at all).
    health:
      m.health ??
      (m.usage?.latency_ms != null
        ? { state: "ok", latency_ms: m.usage.latency_ms, source: "traffic" }
        : { state: "idle", latency_ms: null }),
    usage: m.usage,
  });
}

const catalog: CatalogEntry[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    logo_color: "#4D6BFE",
    tag: "official",
    website: "https://deepseek.com/",
    rating: 4.8,
    endpoint: "https://api.deepseek.com",
    protocol: "openai",
    desc: "off-peak ¥4.5 / ¥13.5 · 12.4k users",
    price_ref: {
      model_id: "deepseek-chat (V3)",
      display_name: "DeepSeek Chat (V3)",
      input: "0.27",
      output: "1.10",
      currency: "USD",
      // DeepSeek's published shape: the rates above are the peak ones, halved
      // outside Beijing business hours. Here so the shelf's tier chip, the
      // vendor clock in its tooltip and the "now" chip are all exercisable
      // offline.
      off_peak: { in: "0.135", out: "0.55", cache_read: "0.02", cache_creation: "0" },
      peak_hours: {
        tz_offset: 480,
        windows: [
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "09:00", end: "12:00" },
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "14:00", end: "18:00" },
        ],
      },
    },
    currency: "CNY",
    billing: "payg",
    added: true,
    models: ["deepseek-chat (V3)", "deepseek-reasoner (R1)"],
    // Official Anthropic-compatible endpoint (api.deepseek.com/anthropic)
    endpoints: [
      { protocol: "anthropic", endpoint: "https://api.deepseek.com/anthropic", models: ["deepseek-chat (V3)", "deepseek-reasoner (R1)"] },
    ],
  },
  {
    id: "kimi",
    name: "Kimi",
    logo_color: "#111111",
    tag: "official",
    rating: 4.6,
    endpoint: "https://api.moonshot.cn",
    protocol: "openai",
    desc: "¥49 /mo plan · 8.1k users",
    currency: "CNY",
    billing: "plan",
    added: false,
    models: ["kimi-k2-0905-preview", "moonshot-v1-128k"],
  },
  {
    id: "qwen",
    name: "Qwen",
    logo_color: "#615CED",
    tag: "free",
    rating: 4.5,
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    protocol: "openai",
    desc: "¥2 /M in · ¥6 /M out · 1M tokens for new users",
    price_ref: {
      model_id: "qwen-max",
      display_name: "Qwen Max",
      input: "1.60",
      output: "6.40",
      currency: "USD",
      // Qwen is Alibaba's, and Alibaba prices most of its models in length bands:
      // here so the shelf's tooltip and the dialog's per-model line are both
      // exercisable offline.
      long_context: { over: 128000, in: "4.80", out: "19.20", cache_read: "0.96", cache_creation: "0" },
    },
    currency: "USD",
    billing: "payg",
    added: false,
    models: ["qwen-max", "qwen-plus"],
  },
  {
    id: "groq",
    name: "Groq",
    logo_color: "#F55036",
    tag: "free",
    rating: 4.7,
    endpoint: "https://api.groq.com/openai/v1",
    protocol: "openai",
    desc: "Free tier · Llama · ultra-fast inference",
    currency: "USD",
    billing: "unl",
    added: false,
    models: ["llama-3.3-70b-versatile"],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    logo_color: "#6467F2",
    tag: "aggregate",
    rating: 4.3,
    endpoint: "https://openrouter.ai/api/v1",
    protocol: "openai",
    desc: "Upstream price · +5.5% · 500+ models, one key",
    price_ref: {
      model_id: "openai/gpt-5.2",
      display_name: "GPT-5.2",
      input: "1.75",
      output: "14",
      currency: "USD",
    },
    currency: "USD",
    billing: "payg",
    added: false,
    // Serves `deepseek-chat (V3)` without pricing it — the shape most real
    // groups have (two providers, one of them with a published price), and what
    // makes the grouped view's dash cells visible in `pnpm dev`.
    models: ["anthropic/claude-sonnet-4.6", "openai/gpt-5.2", "deepseek-chat (V3)"],
  },
  {
    id: "ollama",
    name: "Ollama",
    logo_color: "#1c1c1e",
    tag: "local",
    rating: 4.9,
    endpoint: "http://localhost:11434",
    protocol: "openai",
    desc: "Runs locally · Fully offline",
    currency: "USD",
    billing: "unl",
    // Mirrors the backend: added derives from Ollama existing in the provider list
    added: true,
    models: ["qwen3:32b", "llama3.3:70b"],
  },
  // Multi-protocol samples from the cc-switch port (anthropic / openai fingerprints)
  {
    id: "anthropic",
    name: "Anthropic",
    logo_color: "#D97757",
    tag: "official",
    rating: 4.9,
    endpoint: "https://api.anthropic.com",
    currency: "USD",
    // Charges both ways at this one address — an API and a Pro/Max subscription.
    // The catalog cannot pick for the user, so the add dialog asks.
    billing: "both",
    added: false,
    models: ["claude-sonnet-5", "claude-opus-5", "claude-haiku-4-5"],
    protocol: "anthropic",
    desc: "Official pricing",
    price_ref: {
      model_id: "claude-opus-5",
      display_name: "Claude Opus 5",
      input: "5",
      output: "25",
      currency: "USD",
    },
  },
  {
    id: "zhipu-glm",
    name: "Zhipu GLM",
    logo_color: "#3859FF",
    tag: "official",
    rating: 4.4,
    endpoint: "https://open.bigmodel.cn/api/coding/paas/v4",
    currency: "USD",
    billing: "plan",
    // One of the four catalog entries whose vendor can be asked how much of the
    // plan is spent with nothing but the provider's key — the mock carries it so
    // the add modal's ceiling fields are reachable in `pnpm dev`. The other
    // fourteen plan entries publish no such endpoint, and `kimi` below is one of
    // them: between the two, both sides of that gate are exercisable.
    plan_query: { template: "zhipu" },
    added: false,
    models: ["glm-5", "glm-5-air"],
    protocol: "openai",
    desc: "Official pricing · from ¥118 /mo",
    price_ref: {
      model_id: "glm-5",
      display_name: "GLM-5",
      input: "0.60",
      output: "2.20",
      currency: "USD",
    },
    // Merged from the former zhipu-glm-anthropic sibling entry
    endpoints: [
      { protocol: "anthropic", endpoint: "https://open.bigmodel.cn/api/anthropic", models: ["glm-5", "glm-5-air"] },
    ],
  },
  {
    id: "packycode",
    name: "PackyCode",
    logo_color: "#0F9D58",
    tag: "third",
    rating: 4.2,
    endpoint: "https://www.packyapi.ai/v1",
    // Priced but not listed — the shape 15 of the real catalog's priced entries
    // have (they price a model their own list spells differently), and what
    // makes the "by model" view's price range visible in `pnpm dev`: this rate
    // is deliberately not Qwen's.
    price_ref: {
      model_id: "qwen-max",
      display_name: "Qwen Max",
      input: "1.40",
      output: "5.60",
      currency: "USD",
    },
    currency: "USD",
    billing: "payg",
    added: false,
    models: [],
    protocol: "openai",
  },
];

/** The Dashboard's fixtures, computed from one table per window.
 *
 * In the backend every tile is a view of the `usage` rows over the same window
 * (`vm::build_dashboard`): the trend buckets them by day, the two breakdowns
 * group them by provider and by agent, and the headline counts the request log —
 * those rows plus the requests that never reached a provider. Hand-written tiles
 * drift apart from all of that (the numbers here used to disagree about their
 * own total, by provider and by agent and against the chart), and a mock that
 * disagrees with itself is a `pnpm dev` session reporting that a screen is fine
 * when it is not. So the day totals below are authored and everything else is
 * derived.
 *
 * What is *not* derived: the per-provider totals the Apps screen shows. Those
 * are a different window's fixture (`providers[].usage`) and the two are not
 * linked, so a provider's own card can still disagree with this page.
 */

/** One window's day totals: the trend's own numbers, and the source of the rest. */
type WindowDays = { date: string; requests: number; tokens: number }[];

/** How a window's requests and cost split across providers.
 *
 * `offPeakRatio` is what the same request costs at the row's other rates: 1 for
 * a vendor with no schedule, and below 1 for DeepSeek, whose rows carry one —
 * which is what puts a non-zero peak premium on this page.
 */
const PROVIDER_MIX = [
  { id: "deepseek", name: "DeepSeek", color: "#4D6BFE", share: 0.585, costPerRequest: 0.036, offPeakRatio: 0.892 },
  { id: "kimi", name: "Kimi", color: "#555555", share: 0.215, costPerRequest: 0.037, offPeakRatio: 1 },
  { id: "glm", name: "GLM", color: "#3859FF", share: 0.15, costPerRequest: 0.035, offPeakRatio: 1 },
  { id: "ollama", name: "Ollama", color: "#1c1c1e", share: 0.05, costPerRequest: 0, offPeakRatio: 1 },
];

/** Which agents drive the traffic, across every provider. Claude Code heads the
    list because it heads the real one: it is what most of this traffic is. */
const AGENT_MIX: { agent: AgentId; share: number }[] = [
  { agent: "claude", share: 0.77 },
  { agent: "codex", share: 0.2 },
  { agent: "opencode", share: 0.03 },
];

/** Split `total` by `shares` so the parts add back up to the whole. Largest
    remainder, because the point of this file is that the parts *do* add up. */
function split(total: number, shares: number[]): number[] {
  const exact = shares.map((s) => total * s);
  const out = exact.map(Math.floor);
  let rest = total - out.reduce((n, v) => n + v, 0);
  const byRemainder = exact
    .map((v, i) => ({ i, frac: v - Math.floor(v) }))
    .sort((a, b) => b.frac - a.frac);
  for (const { i } of byRemainder) {
    if (rest <= 0) break;
    out[i] += 1;
    rest -= 1;
  }
  return out;
}

const round2 = (n: number) => Math.round(n * 100) / 100;

function buildWindow(
  window: DashboardWindow,
  days: WindowDays,
  opts: { failures: number; deltaPct: number; latency: number; latencyDelta: number },
): DashboardData {
  const requests = days.reduce((n, d) => n + d.requests, 0);
  const tokens = days.reduce((n, d) => n + d.tokens, 0);
  // The headline is the request log: the rows that reached a provider plus the
  // ones that did not (a refused key, an unreachable upstream).
  const headlineRequests = requests + opts.failures;

  const perProvider = split(requests, PROVIDER_MIX.map((p) => p.share));
  const by_provider = PROVIDER_MIX.map((p, i) => {
    const cost = round2(perProvider[i] * p.costPerRequest);
    return {
      id: p.id,
      name: p.name,
      color: p.color,
      requests: perProvider[i],
      // The share is of the *headline*, which is the backend's divisor — so the
      // slices need not reach 100% (traffic with no provider is still traffic).
      pct: Math.round((perProvider[i] * 100) / Math.max(1, headlineRequests)),
      cost,
      cost_off_peak: round2(cost * p.offPeakRatio),
    };
  }).filter((p) => p.requests > 0);

  const cost = round2(by_provider.reduce((n, p) => n + p.cost, 0));
  const costOffPeak = round2(by_provider.reduce((n, p) => n + p.cost_off_peak, 0));
  // Costs are split in cents: the agent rows are the same money as the provider
  // rows, seen from the other side, and they have to add up to it.
  const splitCents = (amount: number) =>
    split(Math.round(amount * 100), AGENT_MIX.map((a) => a.share)).map((c) => c / 100);
  const perAgent = split(requests, AGENT_MIX.map((a) => a.share));
  const perAgentTokens = split(tokens, AGENT_MIX.map((a) => a.share));
  const perAgentCost = splitCents(cost);
  const perAgentOffPeak = splitCents(costOffPeak);
  const by_agent = AGENT_MIX.map((a, i) => ({
    agent: a.agent,
    // The registry's label where it has one; a user-defined agent's traffic is
    // not in this fixture, so the fallback is the id, as in the app.
    label: AGENTS.find((m) => m.id === a.agent)?.label ?? a.agent,
    requests: perAgent[i],
    tokens: fmtTokens(perAgentTokens[i]),
    cost: perAgentCost[i],
    cost_off_peak: perAgentOffPeak[i],
  })).filter((a) => a.requests > 0);

  return {
    window,
    requests: headlineRequests,
    requests_delta_pct: opts.deltaPct,
    // input + output is the trend's own token total, which is how the two are
    // read together on this page.
    input_tokens: Math.round(tokens * 0.85),
    output_tokens: tokens - Math.round(tokens * 0.85),
    cache_read_tokens: Math.round(tokens * 0.85 * 0.57),
    cost,
    cost_off_peak: costOffPeak,
    latency_ms: opts.latency,
    latency_delta_pct: opts.latencyDelta,
    trend: days,
    by_provider,
    by_agent,
    // Derived per request by `getDashboard`, like the backend's.
    filter_providers: [],
    filter_agents: [],
  };
}

const dashboards: Record<DashboardWindow, DashboardData> = {
  today: buildWindow(
    "today",
    [
      { date: "02:00", requests: 18, tokens: 120_000 },
      { date: "05:00", requests: 24, tokens: 150_000 },
      { date: "08:00", requests: 56, tokens: 360_000 },
      { date: "11:00", requests: 62, tokens: 400_000 },
      { date: "14:00", requests: 48, tokens: 310_000 },
      { date: "17:00", requests: 44, tokens: 260_000 },
      { date: "20:00", requests: 32, tokens: 200_000 },
    ],
    { failures: 7, deltaPct: 4, latency: 1100, latencyDelta: 3 },
  ),
  "7d": buildWindow(
    "7d",
    [
      { date: "09-01", requests: 118, tokens: 900_000 },
      { date: "09-02", requests: 132, tokens: 1_000_000 },
      { date: "09-03", requests: 190, tokens: 1_300_000 },
      { date: "09-04", requests: 176, tokens: 1_200_000 },
      { date: "09-05", requests: 228, tokens: 1_500_000 },
      { date: "09-06", requests: 186, tokens: 1_300_000 },
      { date: "09-07", requests: 254, tokens: 1_600_000 },
    ],
    { failures: 12, deltaPct: 12, latency: 1200, latencyDelta: 9 },
  ),
  "30d": buildWindow(
    "30d",
    [
      { date: "08-09", requests: 196, tokens: 1660000 },
      { date: "08-10", requests: 210, tokens: 1140000 },
      { date: "08-11", requests: 211, tokens: 1460000 },
      { date: "08-12", requests: 186, tokens: 1490000 },
      { date: "08-13", requests: 203, tokens: 1560000 },
      { date: "08-14", requests: 179, tokens: 1450000 },
      { date: "08-15", requests: 207, tokens: 910000 },
      { date: "08-16", requests: 150, tokens: 1070000 },
      { date: "08-17", requests: 202, tokens: 1270000 },
      { date: "08-18", requests: 234, tokens: 960000 },
      { date: "08-19", requests: 241, tokens: 1180000 },
      { date: "08-20", requests: 183, tokens: 920000 },
      { date: "08-21", requests: 180, tokens: 1050000 },
      { date: "08-22", requests: 231, tokens: 1430000 },
      { date: "08-23", requests: 178, tokens: 1670000 },
      { date: "08-24", requests: 151, tokens: 1440000 },
      { date: "08-25", requests: 187, tokens: 1280000 },
      { date: "08-26", requests: 188, tokens: 1170000 },
      { date: "08-27", requests: 192, tokens: 1670000 },
      { date: "08-28", requests: 235, tokens: 1660000 },
      { date: "08-29", requests: 168, tokens: 1210000 },
      { date: "08-30", requests: 245, tokens: 1360000 },
      { date: "08-31", requests: 227, tokens: 1040000 },
      { date: "09-01", requests: 189, tokens: 1390000 },
      { date: "09-02", requests: 152, tokens: 1050000 },
      { date: "09-03", requests: 178, tokens: 1530000 },
      { date: "09-04", requests: 227, tokens: 960000 },
      { date: "09-05", requests: 182, tokens: 1340000 },
      { date: "09-06", requests: 152, tokens: 1490000 },
      { date: "09-07", requests: 197, tokens: 1190000 },
    ],
    { failures: 40, deltaPct: 23, latency: 1300, latencyDelta: 5 },
  ),
  // One bar per week, which is what the backend does once the span outgrows a
  // bar per day: the demo's history is older than that limit on purpose.
  all: buildWindow(
    "all",
    [
      { date: "06-30", requests: 940, tokens: 6_900_000 },
      { date: "07-07", requests: 1120, tokens: 8_100_000 },
      { date: "07-14", requests: 980, tokens: 7_400_000 },
      { date: "07-21", requests: 1250, tokens: 9_300_000 },
      { date: "07-28", requests: 1180, tokens: 8_800_000 },
      { date: "08-04", requests: 1310, tokens: 9_700_000 },
      { date: "08-11", requests: 1240, tokens: 9_200_000 },
      { date: "08-18", requests: 1390, tokens: 10_400_000 },
      { date: "08-25", requests: 1280, tokens: 9_500_000 },
      { date: "09-01", requests: 1150, tokens: 8_600_000 },
    ],
    { failures: 96, deltaPct: 31, latency: 1250, latencyDelta: 7 },
  ),
};

const settings: AppSettings = {
  language: "system",
  theme: "dark",
  autostart: true,
  close_to_tray: true,
  gateway_listen: "127.0.0.1:8317",
  // `config_paths` mirrors `takeover::takeover_paths` for the host platform, so
  // the mock answers what the real backend would for each agent's files.
  takeovers: [
    { agent: "claude", label: "Claude Code", placeholder_key: "kw-ag-claude-a1b2", enabled: true, additive: false, protocols: ["anthropic"], config_paths: ["~/.claude/settings.json"] },
    { agent: "codex", label: "Codex", placeholder_key: "kw-ag-codex-c3d4", enabled: true, additive: false, protocols: ["openai"], config_paths: ["~/.codex/config.toml", "~/.codex/auth.json"] },
    { agent: "gemini", label: "Gemini CLI", placeholder_key: null, enabled: false, additive: false, protocols: ["gemini"], config_paths: ["~/.gemini/.env"] },
    { agent: "grokbuild", label: "Grok Build", placeholder_key: null, enabled: false, additive: false, protocols: ["openai"], config_paths: ["~/.grok/config.toml"] },
    { agent: "claude-desktop", label: "Claude Desktop", placeholder_key: null, enabled: false, additive: false, protocols: ["anthropic"], config_paths: [
      "~/Library/Application Support/Claude/claude_desktop_config.json",
      "~/Library/Application Support/Claude-3p/claude_desktop_config.json",
      "~/Library/Application Support/Claude-3p/configLibrary/kiwano.json",
      "~/Library/Application Support/Claude-3p/configLibrary/_meta.json",
    ] },
    { agent: "opencode", label: "OpenCode", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.config/opencode/opencode.json"] },
    { agent: "openclaw", label: "OpenClaw", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.openclaw/openclaw.json"] },
    { agent: "hermes", label: "Hermes", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.hermes/config.yaml"] },
    { agent: "pi", label: "Pi", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.pi/agent/models.json", "~/.pi/agent/settings.json"] },
    { agent: "workbuddy", label: "WorkBuddy", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.workbuddy/models.json"] },
    { agent: "codebuddy", label: "CodeBuddy Code", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.codebuddy/models.json"] },
    { agent: "kimi", label: "Kimi Code CLI", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.kimi/config.toml"] },
    { agent: "qwen", label: "Qwen Code", placeholder_key: null, enabled: false, additive: true, protocols: ["openai"], config_paths: ["~/.qwen/settings.json"] },
    { agent: "cline", label: "Cline", placeholder_key: null, enabled: false, additive: false, protocols: ["openai"], config_paths: ["~/.cline/data/settings/providers.json"] },
  ],
  // Two user-defined agents, so `pnpm dev` can show the whole feature: a route
  // with candidates, and one that is still empty (the state the tab's bind slot
  // exists for).
  custom_agents: [
    {
      id: "long-tasks-3f9a",
      label: "Long tasks",
      note: "batch work at off-peak rates",
      protocol: "openai",
      placeholder_key: "kw-ag-long-tasks-3f9a-b7e1",
    },
    {
      // Null on purpose: this is the row a pre-v24 database has, so `pnpm dev`
      // shows the "not said" state next to the one that was answered.
      id: "scratch-91cd",
      label: "Scratch",
      note: null,
      protocol: null,
      placeholder_key: "kw-ag-scratch-91cd-40aa",
    },
    // Three more, so the routing strategies the built-in agents do not exercise
    // (roundrobin / timewindow / quota) are reachable in `pnpm dev` — they are
    // what the matrix rows below are bound to.
    {
      id: "rotating-pool-4a1f",
      label: "Rotating pool",
      note: "two providers, weighted rotation",
      protocol: "openai",
      placeholder_key: "kw-ag-rotating-pool-4a1f-0d31",
    },
    {
      id: "nightly-batch-7c2e",
      label: "Nightly batch",
      note: "one by day, one inside the night window",
      protocol: "anthropic",
      placeholder_key: "kw-ag-nightly-batch-7c2e-5a90",
    },
    {
      id: "quota-guard-5b8d",
      label: "Quota guard",
      note: "stops routing once its ceiling is reached",
      protocol: "openai",
      placeholder_key: "kw-ag-quota-guard-5b8d-c41e",
    },
  ],
  auto_failover: true,
  request_logs: true,
  compat_shim: true,
  log_retention_days: 0,
  log_max_body_bytes: 0,
  stream_first_byte_secs: 120,
  stream_idle_secs: 120,
  cost_alert: true,
  preferred_currency: "CNY",
  auto_check_update: true,
  dismissed_update: null,
  tz_offset_minutes: 480,
  hub_logged_in: false,
  hub_url: "https://hub.kiwano.cc/catalog.json",
  shelf_sort: null,
  shelf_view: null,
  // Features panel: every flag defaults off in the backend; the dev fixture
  // turns two on so the alerts they shape are reachable in `pnpm dev`.
  feat_cost_forecast: true,
  feat_anomaly_alerts: true,
  feat_agent_limit_alerts: false,
  feat_mcp_self_query: false,
  feat_rule_injection: false,
  feat_tuning_advice: false,
  feat_cache_experiment: false,
};

let idSeq = 100;

/** Dev-only: flips after the first syncHub() so the conditional path is visible. */
let devHubSynced = false;

/** Dev-only dedup for the sample feature alerts (see checkUsageAlerts). */
const devAlerted = new Set<string>();

// Dev storage for rotating keys (spec §4.1 P1 multi-key rotation).
// Keeps the raw key the way the database does, and masks on the way out — the
// same contract the Rust side enforces, so the UI cannot come to depend on
// reading a real key here and break against the desktop build.
type DevApiKeyRow = Omit<ApiKeyEntry, "masked"> & { provider_id: string; key: string };
const devApiKeys: DevApiKeyRow[] = [];

/** Mirrors `mask_key` in src-tauri/src/vm.rs. */
function maskKey(key: string): string {
  const n = key.length;
  if (n > 12) return `${key.slice(0, 6)}…${key.slice(-4)}`;
  if (n > 4) return `…${key.slice(-4)}`;
  return "•".repeat(n);
}

const toApiKeyEntry = ({ provider_id: _p, key, ...rest }: DevApiKeyRow): ApiKeyEntry => ({
  ...rest,
  masked: maskKey(key),
});

async function delay(ms = 120) {
  return new Promise((r) => setTimeout(r, ms));
}

// ── Agent routing strategies (tech.md §4.7) ──

/** One candidate of a route. `weight` is a roundrobin share (see the matrix's
    rotating route); every other strategy reads it as 1. */
function bind(pid: string, priority: number, win?: [string, string]): StrategyBinding {
  const p = providers.find((x) => x.id === pid)!;
  return {
    provider_id: pid,
    provider_name: p.name,
    logo_char: p.logo_char,
    logo_color: p.logo_color,
    priority,
    weight: 1,
    win_start: win?.[0] ?? null,
    win_end: win?.[1] ?? null,
    enabled: p.enabled,
  };
}

const agentRoutes: AgentRoute[] = [
  {
    agent: "claude",
    strategy: "failover",
    config: null,
    // The matrix row rides the queue's tail, so the failover role is on a row
    // that also demonstrates a limit state.
    bindings: [bind("deepseek", 0), bind("kimi", 1), bind("m-payg-cny", 2)],
    limits: [],
  },
  {
    agent: "codex",
    strategy: "single",
    config: null,
    bindings: [bind("deepseek", 0)],
    limits: [],
  },
  {
    // A user-defined agent: a route like any other, over its own key.
    agent: "long-tasks-3f9a",
    strategy: "failover",
    config: null,
    bindings: [bind("deepseek", 0), bind("kimi", 1), bind("m-plan-quota", 2)],
    limits: [],
  },
  {
    // Kimi heads opencode (globally "In use") while standing by in claude's
    // queue — exercises the agent tabs' local vs global "In use" badge.
    agent: "opencode",
    strategy: "single",
    config: null,
    bindings: [bind("kimi", 0)],
    limits: [],
  },
  {
    // Overnight window on the standby exercises the timewindow wraparound path.
    agent: "grokbuild",
    strategy: "timewindow",
    config: null,
    bindings: [bind("deepseek", 0), bind("ollama", 1, ["22:00", "06:00"])],
    limits: [],
  },
  // The matrix's own routes: the three strategies no built-in agent runs.
  {
    // Every candidate serves, in proportion to its weight. Both rows carry an
    // "In use" badge here — which is what distinguishes roundrobin from the
    // failover queues above, where the standby is a note with no badge.
    agent: "rotating-pool-4a1f",
    strategy: "roundrobin",
    config: null,
    bindings: [{ ...bind("m-payg-req", 0), weight: 3 }, bind("m-payg-wan", 1)],
    limits: [],
  },
  {
    // The head serves whenever the window is not in force; the second candidate
    // owns 22:00–06:00, which wraps midnight.
    agent: "nightly-batch-7c2e",
    strategy: "timewindow",
    config: null,
    bindings: [bind("m-payg-trend", 0), bind("m-unl", 1, ["22:00", "06:00"])],
    limits: [],
  },
  {
    // A ceiling the head is over: the gateway refuses to route to it, and the
    // row reads dimmed with the reason in the Status column. The config's unit
    // is the strategy's own vocabulary (requests | tokens) — the currency
    // ceiling that blocked the row is the *provider's*, a different limit.
    // The tail is what "volumes are over" shows: the first backup reads as
    // "Fallback" — first in line, not in use (vm::build_provider_vms).
    agent: "quota-guard-5b8d",
    strategy: "quota",
    config: JSON.stringify({ limit: 500, unit: "requests" }),
    bindings: [bind("m-payg-over", 0), bind("m-payg-trend", 1)],
    limits: [],
  },
];

function strategyOf(agent: AgentId): AgentRoute {
  // The backend lazily creates a Single-strategy route on first takeover
  // (vm::set_agent_takeover); the mock does the same so agents enabled from
  // Settings without a pre-seeded route behave identically.
  let route = agentRoutes.find((r) => r.agent === agent);
  if (!route) {
    route = { agent, strategy: "single", config: null, bindings: [], limits: [] };
    agentRoutes.push(route);
  }
  return route;
}

// Mirror vm::build_provider_vms: the "In use" badge marks the provider(s) that
// would serve a request issued right now under each agent's strategy. Quota
// mirrors vm::quota_over_threshold too: over the ceiling, the head stops
// serving and the first backup reads as fallback, not in use — which backup
// (if any) actually takes a request is the gateway's breakers' runtime call.
// The mock fixtures carry window totals, not per-day ones, so the fixture's
// request count stands in for the day's in `quotaOver`.
function servingNow(): Set<string> {
  const out = new Set<string>();
  const now = new Date();
  const nowMin = now.getHours() * 60 + now.getMinutes();
  const inWin = (n: number, s: string, e: string) => {
    const [sh, sm] = s.split(":").map(Number);
    const [eh, em] = e.split(":").map(Number);
    const a = sh * 60 + sm;
    const b = eh * 60 + em;
    return a <= b ? n >= a && n <= b : n >= a || n <= b;
  };
  for (const r of routedRoutes()) {
    // Disabled bindings and parked providers both drop out, as they do in the
    // gateway's route table.
    const enabled = r.bindings.filter(
      (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
    );
    if (enabled.length === 0) continue;
    const head = enabled[0].provider_id;
    if (r.strategy === "roundrobin") {
      enabled.forEach((b) => out.add(`${r.agent}/${b.provider_id}`));
    } else if (r.strategy === "timewindow") {
      const hit = enabled.find(
        (b) => b.win_start && b.win_end && inWin(nowMin, b.win_start, b.win_end),
      );
      out.add(`${r.agent}/${hit?.provider_id ?? head}`);
    } else if (r.strategy === "quota" && quotaOver(r)) {
      // over the ceiling: nothing serves (the backup is first in line, below)
    } else {
      // single / failover / quota-under: the head (breaker state is gateway runtime)
      out.add(`${r.agent}/${head}`);
    }
  }
  return out;
}

/** The quota strategy's configured first backup, while the head is over its
    threshold — vm::build_provider_vms's `fallback` map, mirrored. */
function quotaFallbackNow(): Set<string> {
  const out = new Set<string>();
  for (const r of routedRoutes()) {
    if (r.strategy !== "quota") continue;
    const enabled = r.bindings.filter(
      (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
    );
    if (enabled.length < 2 || !quotaOver(r)) continue;
    out.add(`${r.agent}/${enabled[1].provider_id}`);
  }
  return out;
}

/** Whether a quota route's head has consumed its config's limit — the mock's
    stand-in for `vm::quota_over_threshold` (which reads the same-day totals). */
function quotaOver(r: AgentRoute): boolean {
  const cfg = r.config ? (JSON.parse(r.config) as { limit: number; unit?: string }) : null;
  if (!cfg || !Number.isFinite(cfg.limit)) return false;
  const head = r.bindings.find(
    (b) => b.enabled && providers.find((p) => p.id === b.provider_id)?.enabled !== false,
  );
  if (!head) return false;
  const p = providers.find((x) => x.id === head.provider_id);
  const consumed =
    cfg.unit === "tokens"
      ? ((p?.usage?.input_tokens ?? 0) + (p?.usage?.output_tokens ?? 0))
      : (p?.usage?.requests ?? 0);
  return consumed >= cfg.limit;
}

/** Recompute every provider's serving/fallback flags from the current routes —
    what `listProviders` (and the mutation paths that re-derive them) show.
    Agents come from the routes (`agentsOf`), not the fixtures' static arrays,
    which drift from `agentRoutes` by design. */
function applyServingFlags() {
  const serving = servingNow();
  const fallback = quotaFallbackNow();
  for (const p of providers) {
    const agents = agentsOf(p.id);
    p.serving_agents = agents.filter((a) => serving.has(`${a}/${p.id}`)) as Provider["serving_agents"];
    p.is_current = p.serving_agents.length > 0;
    const fb = agents.filter((a) => fallback.has(`${a}/${p.id}`));
    if (fb.length > 0) p.fallback_agents = fb;
    else delete p.fallback_agents;
  }
}

// vm::build_provider_vms derives ProviderVm.agents from the bindings; the mock
// fixtures' static arrays drift from agentRoutes, so derive them the same way.
/** Routes whose agent currently points at the gateway — the mock's stand-in
    for the real VM's live-file check. A route the store kept for an agent that
    has been handed its own config back is a plan, not traffic, so it must not
    put the provider in that agent's column or under "In use". */
function routedRoutes(): AgentRoute[] {
  return agentRoutes.filter(
    (r) =>
      // A user-defined agent routes as long as it exists: it has no config
      // file for the takeover list to be reporting on (vm::live_bound_agents).
      settings.custom_agents.some((a) => a.id === r.agent) ||
      settings.takeovers.find((t) => t.agent === r.agent)?.enabled,
  );
}

function agentsOf(pid: string): string[] {
  return routedRoutes()
    .filter((r) => r.bindings.some((b) => b.provider_id === pid))
    .map((r) => r.agent);
}

// Mirror vm::build_provider_vms's failover-queue classification: a non-head
// candidate is a queue member ("Failover queue" note — no badge; standby
// badges read as a contradiction next to "In use") unless its strategy
// rotates through everyone (roundrobin) or it serves its own time window.
function standbyFlags(): { backups: Set<string> } {
  const backups = new Set<string>();
  for (const r of routedRoutes()) {
    const head = r.bindings.filter((b) => b.enabled)[0]?.provider_id;
    if (!head) continue;
    for (const b of r.bindings) {
      if (b.provider_id === head) continue;
      const standby =
        r.strategy === "roundrobin"
          ? false
          : r.strategy === "timewindow"
            ? !(b.win_start && b.win_end)
            : true;
      if (standby) backups.add(b.provider_id);
    }
  }
  return { backups };
}

// ── Request logs (data-plane audit trail; fixtures mirror gateway capture) ──

const requestLogs: RequestLogDetail[] = [
  {
    id: 3,
    ts: "2026-09-09T21:04:11Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "claude-sonnet-4-5",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: "user_acct__session_9f3a",
    is_streaming: false,
    input_tokens: 2095,
    output_tokens: 503,
    cache_read_tokens: 0,
    cache_creation_tokens: 2095,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 1180,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json","x-kw-session":"s-1"}',
    response_headers: '{"content-type":"application/json"}',
    request_size: 214,
    response_size: 331,
    truncated: false,
    // The shim touched this one: history replayed against DeepSeek carried
    // thinking blocks the upstream refuses (cc-switch discussion #3216).
    request_notes:
      "thinking: removed unsupported type \"adaptive\"\nassistant history: removed 2 thinking/redacted_thinking block(s)",
    request_body: '{"model":"claude-sonnet-4-5","stream":false,"messages":[{"role":"user","content":"Refactor the retry loop in forward.rs"}]}',
    response_body: '{"id":"msg_1","type":"message","role":"assistant","model":"claude-sonnet-4-5","content":[{"type":"text","text":"Done — see the diff."}],"usage":{"input_tokens":2095,"output_tokens":503}}',
  },
  {
    id: 2,
    ts: "2026-09-09T20:58:37Z",
    method: "POST",
    path: "/v1/messages",
    query: null,
    agent: "claude",
    attribution: "key",
    provider_id: "deepseek",
    model: "claude-sonnet-4-5",
    status_code: 200,
    error_kind: null,
    error_message: null,
    session_id: "user_acct__session_9f3a",
    is_streaming: true,
    input_tokens: 25,
    output_tokens: 171,
    cache_read_tokens: 11,
    cache_creation_tokens: 3,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: 940,
    first_token_ms: 210,
    request_headers: '{"content-type":"application/json"}',
    response_headers: '{"content-type":"text/event-stream"}',
    request_size: 96,
    response_size: 1240,
    truncated: false,
    request_notes: null,
    request_body: '{"model":"claude-sonnet-4-5","stream":true,"messages":[{"role":"user","content":"hello"}]}',
    response_body: 'event: message_start\ndata: {"type":"message_start"}\n\nevent: content_block_delta\ndata: {"type":"content_block_delta","delta":{"type":"text_delta","text":"hello"}}\n\nevent: message_stop\ndata: {"type":"message_stop"}\n\n',
  },
  {
    id: 1,
    ts: "2026-09-09T20:51:02Z",
    method: "POST",
    path: "/v1/chat/completions",
    query: null,
    agent: "codex",
    attribution: "key",
    provider_id: "deepseek",
    model: null,
    status_code: 502,
    error_kind: "protocol_mismatch",
    error_message: "kiwano-gateway: provider `deepseek` speaks openai; anthropic inbound conversion not implemented for this direction",
    session_id: null,
    is_streaming: false,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    cache_creation_tokens: 0,
    reasoning_tokens: 0,
    usage_missing: false,
    latency_ms: null,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json"}',
    response_headers: null,
    request_size: 74,
    response_size: 0,
    truncated: false,
    request_notes: null,
    request_body: '{"model":"gpt-5.2","messages":[]}',
    response_body: null,
  },
];

/** The status/agent/provider/date predicate shared by the mock's list and
 *  export, so the two cannot disagree — same reason the real store has one
 *  WHERE for both. Date bounds compare as strings, exactly as SQLite does on
 *  the RFC3339 column. */
function matchingLogs(filter?: RequestLogFilter): RequestLogEntry[] {
  return requestLogs.filter(
    (r) =>
      (!filter?.agent || r.agent === filter.agent) &&
      (!filter?.provider_id || r.provider_id === filter.provider_id) &&
      (!filter?.status ||
        (filter.status === "error" && r.status_code >= 400) ||
        (filter.status === "ok" && r.status_code < 400)) &&
      (!filter?.from || r.ts >= filter.from) &&
      (!filter?.to || r.ts < filter.to),
  );
}

/// Which agents the dev fixture reports as *not* installed. Deliberately not
/// all of them: the "Kiwano cannot find this one" menu only has anything to
/// show when something is missing, and these match the versions fixture below.
const NOT_INSTALLED_AGENTS: AgentId[] = ["gemini", "codebuddy", "kimi", "qwen"];

/// Directories the dev user has declared, keyed by agent id — the in-memory
/// stand-in for `Store::manual_agent_dirs`.
const declaredDirs: Partial<Record<AgentId, string>> = {};

/// The name an agent's CLI installs as; a couple of registry ids differ.
const binaryFor = (id: AgentId): string =>
  id === "grokbuild" ? "grok" : id === "claude-desktop" ? "claude" : id;

/// What a declaration reports back. The real backend runs the executable and
/// returns what it printed; the fixture has no filesystem to run, so every
/// declared directory answers with the same plausible version.
const DEV_DECLARED_VERSION = "1.2.3";

export const devApi: KiwanoApi = {
  async getGatewayStatus(): Promise<GatewayStatus> {
    await delay();
    // The rows the matrix marks as over a limit. The gateway is what decides
    // this, and both the dimmed Usage cell and the Status column come from here
    // — a mock that always answers `[]` leaves the state unreachable in `pnpm
    // dev`, which is the only place it can be looked at.
    return {
      running: true,
      port: 8317,
      blocked: MATRIX.filter((m) => m.blocked).map((m) => ({ id: m.id, reason: m.blocked! })),
    };
  },

  async listProviders(filter: AgentId | "all" = "all"): Promise<Provider[]> {
    await delay();
    const serving = servingNow();
    const fallback = quotaFallbackNow();
    const { backups } = standbyFlags();
    return providers
      .filter((p) => filter === "all" || p.agents.includes(filter))
      .map((p) => {
        const agents = agentsOf(p.id);
        return {
          ...p,
          agents: agents as Provider["agents"],
          serving_agents: agents.filter((a) => serving.has(`${a}/${p.id}`)) as Provider["serving_agents"],
          is_current: agents.some((a) => serving.has(`${a}/${p.id}`)),
          // First in line behind an over-threshold primary — a separate claim
          // from "serving", so it gets its own slot on the wire.
          ...(agents.some((a) => fallback.has(`${a}/${p.id}`))
            ? { fallback_agents: agents.filter((a) => fallback.has(`${a}/${p.id}`)) }
            : {}),
          agents_note: backups.has(p.id)
            ? "Failover queue"
            : agents.length
              ? `${agents.length} agent(s)`
              : undefined,
        };
      });
  },

  async addProvider(input: NewProviderInput): Promise<Provider> {
    await delay();
    const endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    const p: Provider = {
      id: input.name.toLowerCase().replace(/\s+/g, "-") + "-" + ++idSeq,
      name: input.name,
      // Which catalog entry it came from: `vm::add_provider` stores it, and the
      // edit dialog reaches the entry through it for the endpoints, the currency
      // and the quota query the stored row may be missing.
      catalog_id: input.catalog_id ?? null,
      // What the entry bills in, or what the user declared — the fixture has no
      // catalog to ask, so a hand-added provider takes the demo's own currency.
      currency: input.prices?.currency ?? "CNY",
      logo_char: input.name.charAt(0).toUpperCase(),
      logo_color: "#555555",
      endpoint: input.endpoint.replace(/^https?:\/\//, ""),
      endpoint_note: endpointNote(input.protocol, endpoints),
      protocol: input.protocol,
      endpoints,
      billing: input.billing,
      limit_unit: input.billing_config.limit_unit,
      enabled: true,
      agents: input.agents ?? [],
      // Optimistic, mirrors vm::add_provider; listProviders recomputes
      serving_agents: [],
      is_current: (input.agents ?? []).length > 0,
      agents_note: (input.agents ?? []).length
        ? `${input.agents!.length} agent(s)`
        : undefined,
      health: { state: "idle", latency_ms: null },
      usage: null,
      advanced: input.advanced,
      model_default: input.model_default.trim() || null,
      plan_query: input.plan_query ?? null,
      plan_limits: input.billing_config.plan_limits ?? null,
      // What the user declared this provider charges; absent = none declared,
      // which is how a provider priced by the Hub's table reads.
      prices: input.prices ?? null,
    };
    providers.unshift(p);
    return p;
  },

  async updateProvider(id: string, input: NewProviderInput): Promise<Provider> {
    await delay();
    const t = providers.find((p) => p.id === id);
    if (!t) throw new Error(`provider ${id} not found`);
    t.name = input.name.trim();
    t.model_default = input.model_default.trim() || null;
    t.endpoint = input.endpoint.replace(/^https?:\/\//, "");
    t.protocol = input.protocol;
    t.endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    t.endpoint_note = endpointNote(input.protocol, t.endpoints);
    t.billing = input.billing;
    // Absent `agents` keeps the existing bindings (mirrors vm::update_provider):
    // which providers serve an agent is edited on the Apps screen, not here.
    if (input.agents !== undefined) t.agents = [...input.agents];
    t.serving_agents = t.agents.filter((a) => servingNow().has(`${a}/${t.id}`));
    t.is_current = t.serving_agents.length > 0;
    const fb = t.agents.filter((a) => quotaFallbackNow().has(`${a}/${t.id}`));
    if (fb.length > 0) (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents = fb;
    else delete (t as { fallback_agents?: Provider["fallback_agents"] }).fallback_agents;
    t.agents_note = t.agents.length ? `${t.agents.length} agent(s)` : undefined;
    // Absent `advanced` keeps existing values (mirrors vm::update_provider).
    if (input.advanced !== undefined) t.advanced = input.advanced;
    if (input.plan_query !== undefined) t.plan_query = input.plan_query;
    t.limit_unit = input.billing_config.limit_unit;
    t.plan_limits = input.billing_config.plan_limits ?? null;
    // Absent `prices` keeps what is stored (mirrors vm::update_provider): the
    // form omits the field whenever its Prices section is not on screen.
    if (input.prices !== undefined) {
      t.prices = input.prices.models.length ? input.prices : null;
    }
    return t;
  },

  async deleteProvider(id: string): Promise<void> {
    await delay();
    const i = providers.findIndex((p) => p.id === id);
    if (i >= 0) providers.splice(i, 1);
    // vm::delete_provider in the mock's smaller world: the provider's bindings
    // go with it, and every agent whose head it was promotes the next
    // candidate, renumbering what is left. Without this the route kept a
    // pointer to a row that is gone — which `pnpm dev` renders as a table with
    // a row missing rather than as the promotion the app actually performs.
    for (const r of agentRoutes) {
      if (!r.bindings.some((b) => b.provider_id === id)) continue;
      const wasHead = r.bindings[0].provider_id === id;
      r.bindings = r.bindings.filter((b) => b.provider_id !== id);
      if (wasHead) r.bindings = r.bindings.map((b, i) => ({ ...b, priority: i }));
    }
  },

  async setProviderEnabled(id: string, enabled: boolean): Promise<void> {
    await delay();
    // vm::set_provider_enabled: the row's own flag, no route touched. What
    // changes is whether it may serve — which `servingNow` reads, the same way
    // the gateway's route table reads `providers.enabled`.
    const target = providers.find((p) => p.id === id);
    if (!target) throw new Error(`provider not found: ${id}`);
    target.enabled = enabled;
    applyServingFlags();
  },

  async testProviderLatency(id: string): Promise<PromptLatency> {
    // A real round trip cannot happen in a browser mock, so this answers the
    // shape the app reads: a model, a latency, and a failure when the provider
    // is one the fixture says is broken.
    await delay(600);
    const p = providers.find((x) => x.id === id);
    if (!p) throw new Error(`provider not found: ${id}`);
    const model = p.model_default ?? "ping-model";
    const checked_at = new Date().toISOString();
    // …and it records the verdict, because that is what the backend does
    // (`vm::test_provider_latency` writes it as the provider's health) and it is
    // what makes the Status cell catch up after a click. A mock that only moved
    // the number on the button would send `pnpm dev` looking at a cell that
    // stays stale — the disagreement this file's header warns about.
    if (p.enabled === false) {
      // Answered and refused: a 401 is the vendor talking, so reachability is
      // not what failed — the key is.
      p.health = {
        state: "error",
        latency_ms: 180,
        source: "test",
        checked_at,
        error: "invalid API key",
      };
      return { provider_id: id, model, latency_ms: 180, status: 401, error: "invalid API key" };
    }
    const base = 220 + (id.length % 7) * 130;
    p.health = { state: "ok", latency_ms: base, source: "test", checked_at };
    return { provider_id: id, model, latency_ms: base, status: 200, error: null };
  },

  async testLatency(endpoint: string): Promise<number> {
    await delay(400);
    const table: Record<string, number> = {
      "api.deepseek.com": 312,
      "api.moonshot.cn": 287,
      "open.bigmodel.cn": 305,
      "localhost:11434": 25,
    };
    return table[endpoint.replace(/^https?:\/\//, "")] ?? 260;
  },

  // Dev probe: pretend the protocol's models route answered (ok), unless the
  // endpoint is empty/decorated with "invalid" — exercises the chip styling
  async testEndpoint(
    _protocol: Protocol,
    endpoint: string,
    apiKey?: string,
    providerId?: string,
  ): Promise<ProbeReport> {
    await delay(500);
    const latency_ms = 200 + Math.floor(Math.random() * 300);
    if (!endpoint.trim() || endpoint.includes("invalid")) {
      return { verdict: "unreachable", status: null, latency_ms, detail: "connection failed (dev)" };
    }
    // Mirrors the backend: a blank key falls back to the stored one, but only
    // for an endpoint that provider already answers on.
    if (!apiKey?.trim() && !storedKeyFor(providerId, endpoint)) {
      return { verdict: "auth", status: 401, latency_ms, detail: "no key for this endpoint (dev)" };
    }
    return { verdict: "ok", status: 200, latency_ms, detail: "12 models listed (dev)" };
  },

  // Dev model list: resolves through the catalog entry matching the endpoint
  // (primary or per-protocol), falling back to a canned OpenAI-style list
  async listModels(
    _protocol: Protocol,
    endpoint: string,
    apiKey: string,
    providerId?: string,
  ): Promise<string[]> {
    await delay(600);
    if (!apiKey.trim() && !storedKeyFor(providerId, endpoint)) {
      throw new Error("no stored key covers this endpoint — enter one to fetch its models");
    }
    // Normalized, like the real matching: the form holds `api.moonshot.cn` where
    // the entry publishes `https://api.moonshot.cn`, and comparing the raw
    // strings missed every entry — so Fetch answered with the generic list even
    // for a provider the catalog knows.
    const ep = endpointKey(endpoint);
    const hit = catalog.find(
      (e) =>
        endpointKey(e.endpoint) === ep ||
        (e.endpoints ?? []).some((x) => endpointKey(x.endpoint) === ep),
    );
    if (hit) {
      return Array.from(new Set([...hit.models, ...(hit.endpoints ?? []).flatMap((x) => x.models ?? [])]));
    }
    return ["gpt-5.2", "gpt-5.2-mini", "o4-mini", "text-embedding-3-large"];
  },

  async listModelPrices(): Promise<ModelPrice[]> {
    await delay();
    // The mock's mirror: one row per catalog `price_ref`, plus the second model
    // DeepSeek prices by time of day — the case the dialog exists to show, and
    // one the catalog cannot (it names a single representative model).
    const rows: ModelPrice[] = catalog
      .filter((e) => e.price_ref)
      .map((e) => ({
        provider_id: e.id,
        model_id: e.price_ref!.model_id,
        display_name: e.price_ref!.display_name,
        input: e.price_ref!.input,
        output: e.price_ref!.output,
        cache_read: "0.03",
        cache_creation: "0",
        currency: e.price_ref!.currency,
        ...(e.price_ref!.off_peak ? { off_peak: e.price_ref!.off_peak } : {}),
        ...(e.price_ref!.peak_hours ? { peak_hours: e.price_ref!.peak_hours } : {}),
        ...(e.price_ref!.long_context ? { long_context: e.price_ref!.long_context } : {}),
      }));
    rows.push({
      provider_id: "deepseek",
      model_id: "deepseek-reasoner (R1)",
      display_name: "DeepSeek Reasoner (R1)",
      input: "0.55",
      output: "2.19",
      cache_read: "0.14",
      cache_creation: "0",
      currency: "USD",
      off_peak: { in: "0.28", out: "1.10", cache_read: "0.07", cache_creation: "0" },
      peak_hours: {
        tz_offset: 480,
        windows: [
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "09:00", end: "12:00" },
          { days: ["mon", "tue", "wed", "thu", "fri"], start: "14:00", end: "18:00" },
        ],
      },
    });
    return rows;
  },

  async openUrl(url: string): Promise<void> {
    // Browser dev: no OS opener. Opening a tab is the closest thing, and the
    // real command is fenced to http(s) anyway.
    window.open(url, "_blank", "noopener");
  },

  async listCatalog(): Promise<CatalogList> {
    await delay();
    // Mirror the backend: `added` derives from the provider list at read
    // time, matching the primary endpoint OR any additional per-protocol
    // endpoint on either side (merged catalog entries stay one row)
    const keys = new Set(
      providers.flatMap((p) => [
        endpointKey(p.endpoint),
        ...(p.endpoints ?? []).map((e) => endpointKey(e.endpoint)),
      ]),
    );
    return {
      total: CATALOG_TOTAL,
      entries: catalog.map((e) => ({
        ...e,
        added:
          keys.has(endpointKey(e.endpoint)) ||
          (e.endpoints ?? []).some((x) => keys.has(endpointKey(x.endpoint))),
      })),
    };
  },

  async getDashboard(window: DashboardWindow, providerId?: string, agentId?: string): Promise<DashboardData> {
    await delay();
    const base = dashboards[window];
    // Filter options mirror the backend: who has traffic in the window,
    // independent of the active filter. Ids resolve through the provider
    // list; by_provider labels are short display names, so unmatched ones
    // fall back to the label itself as the id.
    const filter_providers = base.by_provider.map((p) => ({ id: p.id, label: p.name }));
    const filter_agents = base.by_agent.map((a) => ({ id: a.agent, label: a.label }));
    const data: DashboardData = { ...base, filter_providers, filter_agents };
    // The mock narrows the breakdowns only (headline stats stay fixture-wide).
    // By id, which is what the screen filters on: matching on the display name
    // was a lookup that quietly stopped working the moment the two lists spelled
    // a vendor differently.
    if (providerId) {
      data.by_provider = base.by_provider
        .filter((p) => p.id === providerId)
        .map((p) => ({ ...p, pct: 100 }));
    }
    if (agentId) {
      data.by_agent = base.by_agent.filter((a) => a.agent === agentId);
    }
    return data;
  },

  async getSettings(): Promise<AppSettings> {
    await delay();
    // Fresh copy every call: the caller mutates nothing, but React needs a new
    // reference to re-render after setTakeover mutated the fixture in place.
    return structuredClone(settings);
  },

  async updateSettings(patch: Partial<AppSettings>): Promise<AppSettings> {
    await delay();
    Object.assign(settings, patch);
    return settings;
  },

  async updateCustomAgent(
    id: string,
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const list = settings.custom_agents;
    const index = list.findIndex((a) => a.id === id);
    if (index < 0) throw new Error(`no such custom agent: ${id}`);
    // The label, the note and the protocol move; the id is what routes and keys
    // point at. All three are replaced rather than patched — `null` here means
    // "not said", which is a value the field can hold — so a caller sends the
    // ones it is not changing back unchanged, the same line the backend draws.
    const updated: CustomAgent = {
      ...list[index],
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
    };
    settings.custom_agents = [
      ...list.slice(0, index),
      updated,
      ...list.slice(index + 1),
    ];
    return structuredClone(updated);
  },

  async addCustomAgent(
    label: string,
    note?: string | null,
    protocol?: Protocol | null,
  ): Promise<CustomAgent> {
    await delay();
    const created: CustomAgent = {
      // The same shape vm::add_custom_agent derives, so a copied id reads the
      // same in either mode.
      id: `${label.trim().toLowerCase().replace(/[^a-z0-9]+/g, "-").replace(/^-|-$/g, "") || "custom"}-${Math.random().toString(16).slice(2, 8)}`,
      label: label.trim(),
      note: note?.trim() ? note.trim() : null,
      protocol: protocol ?? null,
      placeholder_key: "",
    };
    created.placeholder_key = `kw-ag-${created.id}-${Math.random().toString(16).slice(2, 6)}`;
    settings.custom_agents = [...settings.custom_agents, created];
    // A route with the same default as the backend's: single, no candidates yet.
    agentRoutes.push({
      agent: created.id,
      strategy: "single",
      config: null,
      bindings: [],
      limits: [],
    });
    return structuredClone(created);
  },

  async removeCustomAgent(id: string): Promise<void> {
    await delay();
    settings.custom_agents = settings.custom_agents.filter((a) => a.id !== id);
    // The route and the key go with it; usage rows are history and stay (the
    // mock has none for these, but the dashboard fixtures are keyed by agent
    // name, so a removed agent's traffic would survive here too).
    const i = agentRoutes.findIndex((r) => r.agent === id);
    if (i >= 0) agentRoutes.splice(i, 1);
  },

  async setTakeover(agent: AgentId, enabled: boolean): Promise<void> {
    await delay();
    const t = settings.takeovers.find((x) => x.agent === agent);
    if (!t) return;
    if (enabled) {
      const suffix = Math.random().toString(16).slice(2, 6);
      t.placeholder_key = `kw-ag-${agent}-${suffix}`;
      t.enabled = true;
    } else {
      // The route goes with the takeover (vm::set_agent_takeover): the agent
      // has its own config back, so it is nobody's candidate — and the
      // providers it named stay in the list, unbound.
      const i = agentRoutes.findIndex((r) => r.agent === agent);
      if (i >= 0) agentRoutes.splice(i, 1);
      t.placeholder_key = null;
      t.enabled = false;
    }
  },

  async importCcSwitch(): Promise<ImportReport> {
    await delay(600);
    return {
      imported: 2,
      skipped: 1,
      detail: [
        "source: ~/.cc-switch/config.json (3 rows)",
        "import ccs-claude-default: DeepSeek (anthropic)",
        "import ccs-codex-default: Groq (openai)",
        "skip claude/claude-official: missing endpoint or key",
      ],
    };
  },

  async syncHub(): Promise<HubSyncReport> {
    await delay(500);
    // Mirror the real conditional sync: the first call downloads, later ones
    // report "unchanged" (manifest sha matched) so both UI branches are
    // reachable in dev.
    const unchanged = devHubSynced;
    devHubSynced = true;
    return {
      fetched: 42,
      synced_at: new Date().toISOString(),
      hub_url: settings.hub_url,
      unchanged,
      pricing_version: 1,
      pricing_unchanged: unchanged,
    };
  },

  async checkUsageAlerts(): Promise<UsageAlert[]> {
    await delay();
    // The two feature flags the fixture turns on each get their sample alert,
    // so the notification path is exercisable in `pnpm dev` — but firing them
    // on every 60s poll would make the dev session unusable, so they answer
    // once per page load, the way the backend's per-period dedup reads.
    const out: UsageAlert[] = [];
    if (settings.feat_cost_forecast && !devAlerted.has("forecast")) {
      devAlerted.add("forecast");
      out.push({
        provider_id: "deepseek",
        provider_name: "DeepSeek",
        used: 31.2,
        limit: 50,
        unit: "CNY",
        kind: "cost_forecast",
        message: "DeepSeek: on pace for 62 CNY this period — past the 50 limit",
      });
    }
    if (settings.feat_anomaly_alerts && !devAlerted.has("anomaly")) {
      devAlerted.add("anomaly");
      out.push({
        provider_id: "",
        provider_name: "",
        used: 0,
        limit: 0,
        unit: "",
        kind: "anomaly",
        message: "Error spike: 19 of the last hour's 23 requests failed (83% — 7-day baseline 2%)",
      });
    }
    return out;
  },

  async getPlanQuota(providerId: string, force?: boolean): Promise<PlanQuotaReport> {
    await delay(600);
    const p = providers.find((x) => x.id === providerId);
    const template = p?.plan_query?.template ?? "kimi";
    if (!p?.plan_query) {
      return {
        provider_id: providerId,
        template,
        success: false,
        error: "This provider has no plan query configured",
        note: null,
        tiers: [],
        queried_at: Date.now(),
        cached: false,
      };
    }
    return {
      provider_id: providerId,
      template,
      success: true,
      error: null,
      note: "Standard plan",
      tiers: [
        { name: "five_hour", utilization: 42.5, resets_at: "2026-09-10T18:00:00Z", used: null, limit: null, unit: null },
        { name: "monthly", utilization: 18.3, resets_at: "2026-09-30", used: 295, limit: 460, unit: "requests" },
      ],
      queried_at: Date.now(),
      cached: !force && Math.random() < 0.5,
    };
  },

  async getCurrencyMeta(): Promise<CurrencyMeta> {
    await delay();
    return {
      currencies: ["USD", "CNY"],
      exchange_rates: { USD: 1.0, CNY: 7.1 },
      preferred: settings.preferred_currency,
    };
  },

  async listApiKeys(providerId: string): Promise<ApiKeyEntry[]> {
    await delay();
    return devApiKeys.filter((k) => k.provider_id === providerId).map(toApiKeyEntry);
  },

  async addApiKey(providerId: string, apiKey: string, label?: string): Promise<ApiKeyEntry> {
    await delay();
    const row: DevApiKeyRow = {
      id: ++idSeq,
      provider_id: providerId,
      key: apiKey.trim(),
      label: label?.trim() || undefined,
      enabled: true,
      created_at: new Date().toISOString(),
    };
    devApiKeys.push(row);
    return toApiKeyEntry(row);
  },

  async deleteApiKey(id: number): Promise<void> {
    await delay();
    const i = devApiKeys.findIndex((k) => k.id === id);
    if (i >= 0) devApiKeys.splice(i, 1);
  },

  async getAgentRoutes(): Promise<AgentRoute[]> {
    await delay();
    // Mirror the backend: only agents with at least one binding get a route
    return agentRoutes
      .filter((r) => r.bindings.length > 0)
      .map((r) => ({ ...r, bindings: r.bindings.map((b) => ({ ...b })) }));
  },

  async setAgentLimits(agent: AgentId, limits: AgentLimit[]): Promise<void> {
    await delay();
    // A window of zero is the absence of that window rather than a ceiling of
    // nothing, mirroring vm::set_agent_limits — a fixture that stored it would
    // show a limit the gateway would not enforce.
    strategyOf(agent).limits = limits.filter((l) => l.period && l.period_limit > 0);
  },

  async updateAgentStrategy(agent: AgentId, strategy: StrategyKind, config?: string | null): Promise<void> {
    await delay();
    strategyOf(agent).strategy = strategy;
    strategyOf(agent).config = config ?? null;
    // Entering roundrobin: reseed the weights as an even split of 100
    // (remainder to the head of the queue), mirroring vm::set_agent_strategy
    if (strategy === "roundrobin") {
      const bs = strategyOf(agent).bindings;
      bs.forEach((b, i) => {
        b.weight = Math.max(1, Math.floor(100 / bs.length) + (i < 100 % bs.length ? 1 : 0));
      });
    }
  },

  async reorderAgentBindings(agent: AgentId, providerIds: string[]): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    route.bindings = providerIds
      .map((pid, i) => {
        const b = route.bindings.find((x) => x.provider_id === pid)!;
        return { ...b, priority: i };
      })
      .sort((a, b) => a.priority - b.priority);
  },

  async updateAgentBinding(
    agent: AgentId,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ): Promise<void> {
    await delay();
    const b = strategyOf(agent).bindings.find((x) => x.provider_id === providerId);
    if (!b) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    if (patch.weight != null) b.weight = Math.max(1, patch.weight);
    // Both bounds are set/cleared together: a half window would never match.
    if (patch.win_start !== undefined || patch.win_end !== undefined) {
      const s = patch.win_start || null;
      const e = patch.win_end || null;
      if (s && e) {
        b.win_start = s;
        b.win_end = e;
      } else {
        b.win_start = null;
        b.win_end = null;
      }
    }
  },

  async addAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    if (route.bindings.some((b) => b.provider_id === providerId)) return;
    route.bindings.push(bind(providerId, route.bindings.length));
    // Mirror the backend: a provider's agents derive from its bindings
    const p = providers.find((x) => x.id === providerId)!;
    if (!p.agents.includes(agent)) p.agents.push(agent);
  },

  async removeAgentBinding(agent: AgentId, providerId: string): Promise<void> {
    await delay();
    const route = strategyOf(agent);
    const i = route.bindings.findIndex((b) => b.provider_id === providerId);
    if (i < 0) throw new Error(`provider ${providerId} is not bound to ${agent}`);
    route.bindings.splice(i, 1);
    const p = providers.find((x) => x.id === providerId)!;
    p.agents = p.agents.filter((a) => a !== agent);
  },

  async applyAgentRoute(target: AgentId, source: AgentId): Promise<void> {
    await delay();
    if (target === source) throw new Error("cannot copy an agent's route onto itself");
    const src = agentRoutes.find((r) => r.agent === source);
    if (!src || src.bindings.length === 0) throw new Error(`${source} has no route to copy`);
    const existing = agentRoutes.find((r) => r.agent === target);
    const before = new Set(existing?.bindings.map((b) => b.provider_id) ?? []);
    const copy: AgentRoute = {
      agent: target,
      strategy: src.strategy,
      config: src.config,
      bindings: src.bindings.map((b) => ({ ...b })),
      // The ceilings are the agent's own, not part of the route being copied:
      // inheriting someone else's budget is not what "copy this route" means.
      limits: existing?.limits ?? [],
    };
    const i = agentRoutes.findIndex((r) => r.agent === target);
    if (i >= 0) agentRoutes[i] = copy;
    else agentRoutes.push(copy);
    // Mirror the backend: a provider's agents derive from its bindings
    for (const pid of before) {
      if (copy.bindings.some((b) => b.provider_id === pid)) continue;
      const p = providers.find((x) => x.id === pid);
      if (p) p.agents = p.agents.filter((a) => a !== target);
    }
    for (const b of copy.bindings) {
      const p = providers.find((x) => x.id === b.provider_id);
      if (p && !p.agents.includes(target)) p.agents.push(target);
    }
  },

  async exportConfig(_path: string): Promise<number> {
    await delay(300);
    return providers.length;
  },

  async importConfig(_path: string): Promise<ConfigShareReport> {
    await delay(400);
    return { providers_added: 1, providers_kept: 2, routes_applied: 1 };
  },

  async getFooterStats(): Promise<FooterStats> {
    await delay();
    return {
      today_requests: 1284,
      today_tokens: 1_900_000,
      hub_synced: true,
      version: "v0.1.2",
    };
  },

  async checkAppUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async getPendingUpdate(): Promise<UpdateInfo | null> {
    await delay();
    return null;
  },

  async downloadAndInstallAppUpdate(): Promise<void> {
    await delay();
  },

  async onUpdateProgress(_cb: (p: UpdateProgress) => void): Promise<() => void> {
    return () => {};
  },

  async detectAgents() {
    await delay(120);
    // Dev fixture: most agents installed, a few not — the missing ones are
    // what the "point Kiwano at it" menu exists for, and they match the
    // versions fixture below. The registry's own ids: `a.id` is an `AgentRef`
    // since agents can also be user-defined, and only built-ins are detectable.
    return AGENTS.map((a) => {
      const id = a.id as AgentId;
      const declared = declaredDirs[id];
      if (declared) {
        return {
          agent: id,
          installed: true,
          path: `${declared}/${binaryFor(id)}`,
          manual: true,
        };
      }
      const installed = !NOT_INSTALLED_AGENTS.includes(id);
      return {
        agent: id,
        installed,
        path: installed ? `/usr/local/bin/${binaryFor(id)}` : null,
        manual: false,
      };
    });
  },

  async agentSearchDirs(_agent: AgentId): Promise<string[]> {
    await delay();
    // The real list is the walk's, which is per-platform and per-tool. The
    // fixture answers with the shared prefixes, which is the shape of it.
    return [
      "~/.local/bin",
      "~/.npm-global/bin",
      "~/n/bin",
      "~/.volta/bin",
      "~/.local/share/mise/shims",
      "~/Library/pnpm",
      "/opt/homebrew/bin",
      "/usr/local/bin",
      "/usr/bin",
      "/bin",
      "/usr/sbin",
      "/sbin",
      // A real one, and the reason the list has to truncate: PATH entries are
      // as long as the platform feels like making them.
      "/var/run/com.apple.security.cryptexd/codex.system/bootstrap/usr/local/bin",
    ];
  },

  async verifyAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    // The real backend runs the executable it finds and reports what it printed.
    // The fixture has no filesystem to look in, so every absolute directory
    // "works" — which leaves the failure branch reachable only by typing a
    // relative path, and the rest of it to the real app.
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async setAgentDir(agent: AgentId, dir: string): Promise<AgentDirHit> {
    await delay();
    const trimmed = dir.trim();
    if (!trimmed.startsWith("/")) throw new Error(`no \`${binaryFor(agent)}\` in ${trimmed}`);
    declaredDirs[agent] = trimmed;
    return { path: `${trimmed}/${binaryFor(agent)}`, version: DEV_DECLARED_VERSION };
  },

  async clearAgentDir(agent: AgentId): Promise<void> {
    await delay();
    delete declaredDirs[agent];
  },

  async probeAgentVersions() {
    await delay(600);
    // A declared agent is one the probe now finds, so it belongs here too —
    // otherwise a directory the user just pointed at would show a tab with no
    // version, which is not what the real backend does.
    const declared = Object.keys(declaredDirs).map((agent) => ({
      agent: agent as AgentId,
      version: DEV_DECLARED_VERSION,
    }));
    return [
      { agent: "claude", version: "2.1.83 (Claude Code)" },
      { agent: "codex", version: "0.42.0" },
      { agent: "grokbuild", version: "0.9.4" },
      { agent: "claude-desktop", version: null },
      { agent: "opencode", version: "1.0.120" },
      { agent: "openclaw", version: "0.23.1" },
      { agent: "hermes", version: "0.8.2" },
      { agent: "pi", version: "0.5.12" },
      { agent: "cline", version: "3.0.62" },
      ...declared,
    ];
  },

  async listRequestLogs(page: number, pageSize: number, filter?: RequestLogFilter): Promise<RequestLogList> {
    await delay();
    const rows = matchingLogs(filter);
    const start = (page - 1) * pageSize;
    return { rows: rows.slice(start, start + pageSize).map((r) => ({ ...r })), total: rows.length };
  },

  async getRequestLog(id: number): Promise<RequestLogDetail | null> {
    await delay();
    return requestLogs.find((r) => r.id === id) ?? null;
  },

  // No filesystem in the browser, so this reports what it would have written
  // rather than pretending to write it — the toolbar's feedback then matches
  // the desktop app's without a second code path here. `_includeBodies` has
  // nothing to do without a file to shape, so it only has to be accepted.
  async exportRequestLogs(
    _path: string,
    filter?: RequestLogFilter,
    _includeBodies?: boolean,
  ): Promise<RequestLogExport> {
    await delay();
    return { rows_written: matchingLogs(filter).length, truncated: false };
  },

  async openLogFolder(): Promise<void> {
    // The browser mock has no filesystem to reveal.
    await delay();
  },

  async clearRequestLogs(): Promise<void> {
    await delay();
    requestLogs.length = 0;
  },
};
