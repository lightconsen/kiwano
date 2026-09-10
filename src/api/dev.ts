// Browser-dev data source — numbers match the design/index.html prototype verbatim.
// During integration src/api/client.ts switches to the Tauri invoke implementation and this file is retired.
import type {
  AgentId,
  AgentRoute,
  AppSettings,
  ConfigShareReport,
  CatalogEntry,
  CatalogList,
  DashboardData,
  DashboardWindow,
  FooterStats,
  HubSyncReport,
  GatewayStatus,
  ImportReport,
  KiwanoApi,
  NewProviderInput,
  ProbeReport,
  Protocol,
  Provider,
  RequestLogDetail,
  RequestLogFilter,
  RequestLogList,
  StrategyBinding,
  StrategyKind,
  UsageAlert,
  ApiKeyEntry,
} from "./types";
import { AGENTS } from "./types";

function protocolNote(protocol: NewProviderInput["protocol"]): string {
  return protocol === "openai"
    ? "OpenAI-compatible"
    : protocol === "gemini"
      ? "Gemini API"
      : "Anthropic";
}

// endpoint_note suffix mirrors vm::endpoint_note ("OpenAI-compatible · +Anthropic")
function endpointNote(
  protocol: NewProviderInput["protocol"],
  endpoints?: { protocol: Protocol }[],
): string {
  const tags = (endpoints ?? []).map((e) =>
    `+${e.protocol === "openai" ? "OpenAI" : e.protocol === "gemini" ? "Gemini" : "Anthropic"}`,
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

const providers: Provider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    logo_char: "D",
    logo_color: "#4D6BFE",
    endpoint: "api.deepseek.com",
    endpoint_note: "OpenAI-compatible · +Anthropic",
    protocol: "openai",
    endpoints: [{ protocol: "anthropic", endpoint: "api.deepseek.com/anthropic" }],
    billing: "payg",
    enabled: true,
    agents: ["claude", "codex"],
    serving_agents: [],
    is_current: true,
    agents_note: "2 agents",
    health: { state: "ok", latency_ms: 312 },
    usage: {
      requests: 796,
      input_tokens: 5_400_000,
      cache_read_tokens: 4_300_000,
      output_tokens: 800_000,
      cost: 28.6,
      latency_ms: 1100,
      quota: { used: 28.6, limit: 50, unit: "cny", resets_at: null },
      spark: null,
    },
  },
  {
    id: "kimi",
    name: "Kimi (Moonshot)",
    logo_char: "K",
    logo_color: "#111111",
    logo_border: true,
    endpoint: "api.moonshot.cn",
    endpoint_note: "OpenAI-compatible",
    protocol: "openai",
    billing: "plan",
    plan_price: "¥49/mo",
    enabled: true,
    agents: ["claude"],
    serving_agents: [],
    is_current: false,
    agents_note: "Failover queue",
    health: { state: "idle", latency_ms: 287 },
    usage: {
      requests: 295,
      input_tokens: 1_600_000,
      cache_read_tokens: 0,
      output_tokens: 300_000,
      cost: 10.9,
      latency_ms: 287,
      quota: { used: 295, limit: 460, unit: "requests", resets_at: "2026-09-30" },
      spark: null,
    },
  },
  {
    id: "glm",
    name: "GLM (Zhipu)",
    logo_char: "G",
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
      output_tokens: 0,
      cost: 6.7,
      latency_ms: null,
      quota: null,
      spark: [11, 9, 10, 6, 8, 4, 5],
    },
  },
  {
    id: "ollama",
    name: "Ollama",
    logo_char: "O",
    logo_color: "#1c1c1e",
    logo_border: true,
    endpoint: "localhost:11434",
    endpoint_note: "qwen3:32b",
    protocol: "openai",
    billing: "unl",
    enabled: true,
    agents: ["gemini"],
    serving_agents: [],
    is_current: false,
    status_badge: "Local",
    agents_note: "1 agent",
    health: { state: "off", latency_ms: null, note: "Not running" },
    usage: {
      requests: 31,
      input_tokens: 150_000,
      cache_read_tokens: 0,
      output_tokens: 50_000,
      cost: null,
      latency_ms: null,
      quota: null,
      spark: null,
    },
  },
];

