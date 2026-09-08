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
  Provider,
  StrategyBinding,
  StrategyKind,
  UsageAlert,
  ApiKeyEntry,
} from "./types";

function protocolNote(protocol: NewProviderInput["protocol"]): string {
  return protocol === "openai"
    ? "OpenAI 兼容"
    : protocol === "gemini"
      ? "Gemini API"
      : "Anthropic";
}

const CATALOG_TOTAL = 177;

const providers: Provider[] = [
  {
    id: "deepseek",
    name: "DeepSeek",
    logo_char: "D",
    logo_color: "#4D6BFE",
    endpoint: "api.deepseek.com",
    endpoint_note: "OpenAI 兼容",
    protocol: "openai",
    billing: "payg",
    enabled: true,
    agents: ["claude", "codex"],
    is_current: true,
    agents_note: "2 个 Agent",
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
    endpoint_note: "OpenAI 兼容",
    protocol: "openai",
    billing: "plan",
    plan_price: "¥49/月",
    enabled: true,
    agents: ["claude"],
    is_current: false,
    status_badge: "备用 #1",
    agents_note: "故障转移队列",
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
    name: "GLM (智谱)",
    logo_char: "G",
    logo_color: "#3859FF",
    endpoint: "open.bigmodel.cn",
    endpoint_note: "OpenAI 兼容",
    protocol: "openai",
    billing: "payg",
    enabled: false,
    agents: [],
    is_current: false,
    health: { state: "off", latency_ms: null, note: "未启用" },
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
    is_current: false,
    status_badge: "本地",
    agents_note: "1 个 Agent",
    health: { state: "off", latency_ms: null, note: "未运行" },
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
    tag_label: "官方",
    rating: 4.8,
    endpoint: "https://api.deepseek.com",
    icon: "deepseek",
    protocol: "openai",
    price_line: "¥4 /M入 · ¥16 /M出",
    billing: "payg",
    users: "12.4k 人在用",
    blurb: "",
    added: true,
    models: ["deepseek-chat (V3)", "deepseek-reasoner (R1)"],
  },
  {
    id: "kimi",
    name: "Kimi",
    logo_char: "K",
    logo_color: "#111111",
    logo_border: true,
    tag: "official",
    tag_label: "官方",
    rating: 4.6,
    endpoint: "https://api.moonshot.cn",
    icon: "kimi",
    protocol: "openai",
    price_line: "¥49 /月套餐",
    billing: "plan",
    users: "8.1k 人在用",
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
    tag_label: "免费额度",
    rating: 4.5,
    endpoint: "https://dashscope.aliyuncs.com/compatible-mode/v1",
    icon: "qwen",
    protocol: "openai",
    price_line: "¥2 /M入 · ¥6 /M出",
    billing: "payg",
    users: "新用户 100 万 tokens",
    blurb: "",
    added: false,
    free_offer: "新用户 100 万 tokens",
    models: ["qwen-max", "qwen-plus"],
  },
  {
    id: "groq",
    name: "Groq",
    logo_char: "G",
    logo_color: "#F55036",
    tag: "free",
    tag_label: "免费",
    rating: 4.7,
    endpoint: "https://api.groq.com/openai/v1",
    icon: "groq",
    protocol: "openai",
    price_line: "Free",
    billing: "unl",
    users: "Llama · 极速推理",
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
    tag_label: "聚合",
    rating: 4.3,
    endpoint: "https://openrouter.ai/api/v1",
    icon: "openrouter",
    protocol: "openai",
    price_line: "上游价",
    price_note: "+5.5%",
    billing: "payg",
    users: "500+ 模型一把 Key",
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
    tag_label: "本地",
    rating: 4.9,
    endpoint: "http://localhost:11434",
    icon: "ollama",
    protocol: "openai",
    price_line: "本地运行",
    billing: "unl",
    users: "完全离线",
    blurb: "",
    added: false,
    models: ["qwen3:32b", "llama3.3:70b"],
  },
  // Multi-protocol samples from the cc-switch port (anthropic / gemini fingerprints)
  {
    id: "anthropic",
    name: "Anthropic",
    logo_char: "A",
    logo_color: "#D97757",
    tag: "official",
    tag_label: "官方",
    rating: 4.9,
    endpoint: "https://api.anthropic.com",
    icon: "anthropic",
    price_line: "官方定价",
    billing: "payg",
    users: "cc-switch 收录",
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
    tag_label: "官方",
    rating: 4.5,
    endpoint: "https://generativelanguage.googleapis.com",
    icon: "gemini",
    price_line: "官方定价",
    billing: "payg",
    users: "cc-switch 收录",
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
    tag_label: "官方",
    rating: 4.4,
    endpoint: "https://open.bigmodel.cn/api/coding/paas/v4",
    icon: "zhipu",
    price_line: "官方定价",
    billing: "payg",
    users: "cc-switch 收录",
    blurb: "",
    added: false,
    models: ["glm-5", "glm-5-air"],
    protocol: "openai",
  },
  {
    id: "zhipu-glm-anthropic",
    name: "Zhipu GLM",
    logo_char: "Z",
    logo_color: "#3859FF",
    tag: "official",
    tag_label: "官方",
    rating: 4.4,
    endpoint: "https://open.bigmodel.cn/api/anthropic",
    icon: "zhipu",
    price_line: "官方定价",
    billing: "payg",
    users: "cc-switch 收录",
    blurb: "",
    added: false,
    models: ["glm-5", "glm-5-air"],
    protocol: "anthropic",
  },
  {
    id: "packycode",
    name: "PackyCode",
    logo_char: "P",
    logo_color: "#0F9D58",
    tag: "third",
    tag_label: "三方",
    rating: 4.2,
    endpoint: "https://www.packyapi.ai/v1",
    icon: "packycode",
    price_line: "按量计费",
    billing: "payg",
    users: "cc-switch 收录",
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

function bind(pid: string, priority: number): StrategyBinding {
  const p = providers.find((x) => x.id === pid)!;
  return {
    provider_id: pid,
    provider_name: p.name,
    logo_char: p.logo_char,
    logo_color: p.logo_color,
    priority,
    weight: 1,
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
    agent: "opencode",
    strategy: "single",
    config: null,
    bindings: [bind("deepseek", 0)],
  },
];

function strategyOf(agent: AgentId): AgentRoute {
  return agentRoutes.find((r) => r.agent === agent)!;
}

export const devApi: KiwanoApi = {
  async getGatewayStatus(): Promise<GatewayStatus> {
    await delay();
    return { running: true, port: 8317 };
  },

  async listProviders(filter: AgentId | "all" = "all"): Promise<Provider[]> {
    await delay();
    return providers.filter(
      (p) => filter === "all" || p.agents.includes(filter),
    );
  },

  async addProvider(input: NewProviderInput): Promise<Provider> {
    await delay();
    const p: Provider = {
      id: input.name.toLowerCase().replace(/\s+/g, "-") + "-" + ++idSeq,
      name: input.name,
      logo_char: input.name.charAt(0).toUpperCase(),
      logo_color: "#555555",
      endpoint: input.endpoint.replace(/^https?:\/\//, ""),
      endpoint_note: protocolNote(input.protocol),
      protocol: input.protocol,
      billing: input.billing,
      enabled: true,
      agents: input.agents,
      is_current: input.agents.length > 0,
      agents_note: input.agents.length
        ? `${input.agents.length} 个 Agent`
        : undefined,
      health: { state: "ok", latency_ms: null },
      usage: null,
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
    t.endpoint_note = protocolNote(input.protocol);
    t.billing = input.billing;
    t.agents = [...input.agents];
    t.is_current = t.agents.length > 0 && t.is_current;
    t.agents_note = t.agents.length ? `${t.agents.length} 个 Agent` : undefined;
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
    target.is_current = target.agents.length > 0;
    const peers = new Set(target.agents);
    for (const p of providers) {
      if (p.id !== id && p.agents.some((a) => peers.has(a))) p.is_current = false;
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

  async listCatalog(): Promise<CatalogList> {
    await delay();
    return { total: CATALOG_TOTAL, entries: catalog };
  },

  async getDashboard(window: DashboardWindow): Promise<DashboardData> {
    await delay();
    return dashboards[window];
  },

  async getSettings(): Promise<AppSettings> {
    await delay();
    return settings;
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
        "skip claude/claude-official: 缺少端点或 Key",
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
    return agentRoutes.map((r) => ({ ...r, bindings: r.bindings.map((b) => ({ ...b })) }));
  },

  async updateAgentStrategy(agent: AgentId, strategy: StrategyKind, config?: string | null): Promise<void> {
    await delay();
    strategyOf(agent).strategy = strategy;
    strategyOf(agent).config = config ?? null;
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
      today_cost: 46.2,
      hub_synced: true,
      version: "v0.1.0 · MVP",
    };
  },
};
