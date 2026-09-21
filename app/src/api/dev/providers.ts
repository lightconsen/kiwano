// The provider table `pnpm dev` starts from: four rows covering the corners of
// the Apps screen's billing × limit cell, and then the matrix rows below.
//
// The loop stays next to the array it fills rather than with the `MATRIX` rows
// it reads: it runs at module-evaluation time, and an imported binding cannot
// be pushed into.

import type { Provider } from "../types";
import { protocolNote } from "./endpoints";
import { MATRIX } from "./matrix";

export const providers: Provider[] = [
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

// The matrix rows join the table above, derived from each spec the way
// `vm::build_provider_vms` derives them: the health follows from the usage the row
// carries, and everything else the spec states is copied.
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