const catalog: CatalogEntry[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    logo_char: "D",
    logo_color: "#4D6BFE",
    tag: "official",
    tag_label: "Official",
    rating: 4.8,
    endpoint: "https://api.deepseek.com",
    icon: "deepseek",
    protocol: "openai",
    price_line: "¥4 /M in · ¥16 /M out",
    billing: "payg",
    users: "12.4k users",
    blurb: "",
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
    logo_char: "K",
    logo_color: "#111111",
    logo_border: true,
    tag: "official",
    tag_label: "Official",
    rating: 4.6,
    endpoint: "https://api.moonshot.cn",
    icon: "kimi",
    protocol: "openai",
    price_line: "¥49 /mo plan",
    billing: "plan",
    users: "8.1k users",
    blurb: "",
    added: false,
    models: ["kimi-k2-0905-preview", "moonshot-v1-128k"],
  },
  {
    id: "qwen",
    name: "Qwen",
    logo_char: "Q",
    logo_color: "#615CED",
    tag: "free",
    tag_label: "Free tier",
    rating: 4.5,
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    icon: "qwen",
    protocol: "openai",
    price_line: "¥2 /M in · ¥6 /M out",
    billing: "payg",
    users: "1M tokens for new users",
    blurb: "",
    added: false,
    free_offer: "1M tokens for new users",
    models: ["qwen-max", "qwen-plus"],
  },
  {
    id: "groq",
    name: "Groq",
    logo_char: "G",
    logo_color: "#F55036",
    tag: "free",
    tag_label: "Free",
    rating: 4.7,
    endpoint: "https://api.groq.com/openai/v1",
    icon: "groq",
    protocol: "openai",
    price_line: "Free",
    billing: "unl",
    users: "Llama · ultra-fast inference",
    blurb: "",
    added: false,
    models: ["llama-3.3-70b-versatile"],
  },
  {
    id: "openrouter",
    name: "OpenRouter",
    logo_char: "O",
    logo_color: "#6467F2",
    tag: "aggregate",
    tag_label: "Aggregator",
    rating: 4.3,
    endpoint: "https://openrouter.ai/api/v1",
    icon: "openrouter",
    protocol: "openai",
    price_line: "Upstream price",
    price_note: "+5.5%",
    billing: "payg",
    users: "500+ models, one key",
    blurb: "",
    added: false,
    models: ["anthropic/claude-sonnet-4.6", "openai/gpt-5.2"],
  },
  {
    id: "ollama",
    name: "Ollama",
    logo_char: "O",
    logo_color: "#1c1c1e",
    logo_border: true,
    tag: "local",
    tag_label: "Local",
    rating: 4.9,
    endpoint: "http://localhost:11434",
    icon: "ollama",
    protocol: "openai",
    price_line: "Runs locally",
    billing: "unl",
    users: "Fully offline",
    blurb: "",
    // Mirrors the backend: added derives from Ollama existing in the provider list
    added: true,
    models: ["qwen3:32b", "llama3.3:70b"],
  },
  // Multi-protocol samples from the cc-switch port (anthropic / gemini fingerprints)
  {
    id: "anthropic",
    name: "Anthropic",
    logo_char: "A",
    logo_color: "#D97757",
    tag: "official",
    tag_label: "Official",
    rating: 4.9,
    endpoint: "https://api.anthropic.com",
    icon: "anthropic",
    price_line: "Official pricing",
    billing: "payg",
    users: "Listed on cc-switch",
    blurb: "",
    added: false,
    models: ["claude-sonnet-5", "claude-opus-5", "claude-haiku-4-5"],
    protocol: "anthropic",
  },
  {
    id: "google-ai-studio",
    name: "Google AI Studio",
    logo_char: "G",
    logo_color: "#1E6FEB",
    tag: "official",
    tag_label: "Official",
    rating: 4.5,
    endpoint: "https://generativelanguage.googleapis.com",
    icon: "gemini",
    price_line: "Official pricing",
    billing: "payg",
    users: "Listed on cc-switch",
    blurb: "",
    added: false,
    models: ["gemini-3.6-flash", "gemini-3.6-pro"],
    protocol: "gemini",
  },
  {
    id: "zhipu-glm",
    name: "Zhipu GLM",
    logo_char: "Z",
    logo_color: "#3859FF",
    tag: "official",
    tag_label: "Official",
    rating: 4.4,
    endpoint: "https://open.bigmodel.cn/api/coding/paas/v4",
    icon: "zhipu",
    price_line: "Official pricing",
    billing: "payg",
    users: "Listed on cc-switch",
    blurb: "",
    added: false,
    models: ["glm-5", "glm-5-air"],
    protocol: "openai",
    // Merged from the former zhipu-glm-anthropic sibling entry
    endpoints: [
      { protocol: "anthropic", endpoint: "https://open.bigmodel.cn/api/anthropic", models: ["glm-5", "glm-5-air"] },
    ],
  },
  {
    id: "packycode",
    name: "PackyCode",
    logo_char: "P",
    logo_color: "#0F9D58",
    tag: "third",
    tag_label: "Third-party",
    rating: 4.2,
    endpoint: "https://www.packyapi.ai/v1",
    icon: "packycode",
    price_line: "Pay-as-you-go",
    billing: "payg",
    users: "Listed on cc-switch",
    blurb: "",
    added: false,
    models: [],
    protocol: "openai",
  },
];

