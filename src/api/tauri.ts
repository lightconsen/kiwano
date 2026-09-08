// Tauri invoke implementation of KiwanoApi — command names map one-to-one to src-tauri/src/lib.rs,
// and the returned VM fields (serde snake_case) align verbatim with src/api/types.ts.
import { invoke } from "@tauri-apps/api/core";
import type {
  AgentDetect,
  AgentId,
  AgentRoute,
  AgentVersionEntry,
  AppSettings,
  CatalogList,
  ConfigShareReport,
  DashboardData,
  DashboardWindow,
  FooterStats,
  GatewayStatus,
  HubSyncReport,
  ImportReport,
  KiwanoApi,
  NewProviderInput,
  Provider,
  RequestLogDetail,
  RequestLogFilter,
  RequestLogList,
  StrategyKind,
  UsageAlert,
  ApiKeyEntry,
} from "./types";

export const tauriApi: KiwanoApi = {
  getGatewayStatus: () => invoke<GatewayStatus>("get_gateway_status"),

  listProviders: async (filter: AgentId | "all" = "all") => {
    const list = await invoke<Provider[]>("list_providers");
    return filter === "all" ? list : list.filter((p) => p.agents.includes(filter));
  },

  addProvider: (input: NewProviderInput) => invoke<Provider>("add_provider", { input }),

  updateProvider: (id: string, input: NewProviderInput) =>
    invoke<Provider>("update_provider", { id, input }),

  deleteProvider: (id: string) => invoke<boolean>("delete_provider", { id }).then(() => undefined),

  enableProvider: (id: string) => invoke<void>("enable_provider", { id }),

  testLatency: (endpoint: string) => invoke<number>("test_latency", { endpoint }),

  listCatalog: () => invoke<CatalogList>("list_catalog"),

  getDashboard: (window: DashboardWindow) => invoke<DashboardData>("get_dashboard", { window }),

  getSettings: () => invoke<AppSettings>("get_settings"),
  syncHub: () => invoke<HubSyncReport>("sync_hub"),

  checkUsageAlerts: () => invoke<UsageAlert[]>("check_usage_alerts"),

  listApiKeys: (providerId: string) => invoke<ApiKeyEntry[]>("list_api_keys", { providerId }),

  addApiKey: (providerId: string, apiKey: string, label?: string) =>
    invoke<ApiKeyEntry>("add_api_key", { providerId, apiKey, label: label ?? null }),

  deleteApiKey: (id: number) =>
    invoke<boolean>("delete_api_key", { id }).then(() => undefined),

  updateSettings: (patch: Partial<AppSettings>) =>
    invoke<AppSettings>("update_settings", { patch }),

  setTakeover: (agent: AgentId, enabled: boolean) =>
    invoke<void>("set_agent_takeover", { agent, enabled }),

  importCcSwitch: () => invoke<ImportReport>("import_cc_switch"),

  getAgentRoutes: () => invoke<AgentRoute[]>("get_agent_routes"),

  updateAgentStrategy: (agent: AgentId, strategy: StrategyKind, config?: string | null) =>
    invoke<void>("update_agent_strategy", { agent, strategy, config: config ?? null }),

  reorderAgentBindings: (agent: AgentId, providerIds: string[]) =>
    invoke<void>("reorder_agent_bindings", { agent, providerIds }),

  exportConfig: (path: string) => invoke<number>("export_config", { path }),

  importConfig: (path: string) =>
    invoke<ConfigShareReport>("import_config", { path }),

  detectAgents: () => invoke<AgentDetect[]>("detect_agents"),

  probeAgentVersions: () => invoke<AgentVersionEntry[]>("probe_agent_versions"),

  listRequestLogs: (page: number, pageSize: number, filter?: RequestLogFilter) =>
    invoke<RequestLogList>("list_request_logs", {
      page,
      pageSize,
      agent: filter?.agent ?? null,
      providerId: filter?.provider_id ?? null,
      status: filter?.status ?? null,
    }),

  getRequestLog: (id: number) => invoke<RequestLogDetail | null>("get_request_log", { id }),

  clearRequestLogs: () => invoke<void>("clear_request_logs"),

  getFooterStats: () => invoke<FooterStats>("get_footer_stats"),
};
