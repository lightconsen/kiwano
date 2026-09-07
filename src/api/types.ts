// Kiwano 前端数据契约 —— 字段与 SQLite 表（tech.md §2.3/§4.7）对齐。
// UI 只通过 src/api/client.ts 的 KiwanoApi 访问数据；组件内不得出现
// mock 字面量或 Tauri invoke 调用。

export type AgentId = "claude" | "codex" | "gemini";
export type Billing = "plan" | "payg" | "unl";

export interface AgentMeta {
  id: AgentId;
  label: string;
  chip_char: string;
  chip_color: string;
  chip_border?: boolean;
}

export const AGENTS: AgentMeta[] = [
  { id: "claude", label: "Claude Code", chip_char: "C", chip_color: "#D97757" },
  { id: "codex", label: "Codex CLI", chip_char: "C", chip_color: "#0F0F0F", chip_border: true },
  { id: "gemini", label: "Gemini CLI", chip_char: "G", chip_color: "#1E6FEB" },
];

export interface ProviderHealth {
  /** ok=健康(当前) idle=待命 off=未启用/未运行 down=故障 */
  state: "ok" | "idle" | "off";
  latency_ms: number | null;
  /** 状态列尾注：未启用 / 未运行 / …（健康时省略） */
  note?: string;
}

export interface QuotaState {
  used: number;
  limit: number;
  /** requests=按请求计（订阅） cny=按金额计（按量限额） */
  unit: "requests" | "cny";
  /** 重置日期 YYYY-MM-DD（订阅周期），按量限额为 null */
  resets_at: string | null;
}

/** 近 7 日用量聚合（usage 表聚合结果，tech.md §2.3） */
export interface UsageSummary {
  requests: number;
  input_tokens: number;
  cache_read_tokens: number;
  output_tokens: number;
  /** 估算费用（¥），unl 不计费为 null */
  cost: number | null;
  latency_ms: number | null;
  quota: QuotaState | null;
  /** 近 7 日趋势采样（未设限额的按量型 sparkline），0-14 归一化 y */
  spark: number[] | null;
}

export interface Provider {
  id: string;
  name: string;
  logo_char: string;
  logo_color: string;
  logo_border?: boolean;
  endpoint: string;
  protocol: "openai" | "anthropic";
  /** 端点副标题后半段：OpenAI 兼容 / qwen3:32b 等 */
  endpoint_note: string;
  billing: Billing;
  /** 订阅型价格行（plan）：¥49/月 */
  plan_price?: string;
  enabled: boolean;
  /** agent_bindings 派生 */
  agents: AgentId[];
  /** 列表排序后当前使用的行（kiwi 左条 + 使用中徽章） */
  is_current: boolean;
  /** 徽章文案：备用 #1 / 本地 / … */
  status_badge?: string;
  /** Agent 列的补充说明：故障转移队列 / N 个 Agent / 未绑定 */
  agents_note?: string;
  health: ProviderHealth;
  usage: UsageSummary | null;
}

export interface NewProviderInput {
  name: string;
  api_key: string;
  endpoint: string;
  protocol: "openai" | "anthropic";
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
  /** 卡片右上标签类别（货架 chip 过滤） */
  tag: "official" | "aggregate" | "free" | "local";
  tag_label: string;
  rating: number;
  /** 添加弹窗预填的请求地址 */
  endpoint: string;
  /** 价格行：¥4 /M入 · ¥16 /M出 等 */
  price_line: string;
  price_note?: string;
  billing: Billing;
  users: string;
  blurb: string;
  added: boolean;
  /** 有免费额度时的一句话 */
  free_offer?: string;
  /** 添加弹窗预置模型选项 */
  models: string[];
}

export interface CatalogList {
  /** Hub 目录总数（货架头部文案） */
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
  /** 网关分配的占位 Key；未接管为 null */
  placeholder_key: string | null;
  enabled: boolean;
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
  hub_logged_in: boolean;
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

/** CC Switch 导入结果（tech.md §4.5：仅 v3.x） */
export interface ImportReport {
  imported: number;
  skipped: number;
  detail: string[];
}

export interface KiwanoApi {
  getGatewayStatus(): Promise<GatewayStatus>;
  listProviders(filter?: AgentId | "all"): Promise<Provider[]>;
  addProvider(input: NewProviderInput): Promise<Provider>;
  /** 更新供应商（api_key 留空表示保持原 Key） */
  updateProvider(id: string, input: NewProviderInput): Promise<Provider>;
  /** 删除供应商；若为某 Agent 主选则自动提升下一个候选 */
  deleteProvider(id: string): Promise<void>;
  /** 启用 = 将该 Provider 设为其绑定 Agent 的当前路由 */
  enableProvider(id: string): Promise<void>;
  testLatency(endpoint: string): Promise<number>;
  listCatalog(): Promise<CatalogList>;
  getDashboard(window: DashboardWindow): Promise<DashboardData>;
  getSettings(): Promise<AppSettings>;
  updateSettings(patch: Partial<AppSettings>): Promise<AppSettings>;
  /** Agent 接管开关（占位 Key 的生成/删除，P1 起含配置改写） */
  setTakeover(agent: AgentId, enabled: boolean): Promise<void>;
  /** 从 CC Switch 导入配置（自动探测 ~/.cc-switch 数据源） */
  importCcSwitch(): Promise<ImportReport>;
  getFooterStats(): Promise<FooterStats>;
}
