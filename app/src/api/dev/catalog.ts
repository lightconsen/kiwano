// The bundled catalog the shelf and the add dialog read, and the stored-key
// guard the probe answers with. Mirrors `crates/core/src/vm/catalog.rs`.

import type { CatalogEntry } from "../types";
import { endpointKey } from "./endpoints";
import { providers } from "./providers";

/** The mock's stand-in for `vm::stored_key_for`: the provider's key, but only
    for an endpoint that provider already answers on — the same guard the real one
    applies, so a probe aimed somewhere else gets no key. */
export function storedKeyFor(providerId: string | undefined, endpoint: string): string | null {
  if (!providerId) return null;
  const p = providers.find((x) => x.id === providerId);
  if (!p) return null;
  const known = [p.endpoint, ...(p.endpoints ?? []).map((e) => e.endpoint)].map(endpointKey);
  return known.includes(endpointKey(endpoint)) ? "sk-stored" : null;
}

export const catalog: CatalogEntry[] = [
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