const dashboard7d: DashboardData = {
  window: "7d",
  requests: 1296,
  requests_delta_pct: 12,
  input_tokens: 7_500_000,
  cache_read_tokens: 4_300_000,
  output_tokens: 1_300_000,
  cost: 46.2,
  latency_ms: 1200,
  latency_delta_pct: 9,
  trend: [
    { date: "09-01", requests: 118, tokens: 900_000 },
    { date: "09-02", requests: 132, tokens: 1_000_000 },
    { date: "09-03", requests: 190, tokens: 1_300_000 },
    { date: "09-04", requests: 176, tokens: 1_200_000 },
    { date: "09-05", requests: 228, tokens: 1_500_000 },
    { date: "09-06", requests: 186, tokens: 1_300_000 },
    { date: "09-07", requests: 254, tokens: 1_600_000 },
  ],
  by_provider: [
    { name: "DeepSeek", color: "#4D6BFE", pct: 62, cost: 28.6 },
    { name: "Kimi", color: "#555555", pct: 23, cost: 10.9 },
    { name: "GLM", color: "#3859FF", pct: 15, cost: 6.7 },
  ],
  by_agent: [
    { agent: "claude", label: "Claude Code", requests: 943, tokens: "6.2M", cost: 33.4 },
    { agent: "codex", label: "Codex", requests: 264, tokens: "1.9M", cost: 9.8 },
    { agent: "gemini", label: "Gemini CLI", requests: 77, tokens: "0.5M", cost: 3.0 },
    { agent: "opencode", label: "OpenCode", requests: 12, tokens: "0.1M", cost: 0.4 },
  ],
  // getDashboard always derives the real option lists from by_provider/by_agent
  filter_providers: [],
  filter_agents: [],
};

