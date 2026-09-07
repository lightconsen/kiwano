// KiwanoApi 的 Tauri invoke 实现 —— 命令名与 src-tauri/src/lib.rs 一一对应，
// 返回的 VM 字段（serde snake_case）与 src/api/types.ts 逐字对齐。
import { invoke } from "@tauri-apps/api/core";
import type {
  AgentId,
  AppSettings,
  CatalogList,
  DashboardData,
  DashboardWindow,
  FooterStats,
  GatewayStatus,
  HubSyncReport,
  ImportReport,
  KiwanoApi,
  NewProviderInput,
  Provider,
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

  updateSettings: (patch: Partial<AppSettings>) =>
    invoke<AppSettings>("update_settings", { patch }),

  setTakeover: (agent: AgentId, enabled: boolean) =>
    invoke<void>("set_agent_takeover", { agent, enabled }),

  importCcSwitch: () => invoke<ImportReport>("import_cc_switch"),

  getFooterStats: () => invoke<FooterStats>("get_footer_stats"),
};
