// Infer a provider's brand icon from its endpoint URL.
// The Apps list stores only logo_char/logo_color (letter avatars); instead of
// persisting an icon per row, the brand mark is derived at render time from
// the endpoint host — the same signal the import flow has. Returns a key from
// the registry in ./index, or null when nothing matches (caller keeps the
// letter-avatar fallback).
import { hasIcon } from "./index";

/** host-substring → registry key; first match wins (specific rules first) */
const RULES: [string, string][] = [
  // Chinese labs / aggregators (specific domain rules before generic vendors)
  ["moonshot", "kimi"],
  ["bigmodel", "zhipu"],
  ["zhipu", "zhipu"],
  ["chatglm", "chatglm"],
  ["dashscope", "qwen"],
  ["bailian", "bailian"],
  ["aliyun", "alibaba"],
  ["minimax", "minimax"],
  ["volces", "doubao"],
  ["volcengine", "doubao"],
  ["doubao", "doubao"],
  ["hunyuan", "hunyuan"],
  ["tencent", "tencent"],
  ["wenxin", "wenxin"],
  ["baidubce", "baidu"],
  ["baidu", "baidu"],
  ["modelscope", "modelscope"],
  ["siliconflow", "siliconflow"],
  ["aihubmix", "aihubmix"],
  ["openrouter", "openrouter"],
  ["packycode", "packycode"],
  ["newapi", "newapi"],
  ["novita", "novita"],
  ["ppio", "ppio"],
  ["stepfun", "stepfun"],
  ["longcat", "longcat"],
  ["xiaomi", "xiaomimimo"],
  ["lingyiwanwu", "yi"],
  ["deepseek", "deepseek"],
  ["kimi", "kimi"],
  ["qwen", "qwen"],
  // International vendors
  ["generativelanguage", "gemini"],
  ["googleapis", "google"],
  ["gemini", "gemini"],
  ["aistudio", "google"],
  ["grok", "grok"],
  ["x.ai", "xai"],
  ["anthropic", "anthropic"],
  ["claude", "claude"],
  ["openai", "openai"],
  ["mistral", "mistral"],
  ["cohere", "cohere"],
  ["perplexity", "perplexity"],
  ["ollama", "ollama"],
  ["nvidia", "nvidia"],
  ["huggingface", "huggingface"],
  ["amazonaws", "aws"],
  ["azure", "azure"],
  ["cloudflare", "cloudflare"],
  ["vercel", "vercel"],
  ["github", "github"],
];

export function iconForEndpoint(endpoint: string): string | null {
  const host = endpoint
    .trim()
    .toLowerCase()
    .replace(/^https?:\/\//, "")
    .split("/")[0]
    .split(":")[0];
  if (!host) return null;
  for (const [needle, icon] of RULES) {
    if (host.includes(needle) && hasIcon(icon)) return icon;
  }
  return null;
}