const dashboards: Record<DashboardWindow, DashboardData> = {
  today: {
    ...dashboard7d,
    window: "today",
    requests: 288,
    requests_delta_pct: 4,
    input_tokens: 1_600_000,
    cache_read_tokens: 900_000,
    output_tokens: 300_000,
    cost: 10.8,
    latency_ms: 1100,
    latency_delta_pct: 3,
    trend: [
      { date: "02:00", requests: 18, tokens: 120_000 },
      { date: "05:00", requests: 24, tokens: 150_000 },
      { date: "08:00", requests: 56, tokens: 360_000 },
      { date: "11:00", requests: 62, tokens: 400_000 },
      { date: "14:00", requests: 48, tokens: 310_000 },
      { date: "17:00", requests: 44, tokens: 260_000 },
      { date: "20:00", requests: 32, tokens: 200_000 },
    ],
    by_provider: [
      { name: "DeepSeek", color: "#4D6BFE", pct: 78, cost: 8.4 },
      { name: "Kimi", color: "#555555", pct: 22, cost: 2.4 },
    ],
    by_agent: [
      { agent: "claude", label: "Claude Code", requests: 201, tokens: "1.5M", cost: 8.1 },
      { agent: "codex", label: "Codex", requests: 66, tokens: "0.4M", cost: 2.3 },
      { agent: "gemini", label: "Gemini CLI", requests: 17, tokens: "0.1M", cost: 0.4 },
      { agent: "opencode", label: "OpenCode", requests: 4, tokens: "0.02M", cost: 0.1 },
    ],
  },
  "7d": dashboard7d,
  "30d": {
    ...dashboard7d,
    window: "30d",
    requests: 5861,
    requests_delta_pct: 23,
    input_tokens: 33_100_000,
    cache_read_tokens: 19_400_000,
    output_tokens: 5_900_000,
    cost: 203.5,
    latency_ms: 1300,
    latency_delta_pct: 5,
    trend: [
      { date: "08-09", requests: 142, tokens: 980_000 },
      { date: "08-14", requests: 176, tokens: 1_150_000 },
      { date: "08-19", requests: 165, tokens: 1_080_000 },
      { date: "08-24", requests: 221, tokens: 1_420_000 },
      { date: "08-29", requests: 246, tokens: 1_560_000 },
      { date: "09-03", requests: 232, tokens: 1_490_000 },
      { date: "09-07", requests: 254, tokens: 1_600_000 },
    ],
  },
};

const settings: AppSettings = {
  language: "zh-CN",
  theme: "dark",
  autostart: true,
  close_to_tray: true,
  gateway_listen: "127.0.0.1:8317",
  takeovers: [
    { agent: "claude", label: "Claude Code", placeholder_key: "kw-ag-claude-a1b2", enabled: true, additive: false },
    { agent: "codex", label: "Codex", placeholder_key: "kw-ag-codex-c3d4", enabled: true, additive: false },
    { agent: "gemini", label: "Gemini CLI", placeholder_key: null, enabled: false, additive: false },
    { agent: "grokbuild", label: "Grok Build", placeholder_key: null, enabled: false, additive: false },
    { agent: "claude-desktop", label: "Claude Desktop", placeholder_key: null, enabled: false, additive: false },
    { agent: "opencode", label: "OpenCode", placeholder_key: null, enabled: false, additive: true },
    { agent: "openclaw", label: "OpenClaw", placeholder_key: null, enabled: false, additive: true },
    { agent: "hermes", label: "Hermes", placeholder_key: null, enabled: false, additive: true },
    { agent: "pi", label: "Pi", placeholder_key: null, enabled: false, additive: true },
  ],
  auto_failover: true,
  request_logs: true,
  log_retention_days: 30,
  telemetry: false,
  cost_alert: true,
  hub_logged_in: false,
  hub_url: "https://hub.kiwano.app/catalog.json",
};

let idSeq = 100;

// Dev storage for rotating keys (spec §4.1 P1 multi-key rotation)
type DevApiKeyRow = ApiKeyEntry & { provider_id: string };
const devApiKeys: DevApiKeyRow[] = [];

async function delay(ms = 120) {
  return new Promise((r) => setTimeout(r, ms));
}

