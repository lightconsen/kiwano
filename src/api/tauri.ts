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
  ProbeReport,
  Protocol,
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

  testEndpoint: (protocol: Protocol, endpoint: string, apiKey?: string) =>
    invoke<ProbeReport>("test_endpoint", { protocol, endpoint, apiKey: apiKey ?? null }),

  listModels: (protocol: Protocol, endpoint: string, apiKey: string) =>
    invoke<string[]>("list_models", { protocol, endpoint, apiKey }),

  listCatalog: () => invoke<CatalogList>("list_catalog"),

  // Invoke args must be camelCase to reach the command's snake_case params
  getDashboard: (window: DashboardWindow, providerId?: string, agentId?: string) =>
    invoke<DashboardData>("get_dashboard", {
      window,
      providerId: providerId ?? null,
      agentId: agentId ?? null,
    }),

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

  updateAgentBinding: (
    agent: AgentId,
    providerId: string,
    patch: { weight?: number; win_start?: string | null; win_end?: string | null },
  ) =>
    // Invoke args must be camelCase to match the Rust command's snake_case
    // params — snake keys here would silently arrive as None in the desktop app
    invoke<void>("update_agent_binding", {
      agent,
      providerId,
      weight: patch.weight ?? null,
      winStart: patch.win_start ?? null,
      winEnd: patch.win_end ?? null,
    }),

  addAgentBinding: (agent: AgentId, providerId: string) =>
    invoke<void>("add_agent_binding", { agent, providerId }),

  removeAgentBinding: (agent: AgentId, providerId: string) =>
    invoke<void>("remove_agent_binding", { agent, providerId }),

  applyAgentRoute: (target: AgentId, source: AgentId) =>
    invoke<void>("apply_agent_route", { target, source }),

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