// ── Agent routing strategies (tech.md §4.7) ──

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
    bindings: [bind("deepseek", 0), bind("kimi", 1)],
  },
  {
    agent: "codex",
    strategy: "single",
    config: null,
    bindings: [bind("deepseek", 0)],
  },
  {
    agent: "gemini",
    strategy: "quota",
    config: '{"limit":500,"unit":"requests"}',
    bindings: [bind("ollama", 0)],
  },
  {
    // Kimi heads opencode (globally "In use") while standing by in claude's
    // queue — exercises the agent tabs' local vs global "In use" badge.
    agent: "opencode",
    strategy: "single",
    config: null,
    bindings: [bind("kimi", 0)],
  },
  {
    // Overnight window on the standby exercises the timewindow wraparound path.
    agent: "grokbuild",
    strategy: "timewindow",
    config: null,
    bindings: [bind("deepseek", 0), bind("ollama", 1, ["22:00", "06:00"])],
  },
];

function strategyOf(agent: AgentId): AgentRoute {
  // The backend lazily creates a Single-strategy route on first takeover
  // (vm::set_agent_takeover); the mock does the same so agents enabled from
  // Settings without a pre-seeded route behave identically.
  let route = agentRoutes.find((r) => r.agent === agent);
  if (!route) {
    route = { agent, strategy: "single", config: null, bindings: [] };
    agentRoutes.push(route);
  }
  return route;
}

// Mirror vm::build_provider_vms: the "In use" badge marks the provider(s) that
// would serve a request issued right now under each agent's strategy. Quota
// stays on the head here — the mock usage has no per-day totals.
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
  for (const r of agentRoutes) {
    const enabled = r.bindings.filter((b) => b.enabled);
    if (enabled.length === 0) continue;
    const head = enabled[0].provider_id;
    if (r.strategy === "roundrobin") {
      enabled.forEach((b) => out.add(`${r.agent}/${b.provider_id}`));
    } else if (r.strategy === "timewindow") {
      const hit = enabled.find(
        (b) => b.win_start && b.win_end && inWin(nowMin, b.win_start, b.win_end),
      );
      out.add(`${r.agent}/${hit?.provider_id ?? head}`);
    } else {
      // single / failover / quota: the head (breaker state is gateway runtime)
      out.add(`${r.agent}/${head}`);
    }
  }
  return out;
}

// vm::build_provider_vms derives ProviderVm.agents from the bindings; the mock
// fixtures' static arrays drift from agentRoutes, so derive them the same way.
function agentsOf(pid: string): string[] {
  return agentRoutes.filter((r) => r.bindings.some((b) => b.provider_id === pid)).map((r) => r.agent);
}

// Mirror vm::build_provider_vms's failover-queue classification: a non-head
// candidate is a queue member ("Failover queue" note — no badge; standby
// badges read as a contradiction next to "In use") unless its strategy
// rotates through everyone (roundrobin) or it serves its own time window.
function standbyFlags(): { backups: Set<string> } {
  const backups = new Set<string>();
  for (const r of agentRoutes) {
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
    latency_ms: 1180,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json","x-kw-session":"s-1"}',
    response_headers: '{"content-type":"application/json"}',
    request_size: 214,
    response_size: 331,
    truncated: false,
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
    latency_ms: 940,
    first_token_ms: 210,
    request_headers: '{"content-type":"application/json"}',
    response_headers: '{"content-type":"text/event-stream"}',
    request_size: 96,
    response_size: 1240,
    truncated: false,
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
    latency_ms: null,
    first_token_ms: null,
    request_headers: '{"content-type":"application/json"}',
    response_headers: null,
    request_size: 74,
    response_size: 0,
    truncated: false,
    request_body: '{"model":"gpt-5.2","messages":[]}',
    response_body: null,
  },
];

export const devApi: KiwanoApi = {
  async getGatewayStatus(): Promise<GatewayStatus> {
    await delay();
    return { running: true, port: 8317 };
  },

  async listProviders(filter: AgentId | "all" = "all"): Promise<Provider[]> {
    await delay();
    const serving = servingNow();
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
      logo_char: input.name.charAt(0).toUpperCase(),
      logo_color: "#555555",
      endpoint: input.endpoint.replace(/^https?:\/\//, ""),
      endpoint_note: endpointNote(input.protocol, endpoints),
      protocol: input.protocol,
      endpoints,
      billing: input.billing,
      enabled: true,
      agents: input.agents,
      // Optimistic, mirrors vm::add_provider; listProviders recomputes
      serving_agents: [],
      is_current: input.agents.length > 0,
      agents_note: input.agents.length
        ? `${input.agents.length} agent(s)`
        : undefined,
      health: { state: "ok", latency_ms: null },
      usage: null,
      advanced: input.advanced,
    };
    providers.unshift(p);
    return p;
  },

  async updateProvider(id: string, input: NewProviderInput): Promise<Provider> {
    await delay();
    const t = providers.find((p) => p.id === id);
    if (!t) throw new Error(`provider ${id} not found`);
    t.name = input.name.trim();
    t.endpoint = input.endpoint.replace(/^https?:\/\//, "");
    t.protocol = input.protocol;
    t.endpoints = input.endpoints?.map((e) => ({
      protocol: e.protocol,
      endpoint: e.endpoint.replace(/^https?:\/\//, ""),
    }));
    t.endpoint_note = endpointNote(input.protocol, t.endpoints);
    t.billing = input.billing;
    t.agents = [...input.agents];
    t.serving_agents = t.agents.filter((a) => servingNow().has(`${a}/${t.id}`));
    t.is_current = t.serving_agents.length > 0;
    t.agents_note = t.agents.length ? `${t.agents.length} agent(s)` : undefined;
    // Absent `advanced` keeps existing values (mirrors vm::update_provider).
    if (input.advanced !== undefined) t.advanced = input.advanced;
    return t;
  },

  async deleteProvider(id: string): Promise<void> {
    await delay();
    const i = providers.findIndex((p) => p.id === id);
    if (i >= 0) providers.splice(i, 1);
  },

  async enableProvider(id: string): Promise<void> {
    await delay();
    const target = providers.find((p) => p.id === id);
    if (!target) return;
    target.enabled = true;
    target.serving_agents = target.agents.filter((a) => servingNow().has(`${a}/${target.id}`));
    target.is_current = target.serving_agents.length > 0;
    const peers = new Set(target.agents);
    for (const p of providers) {
      if (p.id !== id && p.agents.some((a) => peers.has(a))) {
        p.serving_agents = p.serving_agents.filter((a) => !peers.has(a));
        p.is_current = p.serving_agents.length > 0;
      }
    }
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
  async testEndpoint(_protocol: Protocol, endpoint: string): Promise<ProbeReport> {
    await delay(500);
    const latency_ms = 200 + Math.floor(Math.random() * 300);
    if (!endpoint.trim() || endpoint.includes("invalid")) {
      return { verdict: "unreachable", status: null, latency_ms, detail: "connection failed (dev)" };
    }
    return { verdict: "ok", status: 200, latency_ms, detail: "12 models listed (dev)" };
  },

  // Dev model list: resolves through the catalog entry matching the endpoint
  // (primary or per-protocol), falling back to a canned OpenAI-style list
  async listModels(_protocol: Protocol, endpoint: string, apiKey: string): Promise<string[]> {
    await delay(600);
    if (!apiKey.trim()) throw new Error("auth failed — check the API key");
    const ep = endpoint.trim();
    const hit = catalog.find(
      (e) =>
        e.endpoint === ep || (e.endpoints ?? []).some((x) => x.endpoint === ep),
    );
    if (hit) {
      return Array.from(new Set([...hit.models, ...(hit.endpoints ?? []).flatMap((x) => x.models ?? [])]));
    }
    return ["gpt-5.2", "gpt-5.2-mini", "o4-mini", "text-embedding-3-large"];
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
    const filter_providers = base.by_provider.map((p) => ({
      id: providers.find((x) => x.name === p.name)?.id ?? p.name,
      label: p.name,
    }));
    const filter_agents = base.by_agent.map((a) => ({ id: a.agent, label: a.label }));
    const data: DashboardData = { ...base, filter_providers, filter_agents };
    // The mock narrows the breakdowns only (headline stats stay fixture-wide)
    if (providerId) {
      const name = providers.find((x) => x.id === providerId)?.name ?? providerId;
      data.by_provider = base.by_provider
        .filter((p) => p.name === name)
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

  async setTakeover(agent: AgentId, enabled: boolean): Promise<void> {
    await delay();
    const t = settings.takeovers.find((x) => x.agent === agent);
    if (!t) return;
    if (enabled) {
      const suffix = Math.random().toString(16).slice(2, 6);
      t.placeholder_key = `kw-ag-${agent}-${suffix}`;
      t.enabled = true;
    } else {
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
    return {
      fetched: 42,
      synced_at: new Date().toISOString(),
      hub_url: settings.hub_url,
    };
  },

  async checkUsageAlerts(): Promise<UsageAlert[]> {
    await delay();
    return [];
  },

  async listApiKeys(providerId: string): Promise<ApiKeyEntry[]> {
    await delay();
    return devApiKeys.filter((k) => k.provider_id === providerId);
  },

  async addApiKey(providerId: string, apiKey: string, label?: string): Promise<ApiKeyEntry> {
    await delay();
    const row: ApiKeyEntry & { provider_id: string } = {
      id: ++idSeq,
      provider_id: providerId,
      api_key: apiKey.trim(),
      label: label?.trim() || undefined,
      enabled: true,
      created_at: new Date().toISOString(),
    };
    devApiKeys.push(row);
    return row;
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
      version: "v0.1.0",
    };
  },

  async detectAgents() {
    await delay(120);
    // Dev fixture: everything installed so every agent segment stays visible.
    return AGENTS.map((a) => ({
      agent: a.id,
      installed: true,
      path: `/usr/local/bin/${a.id === "grokbuild" ? "grok" : a.id === "claude-desktop" ? "claude" : a.id}`,
    }));
  },

  async probeAgentVersions() {
    await delay(600);
    return [
      { agent: "claude", version: "2.1.83 (Claude Code)" },
      { agent: "codex", version: "0.42.0" },
      { agent: "gemini", version: "0.13.0" },
      { agent: "grokbuild", version: "0.9.4" },
      { agent: "claude-desktop", version: null },
      { agent: "opencode", version: "1.0.120" },
      { agent: "openclaw", version: "0.23.1" },
      { agent: "hermes", version: "0.8.2" },
      { agent: "pi", version: "0.5.12" },
    ];
  },

  async listRequestLogs(page: number, pageSize: number, filter?: RequestLogFilter): Promise<RequestLogList> {
    await delay();
    const rows = requestLogs.filter(
      (r) =>
        (!filter?.agent || r.agent === filter.agent) &&
        (!filter?.provider_id || r.provider_id === filter.provider_id) &&
        (!filter?.status ||
          (filter.status === "error" && r.status_code >= 400) ||
          (filter.status === "ok" && r.status_code < 400)),
    );
    const start = (page - 1) * pageSize;
    return { rows: rows.slice(start, start + pageSize).map((r) => ({ ...r })), total: rows.length };
  },

  async getRequestLog(id: number): Promise<RequestLogDetail | null> {
    await delay();
    return requestLogs.find((r) => r.id === id) ?? null;
  },

  async clearRequestLogs(): Promise<void> {
    await delay();
    requestLogs.length = 0;
  },
};
